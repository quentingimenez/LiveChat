//! Réglages, arrêt manuel et purge d'une guilde.
//!
//! Trois routes du contrat legacy (`adminRoutes.ts`, handlers
//! `PATCH /api/guilds/:guildId/settings`, `POST /api/guilds/:guildId/stop`,
//! `POST /api/guilds/:guildId/purge`).
//!
//! # Écarts assumés
//!
//! - `stop` n'appelle pas de scheduler de playback (le crate `lc-playback` n'a
//!   pas encore d'équivalent de `onManualStop`) : la diffusion WebSocket et la
//!   libération des jobs en base suffisent au contrat HTTP ;
//! - `purge` ferme les sockets via [`super::hub::AdminHub::close_client`], une
//!   par client connecté, faute de méthode « fermer toute une room » dans le
//!   trait ; le compteur `disconnectedOverlaySockets` compte donc des clients
//!   (et non des sockets), ce qui est documenté dans `docs/rust/server-admin.md` ;
//! - la suppression des clients d'ingestion reste non bloquante (avertissement,
//!   compteur à 0) comme dans le legacy.

use std::path::Path as StdPath;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use lc_domain::{Guild, GuildAccess, GuildId};
use lc_protocol::{close_code, BoardChange, BoardChangedPayload, ServerMessage};
use serde_json::{json, Value};
use uuid::Uuid;

use super::auth::AdminAccess;
use super::clients::REVOKED_CLOSE_CODE;
use super::hub::admin_hub;
use super::queries;
use super::{opt_iso, require_db, to_non_empty_string, AdminError};
use crate::app::AppState;
use crate::discord::guild_leave::{DiscordGuildLeaver, LeaveOutcome};

/// Codes des statuts et messages de purge, isolés pour rester lisibles.
const INVALID_GUILD_ID: &str = "invalid_guild_id";
const INVALID_PAYLOAD: &str = "invalid_payload";

/// Durée de vie d'un code admin émis par le panel serveur (7 jours).
const ADMIN_CODE_TTL_HOURS: i64 = 24 * 7;
/// Longueur maximale du nom d'une guilde.
const GUILD_NAME_MAX_CHARS: usize = 60;
/// Tentatives de génération d'identifiant de guilde.
const GUILD_ID_ATTEMPTS: usize = 5;

/// Issue d'une demande de départ Discord, sérialisée dans l'objet `discord`.
#[derive(Debug, Clone)]
struct DiscordDeparture {
    server_id: Option<String>,
    requested: bool,
    outcome: &'static str,
    error: Option<String>,
}

impl DiscordDeparture {
    /// Départ non tenté (non demandé ou aucun serveur Discord associé).
    fn skipped(server_id: Option<String>, requested: bool) -> Self {
        Self {
            server_id,
            requested,
            outcome: "skipped",
            error: None,
        }
    }

    /// Objet JSON `{"serverId", "requested", "outcome", "error"}`.
    fn payload(&self) -> Value {
        json!({
            "serverId": self.server_id.as_deref(),
            "requested": self.requested,
            "outcome": self.outcome,
            "error": self.error.as_deref(),
        })
    }
}

/// Serveur Discord associé à une guilde, s'il existe.
///
/// Une guilde indépendante expose sa liaison (`discord_guild_id`, éventuellement
/// absente) ; une guilde native est elle-même le snowflake Discord. Sans ligne
/// `guilds`, on ne peut se fier qu'à la forme de l'identifiant.
fn discord_server_id(guild_id: &GuildId, access: Option<&GuildAccess>) -> Option<String> {
    match access {
        Some(access) if access.standalone => access.discord_guild_id.clone(),
        Some(_) => Some(guild_id.to_string()),
        None => {
            if guild_id.as_str().bytes().all(|byte| byte.is_ascii_digit()) {
                Some(guild_id.to_string())
            } else {
                None
            }
        }
    }
}

