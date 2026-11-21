//! Tests des actions dangereuses de l'overlay : purge et départ Discord.
//!
//! Les deux routes partagent l'implémentation de la console ; ces tests
//! protègent la surface overlay elle-même : droit `PurgeGuild`, confirmation
//! forte, guilde du jeton uniquement, et parité de réponse avec l'admin.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use lc_domain::GuildRole;
use serde_json::{json, Value};

use crate::discord::guild_leave::{DiscordGuildLeaver, LeaveError, LeaveOutcome};

use super::guild_tests::{token, Harness};

#[derive(Debug, Clone, Copy)]
enum FakeOutcome {
    Left,
    NotMember,
    Api,
}

#[derive(Debug)]
struct FakeLeaver {
    calls: Mutex<Vec<String>>,
    outcome: FakeOutcome,
}

impl FakeLeaver {
    fn new(outcome: FakeOutcome) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            outcome,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("verrou du leaver").clone()
    }
}

#[async_trait]
impl DiscordGuildLeaver for FakeLeaver {
    async fn leave(&self, discord_guild_id: &str) -> Result<LeaveOutcome, LeaveError> {
        self.calls
            .lock()
            .expect("verrou du leaver")
            .push(discord_guild_id.to_owned());
        match self.outcome {
            FakeOutcome::Left => Ok(LeaveOutcome::Left),
            FakeOutcome::NotMember => Ok(LeaveOutcome::NotMember),
            FakeOutcome::Api => Err(LeaveError::Api("boom".to_owned())),
        }
    }
}

const DISCORD_ID: &str = "123456789012345678";

/// Jeton admin de la guilde du harness.
async fn admin_token(harness: &Harness) -> String {
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    token(&admin)
}

/// Lie la guilde standalone du harness à un serveur Discord.
async fn link_discord(harness: &Harness) {
    let discord = lc_domain::GuildId::parse(DISCORD_ID).unwrap();
    assert!(harness
        .db
        .guilds()
        .link_discord(&harness.guild_id, Some(&discord))
        .await
        .unwrap());
}

#[tokio::test]
async fn guild_payload_exposes_purge_guild_to_admins_only() {
    let harness = Harness::new().await;
    let admin_token = admin_token(&harness).await;
    let member = harness
        .join(&harness.server_invite(GuildRole::Member).await, "Membre")
        .await;

    for (token, expected) in [(admin_token, true), (token(&member), false)] {
        let (status, body) = harness
            .call(Method::GET, "/overlay/guild", Some(&token), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["permissions"]["purgeGuild"], expected,
            "le drapeau purgeGuild pilote la section « Zone de danger »"
        );
    }
}

#[tokio::test]
async fn purge_est_refusee_a_un_membre_non_admin() {
    let harness = Harness::new().await;
    let member = harness
        .join(&harness.server_invite(GuildRole::Member).await, "Membre")
        .await;

    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/purge",
            Some(&token(&member)),
            Some(json!({ "confirmGuildId": harness.guild_id.as_str() })),
        )
        .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "forbidden");
    assert!(
        harness
            .db
            .guilds()
            .access(&harness.guild_id)
            .await
            .unwrap()
            .is_some(),
        "la guilde reste présente"
    );
}

