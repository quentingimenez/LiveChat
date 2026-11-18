//! Tests des ajouts d'administration de guilde : occupation et quota du board,
//! vidage partagé avec la console, journal borné à la guilde, origine et
//! historique de la file.
//!
//! Ils réutilisent le harnais complet de [`super::guild_tests`].

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::http::{Method, StatusCode};
use lc_domain::rights::{Action, RightsRole, Surface};
use lc_domain::{
    GuildId, GuildRole, JobStatus, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId,
    MediaKind, MediaStatus, PlaybackJob, PlaybackJobId,
};
use serde_json::{json, Value};
use time::{Duration, UtcDateTime};

use super::guild_tests::{fixed_now, token, Harness};

/// Guilde de test (`MEDIA_BOARD_MAX_TOTAL_MB=64` dans le harnais).
const SERVER_LIMIT_MB: i64 = 64;
const MIB: i64 = 1024 * 1024;

fn iso(date: UtcDateTime) -> String {
    lc_domain::time::to_rfc3339_millis(date)
}

/// Un média prêt de `size_bytes` octets, avec son fichier sous la racine média.
pub(super) async fn seed_asset(
    harness: &Harness,
    name: &str,
    size_bytes: i64,
) -> (MediaAsset, std::path::PathBuf) {
    let path = harness.media_path(name);
    std::fs::write(&path, vec![b'x'; usize::try_from(size_bytes).unwrap()]).unwrap();
    let asset = MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: format!("hash-{name}"),
        source_url: format!("https://example.test/{name}"),
        source_title: None,
        kind: MediaKind::Video,
        mime: "video/mp4".to_owned(),
        duration_sec: Some(10),
        width: Some(1280),
        height: Some(720),
        is_vertical: false,
        storage_path: path.to_string_lossy().into_owned(),
        size_bytes,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: fixed_now(),
        expires_at: fixed_now() + Duration::days(1),
        created_at: fixed_now(),
    };
    harness.db.media_assets().insert(&asset).await.unwrap();
    (asset, path)
}

pub(super) async fn seed_item(
    harness: &Harness,
    guild_id: &GuildId,
    asset: &MediaAsset,
    title: &str,
) -> MediaBoardItem {
    let item = MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: asset.id,
        title: Some(title.to_owned()),
        message: None,
        created_by_discord_user_id: None,
        created_by_name: Some("Alice".to_owned()),
        created_at: fixed_now(),
        artwork_asset_id: None,
        artwork_on_overlay: false,
        display_duration_sec: None,
        display_scale_percent: None,
    };
    harness.db.media_board().create(&item).await.unwrap();
    item
}

fn playback_job(
    guild_id: &GuildId,
    status: JobStatus,
    asset_id: Option<MediaAssetId>,
    finished_at: Option<UtcDateTime>,
) -> PlaybackJob {
    PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: asset_id,
        text: asset_id.is_none().then(|| "gg".to_owned()),
        show_text: false,
        author_name: Some("Léa".to_owned()),
        author_image: None,
        duration_sec: 10,
        priority: 0,
        status,
        resumes_after_job_id: None,
        resume_offset_sec: 0,
        remaining_ms_snapshot: None,
        last_playback_state_at: None,
        submission_date: fixed_now(),
        execution_date: fixed_now(),
        scheduled_at: fixed_now(),
        started_at: None,
        finished_at,
    }
}

async fn seed_job(harness: &Harness, job: &PlaybackJob) {
    harness.db.playback_jobs().create(job).await.unwrap();
}