/// Exécute (ou ignore) le départ Discord et journalise l'issue.
///
/// Une erreur de départ n'est jamais propagée : la purge et le détachement
/// restent des opérations réussies côté panneau.
async fn perform_discord_leave(
    state: &AppState,
    server_id: Option<String>,
    requested: bool,
) -> DiscordDeparture {
    let Some(server_id) = server_id else {
        if requested {
            tracing::info!("départ Discord demandé sans serveur Discord associé : ignoré");
        }
        return DiscordDeparture::skipped(None, requested);
    };

    if !requested {
        return DiscordDeparture::skipped(Some(server_id), false);
    }

    let leaver: &dyn DiscordGuildLeaver = state.discord_leaver.as_ref();
    match leaver.leave(&server_id).await {
        Ok(LeaveOutcome::Left) => {
            tracing::info!(
                discord_guild_id = server_id.as_str(),
                "le bot a quitté le serveur Discord"
            );
            DiscordDeparture {
                server_id: Some(server_id),
                requested: true,
                outcome: "left",
                error: None,
            }
        }
        Ok(LeaveOutcome::NotMember) => {
            tracing::info!(
                discord_guild_id = server_id.as_str(),
                "le bot n'était pas membre du serveur Discord"
            );
            DiscordDeparture {
                server_id: Some(server_id),
                requested: true,
                outcome: "not_member",
                error: None,
            }
        }
        Err(error) => {
            tracing::warn!(
                discord_guild_id = server_id.as_str(),
                %error,
                "départ du serveur Discord impossible"
            );
            DiscordDeparture {
                server_id: Some(server_id),
                requested: true,
                outcome: "error",
                error: Some(error.to_string()),
            }
        }
    }
}

/// Émet un code d'invitation admin à usage unique pour la guilde.
async fn issue_admin_code(
    state: &AppState,
    db: &lc_db::Db,
    guild_id: &GuildId,
) -> Result<Value, AdminError> {
    use crate::http::overlay::guild::{create_invite, invite_payload, NewInvite};

    let now = state.clock.now();
    let invite = NewInvite {
        guild_id: guild_id.clone(),
        role: lc_domain::GuildRole::Admin,
        member_id: None,
        max_uses: Some(1),
        created_by_member_id: None,
        expires_at: Some(now + chrono::Duration::hours(ADMIN_CODE_TTL_HOURS)),
    };
    match create_invite(db, invite, now).await {
        Ok(Some(invite)) => Ok(invite_payload(&invite, now)),
        Ok(None) => Err(AdminError::Internal),
        Err(error) => Err(AdminError::db("création du code admin", error)),
    }
}

/// Counters produced by [`stop_guild_playback`].
#[derive(Debug, Clone, Copy)]
struct PlaybackStop {
    broadcast: usize,
    released_playing_jobs: u64,
}

/// Stop playback for one guild: broadcast, release `PLAYING` jobs, clear
/// `busy_until`.
///
/// `purge` selects the dedicated purge broadcast; the SQL work and the lock
/// clearing are identical for every caller.
async fn stop_guild_playback(
    state: &AppState,
    db: &lc_db::Db,
    guild_id: &GuildId,
    purge: bool,
) -> Result<PlaybackStop, AdminError> {
    let hub = admin_hub(state);
    let broadcast = if purge {
        hub.broadcast_purge(guild_id).await
    } else {
        hub.broadcast_stop(guild_id).await
    };
    let now_ms = state.clock.now().timestamp_millis();
    let released_playing_jobs = queries::release_playing_jobs(db, guild_id.as_str(), now_ms)
        .await
        .map_err(|error| AdminError::db("releasing playback jobs", error))?;
    // `set_busy_until` upserts: only clear it on an existing row, so stopping
    // an unknown guild never creates a `guilds` row.
    let has_row = db
        .guilds()
        .get(guild_id)
        .await
        .map_err(|error| AdminError::db("reading guild", error))?
        .is_some();
    if has_row {
        db.guilds()
            .set_busy_until(guild_id, None)
            .await
            .map_err(|error| AdminError::db("clearing busyUntil", error))?;
    }

    Ok(PlaybackStop {
        broadcast,
        released_playing_jobs,
    })
}

