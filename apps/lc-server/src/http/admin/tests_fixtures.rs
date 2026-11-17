//! Contract fixtures shared with the admin console front end.
//!
//! Every response the console reads is produced here by the real router on a
//! realistic data set, then compared with the JSON file the front-end tests
//! load (`apps/lc-admin-web/src/api/fixtures/*.json`). The comparison is on the
//! *shape* (object keys and value types, recursively), not on the values,
//! because identifiers and dates change on every run.
//!
//! - a server change that renames, removes or retypes a field fails here;
//! - regenerate the files with `LC_WRITE_FIXTURES=1 cargo test -p lc-server
//!   tests_fixtures`, then the front-end tests tell whether the console still
//!   understands the new payloads.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use lc_domain::{
    GuildBan, GuildBanId, GuildId, GuildMemberId, GuildRole, IngestClient, IngestClientId,
    JobStatus, MediaKind, OverlaySessionMode, PairingCode, PairingCodeEntry, PairingMode,
};
use serde_json::{json, Value};

use super::tests::{
    board_item, create_linked_standalone, guild, guild_invite, guild_member, media_asset,
    overlay_client, playback_job, FakeLeaveResult, FakeLeaver, Harness,
};

const GUILD: &str = "112233445566778899";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../lc-admin-web/src/api/fixtures")
}

/// Recursive shape: objects keep their sorted keys, arrays the union of their
/// element shapes, scalars their JSON type. `null` is its own type so that a
/// field becoming nullable (or no longer nullable) is noticed.
fn shape(value: &Value) -> Value {
    match value {
        Value::Null => json!("null"),
        Value::Bool(_) => json!("bool"),
        Value::Number(_) => json!("number"),
        Value::String(_) => json!("string"),
        Value::Array(items) => {
            let mut variants: Vec<Value> = Vec::new();
            for item in items {
                let item_shape = shape(item);
                if !variants.contains(&item_shape) {
                    variants.push(item_shape);
                }
            }
            Value::Array(variants)
        }
        Value::Object(map) => {
            let sorted: BTreeMap<&String, Value> =
                map.iter().map(|(key, item)| (key, shape(item))).collect();
            json!(sorted)
        }
    }
}

/// Writes (`LC_WRITE_FIXTURES=1`) or checks one fixture.
fn fixture(name: &str, body: &Value) {
    let path = fixtures_dir().join(format!("{name}.json"));
    if std::env::var("LC_WRITE_FIXTURES").as_deref() == Ok("1") {
        std::fs::create_dir_all(fixtures_dir()).unwrap();
        let mut text = serde_json::to_string_pretty(body).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        return;
    }
    let stored = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "fixture {} missing ({error}); run with LC_WRITE_FIXTURES=1",
            path.display()
        )
    });
    let stored: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(
        shape(body),
        shape(&stored),
        "the shape of `{name}` changed: regenerate the fixtures with LC_WRITE_FIXTURES=1 \
         and run the admin console tests (npm test in apps/lc-admin-web)"
    );
}

async fn get_ok(harness: &Harness, path: &str) -> Value {
    let (status, body) = harness.call(Method::GET, path).await;
    assert_eq!(status, StatusCode::OK, "GET {path}: {body}");
    body
}

