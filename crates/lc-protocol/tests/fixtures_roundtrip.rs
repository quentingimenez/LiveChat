//! Tests golden du protocole v2 : chaque fixture de `tests/fixtures/` doit se
//! désérialiser dans le bon sens (client→serveur ou serveur→client), porter le
//! bon `type`, puis se re-sérialiser à l'identique (égalité de `serde_json::Value`).
//!
//! Référence : `docs/rust/protocol-v2.md` (FROZEN, 2026-11-09), §14 et §16.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use lc_protocol::{
    close_code, close_reason, is_frame_too_large, is_version_below, media_signature_input,
    media_url_ttl_sec, msg_type, outdated_protocol_reason, parse_simple_version, ClientEnvelope,
    ClientMessage, Envelope, MediaKind, MediaTrigger, ServerEnvelope, ServerMessage,
    StopRequestSource, DEFAULT_MIN_OVERLAY_VERSION, HEARTBEAT_INTERVAL_MS, HELLO_TIMEOUT_MS,
    HTTP_UPGRADE_REQUIRED, LEGACY_SOCKET_IO_PATH, MAX_CLIENT_MESSAGES_PER_SECOND, MAX_FRAME_BYTES,
    MAX_MEDIA_TEXT_CHARS, MAX_PLAYBACK_REMAINING_MS, MEDIA_URL_EXPIRY_MARGIN_SEC,
    MEDIA_URL_TTL_MAX_SEC, MEDIA_URL_TTL_SEC, PEERS_REQUEST_MIN_INTERVAL_MS, PING_INTERVAL_MS,
    PLAYBACK_STATE_MIN_INTERVAL_MS, PONG_TIMEOUT_MS, PROTOCOL_VERSION, WS_PATH,
};

/// Sens de circulation d'une fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// `Envelope<ClientMessage>`
    Client,
    /// `Envelope<ServerMessage>`
    Server,
}

/// Table de référence des 29 fixtures golden : (fichier, sens, type attendu).
const FIXTURES: &[(&str, Direction, &str)] = &[
    // ---- C→S ----
    ("v2_hello_normal.json", Direction::Client, msg_type::HELLO),
    (
        "v2_hello_invite_read_only.json",
        Direction::Client,
        msg_type::HELLO,
    ),
    ("v2_hello_minimal.json", Direction::Client, msg_type::HELLO),
    ("v2_heartbeat.json", Direction::Client, msg_type::HEARTBEAT),
    (
        "v2_playback_state.json",
        Direction::Client,
        msg_type::PLAYBACK_STATE,
    ),
    (
        "v2_playback_ended.json",
        Direction::Client,
        msg_type::PLAYBACK_ENDED,
    ),
    (
        "v2_media_trigger.json",
        Direction::Client,
        msg_type::MEDIA_TRIGGER,
    ),
    (
        "v2_stop_request.json",
        Direction::Client,
        msg_type::STOP_REQUEST,
    ),
    (
        "v2_stop_request_user.json",
        Direction::Client,
        msg_type::STOP_REQUEST,
    ),
    (
        "v2_peers_request.json",
        Direction::Client,
        msg_type::PEERS_REQUEST,
    ),
    (
        "v2_error_render_failed.json",
        Direction::Client,
        msg_type::ERROR,
    ),
    // ---- S→C ----
    (
        "v2_welcome_initial.json",
        Direction::Server,
        msg_type::WELCOME,
    ),
    (
        "v2_welcome_reconnect_current_job.json",
        Direction::Server,
        msg_type::WELCOME,
    ),
    (
        "v2_play_media_resume.json",
        Direction::Server,
        msg_type::PLAY,
    ),
    (
        "v2_play_text_author.json",
        Direction::Server,
        msg_type::PLAY,
    ),
    ("v2_play_tweet_card.json", Direction::Server, msg_type::PLAY),
    (
        "v2_play_tweet_card_rich.json",
        Direction::Server,
        msg_type::PLAY,
    ),
    ("v2_stop_manual.json", Direction::Server, msg_type::STOP),
    ("v2_stop_preempt.json", Direction::Server, msg_type::STOP),
    ("v2_stop_revoked.json", Direction::Server, msg_type::STOP),
    ("v2_stop_purge.json", Direction::Server, msg_type::STOP),
    ("v2_peers.json", Direction::Server, msg_type::PEERS),
    ("v2_peers_empty.json", Direction::Server, msg_type::PEERS),
    (
        "v2_settings_normal.json",
        Direction::Server,
        msg_type::SETTINGS,
    ),
    (
        "v2_settings_invite_read_only.json",
        Direction::Server,
        msg_type::SETTINGS,
    ),
    (
        "v2_ack_stop_request_ok.json",
        Direction::Server,
        msg_type::ACK,
    ),
    (
        "v2_ack_media_trigger_ok.json",
        Direction::Server,
        msg_type::ACK,
    ),
    ("v2_ack_read_only.json", Direction::Server, msg_type::ACK),
    (
        "v2_error_dispatch_failed.json",
        Direction::Server,
        msg_type::ERROR,
    ),
];

