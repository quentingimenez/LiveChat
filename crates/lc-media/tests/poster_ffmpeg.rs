//! Image représentative d'une vidéo, produite avec le vrai `ffmpeg`.
//!
//! Ignoré par défaut (nécessite `ffmpeg` sur le PATH) :
//! `cargo test -p lc-media --test poster_ffmpeg -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use lc_media::poster::{extract_poster, PosterSource};
use lc_media::{MediaConfig, MediaStorage};

fn config(directory: &Path) -> MediaConfig {
    let mut config = MediaConfig::default();
    config.storage.directory = directory.join("media");
    config
}

/// Une vidéo noire pendant 1,5 s puis un motif détaillé : la forme d'un
/// téléversement dont la première frame fait une vignette noire.
fn black_intro_video(config: &MediaConfig, directory: &Path) -> PathBuf {
    let path = directory.join("black-intro.mp4");
    let status = std::process::Command::new(&config.tools.ffmpeg)
        .args([
            "-y",
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black:s=320x240:d=1.5",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2.5:size=320x240:rate=30",
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0[v]",
            "-map",
            "[v]",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .status()
        .expect("ffmpeg démarré");
    assert!(status.success(), "échantillon vidéo non produit");
    path
}

/// Luminosité moyenne (`0..=255`) d'une frame ou d'une image, lue par `ffmpeg`.
fn mean_luma(config: &MediaConfig, input: &Path, at_sec: Option<f64>) -> f64 {
    let raster = tempfile::Builder::new().suffix(".gray").tempfile().unwrap();
    let mut args: Vec<String> = vec![
        "-y".to_owned(),
        "-hide_banner".to_owned(),
        "-v".to_owned(),
        "error".to_owned(),
    ];
    if let Some(at_sec) = at_sec {
        args.push("-ss".to_owned());
        args.push(format!("{at_sec:.3}"));
    }
    args.extend([
        "-i".to_owned(),
        input.display().to_string(),
        "-frames:v".to_owned(),
        "1".to_owned(),
        "-vf".to_owned(),
        "scale=96:-2,format=gray".to_owned(),
        "-f".to_owned(),
        "rawvideo".to_owned(),
        raster.path().display().to_string(),
    ]);
    let status = std::process::Command::new(&config.tools.ffmpeg)
        .args(&args)
        .status()
        .expect("ffmpeg démarré");
    assert!(
        status.success(),
        "raster non produit pour {}",
        input.display()
    );
    let bytes = std::fs::read(raster.path()).unwrap();
    let sum: f64 = bytes.iter().map(|value| f64::from(*value)).sum();
    sum / bytes.len() as f64
}

#[tokio::test]
#[ignore = "nécessite ffmpeg installé"]
async fn une_intro_noire_ne_donne_pas_une_image_noire() {
    let directory = tempfile::TempDir::new().unwrap();
    let config = config(directory.path());
    let input = black_intro_video(&config, directory.path());
    let storage = MediaStorage::from_root(&config.storage.directory);
    storage.ensure().unwrap();

    // Ce que la carte montrait jusqu'ici : la frame à 0,1 s, donc du noir.
    let first_frame = mean_luma(&config, &input, Some(0.1));
    assert!(
        first_frame < 26.0,
        "l'échantillon devrait commencer noir : {first_frame}"
    );

    let cover = extract_poster(
        &config,
        &storage,
        "abc123",
        &PosterSource {
            input: &input,
            width: Some(320),
            height: Some(240),
            duration_sec: Some(4.0),
        },
    )
    .await
    .expect("une image extraite");

    assert_eq!(cover.mime, "image/jpeg");
    // The source is smaller than the limit: the image is not enlarged.
    assert_eq!((cover.width, cover.height), (Some(320), Some(240)));
    assert!(cover.size_bytes >= 1024);
    assert!(cover.storage_path.is_file());
    assert_eq!(
        cover.storage_path,
        config.storage.directory.join("abc123.cover.jpg")
    );

    let poster_luma = mean_luma(&config, &cover.storage_path, None);
    assert!(
        poster_luma > 60.0,
        "image encore sombre malgré une vidéo lumineuse : {poster_luma}"
    );
}

#[tokio::test]
#[ignore = "nécessite ffmpeg installé"]
async fn une_video_entierement_noire_n_a_pas_d_image() {
    let directory = tempfile::TempDir::new().unwrap();
    let config = config(directory.path());
    let input = directory.path().join("black.mp4");
    let status = std::process::Command::new(&config.tools.ffmpeg)
        .args([
            "-y",
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black:s=320x240:d=2:r=30",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&input)
        .status()
        .expect("ffmpeg démarré");
    assert!(status.success(), "échantillon vidéo non produit");
    let storage = MediaStorage::from_root(&config.storage.directory);
    storage.ensure().unwrap();

    let cover = extract_poster(
        &config,
        &storage,
        "noir000",
        &PosterSource {
            input: &input,
            width: Some(320),
            height: Some(240),
            duration_sec: Some(2.0),
        },
    )
    .await;

    // A black card is not better than the glyph the card already shows.
    assert!(cover.is_none(), "aucune image pour une vidéo noire");
    assert!(!config.storage.directory.join("noir000.cover.jpg").exists());
}