/// Remove orphan media files and rows for the supplied candidate ids.
///
/// A file is deleted only when its canonical target stays under the media root
/// and is not a symlink (AUD-V2-S-001/002). The row is still deleted when the
/// file cannot be removed. Returns `(deleted_rows, file_delete_errors)`.
async fn remove_orphan_media(
    state: &AppState,
    db: &lc_db::Db,
    candidate_ids: &[String],
) -> Result<(u64, u64), AdminError> {
    let media_root = match std::fs::canonicalize(&state.config.media_storage_dir) {
        Ok(root) => Some(root),
        Err(error) => {
            tracing::warn!(
                directory = %state.config.media_storage_dir.display(),
                %error,
                "media cleanup: media root cannot be canonicalized, files are kept"
            );
            None
        }
    };
    let mut deleted_orphan_media_assets = 0_u64;
    let mut orphan_media_file_delete_errors = 0_u64;

    for asset_id in candidate_ids {
        let Some(asset) = queries::unreferenced_media(db, asset_id)
            .await
            .map_err(|error| AdminError::db("looking up orphan media", error))?
        else {
            continue;
        };

        if let Some(root) = media_root.as_deref() {
            if let Some(confined) = crate::media::confinement::confined_removal_path(
                root,
                StdPath::new(&asset.storage_path),
            ) {
                match tokio::fs::remove_file(&confined).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        orphan_media_file_delete_errors += 1;
                        tracing::warn!(
                            path = %confined.display(),
                            %error,
                            "media cleanup: media file cannot be removed"
                        );
                    }
                }
            }
        }

        deleted_orphan_media_assets += queries::delete_media_asset(db, asset_id)
            .await
            .map_err(|error| AdminError::db("deleting orphan media", error))?;
    }

    Ok((deleted_orphan_media_assets, orphan_media_file_delete_errors))
}

/// Reject a missing or mismatching `confirmGuildId`.
fn require_guild_confirmation(payload: &Value, guild_id: &GuildId) -> Result<(), AdminError> {
    let confirmation = to_non_empty_string(payload.get("confirmGuildId"));
    if confirmation.as_deref() != Some(guild_id.as_str()) {
        return Err(AdminError::bad_request("confirm_guild_id_mismatch"));
    }

    Ok(())
}

/// Handler `POST /admin/api/guilds` : crée une guilde indépendante de Discord
/// et son premier code admin.
///
/// Corps : `{"name": "Les Sardines"}`. Réponse :
/// `{"created": true, "guild": {"id", "name"}, "adminInvite": {…}}`.
pub(super) async fn create(
    State(state): State<AppState>,
    _access: AdminAccess,
    payload: Json<Value>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let name: String = to_non_empty_string(payload.get("name"))
        .and_then(|raw| crate::http::overlay::guild::normalize_label(&raw, GUILD_NAME_MAX_CHARS))
        .ok_or_else(|| AdminError::bad_request("invalid_name"))?;

    let now = state.clock.now();
    let mut created = None;
    for _ in 0..GUILD_ID_ATTEMPTS {
        let candidate = GuildId::generate();
        match db.guilds().create_standalone(&candidate, &name, now).await {
            Ok(()) => {
                created = Some(candidate);
                break;
            }
            Err(lc_db::DbError::Domain(lc_domain::DomainError::Conflict(_))) => {}
            Err(error) => return Err(AdminError::db("création de la guilde", error)),
        }
    }
    let guild_id = created.ok_or(AdminError::Internal)?;
    let admin_invite = issue_admin_code(&state, db, &guild_id).await?;
    tracing::info!(guild_id = guild_id.as_str(), "guilde indépendante créée");

    Ok(Json(json!({
        "created": true,
        "guild": { "id": guild_id.as_str(), "name": name },
        "adminInvite": admin_invite,
    }))
    .into_response())
}

/// Handler `POST /admin/api/guilds/{guild_id}/admin-code` : émet un nouveau
/// code admin (premier admin d'une guilde existante, ou récupération).
pub(super) async fn admin_code(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let guild_id = parse_guild_id(&raw_guild_id)?;
    let admin_invite = issue_admin_code(&state, db, &guild_id).await?;
    Ok(Json(json!({
        "created": true,
        "guildId": guild_id.as_str(),
        "adminInvite": admin_invite,
    }))
    .into_response())
}

