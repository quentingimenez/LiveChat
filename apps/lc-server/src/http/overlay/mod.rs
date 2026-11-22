//! Routes HTTP `/overlay/*` — parité REST avec l'ancien serveur Node (Phase 3b).
//!
//! ## Périmètre
//!
//! Ce module remplace `server/src/components/overlay/overlayRoutes.ts` et
//! `server/src/services/overlayAuth.ts` (voir `docs/rust/server-overlay.md`).
//! Il expose :
//!
//! | Méthode | Chemin | Authentification |
//! |---------|--------|------------------|
//! | `POST` | `/overlay/pair/consume` | publique (code d'appairage) |
//! | `GET` | `/overlay/media/{assetId}` | URL signée HMAC (`exp`, `sig`, `clientId`) |
//! | `GET` | `/overlay/media-board/items` | `Authorization: Bearer` |
//! | `POST` | `/overlay/media-board/items` | `Authorization: Bearer` — **écriture refusée en `invite_read_only`** |
//! | `POST` | `/overlay/media-board/uploads` | `Authorization: Bearer` — **writes refused in `invite_read_only`** |
//! | `PUT` | `/overlay/media-board/uploads/{uploadId}/parts?offset=` | `Authorization: Bearer` — **writes refused in `invite_read_only`** |
//! | `POST` | `/overlay/media-board/uploads/{uploadId}/complete` | `Authorization: Bearer` — **writes refused in `invite_read_only`** |
//! | `DELETE` | `/overlay/media-board/uploads/{uploadId}` | `Authorization: Bearer` |
//! | `POST` | `/overlay/media-board/items/{itemId}/edit` | `Authorization: Bearer` — **writes refused in `invite_read_only`** |
//! | `POST` | `/overlay/media-board/items/{itemId}/cover` | `Authorization: Bearer` — **writes refused in `invite_read_only`** |
//! | `POST` | `/overlay/media-board/import` | `Authorization: Bearer` (target) + `X-LiveChat-Source-Token` (source) — **writes refused in `invite_read_only`** |
//! | `GET` | `/overlay/media-board/ingestions` | `Authorization: Bearer` |
//! | `GET` | `/overlay/media-board/media/{assetId}` | URL signée (`clientId`, `exp`, `sig`) |
//! | `PATCH` | `/overlay/media-board/items/{itemId}` | `Authorization: Bearer` — **écriture refusée en `invite_read_only`** |
//! | `DELETE` | `/overlay/media-board/items/{itemId}` | `Authorization: Bearer` — **écriture refusée en `invite_read_only`** |
//! | `POST` | `/overlay/say` | `Authorization: Bearer` — speech synthesis (`/dire`), **refused in `invite_read_only`** |
//!
//! `GET /overlay/config` **n'existe pas** en protocole v2 (supprimé, cf.
//! `docs/rust/protocol-v2.md` §8) : la route tombe sur le `fallback` de
//! `crate::app`, qui répond `404 {"error":"request_error"}`.
//!
//! ## Sessions en lecture seule
//!
//! Un jeton dont `overlay_clients.session_mode = INVITE_READ_ONLY` peut lire la
//! board (`GET items`, `GET ingestions`, `GET media`) mais **pas** la modifier :
//! `POST`, `PATCH` et `DELETE` répondent `403 {"error":"read_only_session"}` (garde
//! `media_board::ensure_writable`, AUD-V-003). La garde s'applique après
//! l'authentification Bearer ; le flux binaire du board applique la même règle
//! de visibilité à partir du rôle du client signé.
//!
//! ## Conventions
//!
//! - AUCUN handler ne lit l'horloge système : tout instant vient de
//!   `state.clock` (tests déterministes, cf. `docs/rust/server-core.md` §9).
//! - Les erreurs applicatives sont déjà au format JSON minimal du legacy
//!   (`{"error": "<code>"}`, éventuellement enrichi de `message`/`code`) : le
//!   middleware `normalize_errors` laisse intact un corps JSON.
//! - Les accès base passent par `state.db` (`Option<Db>`) ; une base absente
//!   répond `503 {"error":"database_unavailable"}` plutôt que de paniquer.
//! - Aucun `unwrap`/`expect`/`panic` (lints workspace).
//!
//! ## Handoff Phase 4b / Phase 8
//!
//! - Les URLs signées sont générées par le hub WebSocket (Phase 4b) via
//!   [`crate::media_url`] ; la signature et sa vérification sont partagées.
//! - Les balises `src` (`<video>`, `<img>`) du board ne peuvent pas porter
//!   d'en-tête : chaque item de `GET /overlay/media-board/items` expose donc
//!   `mediaUrl` / `mediaUrlExpiresAt`, une URL signée une heure pour le client
//!   demandeur. Aucun jeton n'est accepté en query string.

