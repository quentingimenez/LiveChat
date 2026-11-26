//! Property-based tests for the frozen protocol v2 contract.
//!
//! Every property below was written against the semantics actually encoded in
//! `src/lib.rs`, read first:
//! - `is_frame_too_large(len)` is `len > MAX_FRAME_BYTES`: the 64 Kio limit is
//!   inclusive, so a frame of exactly `MAX_FRAME_BYTES` is accepted and the
//!   guard flips only strictly above it;
//! - `close_code`/`close_reason` are frozen constant modules (this crate
//!   exposes no `close_reason(code)` function); the repo's total
//!   code-to-reason mapping is wired in the server
//!   (`apps/lc-server/src/ws/hub.rs::close_reason_for_code`);
//! - `media_signature_input` is the frozen `assetId|clientId|exp` HMAC input.
//!
//! Generated values mirror the vocabulary of the golden fixture set in
//! `tests/fixtures/`, and `envelope_roundtrip` asserts `serde_json::Value`
//! equality exactly the way `tests/fixtures_roundtrip.rs` does.

use std::fmt::Debug;

use proptest::collection::vec;
use proptest::option::of;
use proptest::prelude::*;
use proptest::test_runner::{TestCaseError, TestCaseResult};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};

use lc_protocol::{
    is_frame_too_large, is_version_below, media_signature_input, parse_simple_version, AckError,
    AckOutcome, AckPayload, AdminChangeKind, AdminChangedPayload, AuthorPayload, BoardChange,
    BoardChangedPayload, ClientErrorPayload, ClientMessage, CurrentJobPayload,
    DeviceSettingsPayload, DeviceSettingsStatePayload, Envelope, HeartbeatPayload, HelloPayload,
    MediaIngestionError, MediaIngestionPayload, MediaKind, MediaPayload, MediaTrigger,
    MediaTriggerPayload, PeerInfo, PeersPayload, PeersRequestPayload, Platform, PlayPayload,
    PlaybackEndedPayload, PlaybackEndedReason, PlaybackState, PlaybackStatePayload,
    RightsChangedPayload, ServerErrorPayload, ServerMessage, SessionChangedPayload, SessionMode,
    SettingsPayload, StopPayload, StopReason, StopRequestPayload, StopRequestSource, TextPayload,
    TweetCardPayload, TweetCardVideo, WelcomeConfig, WelcomePayload, MAX_FRAME_BYTES,
    PROTOCOL_VERSION,
};

// ---------------------------------------------------------------------------
// Value strategies: plausible field strings mirroring tests/fixtures/
// ---------------------------------------------------------------------------

/// Printable alphabet for fixture-style free text (labels, messages, values).
const TEXT_ALPHABET: &[u8] = b" abcdefghijklmnopqrstuvwxyz0123456789!?-.";

/// Free text shaped like the fixtures' labels, messages and captions.
fn short_text() -> impl Strategy<Value = String> {
    vec(any::<u8>(), 0..=48).prop_map(|bytes| {
        bytes
            .iter()
            .map(|byte| TEXT_ALPHABET[usize::from(*byte) % TEXT_ALPHABET.len()] as char)
            .collect()
    })
}

/// cuid-style opaque identifier, as in "clx7q2x0b0001abcd1234efgh".
fn cuid() -> impl Strategy<Value = String> {
    vec(any::<u8>(), 16).prop_map(|bytes| {
        const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
        let suffix: String = bytes
            .iter()
            .map(|byte| ALPHABET[usize::from(*byte) % ALPHABET.len()] as char)
            .collect();
        format!("clx7q2x0b{suffix}")
    })
}

/// Identifier drawn from the frozen id vocabulary `[a-z0-9-]{1,40}` used by
/// asset and client ids. That vocabulary excludes `|`, which is what keeps
/// the `assetId|clientId|exp` signature input unambiguous.
fn signature_id() -> impl Strategy<Value = String> {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz-";
    vec(any::<u8>(), 1..=40).prop_map(|bytes| {
        bytes
            .iter()
            .map(|byte| ALPHABET[usize::from(*byte) % ALPHABET.len()] as char)
            .collect()
    })
}

