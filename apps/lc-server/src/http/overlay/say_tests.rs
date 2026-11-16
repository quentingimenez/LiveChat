//! In-process tests for `POST /overlay/say`.
//!
//! Piper is replaced by the scripts in `fixtures/` (they copy a WAV sample
//! and record the text received on stdin); the successful cases run the real
//! `ffmpeg`/`ffprobe` found on the `PATH`, like the upload tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use lc_db::Db;
use lc_domain::{JobStatus, MediaKind, MediaStatus, OverlaySessionMode, PlaybackJob};
use serde_json::{json, Value};
use tempfile::TempDir;

use crate::config::Config;
use crate::media::tts::MAX_SPEECH_CHARS;

use super::media_upload_tests::{
    config_pairs, guild, harness_with, insert_client, send_json, READ_ONLY_TOKEN, TOKEN,
};

/// Piper test double shipped with the sources.
fn fake_piper() -> PathBuf {
    let name = if cfg!(windows) {
        "fake-piper.cmd"
    } else {
        "fake-piper.sh"
    };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/http/overlay/fixtures")
        .join(name)
}

/// One second of a 440 Hz tone, PCM 16-bit mono 22 050 Hz (Piper's format).
fn wav_sample() -> Vec<u8> {
    const RATE: u32 = 22_050;
    let samples: Vec<i16> = (0..RATE)
        .map(|index| {
            let phase = f64::from(index) * 440.0 * std::f64::consts::TAU / f64::from(RATE);
            #[allow(clippy::cast_possible_truncation)]
            let sample = (phase.sin() * 12_000.0) as i16;
            sample
        })
        .collect();
    let data_len = u32::try_from(samples.len() * 2).unwrap();
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1_u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

struct Fixture {
    dir: TempDir,
    app: Router,
    db: Db,
}

impl Fixture {
    /// The "voice" given to the fake Piper: the WAV it copies to its output.
    fn voice(dir: &Path) -> PathBuf {
        dir.join("voice.wav")
    }

    /// Text the fake Piper read on stdin, if it ran.
    fn spoken(&self) -> Option<String> {
        let path = self.dir.path().join("voice.wav.txt");
        std::fs::read_to_string(path)
            .ok()
            .map(|text| text.trim().to_owned())
    }

    async fn jobs(&self) -> Vec<PlaybackJob> {
        self.db
            .playback_jobs()
            .list_pending_by_guild(&guild())
            .await
            .unwrap()
    }
}

/// `piper`: `Some(path)` to run that binary, `None` for a missing one.
/// `real_tools`: run the real ffmpeg (needed whenever synthesis succeeds).
async fn fixture(piper: Option<PathBuf>, real_tools: bool) -> Fixture {
    let dir = TempDir::new().unwrap();
    let voice = Fixture::voice(dir.path());
    std::fs::write(&voice, wav_sample()).unwrap();
    let piper = piper.unwrap_or_else(|| dir.path().join("missing-piper"));
    let mut pairs = config_pairs(&dir.path().join("media"), 16, real_tools);
    pairs.push(("PIPER_BIN".to_owned(), piper.to_string_lossy().into_owned()));
    pairs.push((
        "PIPER_VOICE_FR".to_owned(),
        voice.to_string_lossy().into_owned(),
    ));
    let (app, db, _state, _hub) = harness_with(Config::from_pairs(pairs).unwrap()).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;
    insert_client(&db, OverlaySessionMode::InviteReadOnly, READ_ONLY_TOKEN).await;
    Fixture { dir, app, db }
}

fn say_request(token: Option<&str>, body: &Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/overlay/say")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn a_bearer_token_is_required() {
    let fixture = fixture(Some(fake_piper()), false).await;
    let body = json!({"text": "bonjour"});

    let (status, answer) = send_json(&fixture.app, say_request(None, &body)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(answer["error"], "unauthorized");

    let unknown = "9".repeat(64);
    let (status, answer) = send_json(&fixture.app, say_request(Some(&unknown), &body)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(answer["error"], "unauthorized");

    assert_eq!(fixture.spoken(), None);
    assert!(fixture.jobs().await.is_empty());
}

#[tokio::test]
async fn a_read_only_session_is_refused_before_validation() {
    let fixture = fixture(Some(fake_piper()), false).await;

    // Even an invalid body gets the session refusal, like the other writes.
    for body in [json!({"text": "bonjour"}), json!({})] {
        let (status, answer) =
            send_json(&fixture.app, say_request(Some(READ_ONLY_TOKEN), &body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(answer["error"], "read_only_session");
    }

    assert_eq!(fixture.spoken(), None);
    assert!(fixture.jobs().await.is_empty());
}

#[tokio::test]
async fn invalid_bodies_are_refused_without_synthesis() {
    let fixture = fixture(Some(fake_piper()), false).await;

    for body in [
        json!({}),
        json!({"text": "   \n "}),
        json!({"text": 42}),
        json!({"text": "ok", "caption": 1}),
        json!({"text": "ok", "anonymous": "true"}),
        json!(["text"]),
    ] {
        let (status, answer) = send_json(&fixture.app, say_request(Some(TOKEN), &body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(answer["error"], "invalid_payload", "{body}");
    }

    // Malformed JSON.
    let request = Request::builder()
        .method(Method::POST)
        .uri("/overlay/say")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::from("{\"text\":"))
        .unwrap();
    let (status, answer) = send_json(&fixture.app, request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(answer["error"], "invalid_payload");

    assert_eq!(fixture.spoken(), None);
    assert!(fixture.jobs().await.is_empty());
}

#[tokio::test]
async fn a_text_over_the_limit_is_refused_with_the_limit() {
    let fixture = fixture(Some(fake_piper()), false).await;
    let body = json!({"text": "a".repeat(MAX_SPEECH_CHARS + 1)});

    let (status, answer) = send_json(&fixture.app, say_request(Some(TOKEN), &body)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(answer["error"], "text_too_long");
    assert_eq!(answer["maxChars"], 1000);
    assert_eq!(fixture.spoken(), None);
    assert!(fixture.jobs().await.is_empty());
}

#[tokio::test]
async fn a_failed_synthesis_answers_502_and_queues_nothing() {
    let fixture = fixture(None, false).await;

    let (status, answer) = send_json(
        &fixture.app,
        say_request(Some(TOKEN), &json!({"text": "bonjour"})),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(answer["error"], "speech_failed");
    assert!(fixture.jobs().await.is_empty());
}

/// Success: the job is the one `/dire` creates (speech asset, caption shown,
/// author, priority 0, duration of the audio).
#[tokio::test]
#[ignore = "nécessite ffmpeg/ffprobe installés"]
async fn saying_a_text_queues_the_speech_job_of_dire() {
    let fixture = fixture(Some(fake_piper()), true).await;

    let (status, answer) = send_json(
        &fixture.app,
        say_request(
            Some(TOKEN),
            &json!({"text": "  Bonjour   tout le monde \n", "caption": " Coucou "}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["queued"], true);
    // Piper received the collapsed text.
    assert_eq!(fixture.spoken().as_deref(), Some("Bonjour tout le monde"));

    let jobs = fixture.jobs().await;
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!(answer["jobId"], job.id.to_canonical());
    assert_eq!(answer["durationSec"], job.duration_sec);
    assert_eq!(job.status, JobStatus::Pending);
    assert_eq!(job.priority, 0);
    assert_eq!(job.resume_offset_sec, 0);
    assert_eq!(job.text.as_deref(), Some("Coucou"));
    assert!(job.show_text);
    // `insert_client` gives the overlay the default author "Alice".
    assert_eq!(job.author_name.as_deref(), Some("Alice"));
    assert_eq!(job.author_image, None);
    assert!(job.duration_sec >= 1);

    let asset = fixture
        .db
        .media_assets()
        .get(job.media_asset_id.expect("speech asset"))
        .await
        .unwrap()
        .expect("stored asset");
    assert_eq!(asset.kind, MediaKind::Audio);
    assert_eq!(asset.status, MediaStatus::Ready);
    assert_eq!(asset.mime, "audio/mpeg");
    assert_eq!(asset.duration_sec, Some(job.duration_sec));
    assert!(Path::new(&asset.storage_path).is_file());
}

/// `anonymous` is `/cdire`: same job without the author, and no caption
/// means no text shown.
#[tokio::test]
#[ignore = "nécessite ffmpeg/ffprobe installés"]
async fn an_anonymous_text_hides_the_author_like_cdire() {
    let fixture = fixture(Some(fake_piper()), true).await;

    let (status, answer) = send_json(
        &fixture.app,
        say_request(Some(TOKEN), &json!({"text": "Surprise", "anonymous": true})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{answer}");
    let jobs = fixture.jobs().await;
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!(job.author_name, None);
    assert_eq!(job.author_image, None);
    assert_eq!(job.text, None);
    assert!(!job.show_text);
    assert!(job.media_asset_id.is_some());
}
