//! Révocation des clients overlay et d'ingestion.
//!
//! Reproduit les handlers legacy `POST /api/overlay-clients/:clientId/revoke` et
//! `POST /api/ingest-clients/:clientId/revoke` :
//!
//! - identifiant vide ⇒ `400 invalid_client_id` ; identifiant inexistant ou
//!   mal formé ⇒ `404 <type>_client_not_found` (le legacy cherchait par chaîne,
//!   une valeur invalide n'était donc qu'un « introuvable ») ;
//! - la révocation est idempotente : `revoked` vaut `true` seulement si la ligne
//!   était encore active ;
//! - un client overlay révoqué voit ses sockets fermées (`4402`).

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use lc_domain::{IngestClientId, OverlayClientId};
use serde_json::json;

use super::auth::AdminAccess;
use super::hub::admin_hub;
use super::{require_db, AdminError};
use crate::app::AppState;

const INVALID_CLIENT_ID: &str = "invalid_client_id";
const OVERLAY_NOT_FOUND: &str = "overlay_client_not_found";
const INGEST_NOT_FOUND: &str = "ingest_client_not_found";
/// Code de fermeture WebSocket d'un client révoqué (convention du protocole).
const REVOKED_CLOSE_CODE: u16 = 4402;

/// Handler `POST /admin/api/overlay-clients/{client_id}/revoke`.
pub(super) async fn revoke_overlay(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_client_id): Path<String>,
) -> Result<Response, AdminError> {
    let raw_client_id = validate_client_id(&raw_client_id)?;
    let client_id = OverlayClientId::parse(raw_client_id)
        .map_err(|_| AdminError::not_found(OVERLAY_NOT_FOUND))?;
    let db = require_db(&state)?;

    let client = db
        .overlay_clients()
        .get(client_id)
        .await
        .map_err(|error| AdminError::db("lecture du client overlay", error))?
        .ok_or_else(|| AdminError::not_found(OVERLAY_NOT_FOUND))?;

    let revoked = db
        .overlay_clients()
        .revoke(client_id, state.clock.now())
        .await
        .map_err(|error| AdminError::db("révocation du client overlay", error))?;

    if revoked {
        let hub = admin_hub(&state);
        let closed = hub.close_client(&client.id.as_uuid(), REVOKED_CLOSE_CODE);
        tracing::info!(
            client_id = %client.id.to_canonical(),
            guild_id = client.guild_id.as_str(),
            closed,
            "client overlay révoqué depuis le panneau"
        );
    }

    Ok(Json(json!({
        "found": true,
        "revoked": revoked,
        "clientId": client.id.to_canonical(),
        "guildId": client.guild_id.as_str(),
        "label": client.label,
    }))
    .into_response())
}

/// Handler `POST /admin/api/ingest-clients/{client_id}/revoke`.
pub(super) async fn revoke_ingest(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_client_id): Path<String>,
) -> Result<Response, AdminError> {
    let raw_client_id = validate_client_id(&raw_client_id)?;
    let client_id = IngestClientId::parse(raw_client_id)
        .map_err(|_| AdminError::not_found(INGEST_NOT_FOUND))?;
    let db = require_db(&state)?;

    let client = db
        .ingest_clients()
        .get(client_id)
        .await
        .map_err(|error| AdminError::db("lecture du client d'ingestion", error))?
        .ok_or_else(|| AdminError::not_found(INGEST_NOT_FOUND))?;

    let revoked = db
        .ingest_clients()
        .revoke(client_id, state.clock.now())
        .await
        .map_err(|error| AdminError::db("révocation du client d'ingestion", error))?;

    if revoked {
        tracing::info!(
            client_id = %client.id.to_canonical(),
            guild_id = client.guild_id.as_str(),
            "client d'ingestion révoqué depuis le panneau"
        );
    }

    Ok(Json(json!({
        "found": true,
        "revoked": revoked,
        "clientId": client.id.to_canonical(),
        "guildId": client.guild_id.as_str(),
        "label": client.label,
    }))
    .into_response())
}

/// Refuse un identifiant vide (`400 invalid_client_id`), sinon renvoie la
/// valeur trimée.
fn validate_client_id(raw: &str) -> Result<&str, AdminError> {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        Err(AdminError::bad_request(INVALID_CLIENT_ID))
    } else {
        Ok(trimmed)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn identifiant_client_valide_ou_400() {
        assert_eq!(validate_client_id("  abc  ").expect("valeurs"), "abc");
        assert_eq!(
            validate_client_id("").unwrap_err().into_response().status(),
            axum::http::StatusCode::BAD_REQUEST
        );
        assert_eq!(
            validate_client_id("   ")
                .unwrap_err()
                .into_response()
                .status(),
            axum::http::StatusCode::BAD_REQUEST
        );
    }
}
