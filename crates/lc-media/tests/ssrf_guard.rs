//! Tests d'intégration de la garde SSRF/LFI (P0-1).
//!
//! Ces tests n'ont besoin d'aucun accès réseau : la garde doit refuser une
//! cible interne **avant** d'ouvrir la moindre connexion. Le faux serveur
//! loopback sert de sentinelle : si une connexion était tentée, `accept()`
//! la verrait.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::ErrorKind;
use std::net::TcpListener;
use std::time::{Duration, Instant};

use lc_media::guard::{parse_media_url, resolve_redirect, validate_target};
use lc_media::{MediaConfig, MediaErrorCode, MediaPipeline};

fn config(temp: &tempfile::TempDir) -> MediaConfig {
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    config
}

#[tokio::test]
async fn loopback_source_is_refused_before_any_connection() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();

    let temp = tempfile::TempDir::new().unwrap();
    let pipeline = MediaPipeline::new(config(&temp)).unwrap();
    let url = format!("http://127.0.0.1:{port}/video.mp4");
    let error = pipeline.ingest_url(&url).await.unwrap_err();

    assert_eq!(error.code(), MediaErrorCode::UnsupportedSource);
    assert!(
        error.message().contains("interne") || error.message().contains("boucle locale"),
        "message inattendu : {}",
        error.message()
    );

    // Sentinelle : aucune connexion ne doit être acceptée.
    let deadline = Instant::now() + Duration::from_millis(400);
    loop {
        match listener.accept() {
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            other => panic!("une connexion a été acceptée malgré le refus : {other:?}"),
        }
    }
}

#[tokio::test]
async fn unsafe_schemes_and_paths_are_refused_without_network() {
    let temp = tempfile::TempDir::new().unwrap();
    let pipeline = MediaPipeline::new(config(&temp)).unwrap();
    let cases = [
        "file:/etc/passwd",
        "file:C:\\Windows\\win.ini",
        "file:///etc/passwd",
        "gtts:bonjour",
        "\\\\server\\share\\video.mp4",
        "//server/share/video.mp4",
        "C:\\video.mp4",
        "ftp://example.com/video.mp4",
        "data:video/mp4;base64,AAAA",
        "",
    ];
    for raw in cases {
        let error = pipeline
            .ingest_url(raw)
            .await
            .expect_err(&format!("la source {raw:?} devait être refusée"));
        assert_eq!(
            error.code(),
            MediaErrorCode::UnsupportedSource,
            "{raw:?} : {error}"
        );
    }
}

#[tokio::test]
async fn internal_redirect_targets_are_refused() {
    let base =
        parse_media_url("https://www.tiktok.com/@livechat/video/7300000000000000000").unwrap();
    let next = resolve_redirect(&base, "http://127.0.0.1:8080/secret.mp4").unwrap();
    let error = validate_target(next.as_str()).await.unwrap_err();
    assert_eq!(error.code(), MediaErrorCode::UnsupportedSource);
    assert!(
        error.message().contains("boucle locale") || error.message().contains("interne"),
        "message inattendu : {}",
        error.message()
    );

    let file_redirect = resolve_redirect(&base, "file:///etc/passwd").unwrap_err();
    assert_eq!(file_redirect.code(), MediaErrorCode::UnsupportedSource);

    let relative = resolve_redirect(&base, "../clip.mp4").unwrap();
    assert_eq!(
        relative.as_str(),
        "https://www.tiktok.com/@livechat/clip.mp4"
    );
}

#[tokio::test]
async fn public_ip_literals_are_accepted_without_dns() {
    let target = validate_target("https://93.184.216.34/video.mp4")
        .await
        .unwrap();
    assert_eq!(target.host, "93.184.216.34");
    assert_eq!(target.port, 443);
    assert!(!target.addrs.is_empty());
}

#[tokio::test]
async fn private_and_metadata_addresses_are_refused() {
    for raw in [
        "http://127.0.0.1:8080/video.mp4",
        "http://[::1]:8080/video.mp4",
        "http://169.254.169.254/latest/meta-data/",
        "http://10.0.0.1/video.mp4",
        "http://192.168.0.1/video.mp4",
        "http://[fd00::1]/video.mp4",
    ] {
        let error = validate_target(raw)
            .await
            .expect_err(&format!("{raw} devait être refusée"));
        assert_eq!(error.code(), MediaErrorCode::UnsupportedSource, "{raw}");
    }
}
