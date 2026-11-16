//! In-memory cache of the configurable rights (`role_rights` table).
//!
//! The table is read once at startup ([`RightsStore::reload`]) and reloaded
//! after every successful `PUT /admin/api/rights`. Every enforcement point reads
//! this cache instead of comparing roles directly: overlay handlers, the Discord
//! bot (through [`lc_discord::rights::RightsSource`]) and `POST /ingest`.
//!
//! A lock is never held across an `.await`: checks take a snapshot of the table
//! through [`RightsStore::snapshot`] or the tiny read-lock helpers. The store is
//! shared by cloning (`Arc<RwLock<…>>`), so every `AppState` clone sees the same
//! rights.

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use lc_db::{Db, DbError};
use lc_domain::rights::{Action, RightsRole, RightsTable, Surface};

/// `server_flags` key of the `discordAdminIsAdmin` switch.
pub const DISCORD_ADMIN_IS_ADMIN_FLAG: &str = "discordAdminIsAdmin";

/// Shared rights cache held by [`crate::app::AppState`].
#[derive(Clone, Debug, Default)]
pub struct RightsStore {
    table: Arc<RwLock<RightsTable>>,
}

impl RightsStore {
    /// Store with the built-in defaults (no database configured, or before the
    /// startup load).
    pub fn new() -> Self {
        Self::default()
    }

    /// Store holding an explicit table (tests, startup failure fallback).
    pub fn with_table(table: RightsTable) -> Self {
        Self {
            table: Arc::new(RwLock::new(table)),
        }
    }

    /// Clone of the current table, for a consistent multi-cell computation.
    pub fn snapshot(&self) -> RightsTable {
        self.read().clone()
    }

    /// `true` when `role` may perform `action` on `surface`.
    pub fn allowed(&self, surface: Surface, action: Action, role: RightsRole) -> bool {
        self.read().allowed(surface, action, role)
    }

    /// `true` when a Discord server administrator counts as `admin`.
    pub fn discord_admin_is_admin(&self) -> bool {
        self.read().discord_admin_is_admin()
    }

    /// Replaces the cached table with the one stored in `db`.
    ///
    /// Unknown `surface`/`action`/`role` values are skipped with a warning: the
    /// admin route validates them before writing, so they can only come from a
    /// hand-edited database.
    pub async fn reload(&self, db: &Db) -> Result<(), DbError> {
        let table = load_table(db).await?;
        *self.write() = table;
        Ok(())
    }

    /// Replaces the cached table without touching the database (tests).
    pub fn replace(&self, table: RightsTable) {
        *self.write() = table;
    }