#[tokio::test]
async fn purge_exige_la_confirmation_de_la_guilde_du_jeton() {
    let harness = Harness::new().await;
    let admin_token = admin_token(&harness).await;
    let other = harness.second_guild("Autre guilde").await;

    for confirmation in [Value::Null, json!(other.as_str())] {
        let body = match confirmation {
            Value::Null => json!({}),
            confirmation => json!({ "confirmGuildId": confirmation }),
        };
        let (status, response) = harness
            .call(
                Method::POST,
                "/overlay/admin/purge",
                Some(&admin_token),
                Some(body),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(response["error"], "confirm_guild_id_mismatch");
        assert!(
            harness
                .db
                .guilds()
                .access(&harness.guild_id)
                .await
                .unwrap()
                .is_some(),
            "une confirmation absente ou étrangère ne purge rien"
        );
    }
}

#[tokio::test]
async fn purge_supprime_la_guilde_sans_quitter_discord_par_defaut() {
    let leaver = Arc::new(FakeLeaver::new(FakeOutcome::Left));
    let harness = Harness::with_leaver(leaver.clone()).await;
    link_discord(&harness).await;
    let admin_token = admin_token(&harness).await;

    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/purge",
            Some(&admin_token),
            Some(json!({ "confirmGuildId": harness.guild_id.as_str() })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["purged"], true);
    assert_eq!(body["guildId"], harness.guild_id.as_str());
    assert_eq!(body["discord"]["serverId"], DISCORD_ID);
    assert_eq!(body["discord"]["requested"], false);
    assert_eq!(body["discord"]["outcome"], "skipped");
    assert!(
        leaver.calls().is_empty(),
        "la purge seule ne fait pas quitter Discord"
    );
    assert!(
        harness
            .db
            .guilds()
            .access(&harness.guild_id)
            .await
            .unwrap()
            .is_none(),
        "la guilde est supprimée"
    );

    let (status, body) = harness
        .call(Method::GET, "/overlay/guild", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn discord_leave_fait_quitter_le_bot_sans_purger_la_guilde() {
    let leaver = Arc::new(FakeLeaver::new(FakeOutcome::Left));
    let harness = Harness::with_leaver(leaver.clone()).await;
    link_discord(&harness).await;
    let admin_token = admin_token(&harness).await;

    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/discord-leave",
            Some(&admin_token),
            Some(json!({ "confirmGuildId": harness.guild_id.as_str() })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["discordLeave"]["serverId"], DISCORD_ID);
    assert_eq!(body["discordLeave"]["requested"], true);
    assert_eq!(body["discordLeave"]["outcome"], "left");
    assert_eq!(leaver.calls(), vec![DISCORD_ID.to_owned()]);
    assert!(
        harness
            .db
            .guilds()
            .access(&harness.guild_id)
            .await
            .unwrap()
            .is_some(),
        "la guilde n'est pas supprimée"
    );
}

#[tokio::test]
async fn discord_leave_sans_serveur_lie_n_appelle_pas_le_bot() {
    let leaver = Arc::new(FakeLeaver::new(FakeOutcome::Left));
    let harness = Harness::with_leaver(leaver.clone()).await;
    let admin_token = admin_token(&harness).await;

    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/discord-leave",
            Some(&admin_token),
            Some(json!({ "confirmGuildId": harness.guild_id.as_str() })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["discordLeave"]["serverId"], Value::Null);
    assert_eq!(body["discordLeave"]["outcome"], "skipped");
    assert!(leaver.calls().is_empty());
}

#[tokio::test]
async fn discord_leave_quand_le_bot_n_est_plus_membre_est_idempotent() {
    let leaver = Arc::new(FakeLeaver::new(FakeOutcome::NotMember));
    let harness = Harness::with_leaver(leaver.clone()).await;
    link_discord(&harness).await;
    let admin_token = admin_token(&harness).await;

    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/discord-leave",
            Some(&admin_token),
            Some(json!({ "confirmGuildId": harness.guild_id.as_str() })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["discordLeave"]["outcome"], "not_member");
    assert_eq!(leaver.calls(), vec![DISCORD_ID.to_owned()]);
    assert!(
        harness
            .db
            .guilds()
            .access(&harness.guild_id)
            .await
            .unwrap()
            .is_some(),
        "être déjà absent ne supprime pas la guilde"
    );
}

#[tokio::test]
async fn discord_leave_remonte_un_echec_discord_sans_echouer_la_requete() {
    let leaver = Arc::new(FakeLeaver::new(FakeOutcome::Api));
    let harness = Harness::with_leaver(leaver.clone()).await;
    link_discord(&harness).await;
    let admin_token = admin_token(&harness).await;

    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/discord-leave",
            Some(&admin_token),
            Some(json!({ "confirmGuildId": harness.guild_id.as_str() })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["discordLeave"]["outcome"], "error");
    assert_eq!(body["discordLeave"]["error"], "Discord : boom");
    assert_eq!(leaver.calls(), vec![DISCORD_ID.to_owned()]);
    assert!(
        harness
            .db
            .guilds()
            .access(&harness.guild_id)
            .await
            .unwrap()
            .is_some(),
        "un échec Discord ne supprime pas la guilde"
    );
}

#[tokio::test]
async fn discord_leave_est_refusee_a_un_membre_non_admin() {
    let harness = Harness::new().await;
    let member = harness
        .join(&harness.server_invite(GuildRole::Member).await, "Membre")
        .await;

    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/discord-leave",
            Some(&token(&member)),
            Some(json!({ "confirmGuildId": harness.guild_id.as_str() })),
        )
        .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "forbidden");
}