// Les helpers (`db`, `resolve_media_file`, ...) et certains handlers renvoient
// `Result<_, Response>` : `Response` est le type d'erreur natif d'axum et le
// boxing systématique dégraderait la lisibilité sans gain mesurable sur ces
// chemins I/O. Le lint `result_large_err` est donc neutralisé à l'échelle du
// module (cf. décision dans `docs/rust/server-overlay.md` §2).
#![allow(clippy::result_large_err)]

use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lc_db::MemberChange;
use lc_domain::rights::{Action, RightsTable, Surface};
use lc_domain::{
    GuildId, GuildMember, GuildMemberId, GuildRole, OverlayClient, OverlayClientId, Permission,
};
use lc_protocol::{close_code, close_reason};
use serde_json::Value;
use time::UtcDateTime;
use tracing::{error, info, warn};

use crate::app::AppState;

pub(crate) mod admin;
mod audit;
mod auth;
pub(crate) mod board_clear;
mod board_extras;
mod danger;
mod device_id;
pub(crate) mod devices;
pub(crate) mod discord_link;
pub(crate) mod guild;
mod ingest_code;
mod media;
pub(crate) mod media_board;
pub(crate) mod media_cover;
pub(crate) mod media_edit;
mod media_import;
mod media_ingestion;
mod media_upload;
pub use media_upload::{UploadSessions, MAX_PART_BYTES};
mod pair;
mod personal_devices;
mod profile_discord;
pub(crate) mod queue;
mod say;

pub(crate) use audit::record_playback_stop;
pub(crate) use auth::AuthedOverlayClient;
pub use media::sign_media_url;

/// Caller identity shared by the overlay handlers and the admin console.
///
/// The admin console uses an operator actor: no member is attached, the role
/// is [`GuildRole::Admin`] and every guild permission check passes. Overlay
/// handlers build the actor from their authenticated client, so the extracted
/// logic keeps acting for the calling member.
#[derive(Clone, Debug)]
pub(crate) struct ActorInfo {
    /// Overlay client owning the action (device, signing key, ingestion slot).
    pub(crate) client: OverlayClient,
    /// Guild member attached to the overlay client, if any.
    pub(crate) member: Option<GuildMember>,
    /// `true` when the server operator drives the action (admin console).
    pub(crate) operator: bool,
    /// Effective rights captured for this request (read from the server cache).
    pub(crate) rights: RightsTable,
}

impl ActorInfo {
    /// Actor of an authenticated overlay request.
    pub(crate) fn from_auth(state: &AppState, auth: &AuthedOverlayClient) -> Self {
        Self {
            client: auth.client.clone(),
            member: auth.member.clone(),
            operator: false,
            rights: state.rights.snapshot(),
        }
    }

    /// Operator actor of the admin console.
    ///
    /// It carries an unsaved overlay client so the media ingestion tasks have a
    /// client id and a guild; no socket is ever attached to that id.
    pub(crate) fn operator(state: &AppState, guild_id: &GuildId) -> Self {
        Self::operator_with_client(
            state,
            OverlayClient {
                id: OverlayClientId::new_v4(),
                guild_id: guild_id.clone(),
                label: "Admin console".to_owned(),
                token_hash: String::new(),
                session_mode: lc_domain::OverlaySessionMode::Normal,
                default_author_name: Some("Admin console".to_owned()),
                default_author_image: None,
                created_by_discord_user_id: None,
                device_hash: None,
                last_seen_at: None,
                created_at: state.clock.now(),
                revoked_at: None,
            },
        )
    }