/// Insère une ligne de journal d'administration, comme le middleware le fait.
async fn seed_audit(harness: &Harness, guild_id: &str, action: &str, at_offset_minutes: i64) {
    let at = lc_domain::datetime_to_ms(fixed_now() - Duration::minutes(at_offset_minutes));
    sqlx::query(
        "INSERT INTO admin_audit \
            (at, actor_method, actor_name, action, method, path, status, guild_id, client_ip) \
         VALUES (?, 'secret', 'Owner', ?, 'POST', ?, 200, ?, NULL)",
    )
    .bind(at)
    .bind(action)
    .bind(format!("/admin/api/guilds/{guild_id}/{action}"))
    .bind(guild_id)
    .execute(harness.db.pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn guild_payload_exposes_board_defaults_and_playback() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let (asset, _) = seed_asset(&harness, "board.bin", MIB).await;
    seed_item(&harness, &harness.guild_id, &asset, "Chat qui tombe").await;

    seed_job(
        &harness,
        &playback_job(&harness.guild_id, JobStatus::Done, None, Some(fixed_now())),
    )
    .await;
    seed_job(
        &harness,
        &playback_job(
            &harness.guild_id,
            JobStatus::Failed,
            None,
            Some(fixed_now()),
        ),
    )
    .await;
    seed_job(
        &harness,
        &playback_job(&harness.guild_id, JobStatus::Pending, None, None),
    )
    .await;
    harness
        .db
        .guilds()
        .set_busy_until(&harness.guild_id, Some(fixed_now() + Duration::minutes(2)))
        .await
        .unwrap();

    let (status, body) = harness
        .call(Method::GET, "/overlay/guild", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let board = &body["guild"]["mediaBoard"];
    assert_eq!(board["itemsCount"], 1);
    assert_eq!(board["usedBytes"], MIB);
    assert_eq!(board["maxBytes"], SERVER_LIMIT_MB * MIB);
    assert_eq!(board["remainingBytes"], SERVER_LIMIT_MB * MIB - MIB);
    assert_eq!(board["quotaMb"], Value::Null);
    assert_eq!(board["serverMaxBytes"], SERVER_LIMIT_MB * MIB);

    // Durée effective : défaut du serveur tant que la guilde n'a rien réglé.
    let default_duration = i64::try_from(harness.state.runtime().default_duration()).unwrap();
    assert_eq!(body["guild"]["settings"]["defaultMediaTime"], Value::Null);
    assert_eq!(
        body["guild"]["settings"]["effective"]["defaultMediaTime"],
        default_duration
    );
    assert_eq!(
        body["guild"]["settings"]["effective"]["maxMediaTime"],
        Value::Null
    );

    assert_eq!(body["guild"]["playback"]["done"], 1);
    assert_eq!(body["guild"]["playback"]["failed"], 1);
    assert_eq!(body["guild"]["playback"]["pending"], 1);
    assert_eq!(
        body["guild"]["playback"]["busyUntil"],
        iso(fixed_now() + Duration::minutes(2))
    );

    // Les valeurs propres priment sur le défaut du serveur.
    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/admin/settings",
            Some(&admin_token),
            Some(json!({"defaultMediaTime": 12, "maxMediaTime": 20, "boardQuotaMb": 32})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["guild"]["boardQuotaMb"], 32);

    let (status, body) = harness
        .call(Method::GET, "/overlay/guild", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["guild"]["settings"]["defaultMediaTime"], 12);
    assert_eq!(
        body["guild"]["settings"]["effective"]["defaultMediaTime"],
        12
    );
    assert_eq!(body["guild"]["settings"]["maxMediaTime"], 20);
    assert_eq!(body["guild"]["settings"]["effective"]["maxMediaTime"], 20);
    assert_eq!(body["guild"]["mediaBoard"]["quotaMb"], 32);
    assert_eq!(body["guild"]["mediaBoard"]["maxBytes"], 32 * MIB);
    // La limite du serveur reste exposée même quand la guilde la réduit.
    assert_eq!(
        body["guild"]["mediaBoard"]["serverMaxBytes"],
        SERVER_LIMIT_MB * MIB
    );
}

#[tokio::test]
async fn board_quota_is_manageable_and_bounded_by_the_server_limit() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);

    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/admin/settings",
            Some(&admin_token),
            Some(json!({"boardQuotaMb": 32})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["guild"]["boardQuotaMb"], 32);
    assert_eq!(
        harness
            .db
            .guilds()
            .board_quota_mb(&harness.guild_id)
            .await
            .unwrap(),
        Some(32)
    );

    // `null` revient au quota du serveur.
    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/admin/settings",
            Some(&admin_token),
            Some(json!({"boardQuotaMb": null})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["guild"]["boardQuotaMb"], Value::Null);

    // Bornes : positif, entier, jamais au-dessus de la limite du serveur.
    for payload in [
        json!({"boardQuotaMb": 0}),
        json!({"boardQuotaMb": -3}),
        json!({"boardQuotaMb": "32"}),
        json!({"boardQuotaMb": 1.5}),
    ] {
        let (status, body) = harness
            .call(
                Method::PATCH,
                "/overlay/admin/settings",
                Some(&admin_token),
                Some(payload.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{payload} → {body}");
        assert_eq!(body["error"], "invalid_payload");
    }
    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/admin/settings",
            Some(&admin_token),
            Some(json!({"boardQuotaMb": SERVER_LIMIT_MB + 1})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_board_quota");

    // La limite du serveur elle-même reste acceptée.
    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/admin/settings",
            Some(&admin_token),
            Some(json!({"boardQuotaMb": SERVER_LIMIT_MB})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["guild"]["boardQuotaMb"], SERVER_LIMIT_MB);
}

#[tokio::test]
async fn clear_board_matches_the_name_and_removes_items_files_and_playback() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let (asset, asset_path) = seed_asset(&harness, "board.bin", 4096).await;
    let item = seed_item(&harness, &harness.guild_id, &asset, "Chat qui tombe").await;

    // Une lecture en cours (job texte : aucun média ne la retient) et un verrou.
    let playing = playback_job(&harness.guild_id, JobStatus::Playing, None, None);
    seed_job(&harness, &playing).await;
    harness
        .db
        .guilds()
        .set_busy_until(&harness.guild_id, Some(fixed_now() + Duration::minutes(1)))
        .await
        .unwrap();

    // Une autre guilde garde son board.
    let elsewhere = harness.second_guild("Autre guilde").await;
    let (other_asset, other_path) = seed_asset(&harness, "other.bin", 2048).await;
    let other_item = seed_item(&harness, &elsewhere, &other_asset, "Ailleurs").await;

    // Confirmation absente ou fausse.
    for payload in [
        json!({}),
        json!({"confirm": "autre"}),
        json!({"confirm": " Les Sardines "}),
    ] {
        let (status, body) = harness
            .call(
                Method::POST,
                "/overlay/admin/media-board/clear",
                Some(&admin_token),
                Some(payload.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{payload} → {body}");
        assert_eq!(body["error"], "confirmation_mismatch");
    }
    assert_eq!(
        harness
            .db
            .media_board()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap()
            .len(),
        1
    );

    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/media-board/clear",
            Some(&admin_token),
            Some(json!({"confirm": "Les Sardines"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"]["mediaBoardItems"], 1);
    assert_eq!(body["deleted"]["orphanMediaAssets"], 1);
    assert_eq!(body["deleted"]["orphanMediaFileDeleteErrors"], 0);
    assert_eq!(body["stop"]["releasedPlayingJobs"], 1);

    // Board vidé, fichier et ligne du média supprimés, lecture arrêtée.
    assert!(harness
        .db
        .media_board()
        .list_by_guild(&harness.guild_id)
        .await
        .unwrap()
        .is_empty());
    assert!(!asset_path.exists(), "fichier du média supprimé");
    assert!(harness
        .db
        .media_assets()
        .get(asset.id)
        .await
        .unwrap()
        .is_none());
    assert!(harness
        .db
        .media_board()
        .get(item.id)
        .await
        .unwrap()
        .is_none());
    let stopped = harness
        .db
        .playback_jobs()
        .get(playing.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.status, JobStatus::Done);
    assert!(stopped.finished_at.is_some());
    assert_eq!(
        harness
            .db
            .guilds()
            .get(&harness.guild_id)
            .await
            .unwrap()
            .unwrap()
            .busy_until,
        None
    );

    // L'autre guilde n'a pas bougé.
    assert!(other_path.exists());
    assert_eq!(
        harness
            .db
            .media_board()
            .get(other_item.id)
            .await
            .unwrap()
            .map(|item| item.guild_id),
        Some(elsewhere)
    );

    // Le board vide est bien celui servi à l'overlay.
    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/media-board/items",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn board_admin_routes_are_reserved_to_manage_settings() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let member = harness
        .invite_and_join(&token(&admin), "member", "Léa")
        .await;
    let member_token = token(&member);

    for (method, uri, payload) in [
        (
            Method::PATCH,
            "/overlay/admin/settings",
            Some(json!({"boardQuotaMb": 16})),
        ),
        (
            Method::POST,
            "/overlay/admin/media-board/clear",
            Some(json!({"confirm": "Les Sardines"})),
        ),
        (Method::GET, "/overlay/admin/audit", None),
    ] {
        let (status, body) = harness
            .call(method, uri, Some(&member_token), payload)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri} → {body}");
        assert_eq!(body["error"], "forbidden");
    }
}

#[tokio::test]
async fn audit_lists_only_the_guild_newest_first_and_is_bounded() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let elsewhere = harness.second_guild("Autre guilde").await;

    seed_audit(&harness, harness.guild_id.as_str(), "guild.settings", 30).await;
    seed_audit(&harness, harness.guild_id.as_str(), "guild.board_clear", 20).await;
    seed_audit(&harness, harness.guild_id.as_str(), "guild.purge", 10).await;
    seed_audit(&harness, elsewhere.as_str(), "guild.settings", 5).await;

    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/admin/audit?limit=2",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 2);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    // Du plus récent au plus ancien.
    assert_eq!(items[0]["action"], "guild.purge");
    assert_eq!(items[1]["action"], "guild.board_clear");
    for item in items {
        assert_eq!(item["guildId"], harness.guild_id.as_str());
    }

    // La limite est bornée : 0 → 1, au-delà du maximum → tout ce qui existe.
    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/admin/audit?limit=0",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 1);

    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/admin/audit?limit=10000",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 3);
    for item in body["items"].as_array().unwrap() {
        assert_eq!(item["guildId"], harness.guild_id.as_str());
    }

    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/admin/audit?limit=abc",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_limit");

    // Par défaut : 50 entrées au plus.
    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/admin/audit",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 3);
}

#[tokio::test]
async fn queue_origin_and_history_are_guild_bounded() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let (asset, _) = seed_asset(&harness, "queued.bin", 1024).await;
    seed_item(&harness, &harness.guild_id, &asset, "Chat qui tombe").await;

    // Un job venu du board (média) et un job direct (texte seul). La priorité
    // fixe l'ordre de la file (`priority DESC`), indépendamment des UUID.
    let mut board_job = playback_job(&harness.guild_id, JobStatus::Pending, Some(asset.id), None);
    board_job.priority = 1;
    seed_job(&harness, &board_job).await;
    let direct_job = playback_job(&harness.guild_id, JobStatus::Pending, None, None);
    seed_job(&harness, &direct_job).await;

    // Historique : un terminé et un échec, plus une autre guilde.
    let done = playback_job(
        &harness.guild_id,
        JobStatus::Done,
        Some(asset.id),
        Some(fixed_now() - Duration::minutes(1)),
    );
    seed_job(&harness, &done).await;
    let failed = playback_job(
        &harness.guild_id,
        JobStatus::Failed,
        None,
        Some(fixed_now() - Duration::minutes(5)),
    );
    seed_job(&harness, &failed).await;
    let elsewhere = harness.second_guild("Autre guilde").await;
    seed_job(
        &harness,
        &playback_job(&elsewhere, JobStatus::Done, None, Some(fixed_now())),
    )
    .await;

    let (status, body) = harness
        .call(Method::GET, "/overlay/queue", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let pending = body["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 2);
    let origins: Vec<&str> = pending
        .iter()
        .map(|job| job["origin"].as_str().unwrap())
        .collect();
    assert_eq!(origins, vec!["board", "direct"]);

    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/queue/history?limit=30",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 2);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items[0]["status"], "done");
    assert_eq!(items[0]["title"], "Chat qui tombe");
    assert_eq!(items[0]["authorName"], "Léa");
    assert_eq!(items[0]["origin"], "board");
    assert_eq!(
        items[0]["finishedAt"],
        iso(fixed_now() - Duration::minutes(1))
    );
    assert_eq!(items[1]["status"], "failed");
    assert_eq!(items[1]["origin"], "direct");

    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/queue/history?limit=1",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 1);
    assert_eq!(body["items"][0]["status"], "done");

    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/queue/history?limit=nope",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_limit");

    // Sans le droit `viewQueue`, les deux routes refusent (invité sans droit).
    let guest = harness.invite_and_join(&admin_token, "guest", "Max").await;
    let mut rights = harness.state.rights.snapshot();
    rights.set_override(
        Surface::Overlay,
        Action::ViewQueue,
        RightsRole::Guest,
        false,
    );
    harness.state.rights.replace(rights);
    for uri in ["/overlay/queue", "/overlay/queue/history"] {
        let (status, body) = harness
            .call(Method::GET, uri, Some(&token(&guest)), None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri} → {body}");
        assert_eq!(body["error"], "forbidden");
    }
}

#[tokio::test]
async fn guild_hides_board_and_playback_without_rights() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let member = harness
        .invite_and_join(&token(&admin), "member", "Léa")
        .await;

    let mut rights = harness.state.rights.snapshot();
    rights.set_override(
        Surface::Overlay,
        Action::ViewBoard,
        RightsRole::Member,
        false,
    );
    rights.set_override(
        Surface::Overlay,
        Action::ViewQueue,
        RightsRole::Member,
        false,
    );
    harness.state.rights.replace(rights);

    let (status, body) = harness
        .call(Method::GET, "/overlay/guild", Some(&token(&member)), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["guild"]["mediaBoard"], Value::Null);
    assert_eq!(body["guild"]["playback"], Value::Null);
    assert_eq!(body["permissions"]["viewBoard"], false);
    assert_eq!(body["permissions"]["viewQueue"], false);

    // L'administrateur garde les deux blocs (les cellules admin sont figées).
    let (status, body) = harness
        .call(Method::GET, "/overlay/guild", Some(&token(&admin)), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["guild"]["mediaBoard"].is_object());
    assert!(body["guild"]["playback"].is_object());
}
