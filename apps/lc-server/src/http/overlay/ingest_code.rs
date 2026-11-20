//! Code d'appairage émis depuis l'overlay pour l'extension navigateur.
//!
//! `POST /overlay/ingest-code` délivre un code à usage unique de la table
//! `pairing_codes` (mode `NORMAL`) que l'extension consomme ensuite avec
//! `POST /ingest/pair/consume` : la guilde n'a donc plus besoin de Discord pour
//! appairer une extension.
//!
//! | Cas | Statut | Corps |
//! |-----|--------|-------|
//! | droit `TriggerPlayback` absent (invité) | 403 | `{"error":"forbidden"}` |
//! | overlay sans membre | 409 | `{"error":"member_required"}` |
//! | base absente | 503 | `{"error":"database_unavailable"}` |
//! | collision persistante | 500 | `{"error":"internal_error"}` |
//! | succès | 200 | `{"code","expiresAt","apiBaseUrl"}` |
//!
//! Le code vit 10 minutes ; l'avatar du membre n'est repris que s'il s'agit
//! d'une URL `http(s)` (une image `data:` serait trop volumineuse pour
//! l'extension). L'auteur du code porte le préfixe [`INGEST_CODE_CREATOR_PREFIX`]
//! pour que `POST /overlay/pair/consume` refuse de le consommer comme code
//! d'invitation overlay.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use lc_domain::entities::IngestClient;
use lc_domain::ids::IngestClientId;
use lc_domain::rights::{Action, Surface};
use lc_domain::{PairingCode, PairingCodeEntry, PairingMode};
use serde_json::json;
use time::Duration;

use crate::app::AppState;
use crate::media::tokens::{generate_ingest_token, hash_ingest_token};

use super::auth::AuthedOverlayClient;
use super::guild::{iso, random_invite_code};
use super::{db, db_failure, error, internal_error, is_conflict, json_response, now};

/// Préfixe de l'auteur d'un code émis par un membre overlay (la colonne
/// `created_by_discord_user_id` n'accepte pas `NULL` : ce marqueur distingue ce
/// code d'un code émis depuis Discord).
pub(super) const INGEST_CODE_CREATOR_PREFIX: &str = "member:";

/// Durée de vie d'un code d'extension.
const INGEST_CODE_TTL_MINUTES: i64 = 10;

/// Tentatives de génération avant abandon (collision de clé primaire).
const INGEST_CODE_ATTEMPTS: usize = 5;

/// Avatar repris pour l'extension : uniquement une URL `http(s)`.
///
/// Les images `data:` sont volontairement écartées : leur taille dépasserait
/// les limites acceptées par l'extension.
fn http_avatar(avatar: Option<&str>) -> Option<String> {
    let value = avatar?.trim();
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Some(value.to_owned())
    } else {
        None
    }
}

