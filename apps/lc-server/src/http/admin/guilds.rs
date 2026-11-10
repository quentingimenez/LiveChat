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

use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use lc_domain::{Guild, GuildId};
use serde_json::{json, Value};
use uuid::Uuid;

use super::auth::AdminAccess;
use super::hub::admin_hub;
use super::queries;
use super::{opt_iso, require_db, to_non_empty_string, AdminError};
use crate::app::AppState;

/// Codes des statuts et messages de purge, isolés pour rester lisibles.
const INVALID_GUILD_ID: &str = "invalid_guild_id";
const INVALID_PAYLOAD: &str = "invalid_payload";

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

    let confirmation = to_non_empty_string(payload.get("confirmGuildId"));
    if confirmation.as_deref() != Some(guild_id.as_str()) {
        return Err(AdminError::bad_request("confirm_guild_id_mismatch"));
    }

    let remove_orphan_media = match payload.get("removeOrphanMedia") {
        None => true,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| AdminError::bad_request(INVALID_PAYLOAD))?,
    };

    // 1. Médias référencés par la guilde, calculés AVANT toute suppression.
    let candidate_media_asset_ids = if remove_orphan_media {
        queries::guild_media_asset_ids(db, guild_id.as_str())
            .await
            .map_err(|error| AdminError::db("médias référencés par la guilde", error))?
    } else {
        Vec::new()
    };

    // 2. Arrêt : diffusion, jobs PLAYING libérés, busyUntil effacé.
    let hub = admin_hub(&state);
    let broadcast = hub.broadcast_stop(&guild_id).await;
    let now_ms = state.clock.now().timestamp_millis();
    let released_playing_jobs = queries::release_playing_jobs(db, guild_id.as_str(), now_ms)
        .await
        .map_err(|error| AdminError::db("libération des jobs de playback", error))?;
    db.guilds()
        .set_busy_until(&guild_id, None)
        .await
        .map_err(|error| AdminError::db("effacement de busyUntil", error))?;

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
        if hub.close_client(&client_id, 1000) {
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
    let deleted_pairing_codes = queries::delete_pairing_codes_for_guild(db, guild_id.as_str())
        .await
        .map_err(|error| AdminError::db("suppression des codes de pairing", error))?;
    let deleted_playback_jobs = queries::delete_playback_jobs_for_guild(db, guild_id.as_str())
        .await
        .map_err(|error| AdminError::db("suppression des jobs de playback", error))?;
    let deleted_meme_board_items =
        queries::delete_meme_board_items_for_guild(db, guild_id.as_str())
            .await
            .map_err(|error| AdminError::db("suppression du meme board", error))?;
    let deleted_guild_settings = queries::delete_guild_settings(db, guild_id.as_str())
        .await
        .map_err(|error| AdminError::db("suppression des réglages de guilde", error))?;

    // 5. Médias devenus orphelins : fichier puis ligne, compteurs séparés.
    let mut deleted_orphan_media_assets = 0_u64;
    let mut orphan_media_file_delete_errors = 0_u64;
    for asset_id in candidate_media_asset_ids {
        let Some(asset) = queries::unreferenced_media(db, &asset_id)
            .await
            .map_err(|error| AdminError::db("recherche des médias orphelins", error))?
        else {
            continue;
        };

        match tokio::fs::remove_file(PathBuf::from(&asset.storage_path)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                orphan_media_file_delete_errors += 1;
                tracing::warn!(
                    path = %asset.storage_path,
                    %error,
                    "purge : suppression du fichier média impossible"
                );
            }
        }

        deleted_orphan_media_assets += queries::delete_media_asset(db, &asset_id)
            .await
            .map_err(|error| AdminError::db("suppression d'un média orphelin", error))?;
    }

    tracing::info!(
        guild_id = guild_id.as_str(),
        released_playing_jobs,
        disconnected_overlay_sockets,
        broadcast,
        "guilde purgée depuis le panneau"
    );

    Ok(Json(json!({
        "purged": true,
        "guildId": guild_id.as_str(),
        "stop": {
            "releasedPlayingJobs": released_playing_jobs,
            "disconnectedOverlaySockets": disconnected_overlay_sockets,
        },
        "deleted": {
            "guildSettings": deleted_guild_settings,
            "overlayClients": deleted_overlay_clients,
            "ingestClients": deleted_ingest_clients,
            "pairingCodes": deleted_pairing_codes,
            "playbackJobs": deleted_playback_jobs,
            "memeBoardItems": deleted_meme_board_items,
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
