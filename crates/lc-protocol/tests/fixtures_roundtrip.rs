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
    close_code, close_reason, is_frame_too_large, media_signature_input, msg_type, ClientEnvelope,
    ClientMessage, Envelope, ServerEnvelope, ServerMessage, HEARTBEAT_INTERVAL_MS,
    HELLO_TIMEOUT_MS, HTTP_UPGRADE_REQUIRED, LEGACY_SOCKET_IO_PATH, MAX_CLIENT_MESSAGES_PER_SECOND,
    MAX_FRAME_BYTES, MAX_MEME_TEXT_CHARS, MAX_PLAYBACK_REMAINING_MS, MEDIA_URL_TTL_SEC,
    PEERS_REQUEST_MIN_INTERVAL_MS, PING_INTERVAL_MS, PLAYBACK_STATE_MIN_INTERVAL_MS,
    PONG_TIMEOUT_MS, PROTOCOL_VERSION, WS_PATH,
};

/// Sens de circulation d'une fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// `Envelope<ClientMessage>`
    Client,
    /// `Envelope<ServerMessage>`
    Server,
}

/// Table de référence des 28 fixtures golden : (fichier, sens, type attendu).
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
        "v2_meme_trigger.json",
        Direction::Client,
        msg_type::MEME_TRIGGER,
    ),
    (
        "v2_stop_request.json",
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
        "v2_ack_meme_trigger_ok.json",
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
fn every_fixture_file_is_covered_by_the_golden_table() {
    let directory = fixture_path("");
    let mut on_disk: Vec<String> = fs::read_dir(&directory)
        .expect("dossier fixtures lisible")
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let is_json = path.extension().and_then(|ext| ext.to_str()) == Some("json");
            is_json.then(|| path.file_name()?.to_str().map(ToOwned::to_owned))?
        })
        .collect();
    on_disk.sort();
    let mut expected: Vec<String> = FIXTURES
        .iter()
        .map(|(name, ..)| (*name).to_owned())
        .collect();
    expected.sort();
    assert_eq!(
        on_disk, expected,
        "la table des fixtures doit référencer exactement les 28 fichiers golden"
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
    assert_eq!(PEERS_REQUEST_MIN_INTERVAL_MS, 2_000);
    assert_eq!(MAX_PLAYBACK_REMAINING_MS, 86_400_000);
    assert_eq!(MAX_MEME_TEXT_CHARS, 500);
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
        "v2_meme_trigger.json",
        "v2_stop_request.json",
    ] {
        let value = read_fixture(fixture);
        assert!(
            serde_json::from_value::<ServerEnvelope>(value).is_err(),
            "{fixture} ne doit pas se parser comme un message serveur"
        );
    }
}
