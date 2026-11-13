//! Codes de pairing : liste, purge des expirés et révocation.
//!
//! Trois routes du contrat legacy :
//!
//! - `GET /admin/api/pairing-codes?guildId&status=all|active|expired` : la liste
//!   est filtrée AVANT le calcul des compteurs, comme dans le legacy (les
//!   `counts` portent donc sur les éléments renvoyés) ;
//! - `DELETE /admin/api/pairing-codes/expired?guildId` : supprime les codes
//!   utilisés ou expirés, éventuellement limités à une guilde ;
//! - `POST /admin/api/pairing-codes/{code}/revoke` : marque utilisé ET expiré un
//!   code encore actif ; `revoked` vaut `false` si le code n'était plus actif.

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use lc_domain::{GuildId, PairingCode};
use serde::Deserialize;
use serde_json::{json, Value};

use super::auth::AdminAccess;
use super::queries;
use super::{opt_iso, require_db, to_iso, AdminError};
use crate::app::AppState;

/// Paramètres de requête des routes de lecture.
#[derive(Debug, Default, Deserialize)]
pub(super) struct PairingQuery {
    #[serde(rename = "guildId")]
    guild_id: Option<String>,
    status: Option<String>,
}

/// Handler `GET /admin/api/pairing-codes`.
pub(super) async fn list(
    State(state): State<AppState>,
    _access: AdminAccess,
    Query(query): Query<PairingQuery>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let now = state.clock.now();
    let guild_filter = non_empty_string(query.guild_id.as_deref());

    let mut codes = match guild_filter.as_deref() {
        Some(raw_guild_id) => match GuildId::parse(raw_guild_id) {
            Ok(guild_id) => db
                .pairing_codes()
                .list_by_guild(&guild_id)
                .await
                .map_err(|error| AdminError::db("codes de pairing de la guilde", error))?,
            // Identifiant de filtre invalide : aucun résultat, comme le
            // `where guildId = "..."` du legacy.
            Err(_) => Vec::new(),
        },
        None => db
            .pairing_codes()
            .list()
            .await
            .map_err(|error| AdminError::db("codes de pairing", error))?,
    };
    codes.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.code.as_str().cmp(right.code.as_str()))
    });

    let status = non_empty_string(query.status.as_deref()).unwrap_or_else(|| "all".to_owned());
    let mut items: Vec<Value> = Vec::with_capacity(codes.len());
    let mut active = 0_i64;

    for code in &codes {
        let is_active = code.is_usable_at(now);
        let keep = match status.as_str() {
            "active" => is_active,
            "expired" => !is_active,
            _ => true,
        };
        if !keep {
            continue;
        }
        if is_active {
            active += 1;
        }

        items.push(json!({
            "code": code.code.as_str(),
            "guildId": code.guild_id.as_str(),
            "createdByDiscordUserId": code.created_by_discord_user_id,
            "authorName": code.author_name,
            "expiresAt": opt_iso(code.expires_at),
            "usedAt": opt_iso(code.used_at),
            "createdAt": to_iso(code.created_at),
            "isActive": is_active,
        }));
    }

    let total = i64::try_from(items.len()).unwrap_or(0);

    Ok(Json(json!({
        "items": items,
        "counts": {
            "total": total,
            "active": active,
            "expired": total - active,
        },
    }))
    .into_response())
}

/// Handler `DELETE /admin/api/pairing-codes/expired`.
pub(super) async fn delete_expired(
    State(state): State<AppState>,
    _access: AdminAccess,
    Query(query): Query<PairingQuery>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let guild_filter = non_empty_string(query.guild_id.as_deref());
    let now_ms = state.clock.now().timestamp_millis();

    let deleted = queries::delete_expired_pairing_codes(db, guild_filter.as_deref(), now_ms)
        .await
        .map_err(|error| AdminError::db("suppression des codes de pairing expirés", error))?;

    Ok(Json(json!({
        "deletedCount": deleted,
        "guildId": guild_filter,
    }))
    .into_response())
}

/// Handler `POST /admin/api/pairing-codes/{code}/revoke`.
pub(super) async fn revoke(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_code): Path<String>,
) -> Result<Response, AdminError> {
    let raw_code = raw_code.trim();
    if raw_code.is_empty() {
        return Err(AdminError::bad_request("invalid_code"));
    }

    let code = PairingCode::parse(raw_code)
        .map_err(|_| AdminError::not_found("pairing_code_not_found"))?;
    let db = require_db(&state)?;

    let entry = db
        .pairing_codes()
        .get(&code)
        .await
        .map_err(|error| AdminError::db("lecture du code de pairing", error))?
        .ok_or_else(|| AdminError::not_found("pairing_code_not_found"))?;

    let now_ms = state.clock.now().timestamp_millis();
    let revoked = queries::revoke_pairing_code(db, code.as_str(), now_ms)
        .await
        .map_err(|error| AdminError::db("révocation du code de pairing", error))?;

    if revoked > 0 {
        tracing::info!(
            code = code.as_str(),
            guild_id = entry.guild_id.as_str(),
            "code de pairing révoqué depuis le panneau"
        );
    }

    Ok(Json(json!({
        "found": true,
        "revoked": revoked > 0,
        "guildId": entry.guild_id.as_str(),
    }))
    .into_response())
}

/// Chaîne trimée non vide, ou `None` (paramètres de requête legacy).
fn non_empty_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
