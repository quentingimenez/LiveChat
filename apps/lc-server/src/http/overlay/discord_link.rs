//! Liaison d'une guilde indépendante à un serveur Discord, depuis l'overlay.
//!
//! | Méthode | Chemin | Droit | Réponse |
//! |---------|--------|-------|---------|
//! | `POST` | `/overlay/admin/discord-link-code` | `ManageSettings` | code à usage unique (10 min) |
//! | `DELETE` | `/overlay/admin/discord-link` | `ManageSettings` | `{"linked": false}` |
//!
//! Le code émis est saisi par un administrateur du serveur Discord avec la
//! commande `/lier` (`/link`) du bot : `DiscordLinkCodesRepository::redeem`
//! pose alors `guilds.discord_guild_id`. Seule une guilde indépendante peut
//! être liée ; un serveur Discord déjà lié à une autre guilde n'est jamais
//! repris silencieusement (le code reste utilisable après avoir délié l'autre
//! guilde).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use chrono::Duration;
use lc_domain::entities::DiscordLinkCode;
use lc_domain::{PairingCode, Permission};
use serde_json::json;

use crate::app::AppState;

use super::auth::AuthedOverlayClient;
use super::{db, db_failure, error, internal_error, is_conflict, json_response, now};

/// Durée de vie d'un code de liaison.
const LINK_CODE_TTL_MINUTES: i64 = 10;
/// Tentatives de génération avant d'abandonner (collision de clé primaire).
const LINK_CODE_ATTEMPTS: usize = 5;
/// Erreur applicative : la guilde n'est pas indépendante.
const GUILD_NOT_STANDALONE: &str = "guild_not_standalone";

/// Vérifie que la guilde de l'overlay est indépendante.
async fn ensure_standalone<'a>(
    state: &'a AppState,
    auth: &AuthedOverlayClient,
) -> Result<&'a lc_db::Db, Response> {
    let db = db(state)?;
    match db.guilds().access(auth.guild_id()).await {
        Ok(Some(access)) if access.standalone => Ok(db),
        Ok(_) => Err(error(StatusCode::CONFLICT, GUILD_NOT_STANDALONE)),
        Err(db_error) => Err(db_failure("overlay_discord_link.access", &db_error)),
    }
}

/// Handler `POST /overlay/admin/discord-link-code`.
///
/// Émet un code court, à usage unique, et supprime les précédents codes non
/// utilisés de la guilde. `409 guild_not_standalone` pour une guilde native.
pub async fn create_link_code(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
) -> Response {
    if let Err(response) = super::guild::require(&auth, Permission::ManageSettings) {
        return response;
    }
    let db = match ensure_standalone(&state, &auth).await {
        Ok(db) => db,
        Err(response) => return response,
    };
    let created_at = now(&state);
    let expires_at = created_at + Duration::minutes(LINK_CODE_TTL_MINUTES);
    for _ in 0..LINK_CODE_ATTEMPTS {
        let Ok(code) = PairingCode::parse(&super::guild::random_invite_code()) else {
            continue;
        };
        let link = DiscordLinkCode {
            code: code.clone(),
            guild_id: auth.guild_id().clone(),
            created_by_member_id: auth.member_id(),
            expires_at,
            used_at: None,
            created_at,
        };
        match db.discord_link_codes().replace_for_guild(&link).await {
            Ok(()) => {
                return json_response(
                    StatusCode::OK,
                    json!({
                        "code": code.as_str(),
                        "expiresAt": super::guild::iso(Some(expires_at)),
                        "commands": ["/lier", "/link"],
                    }),
                );
            }
            Err(db_error) if is_conflict(&db_error) => {}
            Err(db_error) => return db_failure("overlay_discord_link.create_code", &db_error),
        }
    }
    internal_error()
}

