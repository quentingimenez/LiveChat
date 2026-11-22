//! Tests des appareils personnels de « Mon profil »
//! (`GET /overlay/devices`, `DELETE /overlay/devices/{overlay,ingest}/{id}`).
//!
//! Un membre n'y voit que ses propres overlays, extensions et raccourcis iOS ;
//! un appareil d'un autre membre ou d'une autre guilde répond comme un
//! identifiant inconnu. La révocation d'un overlay ferme sa socket `4402` et
//! invalide son jeton, sans exiger le droit `ManageMembers`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use axum::Router;
use lc_domain::{
    GuildId, GuildMemberId, GuildRole, IngestClient, IngestClientId, OverlayClient,
    OverlayClientId, OverlaySessionMode,
};
use lc_protocol::close_code;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::http::admin::hub::AdminHub;
use crate::media::tokens::hash_ingest_token;

use super::auth::hash_token;
use super::guild::member_code_authors;
use super::guild_tests::{call_app, fixed_now, token, Harness, RecordingAdminHub};

fn router_with_hub(harness: &Harness, hub: Arc<RecordingAdminHub>) -> Router {
    crate::app::router(
        harness
            .state
            .clone()
            .with_admin_hub(hub as Arc<dyn AdminHub>),
    )
}

/// Insère un overlay actif rattaché à un membre.
async fn seed_overlay(
    harness: &Harness,
    guild_id: &GuildId,
    member_id: &GuildMemberId,
    label: &str,
    secret: &str,
) -> OverlayClient {
    let client = OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: label.to_owned(),
        token_hash: hash_token(secret),
        session_mode: OverlaySessionMode::Normal,
        default_author_name: None,
        default_author_image: None,
        created_by_discord_user_id: None,
        device_hash: None,
        last_seen_at: Some(fixed_now()),
        created_at: fixed_now(),
        revoked_at: None,
    };
    harness.db.overlay_clients().create(&client).await.unwrap();
    harness
        .db
        .overlay_clients()
        .set_member(client.id, *member_id)
        .await
        .unwrap();
    client
}

/// Insère un client d'ingestion actif au nom d'un membre.
async fn seed_ingest(
    harness: &Harness,
    member_id: &GuildMemberId,
    label: &str,
    secret: &str,
) -> IngestClient {
    let client = IngestClient {
        id: IngestClientId::new_v4(),
        guild_id: harness.guild_id.clone(),
        label: label.to_owned(),
        token_hash: hash_ingest_token(secret),
        default_author_name: "Alice".to_owned(),
        default_author_image: None,
        created_by_discord_user_id: format!("member:{}", member_id.to_canonical()),
        last_seen_at: None,
        created_at: fixed_now(),
        revoked_at: None,
    };
    harness.db.ingest_clients().create(&client).await.unwrap();
    client
}

fn member_id_of(body: &Value) -> GuildMemberId {
    body["memberId"].as_str().unwrap().parse().unwrap()
}

fn item<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap_or_else(|| panic!("appareil {id} absent de {body}"))
}