/// Discord-snowflake-like guild identifier (decimal digits).
fn snowflake() -> impl Strategy<Value = String> {
    any::<u64>().prop_map(|id| id.to_string())
}

/// Pairing token: 64 lowercase hex characters, like `v2_hello_normal.json`.
fn token() -> impl Strategy<Value = String> {
    vec(any::<u8>(), 32).prop_map(|bytes| bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Simple semver string, like the fixtures' "3.0.0".
fn version_string() -> impl Strategy<Value = String> {
    (0u16..=99, 0u16..=99, 0u16..=99)
        .prop_map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
}

/// Envelope frame id, like "cli-hello-0001" / "srv-play-0001".
fn frame_id(prefix: &'static str) -> impl Strategy<Value = String> {
    (0u32..=9_999).prop_map(move |seq| format!("{prefix}-{seq:04}"))
}

/// Unix timestamp in milliseconds, like the fixtures' 1794182400000.
fn unix_ms() -> impl Strategy<Value = i64> {
    1_700_000_000_000..=2_000_000_000_000_i64
}

/// Signed media URL, like the one in `v2_play_media_resume.json`.
fn media_url() -> impl Strategy<Value = String> {
    cuid().prop_map(|asset| {
        format!("https://api.livechat.example/overlay/media/{asset}?sig=9f2c4e6a8b1d")
    })
}

/// Guild icon URL, like the Discord CDN links of the welcome fixtures.
fn icon_url() -> impl Strategy<Value = String> {
    (snowflake(), snowflake())
        .prop_map(|(guild, hash)| format!("https://cdn.discordapp.com/icons/{guild}/{hash}.png"))
}

/// Tweet URL, like "https://x.com/livechat/status/1234567890123456789".
fn tweet_url() -> impl Strategy<Value = String> {
    (short_text(), snowflake())
        .prop_map(|(author, status)| format!("https://x.com/{author}/status/{status}"))
}

/// Tweet embed HTML, like the blockquote of `v2_play_tweet_card.json`.
fn tweet_html() -> impl Strategy<Value = String> {
    short_text().prop_map(|text| format!("<blockquote class=\"twitter-tweet\">{text}</blockquote>"))
}

/// Opaque ingestion id (UUID-shaped in `v2_media_ingestion_ready.json`).
fn ingestion_id() -> impl Strategy<Value = String> {
    vec(any::<u8>(), 16).prop_map(|bytes| {
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        format!("ing-{hex}")
    })
}

/// MIME vocabulary of the fixtures' media payloads.
fn mime() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("image/png".to_owned()),
        Just("image/jpeg".to_owned()),
        Just("video/mp4".to_owned()),
        Just("audio/mpeg".to_owned()),
        Just("audio/ogg".to_owned()),
    ]
}

/// Machine error-code vocabulary documented in the payload doc comments.
fn error_code() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("render_failed".to_owned()),
        Just("dispatch_failed".to_owned()),
        Just("read_only".to_owned()),
        Just("media_ingestion_failed".to_owned()),
        Just("unsupported_media".to_owned()),
        Just("internal_error".to_owned()),
    ]
}

/// Guild volume within the documented `0.0..=1.0` range.
fn volume() -> impl Strategy<Value = f64> {
    any::<u8>().prop_map(|byte| f64::from(byte) / 255.0)
}

fn any_platform() -> impl Strategy<Value = Platform> {
    prop_oneof![
        Just(Platform::Windows),
        Just(Platform::Macos),
        Just(Platform::Linux),
    ]
}

fn any_session_mode() -> impl Strategy<Value = SessionMode> {
    prop_oneof![Just(SessionMode::Normal), Just(SessionMode::InviteReadOnly)]
}

