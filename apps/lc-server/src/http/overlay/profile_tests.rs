//! Tests of the profile linked to a Discord account: `GET /overlay/guild`,
//! `PATCH /overlay/profile` (`useDiscord`) and `refresh_member`, with a fake
//! Discord source (no network).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Duration, TimeZone, Utc};
use lc_db::{Db, DbConfig};
use lc_domain::{
    FixedClock, GuildId, GuildMember, GuildMemberId, GuildRole, PairingCode, PairingCodeEntry,
    PairingMode,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt as _;

use crate::app::AppState;
use crate::config::Config;
use crate::discord::profiles::{
    refresh_member, DiscordProfile, DiscordProfileSource, ProfileError, RefreshOutcome,
};
use crate::shutdown::Shutdown;

/// Discord server of the tests (native guild).
const DISCORD_GUILD: &str = "123456789012345678";
/// Discord account issuing the personal `/pair-code`.
const DISCORD_USER: &str = "4242";

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .single()
        .unwrap()
}

fn native_guild() -> GuildId {
    GuildId::parse(DISCORD_GUILD).unwrap()
}

/// What the fake Discord answers.
#[derive(Clone, Debug)]
enum Answer {
    Profile(DiscordProfile),
    /// The account is not in the server.
    Gone,
    /// The API call fails.
    Fail,
}

/// Fake Discord recording the `(guild, user)` reads.
#[derive(Debug)]
struct FakeProfiles {
    answer: Mutex<Answer>,
    calls: Mutex<Vec<(String, String)>>,
}