/// Fixtures legacy conservées : elles ne font pas partie du golden round-trip
/// (leur type est réécrit en `media.trigger`) mais doivent rester sur disque et
/// être couvertes par le test de couverture.
const LEGACY_FIXTURES: &[&str] = &["v2_meme_trigger_legacy.json"];

/// Chemin absolu d'une fixture.
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Lit une fixture en `serde_json::Value`.
fn read_fixture(name: &str) -> Value {
    let raw = fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|error| panic!("lecture impossible de {name} : {error}"));
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("JSON invalide dans {name} : {error}"))
}

#[test]
fn golden_fixtures_roundtrip_exactly() {
    for (name, direction, expected_type) in FIXTURES {
        let expected = read_fixture(name);
        match direction {
            Direction::Client => {
                let envelope: ClientEnvelope = serde_json::from_value(expected.clone())
                    .unwrap_or_else(|error| {
                        panic!("{name} ne se parse pas en ClientEnvelope : {error}")
                    });
                assert_eq!(
                    envelope.v, PROTOCOL_VERSION,
                    "{name} : version d'enveloppe inattendue"
                );
                assert_eq!(
                    envelope.message.type_name(),
                    *expected_type,
                    "{name} : type de message inattendu"
                );
                assert!(!envelope.id.is_empty(), "{name} : id vide");
                assert!(envelope.ts > 0, "{name} : ts non positif");
                let actual =
                    serde_json::to_value(&envelope).expect("re-sérialisation du client envelope");
                assert_eq!(actual, expected, "{name} : round-trip C→S non conforme");
            }
            Direction::Server => {
                let envelope: ServerEnvelope = serde_json::from_value(expected.clone())
                    .unwrap_or_else(|error| {
                        panic!("{name} ne se parse pas en ServerEnvelope : {error}")
                    });
                assert_eq!(
                    envelope.v, PROTOCOL_VERSION,
                    "{name} : version d'enveloppe inattendue"
                );
                assert_eq!(
                    envelope.message.type_name(),
                    *expected_type,
                    "{name} : type de message inattendu"
                );
                assert!(!envelope.id.is_empty(), "{name} : id vide");
                assert!(envelope.ts > 0, "{name} : ts non positif");
                let actual =
                    serde_json::to_value(&envelope).expect("re-sérialisation du server envelope");
                assert_eq!(actual, expected, "{name} : round-trip S→C non conforme");
            }
        }
    }
}

#[test]
fn legacy_meme_trigger_deserialises_into_media_trigger() {
    let value = read_fixture("v2_meme_trigger_legacy.json");
    let envelope: ClientEnvelope =
        serde_json::from_value(value).expect("l'ancien type meme.trigger doit rester accepté");
    assert_eq!(envelope.message.type_name(), msg_type::MEDIA_TRIGGER);
    let ClientMessage::MediaTrigger(payload) = envelope.message else {
        panic!("meme.trigger doit désérialiser en ClientMessage::MediaTrigger");
    };
    assert_eq!(payload.item_id, "clx7q2x0b0004abcd1234efgh");
    assert_eq!(payload.trigger, MediaTrigger::Shortcut);

    let serialized = serde_json::to_value(Envelope::new(
        "cli-media-trigger-0001",
        1_794_182_400_000,
        ClientMessage::MediaTrigger(payload),
    ))
    .expect("re-sérialisation media.trigger");
    assert_eq!(
        serialized.get("type"),
        Some(&Value::String("media.trigger".to_owned())),
        "la sérialisation doit toujours émettre le nouveau nom"
    );
}