/// Handler `PATCH /admin/api/guilds/{guild_id}/settings`.
///
/// Corps partiel : `defaultMediaTime`, `maxMediaTime`, `displayMediaFull`. Les
/// entiers `<= 0` (ou `null`/chaîne vide) effacent le réglage. Aucun champ
/// fourni, ou un champ mal typé, vaut `400 invalid_payload`.
pub(super) async fn update_settings(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
    payload: Json<Value>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let guild_id = parse_guild_id(&raw_guild_id)?;

    let default_media_time = optional_int_field(&payload, "defaultMediaTime");
    let max_media_time = optional_int_field(&payload, "maxMediaTime");
    let display_media_full = optional_bool_field(&payload, "displayMediaFull");

    if default_media_time.is_none() && max_media_time.is_none() && display_media_full.is_none() {
        return Err(AdminError::bad_request(INVALID_PAYLOAD));
    }

    let default_media_time = default_media_time
        .map(|result| result.map_err(|()| AdminError::bad_request(INVALID_PAYLOAD)))
        .transpose()?;
    let max_media_time = max_media_time
        .map(|result| result.map_err(|()| AdminError::bad_request(INVALID_PAYLOAD)))
        .transpose()?;
    let display_media_full = display_media_full
        .map(|result| result.map_err(|()| AdminError::bad_request(INVALID_PAYLOAD)))
        .transpose()?;

    let existing = db
        .guilds()
        .get(&guild_id)
        .await
        .map_err(|error| AdminError::db("lecture des réglages de guilde", error))?;

    let default_media_time = merge_int(
        default_media_time,
        existing.as_ref().and_then(|guild| guild.default_media_time),
    );
    let max_media_time = merge_int(
        max_media_time,
        existing.as_ref().and_then(|guild| guild.max_media_time),
    );
    let display_media_full = display_media_full.unwrap_or_else(|| {
        existing
            .as_ref()
            .is_some_and(|guild| guild.display_media_full)
    });

    let guild = Guild {
        id: guild_id.clone(),
        name: existing.as_ref().and_then(|guild| guild.name.clone()),
        busy_until: existing.as_ref().and_then(|guild| guild.busy_until),
        default_media_time,
        max_media_time,
        display_media_full,
    };

    db.guilds()
        .upsert(&guild)
        .await
        .map_err(|error| AdminError::db("enregistrement des réglages de guilde", error))?;

    // Relit la ligne persistée (upsert peut avoir conservé d'autres colonnes).
    let persisted = db
        .guilds()
        .get(&guild_id)
        .await
        .map_err(|error| AdminError::db("relecture des réglages de guilde", error))?
        .unwrap_or(guild);

    let hub = admin_hub(&state);
    let broadcast = hub.broadcast_settings(&guild_id).await;
    tracing::debug!(broadcast, guild_id = guild_id.as_str(), "réglages diffusés");

    Ok(Json(json!({
        "updated": true,
        "guild": {
            "id": guild_id.as_str(),
            "defaultMediaTime": persisted.default_media_time,
            "maxMediaTime": persisted.max_media_time,
            "displayMediaFull": persisted.display_media_full,
            "busyUntil": opt_iso(persisted.busy_until),
        },
    }))
    .into_response())
}

/// Handler `POST /admin/api/guilds/{guild_id}/stop`.
///
/// Diffuse un arrêt aux overlays, libère les jobs `PLAYING` et efface
/// `busyUntil`, dans l'ordre du legacy.
pub(super) async fn stop(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let guild_id = parse_guild_id(&raw_guild_id)?;
    let hub = admin_hub(&state);

    let broadcast = hub.broadcast_stop(&guild_id).await;
    let now_ms = state.clock.now().timestamp_millis();
    let released = queries::release_playing_jobs(db, guild_id.as_str(), now_ms)
        .await
        .map_err(|error| AdminError::db("libération des jobs de playback", error))?;
    db.guilds()
        .set_busy_until(&guild_id, None)
        .await
        .map_err(|error| AdminError::db("effacement de busyUntil", error))?;

    tracing::info!(
        guild_id = guild_id.as_str(),
        released,
        broadcast,
        "arrêt manuel demandé depuis le panneau"
    );

    Ok(Json(json!({
        "stopped": true,
        "guildId": guild_id.as_str(),
        "releasedCount": released,
    }))
    .into_response())
}

