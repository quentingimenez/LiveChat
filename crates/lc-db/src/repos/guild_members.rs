//! Repository `guild_members` (profils et rôles par guilde).

use chrono::DateTime;
use lc_domain::entities::GuildMember;
use lc_domain::enums::GuildRole;
use lc_domain::ids::{GuildId, GuildMemberId, OverlayClientId};
use sqlx::SqlitePool;

use crate::convert::{decode, from_ms, from_opt_ms, to_ms, to_opt_ms};
use crate::error::DbResult;

const TABLE: &str = "guild_members";

/// Colonnes de `guild_members`, préfixées pour les jointures.
const MEMBER_COLUMNS: &str = "m.id, m.guild_id, m.display_name, m.avatar, m.role, m.created_at, \
                              m.removed_at";

/// Nombre d'admins actifs de la guilde du membre ciblé par l'instruction.
const ACTIVE_ADMIN_COUNT: &str = "(SELECT COUNT(*) FROM guild_members AS other \
                                  WHERE other.guild_id = guild_members.guild_id \
                                    AND other.role = 'ADMIN' \
                                    AND other.removed_at IS NULL)";

/// Ligne SQLite de `guild_members`.
#[derive(Debug, sqlx::FromRow)]
struct GuildMemberRow {
    id: String,
    guild_id: String,
    display_name: String,
    avatar: Option<String>,
    role: String,
    created_at: i64,
    removed_at: Option<i64>,
}

impl GuildMemberRow {
    fn into_entity(self) -> DbResult<GuildMember> {
        Ok(GuildMember {
            id: decode(TABLE, "id", &self.id, GuildMemberId::parse)?,
            guild_id: decode(TABLE, "guild_id", &self.guild_id, GuildId::parse)?,
            display_name: self.display_name,
            avatar: self.avatar,
            role: decode(TABLE, "role", &self.role, GuildRole::from_db_str)?,
            created_at: from_ms(TABLE, "created_at", self.created_at)?,
            removed_at: from_opt_ms(TABLE, "removed_at", self.removed_at)?,
        })
    }
}

/// Résultat d'une modification protégée par la règle du dernier admin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberChange {
    /// La modification a été appliquée.
    Applied,
    /// Membre inconnu ou déjà retiré.
    NotFound,
    /// Refusé : le membre est le dernier admin actif de sa guilde.
    LastAdmin,
}

/// Repository des membres de guilde.
#[derive(Clone, Debug)]
pub struct GuildMembersRepository {
    pool: SqlitePool,
}