#[test]
fn every_fixture_file_is_covered_by_the_golden_table() {
    let directory = fixture_path("");
    let mut on_disk: Vec<String> = fs::read_dir(&directory)
        .expect("dossier fixtures lisible")
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let is_json = path.extension().and_then(|ext| ext.to_str()) == Some("json");
            let name = path.file_name()?.to_str().map(ToOwned::to_owned)?;
            is_json.then_some(name)
        })
        .collect();
    on_disk.sort();
    let mut expected: Vec<String> = FIXTURES
        .iter()
        .map(|(name, ..)| (*name).to_owned())
        .chain(LEGACY_FIXTURES.iter().map(|name| (*name).to_owned()))
        .collect();
    expected.sort();
    assert_eq!(
        on_disk, expected,
        "la table des fixtures doit référencer exactement les 29 fichiers golden \
         et la fixture legacy"
    );
}

#[test]
fn protocol_constants_are_frozen() {
    assert_eq!(PROTOCOL_VERSION, 2);
    assert_eq!(WS_PATH, "/ws");
    assert_eq!(LEGACY_SOCKET_IO_PATH, "/socket.io/");
    assert_eq!(HTTP_UPGRADE_REQUIRED, 426);
    assert_eq!(HELLO_TIMEOUT_MS, 5_000);
    assert_eq!(MAX_FRAME_BYTES, 65_536);
    assert_eq!(MAX_CLIENT_MESSAGES_PER_SECOND, 20);
    assert_eq!(PLAYBACK_STATE_MIN_INTERVAL_MS, 1_000);
    assert_eq!(HEARTBEAT_INTERVAL_MS, 15_000);
    assert_eq!(PING_INTERVAL_MS, 15_000);
    assert_eq!(PONG_TIMEOUT_MS, 45_000);
    assert_eq!(MEDIA_URL_TTL_SEC, 300);
    assert_eq!(MEDIA_URL_EXPIRY_MARGIN_SEC, 120);
    assert_eq!(MEDIA_URL_TTL_MAX_SEC, 3_600);
    assert_eq!(DEFAULT_MIN_OVERLAY_VERSION, "3.0.0");
    assert_eq!(PEERS_REQUEST_MIN_INTERVAL_MS, 2_000);
    assert_eq!(MAX_PLAYBACK_REMAINING_MS, 86_400_000);
    assert_eq!(MAX_MEDIA_TEXT_CHARS, 500);
    assert_eq!(msg_type::MEDIA_TRIGGER, "media.trigger");
    assert_eq!(msg_type::LEGACY_MEME_TRIGGER, "meme.trigger");
}

#[test]
fn close_codes_and_reasons_are_frozen() {
    assert_eq!(close_code::NORMAL, 1000);
    assert_eq!(close_code::GOING_AWAY, 1001);
    assert_eq!(close_code::PROTOCOL_ERROR, 1002);
    assert_eq!(close_code::UNSUPPORTED_FRAME, 1003);
    assert_eq!(close_code::INTERNAL_ERROR, 1011);
    assert_eq!(close_code::UNAUTHORIZED, 4401);
    assert_eq!(close_code::REVOKED, 4402);
    assert_eq!(close_code::OUTDATED_PROTOCOL, 4403);
    assert_eq!(close_code::FRAME_TOO_LARGE, 4408);
    assert_eq!(close_code::RATE_LIMITED, 4409);

    assert_eq!(close_reason::HEARTBEAT_TIMEOUT, "heartbeat_timeout");
    assert_eq!(close_reason::AUTH_TIMEOUT, "auth_timeout");
    assert_eq!(close_reason::HELLO_REQUIRED, "hello_required");
    assert_eq!(close_reason::OUTDATED_PROTOCOL, "outdated_protocol");
}

#[test]
fn media_url_signature_input_matches_frozen_format() {
    let input = media_signature_input("asset-1", "client-1", 1_794_182_700);
    assert_eq!(input, "asset-1|client-1|1794182700");
}

#[test]
fn frame_size_limit_is_inclusive() {
    assert!(!is_frame_too_large(MAX_FRAME_BYTES));
    assert!(is_frame_too_large(MAX_FRAME_BYTES + 1));
}

#[test]
fn welcome_reconnect_exposes_current_job_for_resume() {
    let value = read_fixture("v2_welcome_reconnect_current_job.json");
    let envelope: ServerEnvelope = serde_json::from_value(value).expect("welcome reconnexion");
    let ServerMessage::Welcome(welcome) = envelope.message else {
        panic!("la fixture reconnexion doit être un welcome");
    };
    assert_eq!(welcome.protocol_version, PROTOCOL_VERSION);
    assert_eq!(welcome.session_mode, lc_protocol::SessionMode::Normal);
    let job = welcome
        .current_job
        .expect("currentJob attendu sur reconnexion");
    assert_eq!(job.play.job_id, "clx7q2x0b0001abcd1234efgh");
    assert_eq!(job.play.duration_sec, 108);
    assert_eq!(job.started_at, 1_794_182_400_000);
    assert_eq!(job.remaining_ms, 84_000);
    let media = job.play.media.expect("média du job courant");
    assert_eq!(media.start_offset_sec, Some(12));
    assert_eq!(media.kind, lc_protocol::MediaKind::Video);
    assert!(media.is_vertical);
}