/// Handler `DELETE /overlay/admin/discord-link`.
///
/// Supprime la liaison de la guilde avec un serveur Discord. Sans effet
/// observable côté serveur Discord, sinon que ses commandes visent à nouveau
/// son snowflake natif.
pub async fn unlink(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = super::guild::require(&auth, Permission::ManageSettings) {
        return response;
    }
    let db = match ensure_standalone(&state, &auth).await {
        Ok(db) => db,
        Err(response) => return response,
    };
    match db.guilds().link_discord(auth.guild_id(), None).await {
        Ok(_) => json_response(StatusCode::OK, json!({"linked": false})),
        Err(db_error) => db_failure("overlay_discord_link.unlink", &db_error),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::http::{Method, StatusCode};
    use chrono::Duration;
    use lc_domain::{
        GuildId, GuildMember, GuildMemberId, GuildRole, OverlayClient, OverlayClientId,
        OverlaySessionMode,
    };
    use serde_json::json;

    use super::super::auth;
    use super::super::guild_tests::{fixed_now, token, Harness};

    /// Jeton d'un overlay admin d'une guilde native de test.
    const NATIVE_TOKEN: &str = "3333333333333333333333333333333333333333333333333333333333333333";

    /// Ajoute un admin overlay à une guilde native (`standalone = 0`).
    async fn native_admin(harness: &Harness, native: &GuildId) {
        harness
            .db
            .guilds()
            .upsert(&lc_domain::Guild {
                id: native.clone(),
                name: Some("Serveur natif".to_owned()),
                busy_until: None,
                default_media_time: None,
                max_media_time: None,
                display_media_full: false,
            })
            .await
            .unwrap();
        let member = GuildMember {
            id: GuildMemberId::new_v4(),
            guild_id: native.clone(),
            display_name: "Admin natif".to_owned(),
            avatar: None,
            role: GuildRole::Admin,
            created_at: fixed_now(),
            removed_at: None,
        };
        harness.db.guild_members().create(&member).await.unwrap();
        let client = OverlayClient {
            id: OverlayClientId::new_v4(),
            guild_id: native.clone(),
            label: "panel-natif".to_owned(),
            token_hash: auth::hash_token(NATIVE_TOKEN),
            session_mode: OverlaySessionMode::Normal,
            default_author_name: None,
            default_author_image: None,
            created_by_discord_user_id: None,
            last_seen_at: None,
            created_at: fixed_now(),
            revoked_at: None,
        };
        harness.db.overlay_clients().create(&client).await.unwrap();
        harness
            .db
            .overlay_clients()
            .set_member(client.id, member.id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn admin_obtient_un_code_qui_remplace_l_ancien() {
        let harness = Harness::new().await;
        let admin = harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
            .await;
        let admin_token = token(&admin);

        let (status, body) = harness
            .call(
                Method::POST,
                "/overlay/admin/discord-link-code",
                Some(&admin_token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let code = body["code"].as_str().unwrap().to_owned();
        assert_eq!(code.len(), 8);
        assert_eq!(body["expiresAt"], json!("2026-11-10T12:10:00.000Z"));
        assert_eq!(body["commands"], json!(["/lier", "/link"]));

        let stored: (String, i64, Option<i64>) =
            sqlx::query_as("SELECT code, expires_at, used_at FROM discord_link_codes")
                .fetch_one(harness.db.pool())
                .await
                .unwrap();
        assert_eq!(stored.0, code);
        assert_eq!(
            stored.1,
            (fixed_now() + Duration::minutes(10)).timestamp_millis()
        );
        assert_eq!(stored.2, None);

        // Un second code remplace le précédent, resté inutilisé.
        let (status, body) = harness
            .call(
                Method::POST,
                "/overlay/admin/discord-link-code",
                Some(&admin_token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_ne!(body["code"].as_str().unwrap(), code);
        let codes: Vec<String> = sqlx::query_scalar("SELECT code FROM discord_link_codes")
            .fetch_all(harness.db.pool())
            .await
            .unwrap();
        assert_eq!(codes, vec![body["code"].as_str().unwrap().to_owned()]);
    }

    #[tokio::test]
    async fn membre_refuse_sur_les_deux_routes() {
        let harness = Harness::new().await;
        let admin = harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
            .await;
        let member = harness
            .invite_and_join(&token(&admin), "member", "Léa")
            .await;
        let member_token = token(&member);

        for (method, uri) in [
            (Method::POST, "/overlay/admin/discord-link-code"),
            (Method::DELETE, "/overlay/admin/discord-link"),
        ] {
            let (status, body) = harness.call(method, uri, Some(&member_token), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri} : {body}");
            assert_eq!(body["error"], "forbidden");
        }
    }

    #[tokio::test]
    async fn guilde_native_refusee() {
        let harness = Harness::new().await;
        let native = GuildId::parse("424242424242424242").unwrap();
        native_admin(&harness, &native).await;

        for (method, uri) in [
            (Method::POST, "/overlay/admin/discord-link-code"),
            (Method::DELETE, "/overlay/admin/discord-link"),
        ] {
            let (status, body) = harness.call(method, uri, Some(NATIVE_TOKEN), None).await;
            assert_eq!(status, StatusCode::CONFLICT, "{uri} : {body}");
            assert_eq!(body["error"], "guild_not_standalone");
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM discord_link_codes")
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn unlink_efface_la_liaison() {
        let harness = Harness::new().await;
        let admin = harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
            .await;
        let admin_token = token(&admin);
        let discord = GuildId::parse("123456789012345678").unwrap();
        assert!(harness
            .db
            .guilds()
            .link_discord(&harness.guild_id, Some(&discord))
            .await
            .unwrap());

        let (status, body) = harness
            .call(
                Method::DELETE,
                "/overlay/admin/discord-link",
                Some(&admin_token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["linked"], false);
        assert_eq!(
            harness
                .db
                .guilds()
                .access(&harness.guild_id)
                .await
                .unwrap()
                .unwrap()
                .discord_guild_id,
            None
        );
    }
}
