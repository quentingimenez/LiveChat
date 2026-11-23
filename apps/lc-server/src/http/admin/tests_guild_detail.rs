//! Integration tests for the guild detail and transposed admin routes.
//!
//! They reuse the harness of [`super::tests`] (full `crate::app::router`, one
//! in-memory database and one session per harness) and assert the contract of
//! `docs/rust/admin-console-api.md` sections 3 and 4.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::http::{header, HeaderValue, Method, StatusCode};
use lc_domain::{
    GuildId, GuildMemberId, GuildRole, JobStatus, MediaBoardItemId, OverlaySessionMode,
};
use serde_json::{json, Value};

use super::tests::{
    board_item, create_linked_standalone, guild, guild_invite, guild_member, media_asset,
    overlay_client, playback_job, FakeLeaveResult, FakeLeaver, Harness,
};
use crate::http::overlay::media_board::{complete_board_item, replace_board_item_media, ItemTitle};
use crate::http::overlay::ActorInfo;

const GUILD: &str = "123456789012345678";
const OTHER_GUILD: &str = "999999999999999999";

fn guild_id() -> GuildId {
    GuildId::parse(GUILD).expect("identifiant de guilde valide")
}

/// Body of a mutation request, with or without a JSON payload.
fn mutation_request(
    harness: &Harness,
    method: Method,
    path: &str,
    body: &Value,
) -> axum::http::Request<axum::body::Body> {
    if body.is_null() {
        harness.request(method, path)
    } else {
        harness.request_json(method, path, body)
    }
}