#[test]
fn welcome_initial_omits_current_job() {
    let value = read_fixture("v2_welcome_initial.json");
    let envelope: ServerEnvelope = serde_json::from_value(value).expect("welcome initial");
    let ServerMessage::Welcome(welcome) = envelope.message else {
        panic!("la fixture initiale doit être un welcome");
    };
    assert!(welcome.current_job.is_none());
    let serialized = serde_json::to_value(Envelope::new(
        "srv-welcome-0001",
        1_794_182_400_000,
        welcome,
    ))
    .expect("re-sérialisation");
    assert!(
        serialized
            .get("payload")
            .and_then(|payload| payload.get("currentJob"))
            .is_none(),
        "currentJob ne doit pas être émis quand aucun job n'est en cours"
    );
}

#[test]
fn hello_minimal_omits_optional_fields() {
    let value = read_fixture("v2_hello_minimal.json");
    let envelope: ClientEnvelope = serde_json::from_value(value).expect("hello minimal");
    let ClientMessage::Hello(hello) = &envelope.message else {
        panic!("la fixture minimale doit être un hello");
    };
    assert!(hello.client_id.is_none());
    assert!(hello.session_mode.is_none());
    let serialized = serde_json::to_value(&envelope).expect("re-sérialisation hello");
    let payload = serialized
        .get("payload")
        .and_then(Value::as_object)
        .expect("payload objet");
    assert!(!payload.contains_key("clientId"));
    assert!(!payload.contains_key("sessionMode"));
}

#[test]
fn envelope_direction_is_enforced() {
    for fixture in [
        "v2_welcome_initial.json",
        "v2_play_media_resume.json",
        "v2_stop_manual.json",
        "v2_settings_normal.json",
    ] {
        let value = read_fixture(fixture);
        assert!(
            serde_json::from_value::<ClientEnvelope>(value).is_err(),
            "{fixture} ne doit pas se parser comme un message client"
        );
    }
    for fixture in [
        "v2_hello_normal.json",
        "v2_heartbeat.json",
        "v2_media_trigger.json",
        "v2_stop_request.json",
    ] {
        let value = read_fixture(fixture);
        assert!(
            serde_json::from_value::<ServerEnvelope>(value).is_err(),
            "{fixture} ne doit pas se parser comme un message serveur"
        );
    }
}

/// Rév. 2, §15.1 : TTL média adaptatif (300 s plancher, durée + 120 s, 3600 s plafond).
#[test]
fn media_url_ttl_follows_lead_revision_2() {
    assert_eq!(media_url_ttl_sec(MediaKind::Image, None), 300);
    assert_eq!(media_url_ttl_sec(MediaKind::Image, Some(120)), 300);
    assert_eq!(media_url_ttl_sec(MediaKind::Video, None), MEDIA_URL_TTL_SEC);
    assert_eq!(media_url_ttl_sec(MediaKind::Video, Some(60)), 300);
    assert_eq!(media_url_ttl_sec(MediaKind::Audio, Some(120)), 300);
    assert_eq!(media_url_ttl_sec(MediaKind::Video, Some(600)), 720);
    assert_eq!(
        media_url_ttl_sec(MediaKind::Audio, Some(9_999)),
        MEDIA_URL_TTL_MAX_SEC
    );
}

/// Rév. 2, §1.6 / §15.5 : comparaison semver simple et plancher de version.
#[test]
fn version_gate_uses_simple_semver() {
    assert_eq!(parse_simple_version("3.0.0"), Some((3, 0, 0)));
    assert_eq!(parse_simple_version("3.0.0-beta.1"), Some((3, 0, 0)));
    assert_eq!(parse_simple_version("3.0.0+sha.abc"), Some((3, 0, 0)));
    assert_eq!(parse_simple_version("3.0"), None);
    assert_eq!(parse_simple_version("abc"), None);
    assert!(is_version_below("2.9.9", "3.0.0"));
    assert!(is_version_below("3.0.0", "3.0.1"));
    assert!(is_version_below("3.0.0", "3.1.0"));
    assert!(!is_version_below("3.0.0", "3.0.0"));
    assert!(!is_version_below("3.1.0", "3.0.0"));
    assert!(is_version_below("not-a-version", "3.0.0"));
    assert_eq!(DEFAULT_MIN_OVERLAY_VERSION, "3.0.0");
}