impl FakeProfiles {
    fn answering(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(answer),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn set(&self, answer: Answer) {
        *self.answer.lock().unwrap() = answer;
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl DiscordProfileSource for FakeProfiles {
    async fn member_profile(
        &self,
        discord_guild_id: &str,
        user_id: &str,
    ) -> Result<Option<DiscordProfile>, ProfileError> {
        self.calls
            .lock()
            .unwrap()
            .push((discord_guild_id.to_owned(), user_id.to_owned()));
        match self.answer.lock().unwrap().clone() {
            Answer::Profile(profile) => Ok(Some(profile)),
            Answer::Gone => Ok(None),
            Answer::Fail => Err(ProfileError::Api("503 Service Unavailable".to_owned())),
        }
    }
}

fn discord_profile(name: &str, image: Option<&str>) -> Answer {
    Answer::Profile(DiscordProfile {
        name: name.to_owned(),
        image: image.map(str::to_owned),
    })
}

struct Harness {
    app: Router,
    db: Db,
    discord: Arc<FakeProfiles>,
    _media: TempDir,
}

impl Harness {
    async fn new(answer: Answer) -> Self {
        let media = TempDir::new().unwrap();
        let config = Config::from_pairs(vec![
            ("API_URL".to_owned(), "http://127.0.0.1:8099".to_owned()),
            (
                "MEDIA_URL_SECRET".to_owned(),
                "secret-test-profile".to_owned(),
            ),
            (
                "MEDIA_STORAGE_DIR".to_owned(),
                media.path().to_string_lossy().into_owned(),
            ),
        ])
        .unwrap();
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let clock: Arc<dyn lc_domain::Clock> = Arc::new(FixedClock::new(fixed_now()));
        let discord = FakeProfiles::answering(answer);
        let source: Arc<dyn DiscordProfileSource> = discord.clone();
        let state = AppState::new(Arc::new(config), Some(db.clone()), Shutdown::detached())
            .with_clock(clock)
            .with_discord_profiles(source);
        Self {
            app: crate::app::router(state),
            db,
            discord,
            _media: media,
        }
    }

    async fn call(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Creates a `/pair-code` and pairs an overlay with it; returns the
    /// client token.
    async fn pair(&self, code: &str, mode: PairingMode, display_name: Option<&str>) -> String {
        self.db
            .pairing_codes()
            .create(&PairingCodeEntry {
                code: PairingCode::parse(code).unwrap(),
                guild_id: native_guild(),
                mode,
                grants_admin: false,
                created_by_discord_user_id: DISCORD_USER.to_owned(),
                author_name: Some("Discordeur".to_owned()),
                author_image: None,
                expires_at: Some(fixed_now() + Duration::minutes(5)),
                used_at: None,
                created_at: fixed_now() - Duration::minutes(1),
            })
            .await
            .unwrap();
        let mut body = json!({"code": code});
        if let Some(name) = display_name {
            body["displayName"] = json!(name);
        }
        let (status, paired) = self
            .call(Method::POST, "/overlay/pair/consume", None, Some(body))
            .await;
        assert_eq!(status, StatusCode::OK, "{paired}");
        paired["clientToken"].as_str().unwrap().to_owned()
    }

    async fn member(&self, token: &str) -> Value {
        let (status, guild) = self
            .call(Method::GET, "/overlay/guild", Some(token), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{guild}");
        guild["member"].clone()
    }
}

#[tokio::test]
async fn the_profile_follows_discord_until_the_member_picks_one() {
    let harness = Harness::new(discord_profile("Pseudo serveur", None)).await;
    let token = harness.pair("MINE01", PairingMode::Normal, None).await;

    let member = harness.member(&token).await;
    assert_eq!(member["displayName"], "Discordeur");
    assert_eq!(member["discordLinked"], true);
    assert_eq!(member["profileSource"], "discord");

    // « Mon profil »: the chosen name wins from now on.
    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/profile",
            Some(&token),
            Some(json!({"displayName": "Choisi"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["member"]["displayName"], "Choisi");
    assert_eq!(body["member"]["profileSource"], "custom");
    assert_eq!(body["member"]["discordLinked"], true);
    let member = harness.member(&token).await;
    assert_eq!(member["displayName"], "Choisi");
    assert_eq!(member["profileSource"], "custom");
    assert!(harness.discord.calls().is_empty());

    // « Reprendre mon profil Discord »: read from Discord right away.
    harness.discord.set(discord_profile(
        "Nouveau pseudo",
        Some("https://cdn.discordapp.com/avatars/4242/abc.png"),
    ));
    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/profile",
            Some(&token),
            Some(json!({"useDiscord": true, "displayName": "Ignoré"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["synced"], true);
    assert_eq!(body["member"]["displayName"], "Nouveau pseudo");
    assert_eq!(
        body["member"]["avatar"],
        "https://cdn.discordapp.com/avatars/4242/abc.png"
    );
    assert_eq!(body["member"]["profileSource"], "discord");
    assert_eq!(
        harness.discord.calls(),
        vec![(DISCORD_GUILD.to_owned(), DISCORD_USER.to_owned())]
    );
    let member = harness.member(&token).await;
    assert_eq!(member["displayName"], "Nouveau pseudo");
    assert_eq!(member["profileSource"], "discord");
}

#[tokio::test]
async fn following_discord_again_keeps_the_profile_when_discord_is_unreachable() {
    let harness = Harness::new(Answer::Fail).await;
    let token = harness.pair("MINE02", PairingMode::Normal, None).await;
    let (status, _) = harness
        .call(
            Method::PATCH,
            "/overlay/profile",
            Some(&token),
            Some(json!({"displayName": "Choisi"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/profile",
            Some(&token),
            Some(json!({"useDiscord": true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Following again, but Discord will be read on the next occasion.
    assert_eq!(body["synced"], false);
    assert_eq!(body["member"]["displayName"], "Choisi");
    assert_eq!(body["member"]["profileSource"], "discord");
    assert_eq!(harness.discord.calls().len(), 1);
}

#[tokio::test]
async fn an_unlinked_member_cannot_follow_discord() {
    let harness = Harness::new(discord_profile("Admin Discord", None)).await;
    // A read-only invitation never borrows its author's account.
    let token = harness
        .pair("GUEST1", PairingMode::InviteReadOnly, Some("Spectateur"))
        .await;
    let member = harness.member(&token).await;
    assert_eq!(member["discordLinked"], false);
    assert_eq!(member["profileSource"], "custom");

    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/profile",
            Some(&token),
            Some(json!({"useDiscord": true})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "discord_not_linked");
    assert_eq!(harness.member(&token).await["displayName"], "Spectateur");
    assert!(harness.discord.calls().is_empty());
}

/// Member following the Discord account [`DISCORD_USER`].
async fn following_member(
    db: &Db,
    guild_id: &GuildId,
    synced_at: Option<DateTime<Utc>>,
) -> GuildMember {
    let member = GuildMember {
        id: GuildMemberId::new_v4(),
        guild_id: guild_id.clone(),
        display_name: "Ancien pseudo".to_owned(),
        avatar: None,
        role: GuildRole::Member,
        created_at: fixed_now() - Duration::days(1),
        removed_at: None,
        discord_user_id: Some(DISCORD_USER.to_owned()),
        profile_custom: false,
        discord_synced_at: synced_at,
    };
    db.guild_members().create(&member).await.unwrap();
    member
}

async fn memory_db() -> Db {
    Db::open_in_memory(&DbConfig::default()).await.unwrap()
}

#[tokio::test]
async fn refresh_member_reads_discord_at_most_every_interval() {
    let db = memory_db().await;
    let discord = FakeProfiles::answering(discord_profile("Pseudo serveur", None));
    let now = fixed_now();
    let member = following_member(&db, &native_guild(), Some(now - Duration::hours(1))).await;

    let outcome = refresh_member(&db, discord.as_ref(), &member, now, false).await;
    assert_eq!(outcome, RefreshOutcome::Fresh);
    assert!(discord.calls().is_empty());

    let later = now + Duration::hours(6);
    let outcome = refresh_member(&db, discord.as_ref(), &member, later, false).await;
    assert_eq!(outcome, RefreshOutcome::Updated);
    let stored = db.guild_members().get(member.id).await.unwrap().unwrap();
    assert_eq!(stored.display_name, "Pseudo serveur");
    assert_eq!(stored.discord_synced_at, Some(later));

    // `force` (« Reprendre mon profil Discord ») ignores the interval.
    discord.set(discord_profile("Encore", None));
    let outcome = refresh_member(&db, discord.as_ref(), &stored, later, true).await;
    assert_eq!(outcome, RefreshOutcome::Updated);
    assert_eq!(discord.calls().len(), 2);
}

#[tokio::test]
async fn refresh_member_keeps_the_profile_of_an_account_that_left() {
    let db = memory_db().await;
    let discord = FakeProfiles::answering(Answer::Gone);
    let member = following_member(&db, &native_guild(), None).await;

    let outcome = refresh_member(&db, discord.as_ref(), &member, fixed_now(), false).await;
    assert_eq!(outcome, RefreshOutcome::OutOfReach);
    let stored = db.guild_members().get(member.id).await.unwrap().unwrap();
    assert_eq!(stored.display_name, "Ancien pseudo");
    // Not asked again before the interval.
    assert_eq!(stored.discord_synced_at, Some(fixed_now()));
    assert!(stored.follows_discord());

    // A failing API changes nothing.
    discord.set(Answer::Fail);
    let outcome = refresh_member(&db, discord.as_ref(), &member, fixed_now(), true).await;
    assert_eq!(outcome, RefreshOutcome::Unavailable);
}

#[tokio::test]
async fn refresh_member_of_a_standalone_guild_uses_its_discord_server() {
    let db = memory_db().await;
    let discord = FakeProfiles::answering(discord_profile("Pseudo serveur", None));
    let standalone = GuildId::generate();
    db.guilds()
        .create_standalone(&standalone, "Les Sardines", fixed_now())
        .await
        .unwrap();
    let member = following_member(&db, &standalone, None).await;

    // No Discord server linked: out of reach, Discord is not asked.
    let outcome = refresh_member(&db, discord.as_ref(), &member, fixed_now(), true).await;
    assert_eq!(outcome, RefreshOutcome::OutOfReach);
    assert!(discord.calls().is_empty());

    db.guilds()
        .link_discord(&standalone, Some(&native_guild()))
        .await
        .unwrap();
    let outcome = refresh_member(&db, discord.as_ref(), &member, fixed_now(), true).await;
    assert_eq!(outcome, RefreshOutcome::Updated);
    assert_eq!(
        discord.calls(),
        vec![(DISCORD_GUILD.to_owned(), DISCORD_USER.to_owned())]
    );
}

#[tokio::test]
async fn refresh_member_skips_a_custom_profile() {
    let db = memory_db().await;
    let discord = FakeProfiles::answering(discord_profile("Pseudo serveur", None));
    let mut member = following_member(&db, &native_guild(), None).await;
    member.profile_custom = true;

    let outcome = refresh_member(&db, discord.as_ref(), &member, fixed_now(), true).await;
    assert_eq!(outcome, RefreshOutcome::NotFollowing);
    assert!(discord.calls().is_empty());
}