fn any_playback_state() -> impl Strategy<Value = PlaybackState> {
    prop_oneof![Just(PlaybackState::Playing), Just(PlaybackState::Paused)]
}

fn any_playback_ended_reason() -> impl Strategy<Value = PlaybackEndedReason> {
    prop_oneof![
        Just(PlaybackEndedReason::Completed),
        Just(PlaybackEndedReason::Manual),
        Just(PlaybackEndedReason::Error),
        Just(PlaybackEndedReason::Local),
    ]
}

fn any_stop_reason() -> impl Strategy<Value = StopReason> {
    prop_oneof![
        Just(StopReason::Manual),
        Just(StopReason::Preempt),
        Just(StopReason::Revoked),
        Just(StopReason::Purge),
    ]
}

fn any_media_trigger() -> impl Strategy<Value = MediaTrigger> {
    prop_oneof![Just(MediaTrigger::Shortcut), Just(MediaTrigger::Ui)]
}

fn any_stop_request_source() -> impl Strategy<Value = StopRequestSource> {
    prop_oneof![
        Just(StopRequestSource::User),
        Just(StopRequestSource::Hotkey),
        Just(StopRequestSource::Admin),
    ]
}

fn any_ack_outcome() -> impl Strategy<Value = AckOutcome> {
    prop_oneof![Just(AckOutcome::Playing), Just(AckOutcome::Queued)]
}

fn any_board_change() -> impl Strategy<Value = BoardChange> {
    prop_oneof![
        Just(BoardChange::Added),
        Just(BoardChange::Updated),
        Just(BoardChange::Deleted),
    ]
}

fn any_media_kind() -> impl Strategy<Value = MediaKind> {
    prop_oneof![
        Just(MediaKind::Image),
        Just(MediaKind::Audio),
        Just(MediaKind::Video),
    ]
}

/// Small scalar JSON values for the unknown-field carry-over maps.
fn small_json_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        (0i64..=4_096).prop_map(Value::from),
        short_text().prop_map(Value::String),
    ]
}