/// Handler `POST /overlay/ingest-code`.
pub async fn create_ingest_code(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
) -> Response {
    // The extension surface gates the pairing code itself (`extension.pair`),
    // not the overlay `triggerPlayback` permission used historically.
    if !state
        .rights
        .allowed(Surface::Extension, Action::Pair, auth.role().into())
    {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let Some(member) = auth.member.as_ref() else {
        return error(StatusCode::CONFLICT, "member_required");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now = now(&state);
    let expires_at = now + Duration::minutes(INGEST_CODE_TTL_MINUTES);

    for _ in 0..INGEST_CODE_ATTEMPTS {
        let Ok(code) = PairingCode::parse(&random_invite_code()) else {
            continue;
        };
        // Le code doit être absent des deux tables : l'overlay cherche d'abord
        // dans `guild_invites`, l'ingestion uniquement dans `pairing_codes`.
        match db.pairing_codes().get(&code).await {
            Ok(None) => {}
            Ok(Some(_)) => continue,
            Err(db_error) => return db_failure("overlay_ingest_code.lookup_pairing", &db_error),
        }
        match db.guild_invites().get(&code).await {
            Ok(None) => {}
            Ok(Some(_)) => continue,
            Err(db_error) => return db_failure("overlay_ingest_code.lookup_invite", &db_error),
        }

        let entry = PairingCodeEntry {
            code,
            guild_id: member.guild_id.clone(),
            mode: PairingMode::Normal,
            grants_admin: false,
            created_by_discord_user_id: format!(
                "{INGEST_CODE_CREATOR_PREFIX}{}",
                member.id.to_canonical()
            ),
            author_name: Some(member.display_name.clone()),
            author_image: http_avatar(member.avatar.as_deref()),
            expires_at: Some(expires_at),
            used_at: None,
            created_at: now,
        };
        match db.pairing_codes().create(&entry).await {
            Ok(()) => {
                return json_response(
                    StatusCode::OK,
                    json!({
                        "code": entry.code.as_str(),
                        "expiresAt": iso(Some(expires_at)),
                        "apiBaseUrl": state.config.api_url.as_str().trim_end_matches('/'),
                    }),
                );
            }
            Err(db_error) if is_conflict(&db_error) => {}
            Err(db_error) => return db_failure("overlay_ingest_code.create", &db_error),
        }
    }
    internal_error()
}

/// Libellé du client d'ingestion créé pour un raccourci iOS.
const IOS_SHORTCUT_LABEL: &str = "Raccourci iOS";

/// Handler `POST /overlay/ingest-client`.
///
/// Crée directement un client d'ingestion pour le membre courant, sans code
/// d'appairage : la board affiche le jeton une seule fois, puis le Raccourci le
/// demande à l'import. Le libellé inclut le pseudo pour ne pas révoquer le
/// client d'un autre membre de la guilde.
pub async fn create_ingest_client(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
) -> Response {
    if !state
        .rights
        .allowed(Surface::Extension, Action::Pair, auth.role().into())
    {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let Some(member) = auth.member.as_ref() else {
        return error(StatusCode::CONFLICT, "member_required");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now = now(&state);
    let name = member.display_name.trim();
    let name = if name.is_empty() {
        member.id.to_canonical()
    } else {
        name.to_owned()
    };
    let label: String = format!("{IOS_SHORTCUT_LABEL} - {name}")
        .chars()
        .take(200)
        .collect();
    let clients = db.ingest_clients();
    match clients.list_by_guild(&member.guild_id).await {
        Ok(existing) => {
            for client in existing
                .iter()
                .filter(|client| client.is_active() && client.label == label)
            {
                if let Err(error) = clients.revoke(client.id, now).await {
                    tracing::warn!(
                        error = %error,
                        client_id = %client.id.to_canonical(),
                        "impossible de révoquer l'ancien client iOS"
                    );
                }
            }
        }
        Err(error) => return db_failure("overlay_ingest_client.list", &error),
    }

    let mut raw_token = generate_ingest_token();
    let mut client = IngestClient {
        id: IngestClientId::new_v4(),
        guild_id: member.guild_id.clone(),
        label: label.clone(),
        token_hash: hash_ingest_token(&raw_token),
        default_author_name: member.display_name.clone(),
        default_author_image: http_avatar(member.avatar.as_deref()),
        created_by_discord_user_id: format!(
            "{INGEST_CODE_CREATOR_PREFIX}{}",
            member.id.to_canonical()
        ),
        last_seen_at: None,
        created_at: now,
        revoked_at: None,
    };

    let mut create_error: Option<lc_db::DbError> = None;
    for attempt in 0..2_u8 {
        match clients.create(&client).await {
            Ok(()) => {
                create_error = None;
                break;
            }
            Err(error) => {
                let retryable = attempt == 0 && is_conflict(&error);
                create_error = Some(error);
                if !retryable {
                    break;
                }
                tracing::warn!(
                    attempt,
                    "collision de hash de token d'ingestion : nouvelle génération"
                );
                raw_token = generate_ingest_token();
                client.token_hash = hash_ingest_token(&raw_token);
            }
        }
    }
    if let Some(error) = create_error {
        return db_failure("overlay_ingest_client.create", &error);
    }

    tracing::info!(
        client_id = %client.id.to_canonical(),
        guild_id = %member.guild_id.as_str(),
        device = %label,
        "client d'ingestion iOS créé"
    );

    json_response(
        StatusCode::OK,
        json!({
            "apiBaseUrl": state.config.api_url.as_str().trim_end_matches('/'),
            "ingestApiToken": raw_token,
            "ingestClientId": client.id.to_canonical(),
            "guildId": member.guild_id.as_str(),
            "deviceName": label,
            "authorName": client.default_author_name,
            "authorImage": client.default_author_image,
        }),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::http::{Method, StatusCode};
    use lc_domain::{GuildRole, PairingCode, PairingCodeEntry, PairingMode};
    use serde_json::{json, Value};
    use time::Duration;

    use super::super::guild_tests::{fixed_now, token, Harness};
    use super::INGEST_CODE_CREATOR_PREFIX;

    /// Émet un code d'extension et vérifie le statut de la réponse.
    async fn issue_code(harness: &Harness, bearer: &str) -> Value {
        let (status, body) = harness
            .call(Method::POST, "/overlay/ingest-code", Some(bearer), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    #[tokio::test]
    async fn membre_emet_un_code_d_extension() {
        let harness = Harness::new().await;
        let admin = harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
            .await;
        let issued = issue_code(&harness, &token(&admin)).await;

        assert_eq!(issued["apiBaseUrl"], "http://127.0.0.1:8099");
        let expected_expiry =
            lc_domain::time::to_rfc3339_millis(fixed_now() + Duration::minutes(10));
        assert_eq!(issued["expiresAt"], expected_expiry);

        let code = PairingCode::parse(issued["code"].as_str().unwrap()).unwrap();
        let entry = harness
            .db
            .pairing_codes()
            .get(&code)
            .await
            .unwrap()
            .expect("code créé");
        assert_eq!(entry.mode, PairingMode::Normal);
        assert_eq!(entry.guild_id, harness.guild_id);
        assert_eq!(entry.author_name.as_deref(), Some("Alice"));
        assert_eq!(
            entry.created_by_discord_user_id,
            format!(
                "{INGEST_CODE_CREATOR_PREFIX}{}",
                admin["memberId"].as_str().unwrap()
            )
        );
        assert_eq!(entry.expires_at, Some(fixed_now() + Duration::minutes(10)));
        assert_eq!(entry.created_at, fixed_now());
        assert_eq!(entry.used_at, None);
    }

    #[tokio::test]
    async fn membre_cree_un_client_ios() {
        let harness = Harness::new().await;
        let admin = harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
            .await;

        let (status, body) = harness
            .call(
                Method::POST,
                "/overlay/ingest-client",
                Some(&token(&admin)),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["apiBaseUrl"], "http://127.0.0.1:8099");
        assert_eq!(body["deviceName"], "Raccourci iOS - Alice");
        assert_eq!(body["ingestApiToken"].as_str().unwrap().len(), 64);

        let clients = harness
            .db
            .ingest_clients()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].label, "Raccourci iOS - Alice");
        assert_eq!(clients[0].default_author_name, "Alice");
        assert!(clients[0].created_by_discord_user_id.starts_with("member:"));
    }

    #[tokio::test]
    async fn reconfigure_le_client_ios_revoque_l_ancien() {
        let harness = Harness::new().await;
        let admin = harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
            .await;

        for _ in 0..2 {
            let (status, body) = harness
                .call(
                    Method::POST,
                    "/overlay/ingest-client",
                    Some(&token(&admin)),
                    None,
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }

        let clients = harness
            .db
            .ingest_clients()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap();
        assert_eq!(
            clients.iter().filter(|client| client.is_active()).count(),
            1
        );
    }

    #[tokio::test]
    async fn invite_refuse_l_emission() {
        let harness = Harness::new().await;
        let admin = harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
            .await;
        let guest = harness
            .invite_and_join(&token(&admin), "guest", "Bob")
            .await;

        let (status, body) = harness
            .call(
                Method::POST,
                "/overlay/ingest-code",
                Some(&token(&guest)),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "forbidden");
        // Aucun code n'a été créé.
        assert!(harness.db.pairing_codes().list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn extension_consomme_le_code() {
        let harness = Harness::new().await;
        let admin = harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
            .await;
        let issued = issue_code(&harness, &token(&admin)).await;
        let code = issued["code"].as_str().unwrap();

        let (status, body) = harness
            .call(
                Method::POST,
                "/ingest/pair/consume",
                None,
                Some(json!({"code": code, "deviceName": "Chrome"})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["guildId"], harness.guild_id.as_str());
        assert_eq!(body["authorName"], "Alice");
        assert_eq!(body["deviceName"], "Chrome");
    }

    #[tokio::test]
    async fn overlay_ne_consomme_pas_un_code_d_extension() {
        let harness = Harness::new().await;
        let admin = harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
            .await;
        let issued = issue_code(&harness, &token(&admin)).await;
        let code = issued["code"].as_str().unwrap();

        let (status, body) = harness
            .call(
                Method::POST,
                "/overlay/pair/consume",
                None,
                Some(json!({"code": code, "displayName": "Intrus"})),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "pairing_code_scope_mismatch");

        // Le code n'a pas été consommé : l'extension peut encore s'appairer.
        let parsed = PairingCode::parse(code).unwrap();
        let entry = harness
            .db
            .pairing_codes()
            .get(&parsed)
            .await
            .unwrap()
            .expect("code conservé");
        assert_eq!(entry.used_at, None);
        let (status, body) = harness
            .call(
                Method::POST,
                "/ingest/pair/consume",
                None,
                Some(json!({"code": code, "deviceName": "Chrome"})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["guildId"], harness.guild_id.as_str());
    }

    #[tokio::test]
    async fn code_discord_toujours_accepte_par_l_overlay() {
        let harness = Harness::new().await;
        harness
            .db
            .pairing_codes()
            .create(&PairingCodeEntry {
                code: PairingCode::parse("DISC42").unwrap(),
                guild_id: harness.guild_id.clone(),
                mode: PairingMode::Normal,
                grants_admin: false,
                created_by_discord_user_id: "123456789012345678".to_owned(),
                author_name: Some("Discordeur".to_owned()),
                author_image: None,
                expires_at: Some(fixed_now() + Duration::minutes(10)),
                used_at: None,
                created_at: fixed_now(),
            })
            .await
            .unwrap();

        let (status, body) = harness
            .call(
                Method::POST,
                "/overlay/pair/consume",
                None,
                Some(json!({"code": "DISC42", "displayName": "Intrus"})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["guildId"], harness.guild_id.as_str());
        assert_eq!(body["role"], "member");
        assert_eq!(body["authorName"], "Discordeur");
    }
}