/// Handler `POST /admin/api/guilds/{guild_id}/discord-unlink`.
///
/// Corps optionnel : `{"leaveDiscord": true|false}` (défaut `false`). Détache
/// une guilde indépendante de son serveur Discord, supprime les codes de
/// liaison en attente, puis fait éventuellement quitter le bot.
pub(super) async fn discord_unlink(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
    payload: Json<Value>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let guild_id = parse_guild_id(&raw_guild_id)?;
    let leave = optional_bool_field(&payload, "leaveDiscord")
        .map(|result| result.map_err(|()| AdminError::bad_request(INVALID_PAYLOAD)))
        .transpose()?
        .unwrap_or(false);

    let access = db
        .guilds()
        .access(&guild_id)
        .await
        .map_err(|error| AdminError::db("lecture du rattachement de guilde", error))?;
    let Some(access) = access else {
        return Err(AdminError::not_found("guild_not_found"));
    };
    if !access.standalone {
        return Err(AdminError::conflict("native_guild_unlink"));
    }
    let Some(discord_guild_id) = access.discord_guild_id.clone() else {
        return Err(AdminError::conflict("not_linked"));
    };

    let unlinked = db
        .guilds()
        .link_discord(&guild_id, None)
        .await
        .map_err(|error| AdminError::db("détachement du serveur Discord", error))?;
    if !unlinked {
        return Err(AdminError::not_found("guild_not_found"));
    }
    db.discord_link_codes()
        .delete_by_guild(&guild_id)
        .await
        .map_err(|error| AdminError::db("suppression des codes de liaison Discord", error))?;

    let discord = perform_discord_leave(&state, Some(discord_guild_id), leave).await;
    tracing::info!(
        guild_id = guild_id.as_str(),
        leave,
        outcome = discord.outcome,
        "liaison Discord retirée depuis le panneau"
    );

    Ok(Json(json!({
        "unlinked": true,
        "guildId": guild_id.as_str(),
        "discord": discord.payload(),
    }))
    .into_response())
}