    /// Operator actor backed by an existing overlay client (media URL signing).
    pub(crate) fn operator_with_client(state: &AppState, client: OverlayClient) -> Self {
        Self {
            client,
            member: None,
            operator: true,
            rights: state.rights.snapshot(),
        }
    }

    /// Guild of the actor.
    pub(crate) fn guild_id(&self) -> &GuildId {
        &self.client.guild_id
    }

    /// Attached member id, if any.
    pub(crate) fn member_id(&self) -> Option<GuildMemberId> {
        self.member.as_ref().map(|member| member.id)
    }

    /// Effective guild role (an operator is always an admin).
    pub(crate) fn role(&self) -> GuildRole {
        if self.operator {
            return GuildRole::Admin;
        }
        match &self.member {
            Some(member) => member.role,
            None => match self.client.session_mode {
                lc_domain::OverlaySessionMode::InviteReadOnly => GuildRole::Guest,
                lc_domain::OverlaySessionMode::Normal => GuildRole::Member,
            },
        }
    }

    /// `true` when the actor may perform `permission`.
    pub(crate) fn can(&self, permission: Permission) -> bool {
        if self.operator {
            return true;
        }
        let action = Action::from_overlay_permission(permission);
        self.rights
            .allowed(Surface::Overlay, action, self.role().into())
    }

    /// `true` for a read-only overlay session (never for an operator).
    pub(crate) fn is_read_only(&self) -> bool {
        !self.operator && self.role() == GuildRole::Guest
    }

    /// Same rule as the overlay board: admins edit everything, members their own
    /// media, and an item with no recorded author stays editable by members.
    pub(crate) fn can_edit_item(&self, creator_member_id: Option<&str>) -> bool {
        if self.operator {
            return true;
        }
        if self.can(Permission::EditAnyMedia) {
            return true;
        }
        if !self.can(Permission::EditOwnMedia) {
            return false;
        }
        match (self.member_id(), creator_member_id) {
            (Some(member_id), Some(creator)) => member_id.to_canonical() == creator,
            _ => true,
        }
    }

    /// Authenticated-client view used by the detached ingestion tasks.
    pub(crate) fn to_authed(&self, at: UtcDateTime) -> AuthedOverlayClient {
        AuthedOverlayClient {
            client: self.client.clone(),
            member: self.member.clone(),
            authenticated_at: at,
        }
    }
}

#[cfg(test)]
mod ban_tests;
#[cfg(test)]
mod board_extras_tests;
#[cfg(test)]
mod danger_tests;
#[cfg(test)]
mod device_admin_tests;
#[cfg(test)]
mod guild_admin_tests;
#[cfg(test)]
mod guild_tests;
#[cfg(test)]
mod ingestion_tests;
#[cfg(test)]
mod media_cover_tests;
#[cfg(test)]
mod media_edit_tests;
#[cfg(test)]
mod media_import_tests;
#[cfg(test)]
mod media_upload_tests;
#[cfg(test)]
mod overlay_audit_tests;
#[cfg(test)]
mod personal_devices_tests;
#[cfg(test)]
mod profile_discord_tests;
#[cfg(test)]
mod profile_tests;
#[cfg(test)]
mod say_tests;
#[cfg(test)]
mod tests;