/// Rév. 2, §1.6 / §15.5 : la `reason` du close 4403 est un JSON `{"minVersion":…}`.
#[test]
fn outdated_protocol_close_reason_is_json() {
    assert_eq!(
        outdated_protocol_reason("3.0.0"),
        r#"{"minVersion":"3.0.0"}"#
    );
}

/// Rév. 2, §5.6 / §15.2 : `stop.request` cible un job et porte une origine informative.
#[test]
fn stop_request_carries_optional_job_and_source() {
    let value = read_fixture("v2_stop_request_user.json");
    let envelope: ClientEnvelope = serde_json::from_value(value).expect("stop.request user");
    let ClientMessage::StopRequest(request) = envelope.message else {
        panic!("la fixture utilisateur doit être un stop.request");
    };
    assert_eq!(request.job_id.as_deref(), Some("clx7q2x0b0001abcd1234efgh"));
    assert_eq!(request.source, Some(StopRequestSource::User));
    let serialized = serde_json::to_value(Envelope::new(
        "cli-stop-request-0002",
        1_794_182_400_000,
        ClientMessage::StopRequest(request),
    ))
    .expect("re-sérialisation stop.request user");
    assert_eq!(
        serialized.get("payload"),
        Some(&serde_json::json!({
            "jobId": "clx7q2x0b0001abcd1234efgh",
            "source": "user"
        }))
    );

    let value = read_fixture("v2_stop_request.json");
    let envelope: ClientEnvelope = serde_json::from_value(value).expect("stop.request vide");
    let ClientMessage::StopRequest(request) = envelope.message else {
        panic!("la fixture vide doit être un stop.request");
    };
    assert!(request.job_id.is_none());
    assert!(request.source.is_none());
    let serialized = serde_json::to_value(Envelope::new(
        "cli-stop-request-0001",
        1_794_182_400_000,
        ClientMessage::StopRequest(request),
    ))
    .expect("re-sérialisation stop.request vide");
    assert_eq!(serialized.get("payload"), Some(&serde_json::json!({})));
}

/// Rév. 2, §6.1 / §15.5 : `welcome.serverVersion` est toujours émis.
#[test]
fn welcome_carries_server_version() {
    let value = read_fixture("v2_welcome_initial.json");
    let envelope: ServerEnvelope = serde_json::from_value(value).expect("welcome initial");
    let ServerMessage::Welcome(welcome) = &envelope.message else {
        panic!("la fixture initiale doit être un welcome");
    };
    assert_eq!(welcome.server_version, "3.0.0");
    let serialized = serde_json::to_value(&envelope).expect("re-sérialisation welcome");
    assert_eq!(
        serialized
            .get("payload")
            .and_then(|payload| payload.get("serverVersion")),
        Some(&Value::String("3.0.0".to_owned()))
    );
}

/// P2/AUD-COR-12 : les champs inconnus de `play` et `tweetCard` sont conservés
/// tels quels à la désérialisation puis réémis à la sérialisation (tolérance
/// §2 du protocole, sans casser les fixtures golden).
#[test]
fn unknown_play_and_tweet_card_fields_are_preserved() {
    let mut value = read_fixture("v2_play_tweet_card_rich.json");
    {
        let payload = value
            .get_mut("payload")
            .and_then(Value::as_object_mut)
            .expect("payload play");
        payload.insert(
            "extensionFuture".to_owned(),
            serde_json::json!({ "x": 1, "nested": [true, null] }),
        );
        let card = payload
            .get_mut("tweetCard")
            .and_then(Value::as_object_mut)
            .expect("tweetCard");
        card.insert("futureCardKey".to_owned(), Value::Bool(true));
    }

    let envelope: ServerEnvelope =
        serde_json::from_value(value.clone()).expect("play avec champs inconnus");
    let ServerMessage::Play(play) = &envelope.message else {
        panic!("la fixture doit être un play");
    };
    assert_eq!(
        play.extra.get("extensionFuture"),
        Some(&serde_json::json!({ "x": 1, "nested": [true, null] }))
    );
    let card = play.tweet_card.as_ref().expect("carte tweet");
    assert_eq!(card.extra.get("futureCardKey"), Some(&Value::Bool(true)));

    let serialized = serde_json::to_value(&envelope).expect("re-sérialisation play");
    assert_eq!(serialized, value);
}