/// Handler `POST /admin/api/guilds/{guild_id}/purge`.
///
/// Corps : `{"confirmGuildId":"<même identifiant>","removeOrphanMedia":true|false}`.
/// `removeOrphanMedia` vaut `true` par défaut ; `confirmGuildId` doit être
/// strictement égal à l'identifiant de la route.
pub(super) async fn purge(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
    payload: Json<Value>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let guild_id = parse_guild_id(&raw_guild_id)?;

    require_guild_confirmation(&payload, &guild_id)?;

    let remove_orphan_media_enabled = match payload.get("removeOrphanMedia") {
        None => true,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| AdminError::bad_request(INVALID_PAYLOAD))?,
    };
    let leave_discord = match payload.get("leaveDiscord") {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| AdminError::bad_request(INVALID_PAYLOAD))?,
    };

    // 0. Rattachement lu AVANT toute suppression : la ligne `guilds` est purgée
    // plus bas et ne serait alors plus lisible.
    let access = db
        .guilds()
        .access(&guild_id)
        .await
        .map_err(|error| AdminError::db("lecture du rattachement de guilde", error))?;
    let discord_server_id = discord_server_id(&guild_id, access.as_ref());

    // 1. Médias référencés par la guilde, calculés AVANT toute suppression.
    let candidate_media_asset_ids = if remove_orphan_media_enabled {
        queries::guild_media_asset_ids(db, guild_id.as_str())
            .await
            .map_err(|error| AdminError::db("médias référencés par la guilde", error))?
    } else {
        Vec::new()
    };

    // 2. Arrêt : diffusion, jobs PLAYING libérés, busyUntil effacé.
    let stop = stop_guild_playback(&state, db, &guild_id, true).await?;
    let hub = admin_hub(&state);

    // 3. Fermeture des overlays connectés de la guilde.
    let guild_overlays = queries::overlay_clients(db)
        .await
        .map_err(|error| AdminError::db("clients overlay de la guilde", error))?;
    let mut disconnected_overlay_sockets = 0_u64;
    for overlay in guild_overlays
        .iter()
        .filter(|overlay| overlay.guild_id == guild_id.as_str())
    {
        let Ok(client_id) = Uuid::parse_str(&overlay.id) else {
            continue;
        };
        if hub.close_client(&client_id, close_code::GUILD_DELETED) {
            disconnected_overlay_sockets += 1;
        }
    }

    // 4. Suppressions, dans l'ordre du legacy (les clients d'ingestion ne
    // bloquent pas la purge en cas d'échec).
    let deleted_ingest_clients = match queries::delete_ingest_clients_for_guild(
        db,
        guild_id.as_str(),
    )
    .await
    {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(%error, guild_id = guild_id.as_str(), "purge : suppression des clients d'ingestion impossible");
            0
        }
    };
    let deleted_overlay_clients = queries::delete_overlay_clients_for_guild(db, guild_id.as_str())
        .await
        .map_err(|error| AdminError::db("suppression des clients overlay", error))?;
    // Invitations puis membres : après les overlays (clé étrangère
    // `overlay_clients.member_id`).
    db.guild_invites()
        .delete_by_guild(&guild_id)
        .await
        .map_err(|error| AdminError::db("suppression des invitations", error))?;
    db.discord_link_codes()
        .delete_by_guild(&guild_id)
        .await
        .map_err(|error| AdminError::db("suppression des codes de liaison Discord", error))?;
    db.guild_members()
        .delete_by_guild(&guild_id)
        .await
        .map_err(|error| AdminError::db("suppression des membres", error))?;
    let deleted_pairing_codes = queries::delete_pairing_codes_for_guild(db, guild_id.as_str())
        .await
        .map_err(|error| AdminError::db("suppression des codes de pairing", error))?;
    let deleted_playback_jobs = queries::delete_playback_jobs_for_guild(db, guild_id.as_str())
        .await
        .map_err(|error| AdminError::db("suppression des jobs de playback", error))?;
    let deleted_media_board_items =
        queries::delete_media_board_items_for_guild(db, guild_id.as_str())
            .await
            .map_err(|error| AdminError::db("suppression du board média", error))?;
    let deleted_guild_settings = queries::delete_guild_settings(db, guild_id.as_str())
        .await
        .map_err(|error| AdminError::db("suppression des réglages de guilde", error))?;

    // 5. Médias devenus orphelins : fichier puis ligne, compteurs séparés.
    let (deleted_orphan_media_assets, orphan_media_file_delete_errors) =
        remove_orphan_media(&state, db, &candidate_media_asset_ids).await?;

    // 6. Départ Discord optionnel, après les suppressions (jamais bloquant).
    let discord = perform_discord_leave(&state, discord_server_id, leave_discord).await;

    tracing::info!(
        guild_id = guild_id.as_str(),
        released_playing_jobs = stop.released_playing_jobs,
        disconnected_overlay_sockets,
        broadcast = stop.broadcast,
        outcome = discord.outcome,
        "guilde purgée depuis le panneau"
    );

    Ok(Json(json!({
        "purged": true,
        "guildId": guild_id.as_str(),
        "stop": {
            "releasedPlayingJobs": stop.released_playing_jobs,
            "disconnectedOverlaySockets": disconnected_overlay_sockets,
        },
        "deleted": {
            "guildSettings": deleted_guild_settings,
            "overlayClients": deleted_overlay_clients,
            "ingestClients": deleted_ingest_clients,
            "pairingCodes": deleted_pairing_codes,
            "playbackJobs": deleted_playback_jobs,
            "mediaBoardItems": deleted_media_board_items,
            "orphanMediaAssets": deleted_orphan_media_assets,
            "orphanMediaFileDeleteErrors": orphan_media_file_delete_errors,
        },
        "discord": discord.payload(),
    }))
    .into_response())
}