impl GuildMembersRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Enregistre un membre.
    pub async fn create(&self, member: &GuildMember) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO guild_members ( \
                id, guild_id, display_name, avatar, role, created_at, removed_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(member.id.to_canonical())
        .bind(member.guild_id.as_str())
        .bind(member.display_name.as_str())
        .bind(member.avatar.as_deref())
        .bind(member.role.as_db_str())
        .bind(to_ms(member.created_at))
        .bind(to_opt_ms(member.removed_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Récupère un membre par identifiant (actif ou retiré).
    pub async fn get(&self, id: GuildMemberId) -> DbResult<Option<GuildMember>> {
        let row = sqlx::query_as::<_, GuildMemberRow>("SELECT * FROM guild_members WHERE id = ?")
            .bind(id.to_canonical())
            .fetch_optional(&self.pool)
            .await?;
        row.map(GuildMemberRow::into_entity).transpose()
    }

    /// Membre **actif** propriétaire d'un overlay.
    ///
    /// `None` si l'overlay n'a pas de membre ou si celui-ci a été retiré.
    pub async fn get_for_overlay_client(
        &self,
        client_id: OverlayClientId,
    ) -> DbResult<Option<GuildMember>> {
        let row = sqlx::query_as::<_, GuildMemberRow>(&format!(
            "SELECT {MEMBER_COLUMNS} FROM guild_members AS m \
             JOIN overlay_clients AS c ON c.member_id = m.id \
             WHERE c.id = ? AND m.removed_at IS NULL"
        ))
        .bind(client_id.to_canonical())
        .fetch_optional(&self.pool)
        .await?;
        row.map(GuildMemberRow::into_entity).transpose()
    }

    /// Liste les membres actifs d'une guilde, du plus ancien au plus récent.
    pub async fn list_by_guild(&self, guild_id: &GuildId) -> DbResult<Vec<GuildMember>> {
        let rows = sqlx::query_as::<_, GuildMemberRow>(
            "SELECT * FROM guild_members WHERE guild_id = ? AND removed_at IS NULL \
             ORDER BY created_at ASC, id ASC",
        )
        .bind(guild_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(GuildMemberRow::into_entity).collect()
    }

    /// Nombre d'admins actifs d'une guilde.
    pub async fn count_active_admins(&self, guild_id: &GuildId) -> DbResult<i64> {
        let count = sqlx::query_scalar(
            "SELECT COUNT(*) FROM guild_members \
             WHERE guild_id = ? AND role = 'ADMIN' AND removed_at IS NULL",
        )
        .bind(guild_id.as_str())
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    /// Met à jour le profil (pseudo, avatar) d'un membre actif.
    pub async fn update_profile(
        &self,
        id: GuildMemberId,
        display_name: &str,
        avatar: Option<&str>,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            "UPDATE guild_members SET display_name = ?, avatar = ? \
             WHERE id = ? AND removed_at IS NULL",
        )
        .bind(display_name)
        .bind(avatar)
        .bind(id.to_canonical())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Change le rôle d'un membre actif.
    ///
    /// La règle « une guilde garde au moins un admin » est évaluée dans la même
    /// instruction que la mise à jour : deux rétrogradations concurrentes ne
    /// peuvent pas laisser la guilde sans admin.
    pub async fn set_role(&self, id: GuildMemberId, role: GuildRole) -> DbResult<MemberChange> {
        let result = sqlx::query(&format!(
            "UPDATE guild_members SET role = ? \
             WHERE id = ? AND removed_at IS NULL \
               AND (role <> 'ADMIN' OR ? = 'ADMIN' OR {ACTIVE_ADMIN_COUNT} > 1)"
        ))
        .bind(role.as_db_str())
        .bind(id.to_canonical())
        .bind(role.as_db_str())
        .execute(&self.pool)
        .await?;
        self.classify(id, result.rows_affected()).await
    }

    /// Retire un membre (exclusion ou départ), sous la même règle du dernier
    /// admin que [`GuildMembersRepository::set_role`].
    ///
    /// Les overlays du membre ne sont pas révoqués ici : voir
    /// `OverlayClientsRepository::revoke_by_member`.
    pub async fn remove(
        &self,
        id: GuildMemberId,
        removed_at: DateTime<chrono::Utc>,
    ) -> DbResult<MemberChange> {
        let result = sqlx::query(&format!(
            "UPDATE guild_members SET removed_at = ? \
             WHERE id = ? AND removed_at IS NULL \
               AND (role <> 'ADMIN' OR {ACTIVE_ADMIN_COUNT} > 1)"
        ))
        .bind(to_ms(removed_at))
        .bind(id.to_canonical())
        .execute(&self.pool)
        .await?;
        self.classify(id, result.rows_affected()).await
    }

    /// Supprime tous les membres d'une guilde (purge). Retourne le nombre de
    /// lignes supprimées.
    ///
    /// Les overlays de la guilde doivent être supprimés avant (clé étrangère
    /// `overlay_clients.member_id`).
    pub async fn delete_by_guild(&self, guild_id: &GuildId) -> DbResult<u64> {
        let result = sqlx::query("DELETE FROM guild_members WHERE guild_id = ?")
            .bind(guild_id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Distingue « introuvable » de « dernier admin » quand aucune ligne n'a
    /// été modifiée.
    async fn classify(&self, id: GuildMemberId, rows_affected: u64) -> DbResult<MemberChange> {
        if rows_affected > 0 {
            return Ok(MemberChange::Applied);
        }
        let active = self.get(id).await?.is_some_and(|member| member.is_active());
        Ok(if active {
            MemberChange::LastAdmin
        } else {
            MemberChange::NotFound
        })
    }
}
