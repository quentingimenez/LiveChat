//! `POST /overlay/pair/consume` — consommation d'un code d'appairage.
//!
//! Parité avec `server/src/components/overlay/overlayRoutes.ts:202-291`.
//!
//! ## Corps de requête
//!
//! ```json
//! { "code": "ABC123", "deviceName": "Poste salon", "appVersion": "2.0.1" }
//! ```
//!
//! - `code` : obligatoire, insensible à la casse et aux espaces de bord, puis
//!   normalisé en MAJUSCULES ;
//! - `deviceName` : facultatif ; repli legacy `Overlay-<authorName|User>` ;
//! - `label` / `clientName` sont acceptés en alias de `deviceName` (anciens
//!   clients) ;
//! - `appVersion` / `sessionMode` sont ignorés pour la décision : le mode vient
//!   TOUJOURS du code d'appairage persisté (`pairing_codes.mode`), jamais du
//!   client (un client ne peut pas s'auto-attribuer une session normale).
//!
//! ## Réponse (200)
//!
//! ```json
//! {
//!   "clientToken": "…64 hex…",
//!   "clientId": "…uuid…",
//!   "guildId": "123456789012345678",
//!   "apiBaseUrl": "https://api.exemple.tld",
//!   "deviceName": "Poste salon",
//!   "authorName": "Alice" | null,
//!   "sessionMode": "normal" | "invite_read_only",
//!   "protocolVersion": 2
//! }
//! ```
//!
//! `protocolVersion` est un ajout additif v2 (le legacy ne le renvoyait pas) :
//! il permet au client Tauri de refuser un serveur incompatible avant le
//! handshake WebSocket. `clientToken` est la seule fois où le token brut existe
//! côté serveur ; il n'est jamais relu (seul son SHA-256 est stocké).
//!
//! ## Erreurs
//!
//! | Statut | Corps |
//! |--------|-------|
//! | 400 | `{"error":"invalid_payload"}` (code absent) |
//! | 404 | `{"error":"pairing_code_invalid_or_expired"}` |
//! | 409 | `{"error":"overlay_client_conflict"}` (collision de token, très improbable) |
//! | 500 | `{"error":"internal_error"}` |
//! | 503 | `{"error":"database_unavailable"}` |

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use lc_domain::{OverlayClient, OverlayClientId, OverlaySessionMode, PairingCode};

use crate::app::AppState;

use super::{db, db_failure, error, is_conflict, json_response, now};

/// Corps d'appairage (tolérant : les alias legacy sont acceptés).
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PairConsumeRequest {
    /// Code d'appairage saisi par l'utilisateur.
    #[serde(default)]
    code: Option<String>,
    /// Nom d'appareil demandé (nom de la fenêtre/du poste).
    #[serde(default, alias = "label", alias = "clientName")]
    device_name: Option<String>,
    /// Version de l'application overlay (journalisation uniquement).
    #[serde(default)]
    app_version: Option<String>,
    /// Ignoré : le mode effectif est celui du code persisté.
    #[serde(default)]
    /// Champ legacy accepté puis ignoré : le mode vient du code persisté.
    #[allow(dead_code)]
    session_mode: Option<String>,
}

/// Longueur maximale d'un libellé de client (garde-fou base/affichage).
const MAX_LABEL_LEN: usize = 200;