    fn read(&self) -> RwLockReadGuard<'_, RightsTable> {
        self.table
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, RightsTable> {
        self.table
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Loads the persisted overrides and the `discordAdminIsAdmin` flag.
pub async fn load_table(db: &Db) -> Result<RightsTable, DbError> {
    let rows = sqlx::query_as::<_, (String, String, String, i64)>(
        "SELECT surface, action, role, allowed FROM role_rights",
    )
    .fetch_all(db.pool())
    .await
    .map_err(DbError::from)?;

    let mut table = RightsTable::new();
    for (surface, action, role, allowed) in rows {
        let Some(surface) = Surface::from_id(surface.trim()) else {
            tracing::warn!(%surface, "role_rights row ignored: unknown surface");
            continue;
        };
        let Some(action) = Action::from_id(action.trim()) else {
            tracing::warn!(%action, "role_rights row ignored: unknown action");
            continue;
        };
        let Some(role) = RightsRole::from_id(role.trim()) else {
            tracing::warn!(%role, "role_rights row ignored: unknown role");
            continue;
        };
        table.set_override(surface, action, role, allowed != 0);
    }

    if let Some(value) = server_flag(db, DISCORD_ADMIN_IS_ADMIN_FLAG).await? {
        match value.trim() {
            "true" | "1" => table.set_discord_admin_is_admin(true),
            "false" | "0" => table.set_discord_admin_is_admin(false),
            other => tracing::warn!(
                flag = DISCORD_ADMIN_IS_ADMIN_FLAG,
                value = other,
                "server_flags value ignored (expected a boolean)"
            ),
        }
    }

    Ok(table)
}

/// One cell accepted by the admin route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RightsCell {
    /// Surface configured.
    pub surface: Surface,
    /// Action configured.
    pub action: Action,
    /// Role configured.
    pub role: RightsRole,
    /// New value.
    pub allowed: bool,
}

/// Applies the provided cells in one transaction (upsert).
///
/// The admin console sends **only the cells the operator changed**
/// (`rightsPayload` in `apps/lc-admin-web/src/rights.ts`), so omitted cells keep
/// their stored override. An override explicitly set back to its built-in
/// default stays stored but is indistinguishable from the default, which is what
/// the console's « Valeurs par défaut » button produces.
///
/// `discord_admin_is_admin` is only written when `Some`, so a `PUT` that sends
/// only cells keeps the current switch.
pub async fn save_table(
    db: &Db,
    cells: &[RightsCell],
    discord_admin_is_admin: Option<bool>,
) -> Result<(), DbError> {
    let mut tx = db.pool().begin().await.map_err(DbError::Sqlx)?;

    for cell in cells {
        sqlx::query(
            "INSERT INTO role_rights (surface, action, role, allowed) VALUES (?, ?, ?, ?) \
             ON CONFLICT (surface, action, role) DO UPDATE SET allowed = excluded.allowed",
        )
        .bind(cell.surface.id())
        .bind(cell.action.id())
        .bind(cell.role.id())
        .bind(if cell.allowed { 1_i64 } else { 0_i64 })
        .execute(&mut *tx)
        .await
        .map_err(DbError::Sqlx)?;
    }

    if let Some(value) = discord_admin_is_admin {
        sqlx::query(
            "INSERT INTO server_flags (key, value) VALUES (?, ?) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(DISCORD_ADMIN_IS_ADMIN_FLAG)
        .bind(if value { "true" } else { "false" })
        .execute(&mut *tx)
        .await
        .map_err(DbError::Sqlx)?;
    }

    tx.commit().await.map_err(DbError::Sqlx)
}

/// Reads one `server_flags` row.
async fn server_flag(db: &Db, key: &str) -> Result<Option<String>, DbError> {
    sqlx::query_scalar::<_, String>("SELECT value FROM server_flags WHERE key = ?")
        .bind(key)
        .fetch_optional(db.pool())
        .await
        .map_err(DbError::from)
}

impl lc_discord::rights::RightsSource for RightsStore {
    fn table(&self) -> RightsTable {
        self.snapshot()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use lc_db::{Db, DbConfig};
    use lc_domain::rights::{Action, RightsRole, Surface};

    use super::{load_table, save_table, RightsCell, RightsStore};

    #[tokio::test]
    async fn defaults_are_used_without_any_row() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let store = RightsStore::new();
        store.reload(&db).await.unwrap();

        assert!(store.discord_admin_is_admin());
        assert!(!store.allowed(Surface::Overlay, Action::TriggerPlayback, RightsRole::Guest));
        assert!(store.allowed(
            Surface::Overlay,
            Action::TriggerPlayback,
            RightsRole::Member
        ));
    }

    #[tokio::test]
    async fn rows_and_flag_are_loaded() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        save_table(
            &db,
            &[RightsCell {
                surface: Surface::Discord,
                action: Action::Stop,
                role: RightsRole::Guest,
                allowed: true,
            }],
            Some(false),
        )
        .await
        .unwrap();

        let table = load_table(&db).await.unwrap();
        assert!(!table.discord_admin_is_admin());
        assert!(table.allowed(Surface::Discord, Action::Stop, RightsRole::Guest));
        assert!(!table.allowed(Surface::Discord, Action::Stop, RightsRole::Member));
    }

    #[tokio::test]
    async fn save_upserts_cells_and_keeps_the_flag_when_absent() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let first = RightsCell {
            surface: Surface::Overlay,
            action: Action::TriggerPlayback,
            role: RightsRole::Guest,
            allowed: true,
        };
        save_table(&db, &[first], Some(false)).await.unwrap();
        assert!(!load_table(&db).await.unwrap().discord_admin_is_admin());

        // A later PUT only carries the cells the operator changed: the first
        // override must survive.
        let second = RightsCell {
            surface: Surface::Extension,
            action: Action::Send,
            role: RightsRole::Guest,
            allowed: true,
        };
        save_table(&db, &[second], None).await.unwrap();
        let table = load_table(&db).await.unwrap();
        assert!(!table.discord_admin_is_admin());
        assert!(table.allowed(Surface::Extension, Action::Send, RightsRole::Guest));
        assert!(table.allowed(Surface::Overlay, Action::TriggerPlayback, RightsRole::Guest));
    }

    #[tokio::test]
    async fn saving_the_same_cell_twice_updates_it() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let cell = |allowed| RightsCell {
            surface: Surface::Overlay,
            action: Action::AddMedia,
            role: RightsRole::Guest,
            allowed,
        };
        save_table(&db, &[cell(true)], None).await.unwrap();
        assert!(load_table(&db).await.unwrap().allowed(
            Surface::Overlay,
            Action::AddMedia,
            RightsRole::Guest
        ));
        save_table(&db, &[cell(false)], None).await.unwrap();
        assert!(!load_table(&db).await.unwrap().allowed(
            Surface::Overlay,
            Action::AddMedia,
            RightsRole::Guest
        ));
    }

    #[tokio::test]
    async fn corrupt_rows_are_ignored() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        // The table CHECK constraints freeze surface and role values; an
        // unknown action can still be written by a hand-edited database.
        sqlx::query(
            "INSERT INTO role_rights (surface, action, role, allowed) \
             VALUES ('overlay', 'inconnue', 'guest', 1)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        let table = load_table(&db).await.unwrap();
        assert_eq!(table.override_count(), 0);
    }

    #[test]
    fn a_store_built_from_a_table_and_its_discord_view_use_that_table() {
        let mut table = lc_domain::rights::RightsTable::new();
        table.set_override(Surface::Discord, Action::Tts, RightsRole::Guest, true);
        let store = RightsStore::with_table(table);
        assert!(store.allowed(Surface::Discord, Action::Tts, RightsRole::Guest));

        // The Discord bot reads the rights through `RightsSource`.
        let seen = lc_discord::rights::RightsSource::table(&store);
        assert!(seen.allowed(Surface::Discord, Action::Tts, RightsRole::Guest));
        assert_eq!(seen.override_count(), 1);
    }
}