// ---------------------------------------------------------------------------
// GET /admin/api/guilds/{id}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn guild_detail_returns_the_enriched_overview_entry() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    db.guilds().upsert(&guild(GUILD)).await.unwrap();

    let member = guild_member(&guild_id, GuildRole::Admin);
    db.guild_members().create(&member).await.unwrap();

    let client = overlay_client(&guild_id, OverlaySessionMode::Normal);
    db.overlay_clients().create(&client).await.unwrap();
    db.overlay_clients()
        .set_member(client.id, member.id)
        .await
        .unwrap();

    db.guild_invites()
        .create(&guild_invite("ABCDEFGH", &guild_id))
        .await
        .unwrap();
    db.playback_jobs()
        .create(&playback_job(&guild_id, JobStatus::Pending))
        .await
        .unwrap();

    let (status, body) = harness
        .call(Method::GET, &format!("/admin/api/guilds/{GUILD}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(body["id"], GUILD);
    assert_eq!(body["guestsCanViewBoard"], true);
    assert_eq!(body["boardQuotaBytes"], Value::Null);
    assert_eq!(body["discord"]["botPresent"], Value::Null);
    assert_eq!(body["settings"]["defaultMediaTime"], 7);
    assert_eq!(body["overlays"]["total"], 1);
    assert_eq!(
        body["overlays"]["clients"][0]["memberId"],
        member.id.to_canonical()
    );
    assert_eq!(body["overlays"]["clients"][0]["role"], "admin");
    assert_eq!(body["members"]["count"], 1);
    assert_eq!(body["members"]["items"][0]["isSelf"], false);
    assert_eq!(body["invites"]["count"], 1);
    assert_eq!(body["invites"]["items"][0]["code"], "ABCDEFGH");
    assert_eq!(body["queue"]["pending"].as_array().map(Vec::len), Some(1));
    assert!(body["mediaBoard"]["maxBytes"].as_i64().unwrap() > 0);
}

#[tokio::test]
async fn guild_detail_unknown_guild_is_404() {
    let harness = Harness::new().await;

    let (status, body) = harness
        .call(Method::GET, &format!("/admin/api/guilds/{OTHER_GUILD}"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "guild_not_found");

    let (status, body) = harness
        .call(Method::GET, "/admin/api/guilds/not-a-guild")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_guild_id");
}

// ---------------------------------------------------------------------------
// GET /admin/api/guilds/{id}/media-board/items
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_media_board_items_match_the_overlay_list_shape() {
    let harness = Harness::with_pairs(&[("MEDIA_URL_SECRET", "secret-admin-board")]).await;
    let db = harness.db();
    let guild_id = guild_id();
    db.guilds().upsert(&guild(GUILD)).await.unwrap();
    let client = overlay_client(&guild_id, OverlaySessionMode::Normal);
    db.overlay_clients().create(&client).await.unwrap();

    let asset = media_asset(1024, &harness.path("board-item.png"));
    db.media_assets().insert(&asset).await.unwrap();
    db.media_board()
        .create(&board_item(&guild_id, asset.id))
        .await
        .unwrap();

    let (status, body) = harness
        .call(
            Method::GET,
            &format!("/admin/api/guilds/{GUILD}/media-board/items"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["mediaAssetId"], asset.id.to_canonical());
    assert_eq!(body["items"][0]["canEdit"], true);
    assert!(body["items"][0]["mediaUrl"]
        .as_str()
        .unwrap()
        .contains(&asset.id.to_canonical()));

    // A guild id in the query that does not match the path is refused, like the
    // overlay route.
    let (status, body) = harness
        .call(
            Method::GET,
            &format!("/admin/api/guilds/{GUILD}/media-board/items?guildId={OTHER_GUILD}"),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "forbidden");
}

// ---------------------------------------------------------------------------
// Transposed member routes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_member_update_and_remove() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    let member = guild_member(&guild_id, GuildRole::Member);
    db.guild_members().create(&member).await.unwrap();
    // A second admin keeps the last-admin guard out of the way of the removal.
    let other_admin = guild_member(&guild_id, GuildRole::Admin);
    db.guild_members().create(&other_admin).await.unwrap();
    let member_id = member.id.to_canonical();
    let path = format!("/admin/api/guilds/{GUILD}/members/{member_id}");

    let (status, body) = harness
        .call_json(Method::PATCH, &path, &json!({"role": "admin"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["updated"], true);
    assert_eq!(body["member"]["role"], "admin");

    let (status, body) = harness
        .call_json(Method::PATCH, &path, &json!({"role": "owner"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_role");

    let (status, body) = harness.call(Method::DELETE, &path).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removed"], true);

    let (status, body) = harness.call(Method::DELETE, &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "member_not_found");
}

// ---------------------------------------------------------------------------
// Transposed invite routes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_invite_create_and_revoke() {
    let harness = Harness::new().await;
    let db = harness.db();
    db.guilds().upsert(&guild(GUILD)).await.unwrap();

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/invites"),
            &json!({"role": "member", "expiresInHours": 2, "maxUses": 3}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["created"], true);
    assert_eq!(body["invite"]["role"], "member");
    assert_eq!(body["invite"]["maxUses"], 3);
    let code = body["invite"]["code"].as_str().unwrap().to_owned();

    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/invites/{code}/revoke"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["found"], true);
    assert_eq!(body["revoked"], true);

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/invites"),
            &json!({"role": "owner"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_role");

    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/invites/ZZZZZZZZ/revoke"),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "invite_not_found");
}

// ---------------------------------------------------------------------------
// Transposed Discord link code
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_discord_link_code_needs_a_standalone_guild() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    create_linked_standalone(db, &guild_id, "424242424242424242").await;

    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/discord-link-code"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["code"].as_str().map(str::len), Some(8));
    assert_eq!(body["commands"], json!(["/lier", "/link"]));

    let native = "777777777777777777";
    db.guilds().upsert(&guild(native)).await.unwrap();
    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/admin/api/guilds/{native}/discord-link-code"),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "guild_not_standalone");
}

// ---------------------------------------------------------------------------
// Transposed device code
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_device_code_is_bound_to_the_body_member() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    let member = guild_member(&guild_id, GuildRole::Member);
    db.guild_members().create(&member).await.unwrap();
    let member_id = member.id.to_canonical();

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/device-code"),
            &json!({"memberId": member_id}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["created"], true);
    assert_eq!(body["invite"]["memberId"], member_id);
    assert_eq!(body["invite"]["role"], "member");

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/device-code"),
            &json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/device-code"),
            &json!({"memberId": GuildMemberId::new_v4().to_canonical()}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "member_not_found");
}

// ---------------------------------------------------------------------------
// Transposed queue routes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_queue_remove_and_clear() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    let first = playback_job(&guild_id, JobStatus::Pending);
    let second = playback_job(&guild_id, JobStatus::Pending);
    db.playback_jobs().create(&first).await.unwrap();
    db.playback_jobs().create(&second).await.unwrap();

    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!(
                "/admin/api/guilds/{GUILD}/queue/{}",
                first.id.to_canonical()
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removed"], true);

    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!("/admin/api/guilds/{GUILD}/queue/nope"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["removed"], false);

    let (status, body) = harness
        .call(Method::DELETE, &format!("/admin/api/guilds/{GUILD}/queue"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removedCount"], 1);
}

#[tokio::test]
async fn admin_queue_reorders_pending_jobs() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    let first = playback_job(&guild_id, JobStatus::Pending);
    let second = playback_job(&guild_id, JobStatus::Pending);
    db.playback_jobs().create(&first).await.unwrap();
    db.playback_jobs().create(&second).await.unwrap();

    let queue_ids = |body: &Value| -> Vec<String> {
        body["queue"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .map(|job| job["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let (status, body) = harness
        .call(Method::GET, &format!("/admin/api/guilds/{GUILD}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        queue_ids(&body),
        vec![first.id.to_canonical(), second.id.to_canonical()]
    );

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/queue/reorder"),
            &json!({"jobIds": [second.id.to_canonical(), first.id.to_canonical()]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reordered"], true);

    let (_, body) = harness
        .call(Method::GET, &format!("/admin/api/guilds/{GUILD}"))
        .await;
    assert_eq!(
        queue_ids(&body),
        vec![second.id.to_canonical(), first.id.to_canonical()]
    );

    // Ordre périmé : la console recharge la file au lieu d'écrire un ordre
    // partiel.
    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/queue/reorder"),
            &json!({"jobIds": [first.id.to_canonical()]}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale_queue");
}

// ---------------------------------------------------------------------------
// Transposed media-board item routes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_media_item_update_and_delete() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    let asset = media_asset(512, &harness.path("item.png"));
    db.media_assets().insert(&asset).await.unwrap();
    let item = board_item(&guild_id, asset.id);
    db.media_board().create(&item).await.unwrap();
    let item_id = item.id.to_canonical();
    let path = format!("/admin/api/guilds/{GUILD}/media-board/items/{item_id}");

    let (status, body) = harness
        .call_json(Method::PATCH, &path, &json!({"title": "Titre admin"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["updated"], true);
    assert_eq!(body["item"]["title"], "Titre admin");

    let (status, body) = harness.call_json(Method::PATCH, &path, &json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");

    let (status, body) = harness.call(Method::DELETE, &path).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], true);

    // Unknown item ids keep the overlay behaviour (`200 {"deleted":false}`).
    let unknown = MediaBoardItemId::new_v4().to_canonical();
    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!("/admin/api/guilds/{GUILD}/media-board/items/{unknown}"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deleted"], false);

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            &format!("/admin/api/guilds/{GUILD}/media-board/items/{unknown}"),
            &json!({"title": "x"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "item_not_found");
}

// ---------------------------------------------------------------------------
// Extended settings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn settings_extension_sets_name_guest_flag_and_board_quota() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            &format!("/admin/api/guilds/{GUILD}/settings"),
            &json!({
                "name": "Les Sardines",
                "guestsCanViewBoard": false,
                "boardQuotaMb": 128,
                "defaultMediaTime": 9,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["updated"], true);
    assert_eq!(body["name"], "Les Sardines");
    assert_eq!(body["guestsCanViewBoard"], false);
    assert_eq!(body["boardQuotaBytes"], 128 * 1024 * 1024_i64);
    // Backwards-compatible legacy block.
    assert_eq!(body["guild"]["id"], GUILD);
    assert_eq!(body["guild"]["defaultMediaTime"], 9);
    assert_eq!(body["guild"]["name"], "Les Sardines");

    let stored = db.guilds().get(&guild_id).await.unwrap().unwrap();
    assert_eq!(stored.name.as_deref(), Some("Les Sardines"));
    assert_eq!(
        db.guilds().board_quota_mb(&guild_id).await.unwrap(),
        Some(128)
    );
    assert!(
        !db.guilds()
            .access(&guild_id)
            .await
            .unwrap()
            .unwrap()
            .guests_can_view_board
    );

    // `null` restores the server-wide quota.
    let (status, body) = harness
        .call_json(
            Method::PATCH,
            &format!("/admin/api/guilds/{GUILD}/settings"),
            &json!({"boardQuotaMb": null}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["boardQuotaBytes"], Value::Null);
    assert_eq!(db.guilds().board_quota_mb(&guild_id).await.unwrap(), None);

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            &format!("/admin/api/guilds/{GUILD}/settings"),
            &json!({"name": 3}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_name");

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            &format!("/admin/api/guilds/{GUILD}/settings"),
            &json!({"boardQuotaMb": 0}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");
}

#[tokio::test]
async fn guild_board_quota_is_enforced_when_adding_media() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    db.guilds().upsert(&guild(GUILD)).await.unwrap();
    db.guilds()
        .set_board_quota_mb(&guild_id, Some(1))
        .await
        .unwrap();

    let asset = media_asset(2 * 1024 * 1024, &harness.path("too-big.png"));
    db.media_assets().insert(&asset).await.unwrap();
    let now = harness.state().clock.now();
    let actor = ActorInfo::operator(harness.state(), &guild_id);
    let authed = actor.to_authed(now);

    let failure = complete_board_item(
        harness.state(),
        &authed,
        db,
        asset.clone(),
        ItemTitle::Requested(None),
        None,
        now,
    )
    .await
    .expect_err("the guild quota must refuse the asset");
    assert_eq!(failure.code, "media_ingestion_failed");
    assert!(
        failure
            .message
            .as_deref()
            .is_some_and(|message| message.contains("1 Mo")),
        "{failure:?}"
    );

    // Without the override, the server limit (15 GiB in the test config) fits.
    db.guilds()
        .set_board_quota_mb(&guild_id, None)
        .await
        .unwrap();
    let completed = complete_board_item(
        harness.state(),
        &authed,
        db,
        asset,
        ItemTitle::Requested(None),
        None,
        now,
    )
    .await
    .expect("the server quota must accept the asset");
    assert!(completed.created);
}

#[tokio::test]
async fn an_item_already_on_the_board_pins_its_asset_again() {
    // A concurrent re-ingestion of the same link can overwrite the asset's
    // expiry with the cache TTL after the item was created. Asking the board
    // again must pin the asset even when the item already exists, otherwise
    // the card stays `created:false` and the media expires on the board.
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    db.guilds().upsert(&guild(GUILD)).await.unwrap();
    let asset = media_asset(1024, &harness.path("clip.png"));
    db.media_assets().insert(&asset).await.unwrap();
    let now = harness.state().clock.now();
    let actor = ActorInfo::operator(harness.state(), &guild_id);
    let authed = actor.to_authed(now);

    let first = complete_board_item(
        harness.state(),
        &authed,
        db,
        asset.clone(),
        ItemTitle::Requested(None),
        None,
        now,
    )
    .await
    .expect("the board accepts the asset");
    assert!(first.created);

    // What the second ingestion does to the row: back to the cache TTL.
    sqlx::query("UPDATE media_assets SET expires_at = ? WHERE id = ?")
        .bind(lc_domain::datetime_to_ms(now) + 3_600_000)
        .bind(asset.id.to_canonical())
        .execute(db.pool())
        .await
        .unwrap();

    let again = complete_board_item(
        harness.state(),
        &authed,
        db,
        asset.clone(),
        ItemTitle::Requested(None),
        None,
        now,
    )
    .await
    .expect("the existing item is still accepted");
    assert!(!again.created, "the item already existed");
    let stored = db.media_assets().get(asset.id).await.unwrap().unwrap();
    assert!(
        stored.expires_at > now + time::Duration::hours(24),
        "the asset is pinned again instead of staying on the cache TTL: {}",
        stored.expires_at
    );
}

#[tokio::test]
async fn media_board_concurrent_additions_never_exceed_the_guild_quota() {
    // Two ingestions at the same instant used to check the quota outside the
    // insertion transaction: both saw 0 used and both inserted, overshooting
    // `board_quota_mb`. The check now shares the insertion transaction.
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    db.guilds().upsert(&guild(GUILD)).await.unwrap();
    db.guilds()
        .set_board_quota_mb(&guild_id, Some(3))
        .await
        .unwrap();
    let now = harness.state().clock.now();
    let actor = ActorInfo::operator(harness.state(), &guild_id);
    let authed = actor.to_authed(now);

    let first = media_asset(2 * 1024 * 1024, &harness.path("race-a.png"));
    let second = media_asset(2 * 1024 * 1024, &harness.path("race-b.png"));
    db.media_assets().insert(&first).await.unwrap();
    db.media_assets().insert(&second).await.unwrap();

    let (first_result, second_result) = tokio::join!(
        complete_board_item(
            harness.state(),
            &authed,
            db,
            first,
            ItemTitle::Requested(None),
            None,
            now,
        ),
        complete_board_item(
            harness.state(),
            &authed,
            db,
            second,
            ItemTitle::Requested(None),
            None,
            now,
        ),
    );

    let successes = usize::from(first_result.is_ok()) + usize::from(second_result.is_ok());
    assert_eq!(successes, 1, "{first_result:?} / {second_result:?}");
    let failure = first_result
        .err()
        .or_else(|| second_result.err())
        .expect("one of the two ingestions is refused");
    assert_eq!(failure.code, "media_ingestion_failed");
    assert!(
        failure
            .message
            .as_deref()
            .is_some_and(|message| message.contains("3 Mo")),
        "{failure:?}"
    );

    let items = db.media_board().list_by_guild(&guild_id).await.unwrap();
    assert_eq!(items.len(), 1);
    let board_bytes: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(a.size_bytes), 0) \
           FROM media_board_items b \
           JOIN media_assets a ON a.id = b.media_asset_id \
          WHERE b.guild_id = ?",
    )
    .bind(guild_id.as_str())
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert!(
        board_bytes <= 3 * 1024 * 1024,
        "the board holds {board_bytes} bytes for a 3 MiB quota"
    );
}

#[tokio::test]
async fn media_board_replace_media_is_refused_when_it_exceeds_the_guild_quota() {
    // Replacing checks `used - previous + new`: outside the transaction, two
    // concurrent replacements each freed the same previous asset and both
    // passed. A single too-large replacement must already be refused, and a
    // fitting one must still be accepted on the size difference.
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = guild_id();
    db.guilds().upsert(&guild(GUILD)).await.unwrap();
    db.guilds()
        .set_board_quota_mb(&guild_id, Some(2))
        .await
        .unwrap();
    let now = harness.state().clock.now();
    let actor = ActorInfo::operator(harness.state(), &guild_id);
    let authed = actor.to_authed(now);

    let original = media_asset(2 * 1024 * 1024, &harness.path("original.png"));
    db.media_assets().insert(&original).await.unwrap();
    let item = board_item(&guild_id, original.id);
    db.media_board().create(&item).await.unwrap();

    let bigger = media_asset(3 * 1024 * 1024, &harness.path("bigger.png"));
    db.media_assets().insert(&bigger).await.unwrap();
    let failure = replace_board_item_media(harness.state(), &authed, db, item.id, bigger, now)
        .await
        .expect_err("the replacement would exceed the guild quota");
    assert_eq!(failure.code, "media_ingestion_failed");
    assert!(
        failure
            .message
            .as_deref()
            .is_some_and(|message| message.contains("2 Mo")),
        "{failure:?}"
    );
    let stored = db.media_board().get(item.id).await.unwrap().unwrap();
    assert_eq!(
        stored.media_asset_id, original.id,
        "the refused replacement leaves the item untouched"
    );

    let smaller = media_asset(1024 * 1024, &harness.path("smaller.png"));
    let smaller_id = smaller.id;
    db.media_assets().insert(&smaller).await.unwrap();
    let completed = replace_board_item_media(harness.state(), &authed, db, item.id, smaller, now)
        .await
        .expect("the smaller replacement fits the quota");
    assert!(!completed.created);
    assert!(completed.replaced);
    assert_eq!(completed.item.media_asset_id, smaller_id);
}

// ---------------------------------------------------------------------------
// Discord leave
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discord_leave_requires_confirmation_and_does_not_unlink() {
    let leaver = FakeLeaver::new(FakeLeaveResult::Left);
    let harness = Harness::with_leaver(leaver.clone()).await;
    let db = harness.db();
    let guild_id = guild_id();
    create_linked_standalone(db, &guild_id, "424242424242424242").await;

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/discord-leave"),
            &json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "confirm_guild_id_mismatch");
    assert!(leaver.calls().is_empty());

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/discord-leave"),
            &json!({"confirmGuildId": GUILD}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["discordLeave"]["serverId"], "424242424242424242");
    assert_eq!(body["discordLeave"]["requested"], true);
    assert_eq!(body["discordLeave"]["outcome"], "left");
    assert_eq!(leaver.calls(), vec!["424242424242424242".to_owned()]);

    // The guild and its Discord link are untouched.
    let stored = db.guilds().access(&guild_id).await.unwrap().unwrap();
    assert_eq!(
        stored.discord_guild_id.as_deref(),
        Some("424242424242424242")
    );
    assert!(db.guilds().get(&guild_id).await.unwrap().is_some());
}

// ---------------------------------------------------------------------------
// Session and origin checks on every mutation
// ---------------------------------------------------------------------------

fn mutations() -> Vec<(Method, String, Value)> {
    let member = GuildMemberId::new_v4().to_canonical();
    let item = MediaBoardItemId::new_v4().to_canonical();
    let job = uuid::Uuid::new_v4().to_string();
    vec![
        (
            Method::PATCH,
            format!("/admin/api/guilds/{GUILD}/members/{member}"),
            json!({"role": "member"}),
        ),
        (
            Method::DELETE,
            format!("/admin/api/guilds/{GUILD}/members/{member}"),
            Value::Null,
        ),
        (
            Method::POST,
            format!("/admin/api/guilds/{GUILD}/invites"),
            json!({"role": "member"}),
        ),
        (
            Method::POST,
            format!("/admin/api/guilds/{GUILD}/invites/ABCDEFGH/revoke"),
            Value::Null,
        ),
        (
            Method::POST,
            format!("/admin/api/guilds/{GUILD}/discord-link-code"),
            Value::Null,
        ),
        (
            Method::POST,
            format!("/admin/api/guilds/{GUILD}/device-code"),
            json!({"memberId": member}),
        ),
        (
            Method::DELETE,
            format!("/admin/api/guilds/{GUILD}/queue"),
            Value::Null,
        ),
        (
            Method::DELETE,
            format!("/admin/api/guilds/{GUILD}/queue/{job}"),
            Value::Null,
        ),
        (
            Method::POST,
            format!("/admin/api/guilds/{GUILD}/queue/reorder"),
            json!({"jobIds": []}),
        ),
        (
            Method::PATCH,
            format!("/admin/api/guilds/{GUILD}/media-board/items/{item}"),
            json!({"title": "x"}),
        ),
        (
            Method::DELETE,
            format!("/admin/api/guilds/{GUILD}/media-board/items/{item}"),
            Value::Null,
        ),
        (
            Method::PATCH,
            format!("/admin/api/guilds/{GUILD}/settings"),
            json!({"name": "x"}),
        ),
        (
            Method::POST,
            format!("/admin/api/guilds/{GUILD}/discord-leave"),
            json!({"confirmGuildId": GUILD}),
        ),
    ]
}

#[tokio::test]
async fn mutations_require_a_session() {
    let harness = Harness::new().await;

    for (method, path, body) in mutations() {
        let request = if body.is_null() {
            harness.request_without_session(method.clone(), &path)
        } else {
            harness.request_json_without_session(method.clone(), &path, &body)
        };
        let (status, _, bytes) = harness.send(request).await;
        let payload = super::tests::json_or_null(&bytes);
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        assert_eq!(payload["error"], "unauthorized", "{method} {path}");
    }
}

#[tokio::test]
async fn mutations_reject_a_foreign_origin() {
    let harness = Harness::new().await;

    for (method, path, body) in mutations() {
        let mut request = mutation_request(&harness, method.clone(), &path, &body);
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        let (status, _, bytes) = harness.send(request).await;
        let payload = super::tests::json_or_null(&bytes);
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(payload["error"], "invalid_origin", "{method} {path}");
    }

    // The same requests from the panel origin are accepted by the CSRF layer
    // (their handler-level result is irrelevant here).
    for (method, path, body) in mutations() {
        let request = mutation_request(&harness, method.clone(), &path, &body);
        let (status, _, bytes) = harness.send(request).await;
        let payload = super::tests::json_or_null(&bytes);
        assert_ne!(payload["error"], "invalid_origin", "{method} {path}");
        assert_ne!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
}