/// Unknown-field carry-over map for `#[serde(flatten)]` payloads, with keys
/// that cannot collide with the frozen wire fields (mirrors the
/// `extensionFuture` unknown key of `v2_play_tweet_card_rich.json`).
fn extra_fields() -> impl Strategy<Value = Map<String, Value>> {
    vec(small_json_value(), 0..=4).prop_map(|values| {
        values
            .into_iter()
            .enumerate()
            .map(|(index, value)| (format!("extensionFuture{index}"), value))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Payload strategies
// ---------------------------------------------------------------------------

fn media_payload() -> impl Strategy<Value = MediaPayload> {
    (
        (cuid(), media_url(), mime(), any_media_kind()),
        (
            of(0u64..=7_200),
            any::<bool>(),
            of(0u64..=600),
            of(cuid()),
            of(media_url()),
            of(1i64..=200),
        ),
    )
        .prop_map(
            |(
                (asset_id, url, mime, kind),
                (
                    duration_sec,
                    is_vertical,
                    start_offset_sec,
                    artwork_asset_id,
                    artwork_url,
                    display_scale_percent,
                ),
            )| {
                MediaPayload {
                    asset_id,
                    url,
                    mime,
                    kind,
                    duration_sec,
                    is_vertical,
                    start_offset_sec,
                    artwork_asset_id,
                    artwork_url,
                    display_scale_percent,
                }
            },
        )
}

fn tweet_card_video() -> impl Strategy<Value = TweetCardVideo> {
    (
        media_url(),
        of(mime()),
        of(any::<bool>()),
        of(cuid()),
        of(0u64..=7_200),
    )
        .prop_map(
            |(url, mime, is_vertical, source_status_id, duration_sec)| TweetCardVideo {
                url,
                mime,
                is_vertical,
                source_status_id,
                duration_sec,
            },
        )
}

fn tweet_card_payload() -> impl Strategy<Value = TweetCardPayload> {
    (
        (tweet_url(), tweet_html(), short_text(), of(tweet_url())),
        (
            of(short_text()),
            of(cuid()),
            of(media_url()),
            of(mime()),
            of(any::<bool>()),
            of(vec(tweet_card_video(), 0..=3)),
            extra_fields(),
        ),
    )
        .prop_map(
            |(
                (url, html, author_name, author_url),
                (
                    caption,
                    current_status_id,
                    video_url,
                    video_mime,
                    video_is_vertical,
                    videos,
                    extra,
                ),
            )| {
                TweetCardPayload {
                    url,
                    html,
                    author_name,
                    author_url,
                    caption,
                    current_status_id,
                    video_url,
                    video_mime,
                    video_is_vertical,
                    videos,
                    extra,
                }
            },
        )
}

fn play_payload() -> impl Strategy<Value = PlayPayload> {
    (
        cuid(),
        of(media_payload()),
        (short_text(), any::<bool>()),
        (short_text(), of(media_url()), any::<bool>()),
        of(tweet_card_payload()),
        0u64..=600,
        extra_fields(),
    )
        .prop_map(
            |(
                job_id,
                media,
                (text_value, text_enabled),
                (author_name, author_image, author_enabled),
                tweet_card,
                duration_sec,
                extra,
            )| {
                PlayPayload {
                    job_id,
                    media,
                    text: TextPayload {
                        value: text_value,
                        enabled: text_enabled,
                    },
                    author: AuthorPayload {
                        name: author_name,
                        image: author_image,
                        enabled: author_enabled,
                    },
                    tweet_card,
                    duration_sec,
                    extra,
                }
            },
        )
}

fn current_job_payload() -> impl Strategy<Value = CurrentJobPayload> {
    (play_payload(), unix_ms(), 0u64..=86_400_000).prop_map(|(play, started_at, remaining_ms)| {
        CurrentJobPayload {
            play,
            started_at,
            remaining_ms,
        }
    })
}

fn welcome_config() -> impl Strategy<Value = WelcomeConfig> {
    (any::<bool>(), 0u64..=600, of(0u64..=3_600)).prop_map(
        |(show_text_default, default_media_time, max_media_time)| WelcomeConfig {
            show_text_default,
            default_media_time,
            max_media_time,
        },
    )
}

fn welcome_payload() -> impl Strategy<Value = WelcomePayload> {
    (
        (snowflake(), of(short_text()), of(icon_url()), cuid()),
        (
            any_session_mode(),
            version_string(),
            unix_ms(),
            welcome_config(),
            of(current_job_payload()),
        ),
    )
        .prop_map(
            |(
                (guild_id, guild_name, guild_icon_url, client_id),
                (session_mode, server_version, server_time, config, current_job),
            )| {
                WelcomePayload {
                    protocol_version: PROTOCOL_VERSION,
                    guild_id,
                    guild_name,
                    guild_icon_url,
                    client_id,
                    session_mode,
                    server_version,
                    server_time,
                    config,
                    paused: false,
                    current_job,
                }
            },
        )
}

fn settings_payload() -> impl Strategy<Value = SettingsPayload> {
    (
        snowflake(),
        any_session_mode(),
        any::<bool>(),
        0u64..=600,
        of(0u64..=3_600),
    )
        .prop_map(
            |(guild_id, session_mode, show_text_default, default_media_time, max_media_time)| {
                SettingsPayload {
                    guild_id,
                    session_mode,
                    show_text_default,
                    default_media_time,
                    max_media_time,
                }
            },
        )
}

fn peer_info() -> impl Strategy<Value = PeerInfo> {
    (cuid(), short_text()).prop_map(|(client_id, label)| PeerInfo { client_id, label })
}

fn peers_payload() -> impl Strategy<Value = PeersPayload> {
    (snowflake(), vec(peer_info(), 0..=8))
        .prop_map(|(guild_id, peers)| PeersPayload { guild_id, peers })
}

fn stop_payload() -> impl Strategy<Value = StopPayload> {
    (any_stop_reason(), of(cuid())).prop_map(|(reason, job_id)| StopPayload { reason, job_id })
}

fn ack_payload() -> impl Strategy<Value = AckPayload> {
    (
        frame_id("cli"),
        any::<bool>(),
        of((error_code(), short_text())),
        of(any_ack_outcome()),
        of(0u32..=99),
        of(cuid()),
    )
        .prop_map(
            |(ref_id, ok, error, outcome, position, job_id)| AckPayload {
                ref_id,
                ok,
                error: error.map(|(code, message)| AckError { code, message }),
                outcome,
                position,
                job_id,
            },
        )
}

fn media_ingestion_error() -> impl Strategy<Value = MediaIngestionError> {
    (error_code(), of(short_text()))
        .prop_map(|(code, message)| MediaIngestionError { code, message })
}

fn media_ingestion_payload() -> impl Strategy<Value = MediaIngestionPayload> {
    (
        ingestion_id(),
        any::<bool>(),
        of(any::<bool>()),
        of(cuid()),
        of(cuid()),
        of(media_ingestion_error()),
    )
        .prop_map(|(ingestion_id, ok, created, item_id, job_id, error)| {
            MediaIngestionPayload {
                ingestion_id,
                ok,
                created,
                item_id,
                job_id,
                error,
            }
        })
}

fn board_changed_payload() -> impl Strategy<Value = BoardChangedPayload> {
    (any_board_change(), cuid())
        .prop_map(|(change, item_id)| BoardChangedPayload { change, item_id })
}

fn any_admin_change() -> impl Strategy<Value = AdminChangeKind> {
    prop_oneof![
        Just(AdminChangeKind::Members),
        Just(AdminChangeKind::Invites),
        Just(AdminChangeKind::PairingCodes),
        Just(AdminChangeKind::Bans),
        Just(AdminChangeKind::Audit),
    ]
}

fn admin_changed_payload() -> impl Strategy<Value = AdminChangedPayload> {
    (snowflake(), any_admin_change())
        .prop_map(|(guild_id, kind)| AdminChangedPayload { guild_id, kind })
}

fn device_settings_payload() -> impl Strategy<Value = DeviceSettingsPayload> {
    (of(volume()), of(any::<bool>()), of(any::<bool>())).prop_map(
        |(volume, media_muted, paused)| DeviceSettingsPayload {
            volume,
            media_muted,
            paused,
        },
    )
}

fn session_changed_payload() -> impl Strategy<Value = SessionChangedPayload> {
    any_session_mode().prop_map(|session_mode| SessionChangedPayload { session_mode })
}

/// Client-to-server messages: pick a kind, then generate plausible fields
/// mirroring the fixture set.
fn client_message() -> impl Strategy<Value = ClientMessage> {
    prop_oneof![
        1 => (token(), version_string(), of(cuid()), of(any_platform()), of(any_session_mode()))
            .prop_map(|(token, app_version, client_id, platform, session_mode)| {
                ClientMessage::Hello(HelloPayload {
                    token,
                    app_version,
                    client_id,
                    platform,
                    session_mode,
                })
            }),
        1 => (cuid(), snowflake(), version_string(), of(any_session_mode()))
            .prop_map(|(client_id, guild_id, app_version, session_mode)| {
                ClientMessage::Heartbeat(HeartbeatPayload {
                    client_id,
                    guild_id,
                    app_version,
                    session_mode,
                })
            }),
        1 => (cuid(), any_playback_state(), of(0u64..=86_400_000))
            .prop_map(|(job_id, state, remaining_ms)| {
                ClientMessage::PlaybackState(PlaybackStatePayload {
                    job_id,
                    state,
                    remaining_ms,
                })
            }),
        1 => (cuid(), any_playback_ended_reason())
            .prop_map(|(job_id, reason)| {
                ClientMessage::PlaybackEnded(PlaybackEndedPayload { job_id, reason })
            }),
        1 => (cuid(), error_code(), short_text())
            .prop_map(|(job_id, code, message)| {
                ClientMessage::Error(ClientErrorPayload { job_id, code, message })
            }),
        1 => (of(cuid()), of(any_stop_request_source()))
            .prop_map(|(job_id, source)| {
                ClientMessage::StopRequest(StopRequestPayload { job_id, source })
            }),
        1 => (cuid(), any_media_trigger(), of(short_text()))
            .prop_map(|(item_id, trigger, message)| {
                ClientMessage::MediaTrigger(MediaTriggerPayload {
                    item_id,
                    trigger,
                    message,
                })
            }),
        1 => Just(ClientMessage::PeersRequest(PeersRequestPayload {})),
        1 => (volume(), any::<bool>(), any::<bool>())
            .prop_map(|(volume, media_muted, paused)| {
                ClientMessage::DeviceSettingsState(DeviceSettingsStatePayload {
                    volume,
                    media_muted,
                    paused,
                })
            }),
    ]
}

/// Server-to-client messages: pick a kind, then generate plausible fields
/// mirroring the fixture set.
fn server_message() -> impl Strategy<Value = ServerMessage> {
    prop_oneof![
        1 => welcome_payload().prop_map(ServerMessage::Welcome),
        1 => settings_payload().prop_map(ServerMessage::Settings),
        1 => peers_payload().prop_map(ServerMessage::Peers),
        1 => play_payload().prop_map(ServerMessage::Play),
        1 => stop_payload().prop_map(ServerMessage::Stop),
        1 => ack_payload().prop_map(ServerMessage::Ack),
        1 => (error_code(), short_text(), of(cuid()))
            .prop_map(|(code, message, job_id)| {
                ServerMessage::Error(ServerErrorPayload { code, message, job_id })
            }),
        1 => media_ingestion_payload().prop_map(ServerMessage::MediaIngestion),
        1 => board_changed_payload().prop_map(ServerMessage::BoardChanged),
        1 => admin_changed_payload().prop_map(ServerMessage::AdminChanged),
        1 => device_settings_payload().prop_map(ServerMessage::DeviceSettings),
        1 => session_changed_payload().prop_map(ServerMessage::SessionChanged),
        1 => Just(ServerMessage::RightsChanged(RightsChangedPayload {})),
    ]
}

/// Direction of a generated envelope. Both variants are boxed to keep the
/// enum itself small (`clippy::large_enum_variant`).
#[derive(Debug)]
enum WireDirection {
    /// Envelope of a client-to-server message.
    Client(Box<Envelope<ClientMessage>>),
    /// Envelope of a server-to-client message.
    Server(Box<Envelope<ServerMessage>>),
}

fn any_envelope() -> impl Strategy<Value = WireDirection> {
    prop_oneof![
        (frame_id("cli"), unix_ms(), client_message()).prop_map(|(id, ts, message)| {
            WireDirection::Client(Box::new(Envelope::new(id, ts, message)))
        }),
        (frame_id("srv"), unix_ms(), server_message()).prop_map(|(id, ts, message)| {
            WireDirection::Server(Box::new(Envelope::new(id, ts, message)))
        }),
    ]
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// Byte payloads whose lengths concentrate around the frozen boundary.
fn boundary_payload() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Exact neighbourhood of MAX_FRAME_BYTES, on both sides of the limit.
        4 => ((MAX_FRAME_BYTES as i64 - 4)..=(MAX_FRAME_BYTES as i64 + 4))
            .prop_map(|len| vec![0xA5u8; len as usize]),
        // Ordinary small payloads with random bytes.
        1 => vec(any::<u8>(), 0..=256),
        // Far past the boundary.
        1 => ((MAX_FRAME_BYTES + 1)..=(MAX_FRAME_BYTES + 1_024))
            .prop_map(|len| vec![0x5Au8; len]),
    ]
}

// Generated `major.minor.patch` strings with each part in 0..=999:
// `parse_simple_version` round-trips the triple, and `is_version_below`
// agrees exactly with strict lexicographic triple ordering (frozen rule
// §1.6), so it is strictly monotonic and never true for equal versions.
proptest! {
    #[test]
    fn version_strings_parse_and_compare(
        left in (0u64..=999, 0u64..=999, 0u64..=999),
        right in (0u64..=999, 0u64..=999, 0u64..=999),
    ) {
        let left_string = format!("{}.{}.{}", left.0, left.1, left.2);
        let right_string = format!("{}.{}.{}", right.0, right.1, right.2);

        prop_assert_eq!(parse_simple_version(&left_string), Some(left));
        prop_assert_eq!(parse_simple_version(&right_string), Some(right));
        prop_assert_eq!(is_version_below(&left_string, &right_string), left < right);
    }
}

// `is_frame_too_large` contract, read from src/lib.rs: `len > MAX_FRAME_BYTES`.
// The 64 Kio limit is INCLUSIVE: a frame of exactly `MAX_FRAME_BYTES` bytes is
// accepted (pinned by `fixtures_roundtrip::frame_size_limit_is_inclusive`)
// and the guard flips only strictly above it.
proptest! {
    #[test]
    fn frame_size_guard_boundary(payload in boundary_payload()) {
        prop_assert_eq!(
            is_frame_too_large(payload.len()),
            payload.len() > MAX_FRAME_BYTES
        );
    }
}

/// Serializes, deserializes and re-serializes an envelope, asserting the
/// round-trip STABILITY of the reserialization (`serde_json::Value`
/// equality between the two serializations) plus structural equality of the
/// parsed envelope. Only the golden files pin the on-wire shape:
/// `tests/fixtures_roundtrip.rs` and the fuzz crate's
/// `golden_protocol_fixtures_match_their_file_value` compare the
/// reserialization against the committed fixture values.
fn assert_envelope_roundtrips<M>(envelope: &Envelope<M>) -> TestCaseResult
where
    M: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let wire = serde_json::to_value(envelope)
        .map_err(|error| TestCaseError::fail(format!("envelope serialization failed: {error}")))?;
    let parsed: Envelope<M> = serde_json::from_value(wire.clone()).map_err(|error| {
        TestCaseError::fail(format!("envelope deserialization failed: {error}"))
    })?;
    let again = serde_json::to_value(&parsed).map_err(|error| {
        TestCaseError::fail(format!("envelope re-serialization failed: {error}"))
    })?;

    prop_assert_eq!(again, wire);
    prop_assert_eq!(&parsed, envelope);
    Ok(())
}

proptest! {
    #[test]
    fn envelope_roundtrip(direction in any_envelope()) {
        match direction {
            WireDirection::Client(envelope) => assert_envelope_roundtrips(&envelope)?,
            WireDirection::Server(envelope) => assert_envelope_roundtrips(&envelope)?,
        }
    }
}

proptest! {
    /// `media_signature_input(asset_id: &str, client_id: &str, exp_sec: u64)`
    /// builds the frozen HMAC input `assetId|clientId|exp` (src/lib.rs, doc
    /// §8). Ids come from the frozen vocabulary `[a-z0-9-]{1,40}`, which
    /// cannot contain the `|` separator; under that constraint the input is
    /// injective — two different tuples can never share a signature input,
    /// so a signature minted for one (asset, client, expiry) triple cannot
    /// be replayed for another.
    #[test]
    fn media_signature_input_is_injective_without_separator(
        first in (signature_id(), signature_id(), any::<u64>()),
        second in (signature_id(), signature_id(), any::<u64>()),
    ) {
        let first_input = media_signature_input(&first.0, &first.1, first.2);
        let second_input = media_signature_input(&second.0, &second.1, second.2);

        if first != second {
            prop_assert_ne!(first_input, second_input);
        }
    }
}

/// Frozen, not fixed: the `|` separator makes the input ambiguous when an id
/// is allowed to contain it — `("a|b", "c", 1)` and `("a", "b|c", 1)` build
/// the same string. This documents why ids must never contain `|` (the
/// frozen id vocabulary `[a-z0-9-]{1,40}` excludes it) rather than changing
/// the frozen signature format.
#[test]
fn separator_in_ids_makes_inputs_ambiguous() {
    assert_eq!(
        media_signature_input("a|b", "c", 1),
        media_signature_input("a", "b|c", 1)
    );
}

// ---------------------------------------------------------------------------
// device.settings.state / device.settings acceptance
// ---------------------------------------------------------------------------

/// Volumes for the `device.settings.state` payload: the documented
/// `0.0..=1.0` unit range plus the values the guard must refuse — NaN, both
/// infinities and finite volumes on either side of the range.
fn state_volume() -> impl Strategy<Value = f64> {
    prop_oneof![
        4 => 0.0f64..=1.0,
        1 => Just(f64::NAN),
        1 => Just(f64::INFINITY),
        1 => Just(f64::NEG_INFINITY),
        2 => -10.0f64..-0.0001,
        2 => 1.0001f64..=10.0,
    ]
}

proptest! {
    /// `DeviceSettingsStatePayload::is_valid` mirrors the documented finite
    /// `0.0..=1.0` unit range of the `volume` field (src/lib.rs), but unlike
    /// a dead mirror the generator REACHES the invalid values (NaN, ±∞ and
    /// out-of-range volumes), so both a guard that always returns `true`
    /// and a guard whose `&&` was weakened to `||` fail.
    #[test]
    fn device_settings_state_validity_pins_the_finite_unit_range(
        volume in state_volume(),
    ) {
        let payload = DeviceSettingsStatePayload {
            volume,
            media_muted: false,
            paused: false,
        };
        prop_assert_eq!(
            payload.is_valid(),
            volume.is_finite() && (0.0..=1.0).contains(&volume)
        );
    }

    /// An all-`None` `DeviceSettingsPayload` is empty and serializes to a
    /// bare `{}` JSON object; a payload with any field set is not empty and
    /// its wire value is a non-empty JSON object (every field is
    /// `skip_serializing_if = "Option::is_none"`, src/lib.rs).
    #[test]
    fn empty_device_settings_serializes_to_empty_object(
        payload in device_settings_payload(),
    ) {
        let expected_empty = payload.volume.is_none()
            && payload.media_muted.is_none()
            && payload.paused.is_none();
        prop_assert_eq!(payload.is_empty(), expected_empty);
        let value = serde_json::to_value(payload).map_err(|error| {
            TestCaseError::fail(format!("device settings serialization failed: {error}"))
        })?;
        if payload.is_empty() {
            prop_assert_eq!(value, json!({}));
        } else {
            let object = value.as_object().ok_or_else(|| {
                TestCaseError::fail("device settings did not serialize to a JSON object")
            })?;
            prop_assert!(
                !object.is_empty(),
                "non-empty payload serialized to an empty object"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Unreadable versions
// ---------------------------------------------------------------------------

proptest! {
    /// An app version `parse_simple_version` cannot read is below any
    /// readable minimum, and an unreadable minimum also refuses a readable
    /// app version: both directions fall back to the `_ => true` arm of
    /// `is_version_below` (frozen rule §1.6: unreadable means obsolete,
    /// refuse by safety).
    #[test]
    fn unreadable_app_version_is_always_below(s in any::<String>()) {
        prop_assume!(parse_simple_version(&s).is_none());
        prop_assert!(
            is_version_below(&s, "1.0.0"),
            "unreadable app version {s:?} was not below 1.0.0"
        );
        prop_assert!(
            is_version_below("1.0.0", &s),
            "unreadable minimum version {s:?} did not refuse 1.0.0"
        );
        for malformed in ["", "abc", "1", "1.2", "1.2.3.4", "x.y.z"] {
            prop_assert!(
                is_version_below(malformed, "1.0.0"),
                "malformed version {malformed:?} was not below 1.0.0"
            );
        }
    }
}