#[tokio::test]
async fn list_returns_only_the_callers_devices_with_their_kind() {
    let harness = Harness::new().await;
    let hub = Arc::new(RecordingAdminHub::default());
    let app = router_with_hub(&harness, hub.clone());

    let alice = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let alice_token = token(&alice);
    let alice_member = member_id_of(&alice);

    let studio = seed_overlay(
        &harness,
        &harness.guild_id,
        &alice_member,
        "Studio",
        "secret-studio",
    )
    .await;
    harness
        .db
        .overlay_clients()
        .update_client_info(studio.id, Some("3.7.0"), Some("windows"))
        .await
        .unwrap();
    hub.set_connected(studio.id.as_uuid(), true);

    // Raccourci iOS créé par le flux de « Mon profil ».
    let (status, ios) = call_app(
        &app,
        Method::POST,
        "/overlay/ingest-client",
        Some(&alice_token),
        &[],
        Some(json!({"deviceName": "iPhone d'Alice"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ios}");
    let ios_id = ios["ingestClientId"].as_str().unwrap().to_owned();

    // Extension appairée par un code, même propriétaire.
    let extension = seed_ingest(
        &harness,
        &alice_member,
        "Chrome - Alice",
        "secret-extension",
    )
    .await;

    // Un autre membre, un appareil d'une autre guilde et un client d'ingestion
    // d'une autre personne ne doivent jamais apparaître.
    let bob = harness
        .join(&harness.server_invite(GuildRole::Member).await, "Bob")
        .await;
    let bob_member = member_id_of(&bob);
    let bob_overlay = seed_overlay(
        &harness,
        &harness.guild_id,
        &bob_member,
        "Bureau de Bob",
        "secret-bob",
    )
    .await;
    let bob_ingest = seed_ingest(&harness, &bob_member, "Firefox - Bob", "secret-bob-ext").await;
    let elsewhere = harness.second_guild("Ailleurs").await;
    let foreign = seed_overlay(
        &harness,
        &elsewhere,
        &alice_member,
        "Ailleurs",
        "secret-ailleurs",
    )
    .await;

    let (status, body) = call_app(
        &app,
        Method::GET,
        "/overlay/devices",
        Some(&alice_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 4, "{body}");

    assert_eq!(
        item(&body, alice["clientId"].as_str().unwrap())["kind"],
        "overlay"
    );
    assert_eq!(
        item(&body, alice["clientId"].as_str().unwrap())["isSelf"],
        true
    );

    let studio_item = item(&body, &studio.id.to_canonical());
    assert_eq!(studio_item["kind"], "overlay");
    assert_eq!(studio_item["label"], "Studio");
    assert_eq!(studio_item["connected"], true);
    assert_eq!(studio_item["sessionMode"], "normal");
    assert_eq!(studio_item["overlayVersion"], "3.7.0");
    assert_eq!(studio_item["platform"], "windows");
    assert_eq!(studio_item["isSelf"], false);
    assert_eq!(
        studio_item["lastSeenAt"],
        lc_domain::time::to_rfc3339_millis(fixed_now())
    );

    let ios_item = item(&body, &ios_id);
    assert_eq!(ios_item["kind"], "ios");
    assert_eq!(ios_item["label"], "Raccourci iOS - Alice");
    assert_eq!(ios_item["connected"], false);
    assert_eq!(ios_item["isSelf"], false);
    assert!(ios_item.get("sessionMode").is_none());

    assert_eq!(
        item(&body, &extension.id.to_canonical())["kind"],
        "extension"
    );

    for hidden in [
        bob_overlay.id.to_canonical(),
        bob_ingest.id.to_canonical(),
        foreign.id.to_canonical(),
        bob["clientId"].as_str().unwrap().to_owned(),
    ] {
        assert!(
            body["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["id"] != hidden.as_str()),
            "appareil étranger {hidden} listé : {body}"
        );
    }

    // Un simple membre (sans `ManageMembers`) a bien accès à sa liste.
    let (status, bob_body) = call_app(
        &app,
        Method::GET,
        "/overlay/devices",
        Some(&token(&bob)),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{bob_body}");
    assert_eq!(bob_body["count"], 3, "{bob_body}");
    assert!(bob_body["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["id"] != alice["clientId"]));
}

#[tokio::test]
async fn revoking_a_personal_overlay_invalidates_its_token_and_closes_its_socket() {
    let harness = Harness::new().await;
    let hub = Arc::new(RecordingAdminHub::default());
    hub.set_deliver(true);
    let app = router_with_hub(&harness, hub.clone());

    let alice = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let alice_token = token(&alice);
    let member = member_id_of(&alice);
    let studio = seed_overlay(
        &harness,
        &harness.guild_id,
        &member,
        "Studio",
        "secret-studio",
    )
    .await;
    let path = format!("/overlay/devices/overlay/{}", studio.id.to_canonical());

    let (status, body) = call_app(&app, Method::DELETE, &path, Some(&alice_token), &[], None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"found": true, "revoked": true}));
    assert_eq!(
        hub.closes(),
        vec![(studio.id.as_uuid(), close_code::REVOKED, String::new())]
    );

    // Le jeton révoqué n'authentifie plus aucune route overlay.
    let (status, _) = call_app(
        &app,
        Method::GET,
        "/overlay/guild",
        Some("secret-studio"),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Une seconde révocation est un 404, jamais un succès silencieux.
    let (status, body) = call_app(&app, Method::DELETE, &path, Some(&alice_token), &[], None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "device_not_found");

    // L'appareil appelant ne se révoque pas lui-même.
    let (status, body) = call_app(
        &app,
        Method::DELETE,
        &format!(
            "/overlay/devices/overlay/{}",
            alice["clientId"].as_str().unwrap()
        ),
        Some(&alice_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "cannot_revoke_self");
    let (status, _) = call_app(
        &app,
        Method::GET,
        "/overlay/guild",
        Some(&alice_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn another_members_or_guilds_overlay_is_not_found() {
    let harness = Harness::new().await;
    let alice = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let bob = harness
        .join(&harness.server_invite(GuildRole::Member).await, "Bob")
        .await;
    let alice_token = token(&alice);

    let bob_overlay = seed_overlay(
        &harness,
        &harness.guild_id,
        &member_id_of(&bob),
        "Bureau de Bob",
        "secret-bob",
    )
    .await;
    let elsewhere = harness.second_guild("Ailleurs").await;
    let elsewhere_overlay = seed_overlay(
        &harness,
        &elsewhere,
        &member_id_of(&alice),
        "Ailleurs",
        "secret-ailleurs",
    )
    .await;

    for id in [
        bob_overlay.id.to_canonical(),
        elsewhere_overlay.id.to_canonical(),
        Uuid::new_v4().to_string(),
        "not-a-uuid".to_owned(),
    ] {
        let (status, body) = harness
            .call(
                Method::DELETE,
                &format!("/overlay/devices/overlay/{id}"),
                Some(&alice_token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id} : {body}");
        assert_eq!(body["error"], "device_not_found");
    }

    // Les appareils étrangers restent actifs.
    assert!(harness
        .db
        .overlay_clients()
        .get(bob_overlay.id)
        .await
        .unwrap()
        .unwrap()
        .is_active());
    assert!(harness
        .db
        .overlay_clients()
        .get(elsewhere_overlay.id)
        .await
        .unwrap()
        .unwrap()
        .is_active());
}

#[tokio::test]
async fn revoking_a_personal_ingest_client_invalidates_its_token() {
    let harness = Harness::new().await;
    let alice = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let alice_token = token(&alice);
    let member = member_id_of(&alice);
    let extension = seed_ingest(&harness, &member, "Chrome - Alice", "secret-extension").await;
    let bob = harness
        .join(&harness.server_invite(GuildRole::Member).await, "Bob")
        .await;
    let bob_ingest = seed_ingest(
        &harness,
        &member_id_of(&bob),
        "Firefox - Bob",
        "secret-bob-ext",
    )
    .await;

    // La propriété du client d'ingestion est bien celle calculée à partir du
    // membre réel, pas une chaîne recopiée dans la fixture.
    let owner = harness
        .db
        .guild_members()
        .get(member)
        .await
        .unwrap()
        .unwrap();
    assert!(member_code_authors(&owner).contains(&extension.created_by_discord_user_id));

    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/devices/ingest/{}", extension.id.to_canonical()),
            Some(&alice_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"found": true, "revoked": true}));

    let revoked = harness
        .db
        .ingest_clients()
        .get(extension.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!revoked.is_active());

    // Le token révoqué n'authentifie plus l'ingestion : un autre client actif
    // existe (Bob), donc la réponse est 401 et non 503.
    let (status, body) = harness
        .call(
            Method::POST,
            "/ingest",
            Some("secret-extension"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"], "unauthorized");

    // Rejouer la révocation, ou révoquer le client d'un autre membre, est un 404.
    for id in [extension.id.to_canonical(), bob_ingest.id.to_canonical()] {
        let (status, body) = harness
            .call(
                Method::DELETE,
                &format!("/overlay/devices/ingest/{id}"),
                Some(&alice_token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id} : {body}");
        assert_eq!(body["error"], "device_not_found");
    }

    // Le client de Bob n'a pas été touché.
    assert!(harness
        .db
        .ingest_clients()
        .get(bob_ingest.id)
        .await
        .unwrap()
        .unwrap()
        .is_active());
}