/// A server with two guilds (a Discord one and a standalone one), members of
/// every role, connected and idle overlays, a queue with a playing and a
/// pending job, board media of several kinds, invites, pairing codes, an
/// ingestion client and audit entries.
async fn seeded_harness() -> (Harness, String) {
    let harness = Harness::with_pairs(&[("MEDIA_URL_SECRET", "fixture-media-secret")]).await;
    let db = harness.db();
    let guild_id = GuildId::parse(GUILD).unwrap();

    let mut discord_guild = guild(GUILD);
    discord_guild.name = Some("Les Copains du Jeudi".to_owned());
    db.guilds().upsert(&discord_guild).await.unwrap();
    // The standalone guild is created like the console does it.
    let (status, created) = harness
        .call_json(
            Method::POST,
            "/admin/api/guilds",
            &json!({ "name": "Studio LiveChat" }),
        )
        .await;
    assert!(status.is_success(), "{status} {created}");
    fixture("create-guild", &created);
    let standalone = created["guild"]["id"].as_str().unwrap().to_owned();

    let mut members = Vec::new();
    for (name, role) in [
        ("Quentin", GuildRole::Admin),
        ("Maëlle", GuildRole::Member),
        ("Théo", GuildRole::Guest),
    ] {
        let mut member = guild_member(&guild_id, role);
        member.display_name = name.to_owned();
        db.guild_members().create(&member).await.unwrap();
        members.push(member);
    }

    for (index, member) in members.iter().enumerate() {
        let mode = if member.role == GuildRole::Guest {
            OverlaySessionMode::InviteReadOnly
        } else {
            OverlaySessionMode::Normal
        };
        let mut client = overlay_client(&guild_id, mode);
        client.label = format!("PC-{}", ["Salon", "Bureau", "Portable"][index]);
        client.last_seen_at = Some(Utc::now() - Duration::minutes(5));
        db.overlay_clients().create(&client).await.unwrap();
        db.overlay_clients()
            .set_member(client.id, member.id)
            .await
            .unwrap();
    }

    db.guild_invites()
        .create(&guild_invite("ABCDEFGH", &guild_id))
        .await
        .unwrap();
    db.guild_bans()
        .create(&GuildBan {
            id: GuildBanId::new_v4(),
            guild_id: guild_id.clone(),
            member_id: Some(GuildMemberId::new_v4()),
            discord_user_id: Some("1009".to_owned()),
            display_name: "Ancien membre".to_owned(),
            avatar: None,
            banned_by: Some("Quentin".to_owned()),
            created_at: Utc::now() - Duration::days(1),
        })
        .await
        .unwrap();

    for (index, kind) in [MediaKind::Video, MediaKind::Audio, MediaKind::Image]
        .into_iter()
        .enumerate()
    {
        // Under MEDIA_STORAGE_DIR, where the media routes and the editor look.
        let media_dir = harness.state().config.media_storage_dir.clone();
        std::fs::create_dir_all(&media_dir).unwrap();
        let file = media_dir.join(format!("media-{index}.bin"));
        std::fs::write(&file, vec![0_u8; 2048]).unwrap();
        let mut asset = media_asset(2_000_000 + i64::try_from(index).unwrap(), &file);
        asset.kind = kind;
        db.media_assets().insert(&asset).await.unwrap();
        let mut item = board_item(&guild_id, asset.id);
        item.title = Some(["Clip du but", "Applaudissements", "Mème"][index].to_owned());
        db.media_board().create(&item).await.unwrap();
    }

    let mut playing = playback_job(&guild_id, JobStatus::Playing);
    playing.started_at = Some(Utc::now() - Duration::seconds(3));
    playing.text = Some("Bonne soirée à tous".to_owned());
    db.playback_jobs().create(&playing).await.unwrap();
    db.playback_jobs()
        .create(&playback_job(&guild_id, JobStatus::Pending))
        .await
        .unwrap();

    let now = Utc::now();
    for (code, expires, used) in [
        ("JKLMNPQR", Some(now + Duration::minutes(10)), None),
        ("STUVWXYZ", Some(now - Duration::minutes(10)), None),
    ] {
        db.pairing_codes()
            .create(&PairingCodeEntry {
                code: PairingCode::parse(code).unwrap(),
                guild_id: guild_id.clone(),
                mode: PairingMode::Normal,
                grants_admin: false,
                created_by_discord_user_id: "1001".to_owned(),
                author_name: Some("Quentin".to_owned()),
                author_image: None,
                expires_at: expires,
                used_at: used,
                created_at: now,
            })
            .await
            .unwrap();
    }

    db.ingest_clients()
        .create(&IngestClient {
            id: IngestClientId::new_v4(),
            guild_id: guild_id.clone(),
            label: "Extension Firefox".to_owned(),
            token_hash: format!("hash-{}", IngestClientId::new_v4()),
            default_author_name: "Maëlle".to_owned(),
            default_author_image: None,
            created_by_discord_user_id: "1002".to_owned(),
            last_seen_at: Some(now - Duration::hours(2)),
            created_at: now - Duration::days(3),
            revoked_at: None,
        })
        .await
        .unwrap();

    // Two admin mutations so that the audit journal is not empty.
    let (status, body) = harness
        .call_json(
            Method::PATCH,
            &format!("/admin/api/guilds/{GUILD}/settings"),
            &json!({ "defaultMediaTime": 8 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/admin/api/guilds/{standalone}/stop"),
        )
        .await;
    assert!(status.is_success(), "{status} {body}");

    (harness, standalone)
}

#[tokio::test]
async fn admin_console_contract_fixtures() {
    let (harness, standalone) = seeded_harness().await;

    fixture("session", &get_ok(&harness, "/admin/auth/session").await);
    fixture("overview", &get_ok(&harness, "/admin/api/overview").await);
    fixture(
        "guild-detail",
        &get_ok(&harness, &format!("/admin/api/guilds/{GUILD}")).await,
    );
    fixture(
        "guild-detail-standalone",
        &get_ok(&harness, &format!("/admin/api/guilds/{standalone}")).await,
    );
    fixture(
        "media-board-items",
        &get_ok(
            &harness,
            &format!("/admin/api/guilds/{GUILD}/media-board/items"),
        )
        .await,
    );
    fixture("health", &get_ok(&harness, "/admin/api/health").await);
    fixture(
        "ingestions",
        &get_ok(&harness, "/admin/api/ingestions?limit=50").await,
    );
    fixture(
        "audit",
        &get_ok(&harness, "/admin/api/audit?limit=100").await,
    );
    fixture(
        "pairing-codes",
        &get_ok(&harness, "/admin/api/pairing-codes").await,
    );
    fixture(
        "ingest-authors",
        &get_ok(&harness, "/admin/api/ingest-authors").await,
    );
    fixture("rights", &get_ok(&harness, "/admin/api/rights").await);
    let mut runtime = get_ok(&harness, "/admin/api/runtime-settings").await;
    // The harness media folder is a local temp path: keep machine paths out of
    // the committed fixture.
    runtime["readOnly"]["mediaStorageDir"] = json!("/app/data/media");
    fixture("runtime-settings", &runtime);

    let (status, _, bytes) = harness
        .send(harness.request_without_session(Method::GET, "/admin/auth/session"))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    fixture(
        "session-unauthorized",
        &serde_json::from_slice::<Value>(&bytes).unwrap(),
    );
}

async fn mutate(harness: &Harness, method: Method, path: &str, body: Option<Value>) -> Value {
    let (status, response) = match body {
        Some(body) => harness.call_json(method.clone(), path, &body).await,
        None => harness.call(method.clone(), path).await,
    };
    assert!(status.is_success(), "{method} {path}: {status} {response}");
    response
}

fn first_id(list: &Value) -> String {
    list[0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no id in {list}"))
        .to_owned()
}

/// Every mutation response the console reads, recorded like the reads above
/// (`mutation-*.json`) and checked against the front-end types at compile
/// time (`src/api/fixtures.check.ts`).
#[tokio::test]
async fn admin_console_mutation_fixtures() {
    let (harness, standalone) = seeded_harness().await;
    let base = format!("/admin/api/guilds/{GUILD}");
    let detail = get_ok(&harness, &base).await;
    // The guest: changing or removing the only admin is refused (`last_admin`).
    let member = detail["members"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["role"] == "guest")
        .and_then(|entry| entry["id"].as_str())
        .unwrap()
        .to_owned();
    let overlay = first_id(&detail["overlays"]["clients"]);
    let ingest = first_id(&detail["ingest"]["clients"]);
    let job = first_id(&detail["queue"]["pending"]);
    let board = get_ok(&harness, &format!("{base}/media-board/items")).await;
    // The video: images are edited on the board itself, not by the editor.
    let item = board["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["media"]["kind"] == "video")
        .and_then(|entry| entry["id"].as_str())
        .unwrap()
        .to_owned();
    let authors = get_ok(&harness, "/admin/api/ingest-authors").await;

    let record = |name: &str, body: &Value| fixture(&format!("mutation-{name}"), body);

    record(
        "update-guild-settings",
        &mutate(
            &harness,
            Method::PATCH,
            &format!("{base}/settings"),
            Some(json!({ "defaultMediaTime": 7 })),
        )
        .await,
    );
    record(
        "admin-code",
        &mutate(&harness, Method::POST, &format!("{base}/admin-code"), None).await,
    );
    record(
        "create-invite",
        &mutate(
            &harness,
            Method::POST,
            &format!("{base}/invites"),
            Some(json!({ "role": "member", "expiresInHours": 24, "maxUses": null })),
        )
        .await,
    );
    record(
        "revoke-invite",
        &mutate(
            &harness,
            Method::POST,
            &format!("{base}/invites/ABCDEFGH/revoke"),
            None,
        )
        .await,
    );
    record(
        "discord-link-code",
        &mutate(
            &harness,
            Method::POST,
            &format!("/admin/api/guilds/{standalone}/discord-link-code"),
            Some(json!({})),
        )
        .await,
    );
    record(
        "device-code",
        &mutate(
            &harness,
            Method::POST,
            &format!("{base}/device-code"),
            Some(json!({ "memberId": member })),
        )
        .await,
    );
    record(
        "patch-member",
        &mutate(
            &harness,
            Method::PATCH,
            &format!("{base}/members/{member}"),
            Some(json!({ "role": "member" })),
        )
        .await,
    );
    record(
        "patch-board-item",
        &mutate(
            &harness,
            Method::PATCH,
            &format!("{base}/media-board/items/{item}"),
            Some(json!({ "title": "Nouveau titre" })),
        )
        .await,
    );
    record(
        "edit-board-item",
        &mutate(
            &harness,
            Method::PATCH,
            &format!("{base}/media-board/items/{item}"),
            Some(json!({ "replace": true, "recipe": { "segments": [[0.5, 1.5]] } })),
        )
        .await,
    );
    // The edit runs in the background on the video file: let it finish before
    // the item is deleted, so the cache purge below never meets a file the
    // edit still holds open (Windows refuses to delete it).
    for _ in 0..500 {
        if harness.state().ingestions.active(Utc::now()).is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        harness.state().ingestions.active(Utc::now()).is_empty(),
        "the board edit is still running"
    );
    record(
        "delete-board-item",
        &mutate(
            &harness,
            Method::DELETE,
            &format!("{base}/media-board/items/{item}"),
            None,
        )
        .await,
    );
    record(
        "remove-queue-job",
        &mutate(
            &harness,
            Method::DELETE,
            &format!("{base}/queue/{job}"),
            None,
        )
        .await,
    );
    record(
        "clear-queue",
        &mutate(&harness, Method::DELETE, &format!("{base}/queue"), None).await,
    );
    record(
        "update-overlay-client",
        &mutate(
            &harness,
            Method::PATCH,
            &format!("/admin/api/overlay-clients/{overlay}"),
            Some(json!({ "label": "PC-Cuisine", "sessionMode": "normal" })),
        )
        .await,
    );
    record(
        "overlay-guild-settings",
        &mutate(
            &harness,
            Method::PATCH,
            &format!("/admin/api/overlay-clients/{overlay}/guild-settings"),
            Some(json!({ "volume": 0.5 })),
        )
        .await,
    );
    record(
        "revoke-overlay-client",
        &mutate(
            &harness,
            Method::POST,
            &format!("/admin/api/overlay-clients/{overlay}/revoke"),
            None,
        )
        .await,
    );
    let author = &authors["items"][0];
    record(
        "create-ingest-client",
        &mutate(
            &harness,
            Method::POST,
            "/admin/api/ingest-clients",
            Some(json!({
                "guildId": GUILD,
                "authorDiscordUserId": author["discordUserId"],
                "label": "Extension Chrome",
            })),
        )
        .await,
    );
    record(
        "revoke-ingest-client",
        &mutate(
            &harness,
            Method::POST,
            &format!("/admin/api/ingest-clients/{ingest}/revoke"),
            None,
        )
        .await,
    );
    record(
        "revoke-pairing-code",
        &mutate(
            &harness,
            Method::POST,
            "/admin/api/pairing-codes/JKLMNPQR/revoke",
            None,
        )
        .await,
    );
    record(
        "delete-expired-pairing-codes",
        &mutate(
            &harness,
            Method::DELETE,
            "/admin/api/pairing-codes/expired",
            None,
        )
        .await,
    );
    record(
        "purge-media-cache",
        &mutate(&harness, Method::POST, "/admin/api/media/cache/purge", None).await,
    );
    record(
        "clean-media-orphans",
        &mutate(
            &harness,
            Method::POST,
            "/admin/api/media/orphans/clean",
            None,
        )
        .await,
    );
    record(
        "stop-guild",
        &mutate(&harness, Method::POST, &format!("{base}/stop"), None).await,
    );
    let confirm = Some(json!({ "confirmGuildId": GUILD }));
    let banned = detail["members"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["role"] == "member")
        .and_then(|entry| entry["id"].as_str())
        .unwrap()
        .to_owned();
    let ban = mutate(
        &harness,
        Method::POST,
        &format!("{base}/members/{banned}/ban"),
        None,
    )
    .await;
    record("ban-member", &ban);
    let ban_id = ban["ban"]["id"].as_str().unwrap().to_owned();
    record(
        "unban",
        &mutate(
            &harness,
            Method::DELETE,
            &format!("{base}/bans/{ban_id}"),
            None,
        )
        .await,
    );
    record(
        "remove-member",
        &mutate(
            &harness,
            Method::DELETE,
            &format!("{base}/members/{member}"),
            None,
        )
        .await,
    );
    record(
        "revoke-guild-clients",
        &mutate(
            &harness,
            Method::POST,
            &format!("{base}/revoke-clients"),
            confirm.clone(),
        )
        .await,
    );
    record(
        "clear-media-board",
        &mutate(
            &harness,
            Method::POST,
            &format!("{base}/media-board/clear"),
            confirm.clone(),
        )
        .await,
    );
    record(
        "purge-guild",
        &mutate(&harness, Method::POST, &format!("{base}/purge"), confirm).await,
    );
}

/// Discord unlink and leave need a linked standalone guild and a bot that
/// can leave (a fake one here), with the very bodies the console sends.
#[tokio::test]
async fn admin_console_discord_mutation_fixtures() {
    let harness = Harness::with_leaver(FakeLeaver::new(FakeLeaveResult::Left)).await;
    let linked = GuildId::parse("900000000000000001").unwrap();
    create_linked_standalone(harness.db(), &linked, "123456789012345678").await;
    harness.db().guilds().upsert(&guild(GUILD)).await.unwrap();

    fixture(
        "mutation-discord-leave",
        &mutate(
            &harness,
            Method::POST,
            &format!("/admin/api/guilds/{GUILD}/discord-leave"),
            Some(json!({ "confirmGuildId": GUILD })),
        )
        .await,
    );
    fixture(
        "mutation-discord-unlink",
        &mutate(
            &harness,
            Method::POST,
            &format!("/admin/api/guilds/{linked}/discord-unlink"),
            Some(json!({ "leaveDiscord": false })),
        )
        .await,
    );
}

#[test]
fn shape_ignores_values_but_not_types_or_keys() {
    assert_eq!(
        shape(&json!({"a": 1, "b": "x"})),
        shape(&json!({"b": "y", "a": 2}))
    );
    assert_ne!(shape(&json!({"a": 1})), shape(&json!({"a": "1"})));
    assert_ne!(shape(&json!({"a": 1})), shape(&json!({"a": 1, "b": 2})));
    assert_ne!(shape(&json!({"a": null})), shape(&json!({"a": 1})));
    assert_eq!(shape(&json!([1, 2, 3])), shape(&json!([4])));
}