/// Sous-routeur monté par `crate::http::mod` puis `crate::app::router`.
///
/// Sous-routeur `/overlay`, monté tel quel (fusion sans préfixe) par
/// [`crate::app::router`] : il porte donc lui-même le préfixe `/overlay`,
/// comme [`crate::http::ingest::router`].
///
/// Les routes du board média exigent l'extracteur [`auth::AuthedOverlayClient`]
/// (Bearer uniquement, exécuté avant le handler) ; l'appairage et le média
/// signé ont leurs propres contrôles (code d'appairage / signature HMAC).
pub fn router() -> Router<AppState> {
    let authed = Router::new()
        .route(
            "/overlay/media-board/items",
            get(media_board::list_items).post(media_board::create_item),
        )
        .route(
            "/overlay/media-board/uploads",
            post(media_upload::create_upload),
        )
        .route(
            "/overlay/media-board/uploads/{upload_id}",
            axum::routing::delete(media_upload::cancel_upload),
        )
        .route(
            "/overlay/media-board/uploads/{upload_id}/parts",
            axum::routing::put(media_upload::put_part),
        )
        .route(
            "/overlay/media-board/uploads/{upload_id}/complete",
            post(media_upload::complete_upload),
        )
        .route(
            "/overlay/media-board/import",
            post(media_import::import_items),
        )
        .route(
            "/overlay/media-board/ingestions",
            get(media_board::list_ingestions),
        )
        .route(
            "/overlay/media-board/items/{item_id}",
            axum::routing::patch(media_board::update_item).delete(media_board::delete_item),
        )
        .route(
            "/overlay/media-board/items/{item_id}/edit",
            post(media_edit::edit_item)
                .layer(DefaultBodyLimit::max(media_edit::MAX_EDIT_BODY_BYTES)),
        )
        .route(
            "/overlay/media-board/items/{item_id}/cover",
            post(media_cover::set_cover),
        )
        .route(
            "/overlay/media-board/items/{item_id}/favorite",
            axum::routing::put(board_extras::put_favorite),
        )
        .route(
            "/overlay/media-board/items/{item_id}/artwork",
            axum::routing::put(board_extras::put_artwork).delete(board_extras::delete_artwork),
        )
        .route(
            "/overlay/media-board/media/{asset_id}",
            get(media_board::get_media),
        )
        .route("/overlay/guild", get(guild::get_guild))
        .route(
            "/overlay/profile",
            axum::routing::patch(guild::update_profile),
        )
        .route(
            "/overlay/profile/discord-link",
            post(profile_discord::link_discord),
        )
        .route("/overlay/device-code", post(guild::create_device_code))
        .route(
            "/overlay/ingest-code",
            post(ingest_code::create_ingest_code),
        )
        .route(
            "/overlay/ingest-client",
            post(ingest_code::create_ingest_client),
        )
        .route(
            "/overlay/devices",
            get(personal_devices::list_personal_devices),
        )
        .route(
            "/overlay/devices/overlay/{client_id}",
            axum::routing::delete(personal_devices::revoke_personal_overlay),
        )
        .route(
            "/overlay/devices/ingest/{client_id}",
            axum::routing::delete(personal_devices::revoke_personal_ingest),
        )
        .route("/overlay/leave", post(guild::leave))
        .route(
            "/overlay/queue",
            get(queue::get_queue).delete(queue::clear_queue),
        )
        .route("/overlay/queue/history", get(queue::get_history))
        .route("/overlay/queue/reorder", post(queue::reorder_queue))
        .route(
            "/overlay/queue/{job_id}",
            axum::routing::delete(queue::remove_job),
        )
        .route("/overlay/say", post(say::say));

    // Administrative routes of the guild: one audit layer journals each
    // successful mutation with the token's guild and member (see `audit`).
    let admin = Router::new()
        .route("/overlay/admin/members", get(admin::list_members))
        .route(
            "/overlay/admin/members/{member_id}",
            axum::routing::patch(admin::update_member).delete(admin::remove_member_handler),
        )
        .route(
            "/overlay/admin/members/{member_id}/ban",
            post(admin::ban_member_handler),
        )
        .route("/overlay/admin/bans", get(admin::list_bans))
        .route(
            "/overlay/admin/bans/{ban_id}",
            axum::routing::delete(admin::unban_handler),
        )
        .route("/overlay/admin/devices", get(devices::list_devices))
        .route(
            "/overlay/admin/devices/{client_id}",
            axum::routing::patch(devices::update_device).delete(devices::revoke_device),
        )
        .route(
            "/overlay/admin/devices/{client_id}/guild-settings",
            axum::routing::patch(devices::update_guild_settings),
        )
        .route(
            "/overlay/admin/device-codes",
            post(devices::create_device_code),
        )
        .route(
            "/overlay/admin/invites",
            get(admin::list_invites).post(admin::create_invite_handler),
        )
        .route(
            "/overlay/admin/invites/{code}/revoke",
            post(admin::revoke_invite),
        )
        .route(
            "/overlay/admin/pairing-codes",
            get(admin::list_pairing_codes),
        )
        .route(
            "/overlay/admin/pairing-codes/expired",
            axum::routing::delete(admin::purge_pairing_codes),
        )
        .route(
            "/overlay/admin/pairing-codes/{code}/revoke",
            post(admin::revoke_pairing_code),
        )
        .route(
            "/overlay/admin/settings",
            axum::routing::patch(admin::update_settings),
        )
        .route(
            "/overlay/admin/media-board/clear",
            post(admin::clear_media_board),
        )
        .route("/overlay/admin/purge", post(danger::purge))
        .route("/overlay/admin/discord-leave", post(danger::discord_leave))
        .route("/overlay/admin/audit", get(admin::list_audit))
        .route(
            "/overlay/admin/discord-link-code",
            post(discord_link::create_link_code),
        )
        .route(
            "/overlay/admin/discord-link",
            axum::routing::delete(discord_link::unlink),
        )
        .layer(axum::middleware::from_fn(audit::overlay_admin_audit));

    Router::new()
        .route("/overlay/pair/consume", post(pair::consume_pairing))
        .route("/overlay/media/{asset_id}", get(media::get_media))
        .merge(authed)
        .merge(admin)
}