/// Handler `POST /overlay/pair/consume`.
pub async fn consume_pairing(
    State(state): State<AppState>,
    payload: Result<Json<PairConsumeRequest>, JsonRejection>,
) -> Response {
    // AUD-A2-009 : JSON malformé → `400 invalid_payload` dédié (comme le legacy
    // qui traite un corps illisible comme une charge utile invalide), au lieu de
    // la réécriture générique `request_error`.
    let payload = match payload {
        Ok(Json(payload)) => payload,
        Err(rejection) => {
            tracing::debug!(
                error = %rejection.body_text(),
                "corps JSON invalide sur POST /overlay/pair/consume"
            );
            return error(StatusCode::BAD_REQUEST, "invalid_payload");
        }
    };
    let requested_code = payload
        .code
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    if requested_code.is_empty() {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    }

    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now = now(&state);

    // Nettoyage opportuniste des codes expirés (parité legacy) : un échec est
    // journalisé mais ne bloque pas la consommation du code courant.
    if let Err(db_error) = db.pairing_codes().delete_expired(now).await {
        tracing::warn!(error = %db_error, "nettoyage des codes d'appairage expirés impossible");
    }

    let code = match PairingCode::parse(&requested_code) {
        Ok(code) => code,
        Err(_) => return error(StatusCode::NOT_FOUND, "pairing_code_invalid_or_expired"),
    };
    let entry = match db.pairing_codes().consume(&code, now).await {
        Ok(Some(entry)) => entry,
        Ok(None) => return error(StatusCode::NOT_FOUND, "pairing_code_invalid_or_expired"),
        Err(db_error) => return db_failure("overlay_pair.consume_code", &db_error),
    };

    let session_mode = match entry.mode {
        lc_domain::PairingMode::InviteReadOnly => OverlaySessionMode::InviteReadOnly,
        lc_domain::PairingMode::Normal => OverlaySessionMode::Normal,
    };

    let requested_label = payload
        .device_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(truncate_label);
    let author_name = entry.author_name.clone();
    let label = requested_label.unwrap_or_else(|| {
        let suffix = author_name.as_deref().unwrap_or("User");
        truncate_label(&format!("Overlay-{suffix}"))
    });

    // Révoque les clients actifs de la même guilde portant le même libellé
    // (parité legacy : un poste ne peut avoir qu'une session active).
    match db.overlay_clients().list_by_guild(&entry.guild_id).await {
        Ok(clients) => {
            for client in clients {
                if client.is_active() && client.label == label {
                    if let Err(db_error) = db.overlay_clients().revoke(client.id, now).await {
                        return db_failure("overlay_pair.revoke_previous_client", &db_error);
                    }
                }
            }
        }
        Err(db_error) => return db_failure("overlay_pair.list_previous_clients", &db_error),
    }

    let token = super::auth::generate_token();
    let client = OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: entry.guild_id.clone(),
        label: label.clone(),
        token_hash: super::auth::hash_token(&token),
        session_mode,
        default_author_name: author_name.clone(),
        default_author_image: entry.author_image.clone(),
        created_by_discord_user_id: Some(entry.created_by_discord_user_id.clone()),
        last_seen_at: None,
        created_at: now,
        revoked_at: None,
    };
    if let Err(db_error) = db.overlay_clients().create(&client).await {
        if is_conflict(&db_error) {
            return error(StatusCode::CONFLICT, "overlay_client_conflict");
        }
        return db_failure("overlay_pair.create_client", &db_error);
    }

    tracing::info!(
        client_id = %client.id,
        guild_id = %client.guild_id,
        session_mode = %session_mode.as_wire_str(),
        app_version = payload.app_version.as_deref().unwrap_or(""),
        "overlay appairé"
    );

    json_response(
        StatusCode::OK,
        serde_json::json!({
            "clientToken": token,
            "clientId": client.id.to_canonical(),
            "guildId": client.guild_id.as_str(),
            "apiBaseUrl": state.config.api_origin(),
            "deviceName": client.label,
            "authorName": author_name,
            "sessionMode": session_mode.as_wire_str(),
            "protocolVersion": lc_protocol::PROTOCOL_VERSION,
        }),
    )
}

/// Tronque un libellé en respectant les frontières de caractères UTF-8.
fn truncate_label(raw: &str) -> String {
    let mut value = raw.trim().to_owned();
    if value.chars().count() > MAX_LABEL_LEN {
        value = value.chars().take(MAX_LABEL_LEN).collect();
    }
    value
}