/// Handler `POST /admin/api/guilds/{guild_id}/revoke-clients`.
///
/// Revokes all active overlay and ingest clients of the guild, marks active
/// members as removed, and deletes pending pairing codes and invites. The
/// guild, its settings, board, playback history, media and Discord link are
/// kept.
pub(super) async fn revoke_clients(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
    payload: Json<Value>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let guild_id = parse_guild_id(&raw_guild_id)?;
    require_guild_confirmation(&payload, &guild_id)?;

    // No early return for a guild without a `guilds` row: the overview also
    // lists guilds that only exist through their clients, and those must be
    // revocable too (an unknown guild simply yields zero counts).
    let stop = stop_guild_playback(&state, db, &guild_id, false).await?;
    let now_ms = state.clock.now().timestamp_millis();

    let revoked_overlay_ids =
        queries::revoke_overlay_clients_for_guild(db, guild_id.as_str(), now_ms)
            .await
            .map_err(|error| AdminError::db("revoking overlay clients", error))?;
    let hub = admin_hub(&state);
    let mut disconnected_overlay_sockets = 0_u64;
    for overlay_id in &revoked_overlay_ids {
        let Ok(client_id) = Uuid::parse_str(overlay_id) else {
            continue;
        };
        if hub.close_client(&client_id, REVOKED_CLOSE_CODE) {
            disconnected_overlay_sockets += 1;
        }
    }

    let revoked_ingest_clients =
        queries::revoke_ingest_clients_for_guild(db, guild_id.as_str(), now_ms)
            .await
            .map_err(|error| AdminError::db("revoking ingest clients", error))?;
    let removed_members = queries::remove_members_for_guild(db, guild_id.as_str(), now_ms)
        .await
        .map_err(|error| AdminError::db("removing guild members", error))?;
    let deleted_pairing_codes = queries::delete_pairing_codes_for_guild(db, guild_id.as_str())
        .await
        .map_err(|error| AdminError::db("deleting pairing codes", error))?;
    let deleted_invites = db
        .guild_invites()
        .delete_by_guild(&guild_id)
        .await
        .map_err(|error| AdminError::db("deleting invites", error))?;

    tracing::info!(
        guild_id = guild_id.as_str(),
        released_playing_jobs = stop.released_playing_jobs,
        disconnected_overlay_sockets,
        overlay_clients = revoked_overlay_ids.len(),
        ingest_clients = revoked_ingest_clients,
        members = removed_members,
        pairing_codes = deleted_pairing_codes,
        invites = deleted_invites,
        "guild clients revoked from the admin panel"
    );

    Ok(Json(json!({
        "guildId": guild_id.as_str(),
        "stop": {
            "releasedPlayingJobs": stop.released_playing_jobs,
            "disconnectedOverlaySockets": disconnected_overlay_sockets,
        },
        "revoked": {
            "overlayClients": revoked_overlay_ids.len(),
            "ingestClients": revoked_ingest_clients,
            "members": removed_members,
            "pairingCodes": deleted_pairing_codes,
            "invites": deleted_invites,
        },
    }))
    .into_response())
}

/// Handler `POST /admin/api/guilds/{guild_id}/media-board/clear`.
///
/// Deletes the guild's media board items and the media that become orphans.
/// Media still referenced by a playback job or another board is kept.
pub(super) async fn clear_media_board(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
    payload: Json<Value>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let guild_id = parse_guild_id(&raw_guild_id)?;
    require_guild_confirmation(&payload, &guild_id)?;

    let candidate_media_asset_ids = queries::guild_board_media_asset_ids(db, guild_id.as_str())
        .await
        .map_err(|error| AdminError::db("board media asset ids", error))?;
    let item_ids = queries::guild_board_item_ids(db, guild_id.as_str())
        .await
        .map_err(|error| AdminError::db("board media item ids", error))?;
    let deleted_media_board_items =
        queries::delete_media_board_items_for_guild(db, guild_id.as_str())
            .await
            .map_err(|error| AdminError::db("deleting media board items", error))?;
    if deleted_media_board_items > 0 {
        let hub = admin_hub(&state);
        for item_id in item_ids {
            hub.broadcast_to_guild(
                &guild_id,
                ServerMessage::BoardChanged(BoardChangedPayload {
                    change: BoardChange::Deleted,
                    item_id,
                }),
            );
        }
    }
    let (deleted_orphan_media_assets, orphan_media_file_delete_errors) =
        remove_orphan_media(&state, db, &candidate_media_asset_ids).await?;

    tracing::info!(
        guild_id = guild_id.as_str(),
        media_board_items = deleted_media_board_items,
        orphan_media_assets = deleted_orphan_media_assets,
        orphan_media_file_delete_errors,
        "media board cleared from the admin panel"
    );

    Ok(Json(json!({
        "guildId": guild_id.as_str(),
        "deleted": {
            "mediaBoardItems": deleted_media_board_items,
            "orphanMediaAssets": deleted_orphan_media_assets,
            "orphanMediaFileDeleteErrors": orphan_media_file_delete_errors,
        },
    }))
    .into_response())
}