/// Instant courant, toujours lu depuis l'horloge injectée.
fn now(state: &AppState) -> UtcDateTime {
    state.clock.now()
}

/// Accès à la base ou réponse `503` normalisée.
fn db(state: &AppState) -> Result<&lc_db::Db, Response> {
    state.db.as_ref().ok_or_else(database_unavailable)
}

/// Réponse JSON applicative.
fn json_response(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// Erreur JSON minimale du legacy (`{"error": code}`).
fn error(status: StatusCode, code: &'static str) -> Response {
    crate::http::error::json_error(status, code)
}

/// `401 {"error":"unauthorized"}`.
fn unauthorized() -> Response {
    error(StatusCode::UNAUTHORIZED, "unauthorized")
}

/// `500 {"error":"internal_error"}`.
fn internal_error() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
}

/// `503 {"error":"database_unavailable"}` (serveur démarré sans `DATABASE_URL`).
fn database_unavailable() -> Response {
    error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable")
}

/// Journalise une erreur `lc-db` et renvoie `500 internal_error`.
fn db_failure(operation: &'static str, db_error: &lc_db::DbError) -> Response {
    error!(operation, error = %db_error, "échec d'accès base sur une route overlay");
    internal_error()
}

/// Complète au démarrage les exclusions qu'un crash a laissées inachevées.
///
/// Un ban peut être commité sans que le membre ait été retiré (crash entre les
/// deux écritures). Le démarrage rejoue l'exclusion pour ces bans ; il ne
/// supprime **jamais** le ban : une décision de sécurité ne doit pas disparaître
/// au redémarrage. Un échec (dernier admin, erreur base) conserve le ban et le
/// journalise ; l'authentification overlay refuse de toute façon un membre
/// banni, même si sa ligne `guild_members` est encore active (fail-closed).
pub(crate) async fn reconcile_member_bans(state: &AppState) {
    let Some(db) = state.db.as_ref() else {
        return;
    };
    let bans = match db.guild_bans().list_bans_of_active_members().await {
        Ok(bans) => bans,
        Err(error) => {
            warn!(error = %error, "réconciliation des bans impossible");
            return;
        }
    };
    for ban in bans {
        let Some(member_id) = ban.member_id else {
            continue;
        };
        let member = match db.guild_members().get(member_id).await {
            Ok(Some(member)) if member.is_active() => member,
            Ok(_) => continue,
            Err(error) => {
                warn!(
                    error = %error,
                    member_id = %member_id.to_canonical(),
                    "réconciliation d'un ban : membre illisible, ban conservé"
                );
                continue;
            }
        };
        match guild::remove_member(
            state,
            db,
            &member,
            now(state),
            close_code::REVOKED,
            close_reason::MEMBER_BANNED,
        )
        .await
        {
            Ok(MemberChange::Applied) => {
                info!(
                    member_id = %member_id.to_canonical(),
                    "exclusion d'un membre banni complétée au démarrage ; ban conservé"
                );
            }
            Ok(MemberChange::LastAdmin) => {
                warn!(
                    member_id = %member_id.to_canonical(),
                    "ban incohérent d'un dernier admin : exclusion non appliquée, ban conservé"
                );
            }
            Ok(MemberChange::NotFound) => {
                warn!(
                    member_id = %member_id.to_canonical(),
                    "membre disparu pendant la réconciliation : ban conservé"
                );
            }
            Err(response) => {
                warn!(
                    member_id = %member_id.to_canonical(),
                    status = %response.status(),
                    "échec de l'exclusion au démarrage : ban conservé"
                );
            }
        }
    }
}