/// Valide l'identifiant de guilde du chemin (`400 invalid_guild_id` sinon).
fn parse_guild_id(raw: &str) -> Result<GuildId, AdminError> {
    GuildId::parse(raw).map_err(|_| AdminError::bad_request(INVALID_GUILD_ID))
}

/// Convertit un champ entier optionnel du corps.
///
/// Retourne `None` si le champ est absent, `Some(Ok(valeur))` sinon, où
/// `valeur` est `None` pour `null`/chaîne vide/`<= 0` (effacement) et
/// `Some(entier)` pour un entier positif. `Some(Err(()))` signale un type
/// invalide (`400 invalid_payload`).
fn optional_int_field(payload: &Value, key: &str) -> Option<Result<Option<i64>, ()>> {
    let value = payload.get(key)?;

    Some(match value {
        Value::Null => Ok(None),
        Value::String(text) if text.trim().is_empty() => Ok(None),
        Value::Number(number) => {
            let Some(float) = number.as_f64() else {
                return Some(Err(()));
            };
            let floored = float.floor();
            if floored <= 0.0 {
                Ok(None)
            } else {
                // `as i64` sature (Rust >= 1.45) : pas de panic sur les extrêmes.
                Ok(Some(floored as i64))
            }
        }
        _ => Err(()),
    })
}

/// Convertit un champ booléen optionnel du corps (`Some(Err)` si mal typé).
fn optional_bool_field(payload: &Value, key: &str) -> Option<Result<bool, ()>> {
    payload.get(key).map(|value| value.as_bool().ok_or(()))
}

/// Fusionne une valeur fournie (`Option<Option<i64>>`) avec la valeur actuelle.
fn merge_int(provided: Option<Option<i64>>, existing: Option<i64>) -> Option<i64> {
    match provided {
        Some(value) => value,
        None => existing,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use serde_json::json;

    use super::*;

    #[test]
    fn identifiant_de_guilde_valide_ou_400() {
        assert!(parse_guild_id("1234567890").is_ok());
        assert_eq!(
            parse_guild_id("pas-un-id")
                .unwrap_err()
                .into_response()
                .status(),
            axum::http::StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn champs_entiers_optionnels() {
        let body = json!({
            "defaultMediaTime": 12.9,
            "maxMediaTime": null,
            "clean": "",
            "zero": 0.5,
            "text": "12",
            "negative": -5,
        });

        assert_eq!(optional_int_field(&body, "absent"), None);
        assert_eq!(
            optional_int_field(&body, "defaultMediaTime"),
            Some(Ok(Some(12)))
        );
        assert_eq!(optional_int_field(&body, "maxMediaTime"), Some(Ok(None)));
        assert_eq!(optional_int_field(&body, "clean"), Some(Ok(None)));
        assert_eq!(optional_int_field(&body, "zero"), Some(Ok(None)));
        assert_eq!(optional_int_field(&body, "text"), Some(Err(())));
        assert_eq!(optional_int_field(&body, "negative"), Some(Ok(None)));
    }

    #[test]
    fn champs_booleens_optionnels() {
        let body = json!({ "displayMediaFull": true, "text": "yes" });

        assert_eq!(optional_bool_field(&body, "absent"), None);
        assert_eq!(
            optional_bool_field(&body, "displayMediaFull"),
            Some(Ok(true))
        );
        assert_eq!(optional_bool_field(&body, "text"), Some(Err(())));
    }

    #[test]
    fn fusion_des_valeurs() {
        assert_eq!(merge_int(Some(None), Some(10)), None);
        assert_eq!(merge_int(Some(Some(3)), Some(10)), Some(3));
        assert_eq!(merge_int(None, Some(10)), Some(10));
        assert_eq!(merge_int(None, None), None);
    }
}