/// Vrai si ce client relève d'un membre banni ou porte un appareil banni.
///
/// Fail-closed, et partagé par l'authentification HTTP et WebSocket : un crash
/// ou une course d'appairage peut laisser un client actif derrière un membre
/// déjà retiré (`remove_member` révoque les clients avant la ligne membre, mais
/// les états hérités existent). La ligne membre est donc relue **même
/// retirée** ; un membre retiré rend le client invalide même sans ban, et son
/// `member_id`/compte Discord lié est confronté à `guild_bans`, puis l'empreinte
/// matérielle du client à `guild_ban_devices`.
pub(crate) async fn client_banned(db: &lc_db::Db, client: &OverlayClient) -> lc_db::DbResult<bool> {
    let member = db
        .guild_members()
        .get_any_for_overlay_client(client.id)
        .await?;
    if let Some(member) = member.as_ref() {
        if !member.is_active() {
            return Ok(true);
        }
        if db
            .guild_bans()
            .is_member_banned(
                &client.guild_id,
                member.id,
                member.discord_user_id.as_deref(),
            )
            .await?
        {
            return Ok(true);
        }
    }
    if let Some(device_hash) = client.device_hash.as_deref() {
        if db
            .guild_ban_devices()
            .is_device_banned(&client.guild_id, device_hash)
            .await?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Conflit d'unicité quel que soit le chemin de conversion de l'erreur.
///
/// `lc-db` convertit parfois `SQLITE_CONSTRAINT` en `DomainError::Conflict`
/// (`map_sqlx_error`), parfois laisse le `DbError::Sqlx` brut : les deux formes
/// doivent être reconnues, sinon les branches de récupération sont mortes
/// (AUD-A2-010).
fn is_conflict(db_error: &lc_db::DbError) -> bool {
    db_error.is_unique_violation()
        || matches!(
            db_error,
            lc_db::DbError::Domain(lc_domain::DomainError::Conflict(_))
        )
}

/// Tests ciblés AUD-V-003 : garde `read_only_session` sur les écritures board.
///
/// Ces tests montent le routeur complet (`crate::app::router`) comme
/// `tests.rs`, mais vivent ici pour rester dans le périmètre de la correction ;
/// ils couvrent l'authentification Bearer, le flux média signé et les deux modes
/// de session (`normal` / `invite_read_only`).
#[cfg(test)]
mod session_mode_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::Path;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{header, Method, Request, StatusCode};
    use axum::Router;
    use lc_db::{Db, DbConfig};
    use lc_domain::{
        FixedClock, GuildId, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId, MediaKind,
        MediaStatus, OverlayClient, OverlayClientId, OverlaySessionMode,
    };
    use serde_json::{json, Value};
    use tempfile::TempDir;
    use time::UtcDateTime;
    use tower::ServiceExt as _;

    use crate::app::AppState;
    use crate::config::Config;
    use crate::shutdown::Shutdown;

    use super::auth;

    /// Jeton (64 hex) d'un client en session normale.
    const NORMAL_TOKEN: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    /// Jeton (64 hex) d'un client en session `invite_read_only`.
    const READ_ONLY_TOKEN: &str =
        "2222222222222222222222222222222222222222222222222222222222222222";
    /// Guilde unique des tests de session.
    const GUILD: &str = "123456789012345678";

    fn fixed_now() -> UtcDateTime {
        time::macros::utc_datetime!(2026-11-10 12:00:00)
    }

    fn guild() -> GuildId {
        GuildId::parse(GUILD).unwrap()
    }

    fn test_config(media_dir: &Path) -> Config {
        let pairs = vec![
            ("API_URL".to_owned(), "http://127.0.0.1:8099".to_owned()),
            (
                "MEDIA_URL_SECRET".to_owned(),
                "secret-test-session-mode".to_owned(),
            ),
            (
                "MEDIA_STORAGE_DIR".to_owned(),
                media_dir.to_string_lossy().into_owned(),
            ),
            ("MEDIA_BOARD_MAX_TOTAL_MB".to_owned(), "64".to_owned()),
        ];
        Config::from_pairs(pairs).unwrap()
    }

    async fn harness(media_dir: &Path) -> (Router, Db) {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let clock: Arc<dyn lc_domain::Clock> = Arc::new(FixedClock::new(fixed_now()));
        let state = AppState::new(
            Arc::new(test_config(media_dir)),
            Some(db.clone()),
            Shutdown::detached(),
        )
        .with_clock(clock);
        (crate::app::router(state), db)
    }

    /// URL de média du board signée pour ce client (`board:` namespace).
    fn signed_board_uri(asset: &MediaAsset, client: &OverlayClient, exp: u64) -> String {
        let asset_key = asset.id.to_canonical();
        let client_key = client.id.to_canonical();
        let signature = crate::media_url::sign(
            b"secret-test-session-mode",
            &format!("board:{asset_key}"),
            &client_key,
            exp,
        )
        .expect("signature board");
        format!(
            "/overlay/media-board/media/{asset_key}?clientId={client_key}&exp={exp}&sig={signature}"
        )
    }

    async fn insert_client(db: &Db, mode: OverlaySessionMode, token: &str) -> OverlayClient {
        let now = fixed_now();
        let client = OverlayClient {
            id: OverlayClientId::new_v4(),
            guild_id: guild(),
            label: "poste-test".to_owned(),
            token_hash: auth::hash_token(token),
            session_mode: mode,
            default_author_name: None,
            default_author_image: None,
            created_by_discord_user_id: None,
            device_hash: None,
            last_seen_at: None,
            created_at: now,
            revoked_at: None,
        };
        db.overlay_clients().create(&client).await.unwrap();
        client
    }

    async fn insert_asset(db: &Db, media_dir: &Path) -> MediaAsset {
        let file_path = media_dir.join("media-test.bin");
        tokio::fs::write(&file_path, b"media-test-body")
            .await
            .unwrap();
        let now = fixed_now();
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: "hash-test-session-mode".to_owned(),
            source_url: "https://example.com/media.mp4".to_owned(),
            source_title: None,
            kind: MediaKind::Video,
            mime: "video/mp4".to_owned(),
            duration_sec: Some(30),
            width: Some(1280),
            height: Some(720),
            is_vertical: false,
            storage_path: file_path.to_string_lossy().into_owned(),
            size_bytes: 15,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: now,
            expires_at: now + time::Duration::days(1),
            created_at: now,
        };
        db.media_assets().insert(&asset).await.unwrap();
        asset
    }

    async fn insert_item(db: &Db, asset: &MediaAsset, title: &str) -> MediaBoardItem {
        let item = MediaBoardItem {
            id: MediaBoardItemId::new_v4(),
            guild_id: guild(),
            media_asset_id: asset.id,
            title: Some(title.to_owned()),
            message: None,
            created_by_discord_user_id: None,
            created_by_name: Some("test".to_owned()),
            created_at: fixed_now(),
            artwork_asset_id: None,
            artwork_on_overlay: false,
            display_duration_sec: None,
            display_scale_percent: None,
        };
        db.media_board().create(&item).await.unwrap();
        item
    }

    fn request(
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        match body {
            Some(body) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                builder
                    .body(Body::from(serde_json::to_vec(body).unwrap()))
                    .unwrap()
            }
            None => builder.body(Body::empty()).unwrap(),
        }
    }

    async fn send_status(app: &Router, request: Request<Body>) -> StatusCode {
        app.clone().oneshot(request).await.unwrap().status()
    }

    async fn send_json(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, body)
    }

    /// Session invitée : lecture autorisée, toutes les écritures refusées.
    #[tokio::test]
    async fn read_only_allows_reads_and_blocks_mutations() {
        let dir = TempDir::new().unwrap();
        let (app, db) = harness(dir.path()).await;
        let client = insert_client(&db, OverlaySessionMode::InviteReadOnly, READ_ONLY_TOKEN).await;
        let asset = insert_asset(&db, dir.path()).await;
        let item = insert_item(&db, &asset, "titre initial").await;

        // GET items et GET media restent autorisés.
        let (status, body) = send_json(
            &app,
            request(
                Method::GET,
                "/overlay/media-board/items",
                Some(READ_ONLY_TOKEN),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
        let exp = u64::try_from(fixed_now().unix_timestamp()).unwrap() + 300;
        let media_uri = signed_board_uri(&asset, &client, exp);
        let status = send_status(&app, request(Method::GET, &media_uri, None, None)).await;
        assert_eq!(status, StatusCode::OK);

        // POST : refusé avant même la validation de la charge utile.
        let (status, body) = send_json(
            &app,
            request(
                Method::POST,
                "/overlay/media-board/items",
                Some(READ_ONLY_TOKEN),
                Some(&json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "read_only_session");

        // PATCH : refusé.
        let item_uri = format!("/overlay/media-board/items/{}", item.id.to_canonical());
        let (status, body) = send_json(
            &app,
            request(
                Method::PATCH,
                &item_uri,
                Some(READ_ONLY_TOKEN),
                Some(&json!({"title": "hack"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "read_only_session");

        // DELETE : refusé.
        let (status, body) = send_json(
            &app,
            request(Method::DELETE, &item_uri, Some(READ_ONLY_TOKEN), None),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "read_only_session");

        // L'item est intact en base (aucune mutation partielle).
        let stored = db
            .media_board()
            .get(item.id)
            .await
            .unwrap()
            .expect("item conservé");
        assert_eq!(stored.title.as_deref(), Some("titre initial"));
    }

    /// Session normale : comportement legacy inchangé.
    #[tokio::test]
    async fn normal_session_mutations_stay_allowed() {
        let dir = TempDir::new().unwrap();
        let (app, db) = harness(dir.path()).await;
        insert_client(&db, OverlaySessionMode::Normal, NORMAL_TOKEN).await;
        let asset = insert_asset(&db, dir.path()).await;
        let item = insert_item(&db, &asset, "titre initial").await;

        // POST sans url : on atteint bien la validation legacy `400`, pas la garde.
        let (status, body) = send_json(
            &app,
            request(
                Method::POST,
                "/overlay/media-board/items",
                Some(NORMAL_TOKEN),
                Some(&json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_payload");

        // PATCH autorisé.
        let item_uri = format!("/overlay/media-board/items/{}", item.id.to_canonical());
        let (status, body) = send_json(
            &app,
            request(
                Method::PATCH,
                &item_uri,
                Some(NORMAL_TOKEN),
                Some(&json!({"title": "titre modifié"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["item"]["title"], "titre modifié");

        // DELETE autorisé.
        let (status, body) = send_json(
            &app,
            request(Method::DELETE, &item_uri, Some(NORMAL_TOKEN), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["deleted"], true);
    }

    /// Le repli `?token=` a disparu : un jeton en query n'authentifie plus rien.
    #[tokio::test]
    async fn query_token_no_longer_authenticates_mutations() {
        let dir = TempDir::new().unwrap();
        let (app, db) = harness(dir.path()).await;
        insert_client(&db, OverlaySessionMode::Normal, NORMAL_TOKEN).await;
        let asset = insert_asset(&db, dir.path()).await;
        let item = insert_item(&db, &asset, "titre initial").await;
        let item_uri = format!("/overlay/media-board/items/{}", item.id.to_canonical());

        let uri = format!("{item_uri}?token={NORMAL_TOKEN}");
        let (status, body) = send_json(
            &app,
            request(Method::PATCH, &uri, None, Some(&json!({"title": "hack"}))),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "unauthorized");
    }
}
