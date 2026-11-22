//! Image représentative d'une vidéo qui n'a pas de miniature de plateforme.
//!
//! La première frame d'un média téléversé est souvent noire (fondu
//! d'ouverture, carton de titre, amorce d'encodeur) : une carte qui l'affiche
//! paraît vide. L'image est donc choisie parmi plusieurs frames de la vidéo :
//! une frame quasi noire ou sans le moindre détail n'est jamais retenue, et la
//! plus lumineuse et la plus détaillée près du milieu l'emporte.
//!
//! Les frames sont lues par `ffmpeg` comme de minuscules rasters en niveaux de
//! gris (quelques Kio, aucun décodeur d'image en dépendance) : le choix n'a
//! besoin que de la luminosité moyenne et de l'écart-type d'une frame. La frame
//! retenue est encodée une seule fois en JPEG, à côté de la vidéo
//! (`<hash>.cover.jpg`, le nom des miniatures de plateforme).
//!
//! Tout est au mieux : une vidéo illisible ou un `ffmpeg` absent rendent
//! `None`, la carte garde son glyphe.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::MediaConfig;
use crate::error::MediaErrorCode;
use crate::process;
use crate::storage::{staged_path, MediaStorage};
use crate::thumbnail::image_dimensions;
use crate::transcode::NormalizedCover;

/// Type MIME de l'image produite.
pub const POSTER_MIME: &str = "image/jpeg";

/// Plus grande dimension de l'image produite.
///
/// Une carte du board fait quelques centaines de pixels de large et le tiroir
/// de détail ne la montre pas plus grande.
pub const POSTER_MAX_SIDE: u32 = 640;

/// Largeur du raster lu pour mesurer une frame.
const ANALYSIS_WIDTH: u32 = 96;

/// Fractions de la vidéo où une frame est lue. Le tout début et la toute fin
/// sont écartés : une intro apparaît en fondu, un générique s'efface.
const SAMPLE_FRACTIONS: [f64; 7] = [0.12, 0.25, 0.38, 0.5, 0.62, 0.75, 0.88];

/// Écart minimal entre deux frames lues (secondes).
const MIN_SAMPLE_GAP_SEC: f64 = 0.1;

/// Luminosité moyenne (`0..=255`) sous laquelle une frame est noire.
const BLACK_LUMA: f64 = 26.0;

/// Écart-type de luminosité sous lequel une frame est uniforme (aucun détail).
const FLAT_DETAIL: f64 = 7.0;

/// Écart-type à partir duquel une frame est jugée entièrement détaillée
/// (score `1.0`).
const DETAIL_SATURATION: f64 = 64.0;

/// Poids de la luminosité dans le choix d'une frame.
const LUMA_WEIGHT: f64 = 0.4;
/// Poids du détail dans le choix d'une frame.
const DETAIL_WEIGHT: f64 = 0.5;
/// Poids de la proximité du milieu de la vidéo dans le choix d'une frame.
const MIDDLE_WEIGHT: f64 = 0.1;

/// Qualité JPEG de l'image produite (`ffmpeg -q:v`, 2 = la meilleure).
const POSTER_QUALITY: u8 = 3;

/// Taille minimale de l'image produite : en dessous, le fichier n'est pas un
/// JPEG.
const MIN_POSTER_BYTES: u64 = 1024;

/// Délai maximal d'un appel `ffmpeg` (recherche + une frame, ou un encodage).
const FRAME_TIMEOUT: Duration = Duration::from_secs(15);

/// La vidéo dont on tire l'image.
#[derive(Debug, Clone, Copy)]
pub struct PosterSource<'a> {
    /// Fichier de la vidéo normalisée.
    pub input: &'a Path,
    /// Largeur d'affichage, quand elle est connue.
    pub width: Option<u32>,
    /// Hauteur d'affichage, quand elle est connue.
    pub height: Option<u32>,
    /// Durée de la vidéo, quand elle est connue.
    pub duration_sec: Option<f64>,
}

/// Une frame lue dans la vidéo.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FrameSample {
    /// Position dans la vidéo, `0..=1`.
    fraction: f64,
    /// Position dans la vidéo, en secondes.
    at_sec: f64,
}

/// Luminosité d'une frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FrameStats {
    /// Luma moyenne, `0..=255`.
    mean: f64,
    /// Écart-type de la luma : le détail de l'image.
    detail: f64,
}

impl FrameStats {
    /// Mesure un raster en niveaux de gris (un octet par pixel).
    fn of_luma(raster: &[u8]) -> Option<Self> {
        if raster.is_empty() {
            return None;
        }
        let count = raster.len() as f64;
        let sum: f64 = raster.iter().map(|value| f64::from(*value)).sum();
        let mean = sum / count;
        let variance: f64 = raster
            .iter()
            .map(|value| {
                let delta = f64::from(*value) - mean;
                delta * delta
            })
            .sum::<f64>()
            / count;
        Some(Self {
            mean,
            detail: variance.max(0.0).sqrt(),
        })
    }

    /// Une frame noire : rien à montrer.
    fn is_black(&self) -> bool {
        !self.mean.is_finite() || self.mean < BLACK_LUMA
    }

    /// Une frame montrable : ni noire, ni vide.
    fn is_usable(&self) -> bool {
        !self.is_black() && self.detail.is_finite() && self.detail >= FLAT_DETAIL
    }
}

/// Extrait l'image représentative de `source` et l'écrit à côté de la vidéo.
///
/// `storage` reçoit `<source_hash>.cover.jpg`, la même famille de fichiers que
/// les miniatures de plateforme (`lc_media::thumbnail`). `None` quand aucune
/// frame ne peut être lue ou montrée : l'appelant garde alors son glyphe.
pub async fn extract_poster(
    config: &MediaConfig,
    storage: &MediaStorage,
    source_hash: &str,
    source: &PosterSource<'_>,
) -> Option<NormalizedCover> {
    let samples = sample_times(source.duration_sec);
    if samples.is_empty() {
        return None;
    }
    let mut measured = Vec::with_capacity(samples.len());
    for sample in samples {
        if let Some(stats) = frame_stats(config, source, sample.at_sec).await {
            measured.push((sample, stats));
        }
    }
    let frame = pick_frame(&measured)?;
    encode_poster(config, storage, source_hash, source, frame.at_sec).await
}

/// Positions des frames à lire dans une vidéo de cette durée.
///
/// Une vidéo courte ne lit pas deux fois la même image : les positions trop
/// proches sont écartées. Durée inconnue : aucune frame.
fn sample_times(duration_sec: Option<f64>) -> Vec<FrameSample> {
    let Some(duration) = duration_sec else {
        return Vec::new();
    };
    if !duration.is_finite() || duration <= 0.0 {
        return Vec::new();
    }
    let mut samples: Vec<FrameSample> = Vec::with_capacity(SAMPLE_FRACTIONS.len());
    for fraction in SAMPLE_FRACTIONS {
        let at_sec = duration * fraction;
        if samples
            .last()
            .is_some_and(|last| at_sec - last.at_sec < MIN_SAMPLE_GAP_SEC)
        {
            continue;
        }
        samples.push(FrameSample { fraction, at_sec });
    }
    samples
}

/// La frame retenue parmi des frames mesurées : la meilleure frame montrable,
/// sinon la meilleure frame simplement visible (une vidéo fixe n'a aucun
/// détail mais reste montrable). Aucune frame noire n'est jamais retenue.
fn pick_frame(measured: &[(FrameSample, FrameStats)]) -> Option<FrameSample> {
    best_frame(measured, FrameStats::is_usable)
        .or_else(|| best_frame(measured, |stats| !stats.is_black()))
}

fn best_frame(
    measured: &[(FrameSample, FrameStats)],
    keep: impl Fn(&FrameStats) -> bool,
) -> Option<FrameSample> {
    measured
        .iter()
        .filter(|(_, stats)| keep(stats))
        .max_by(|(left, left_stats), (right, right_stats)| {
            score(left_stats, left.fraction).total_cmp(&score(right_stats, right.fraction))
        })
        .map(|(sample, _)| *sample)
}

/// Note d'une frame : lumineuse, détaillée, et proche du milieu de la vidéo.
fn score(stats: &FrameStats, fraction: f64) -> f64 {
    let luma = (stats.mean / 255.0).clamp(0.0, 1.0);
    let detail = (stats.detail / DETAIL_SATURATION).clamp(0.0, 1.0);
    let middle = 1.0 - (fraction.clamp(0.0, 1.0) - 0.5).abs() * 2.0;
    LUMA_WEIGHT * luma + DETAIL_WEIGHT * detail + MIDDLE_WEIGHT * middle
}

/// Lit une frame en petit raster en niveaux de gris et la mesure.
async fn frame_stats(
    config: &MediaConfig,
    source: &PosterSource<'_>,
    at_sec: f64,
) -> Option<FrameStats> {
    let raster = tempfile::Builder::new()
        .prefix("lc-poster-")
        .suffix(".gray")
        .tempfile()
        .ok()?;
    let args = build_frame_args(source, at_sec, raster.path());
    match process::run_command(
        &config.tools.ffmpeg,
        &args,
        FRAME_TIMEOUT,
        MediaErrorCode::TranscodeFailed,
    )
    .await
    {
        Ok(_) => {}
        Err(error) => {
            tracing::debug!(error = %error.summary(), "frame de la vidéo illisible");
            return None;
        }
    }
    let bytes = std::fs::read(raster.path()).ok()?;
    FrameStats::of_luma(&bytes)
}

/// Encode la frame retenue en JPEG et la place à côté de la vidéo.
async fn encode_poster(
    config: &MediaConfig,
    storage: &MediaStorage,
    source_hash: &str,
    source: &PosterSource<'_>,
    at_sec: f64,
) -> Option<NormalizedCover> {
    let output = poster_path(storage, source_hash);
    let staged = staged_path(&output);
    let args = build_poster_args(source, at_sec, &staged);
    match process::run_command(
        &config.tools.ffmpeg,
        &args,
        FRAME_TIMEOUT,
        MediaErrorCode::TranscodeFailed,
    )
    .await
    {
        Ok(_) => {}
        Err(error) => {
            let _removed = std::fs::remove_file(&staged);
            tracing::warn!(error = %error.summary(), "image de la vidéo non extraite");
            return None;
        }
    }
    if let Err(error) = MediaStorage::commit_file(&staged, &output) {
        let _removed = std::fs::remove_file(&staged);
        tracing::warn!(error = %error.summary(), "image de la vidéo non enregistrée");
        return None;
    }
    let size_bytes = match MediaStorage::file_size(&output) {
        Ok(size_bytes) if size_bytes >= MIN_POSTER_BYTES => size_bytes,
        _ => {
            let _removed = std::fs::remove_file(&output);
            return None;
        }
    };
    let dimensions = std::fs::read(&output)
        .ok()
        .and_then(|bytes| image_dimensions(&bytes));
    Some(NormalizedCover {
        mime: POSTER_MIME.to_owned(),
        width: dimensions.map(|(width, _)| width),
        height: dimensions.map(|(_, height)| height),
        storage_path: output,
        size_bytes,
    })
}

/// Emplacement de l'image d'une vidéo, à côté de son fichier normalisé.
fn poster_path(storage: &MediaStorage, source_hash: &str) -> PathBuf {
    storage
        .output_base_path(source_hash)
        .with_extension("cover.jpg")
}

/// Largeur du raster de sortie : dans le sens de la vidéo, jamais agrandie.
fn target_width(width: Option<u32>, height: Option<u32>, max_side: u32) -> u32 {
    let max_side = max_side.max(2);
    let scaled = match (width, height) {
        // Vertical : la hauteur est le grand côté, la largeur suit.
        (Some(width), Some(height)) if width > 0 && height > 0 && height > width => {
            u64::from(width) * u64::from(max_side) / u64::from(height)
        }
        (Some(width), _) if width > 0 => u64::from(width.min(max_side)),
        _ => u64::from(max_side),
    };
    let rounded = u32::try_from(scaled).unwrap_or(max_side);
    // La largeur est arrondie au pair : le décodeur JPEG et les formats
    // sous-échantillonnés en chroma l'exigent.
    (rounded.max(2) / 2 * 2).max(2)
}

/// Filtre de mise à l'échelle vers un raster mesurable.
fn analysis_filter(source: &PosterSource<'_>) -> String {
    let width = target_width(source.width, source.height, ANALYSIS_WIDTH);
    format!("scale={width}:-2,format=gray")
}

/// Filtre de mise à l'échelle de l'image produite.
fn poster_filter(source: &PosterSource<'_>) -> String {
    let width = target_width(source.width, source.height, POSTER_MAX_SIDE);
    format!("scale={width}:-2")
}

/// Commande `ffmpeg` qui écrit une frame en raster `gray` brut, à l'échelle
/// prévue pour la vidéo.
fn build_frame_args(source: &PosterSource<'_>, at_sec: f64, output: &Path) -> Vec<String> {
    let mut args = vec![
        "-y".to_owned(),
        "-hide_banner".to_owned(),
        "-v".to_owned(),
        "error".to_owned(),
        // Avant `-i` : la recherche passe par l'index au lieu de décoder la
        // vidéo depuis le début.
        "-ss".to_owned(),
        format!("{at_sec:.3}"),
        "-i".to_owned(),
        source.input.display().to_string(),
        "-an".to_owned(),
        "-sn".to_owned(),
        "-dn".to_owned(),
        "-frames:v".to_owned(),
        "1".to_owned(),
        "-vf".to_owned(),
        analysis_filter(source),
        "-f".to_owned(),
        "rawvideo".to_owned(),
    ];
    args.push(output.display().to_string());
    args
}

/// Commande `ffmpeg` qui encode une frame en JPEG.
fn build_poster_args(source: &PosterSource<'_>, at_sec: f64, output: &Path) -> Vec<String> {
    let mut args = vec![
        "-y".to_owned(),
        "-hide_banner".to_owned(),
        "-v".to_owned(),
        "error".to_owned(),
        "-ss".to_owned(),
        format!("{at_sec:.3}"),
        "-i".to_owned(),
        source.input.display().to_string(),
        "-an".to_owned(),
        "-sn".to_owned(),
        "-dn".to_owned(),
        "-frames:v".to_owned(),
        "1".to_owned(),
        "-vf".to_owned(),
        poster_filter(source),
        "-q:v".to_owned(),
        POSTER_QUALITY.to_string(),
        "-f".to_owned(),
        "mjpeg".to_owned(),
    ];
    args.push(output.display().to_string());
    args
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn sample(fraction: f64) -> FrameSample {
        FrameSample {
            fraction,
            at_sec: fraction * 10.0,
        }
    }

    fn stats(mean: f64, detail: f64) -> FrameStats {
        FrameStats { mean, detail }
    }

    #[test]
    fn une_video_sans_duree_n_a_aucune_frame() {
        assert!(sample_times(None).is_empty());
        assert!(sample_times(Some(0.0)).is_empty());
        assert!(sample_times(Some(-4.0)).is_empty());
        assert!(sample_times(Some(f64::NAN)).is_empty());
    }

    #[test]
    fn les_echantillons_evitent_le_debut_et_la_fin() {
        let samples = sample_times(Some(4.0));
        assert_eq!(samples.len(), SAMPLE_FRACTIONS.len());
        let first = samples.first().unwrap();
        let last = samples.last().unwrap();
        assert!(first.at_sec >= 4.0 * 0.1, "première frame : {first:?}");
        assert!(last.at_sec <= 4.0 * 0.9, "dernière frame : {last:?}");
        assert!(samples
            .windows(2)
            .all(|pair| pair[0].at_sec < pair[1].at_sec));
    }

    #[test]
    fn les_echantillons_d_une_video_courte_restent_espaces() {
        let samples = sample_times(Some(0.5));
        assert!(samples.len() >= 2, "frames lues : {samples:?}");
        assert!(
            samples
                .windows(2)
                .all(|pair| pair[1].at_sec - pair[0].at_sec >= MIN_SAMPLE_GAP_SEC - 1e-9),
            "frames trop proches : {samples:?}"
        );
    }

    #[test]
    fn une_frame_noire_est_reconnue() {
        let raster = vec![0u8; 96 * 54];
        let black = FrameStats::of_luma(&raster).unwrap();
        assert!(black.is_black());
        assert!(!black.is_usable());
        assert_eq!(FrameStats::of_luma(&[]), None);
    }

    #[test]
    fn une_frame_unie_visible_n_est_pas_noire_mais_reste_plate() {
        let raster = vec![80u8; 96 * 54];
        let flat = FrameStats::of_luma(&raster).unwrap();
        assert!(!flat.is_black());
        assert!(!flat.is_usable(), "une frame unie n'a aucun détail");
        assert!((flat.mean - 80.0).abs() < 1e-9);
        assert!(flat.detail < 1e-9);
    }

    #[test]
    fn une_frame_detaillee_est_utilisable() {
        let mut raster = vec![0u8; 96 * 54];
        for (index, value) in raster.iter_mut().enumerate() {
            *value = if index % 2 == 0 { 0 } else { 255 };
        }
        let checkerboard = FrameStats::of_luma(&raster).unwrap();
        assert!((checkerboard.mean - 127.5).abs() < 1e-9);
        assert!(checkerboard.detail > FLAT_DETAIL);
        assert!(checkerboard.is_usable());
    }

    #[test]
    fn la_frame_la_plus_lumineuse_et_detaillee_du_milieu_gagne() {
        let measured = [
            (sample(0.12), stats(200.0, 60.0)),
            (sample(0.38), stats(120.0, 40.0)),
            (sample(0.62), stats(220.0, 64.0)),
        ];
        assert_eq!(pick_frame(&measured), Some(sample(0.62)));
    }

    #[test]
    fn a_egalite_la_frame_la_plus_proche_du_milieu_gagne() {
        let measured = [
            (sample(0.12), stats(180.0, 40.0)),
            (sample(0.38), stats(180.0, 40.0)),
            (sample(0.75), stats(180.0, 40.0)),
        ];
        assert_eq!(pick_frame(&measured), Some(sample(0.38)));
    }

    #[test]
    fn une_frame_noire_n_est_jamais_choisie() {
        let measured = [
            (sample(0.12), stats(0.0, 0.0)),
            (sample(0.5), stats(8.0, 3.0)),
            (sample(0.88), stats(16.0, 0.0)),
        ];
        assert_eq!(pick_frame(&measured), None);
    }

    #[test]
    fn une_video_fixe_garde_une_frame_visible() {
        let measured = [
            (sample(0.12), stats(0.0, 0.0)),
            // Visible mais sans aucun détail : mieux qu'une carte noire.
            (sample(0.5), stats(90.0, 0.5)),
        ];
        assert_eq!(pick_frame(&measured), Some(sample(0.5)));
    }

    #[test]
    fn la_largeur_de_sortie_ne_depasse_pas_la_limite() {
        assert_eq!(target_width(Some(1920), Some(1080), 640), 640);
        assert_eq!(target_width(Some(3840), Some(2160), 96), 96);
        assert_eq!(target_width(None, None, 640), 640);
    }

    #[test]
    fn la_largeur_de_sortie_ne_grandit_jamais_la_video() {
        assert_eq!(target_width(Some(320), Some(240), 640), 320);
        assert_eq!(target_width(Some(96), Some(54), 640), 96);
    }

    #[test]
    fn la_largeur_d_une_video_verticale_suit_sa_hauteur() {
        assert_eq!(target_width(Some(1080), Some(1920), 640), 360);
        // Une largeur calculée impaire est arrondie au pair.
        assert_eq!(target_width(Some(1079), Some(1920), 640), 358);
    }

    #[test]
    fn la_commande_echantillonne_un_raster_gris() {
        let args = build_frame_args(
            &PosterSource {
                input: Path::new("clip.mp4"),
                width: Some(1920),
                height: Some(1080),
                duration_sec: Some(30.0),
            },
            3.5,
            Path::new("frame.gray"),
        );
        let seek = args.iter().position(|arg| arg == "-ss").unwrap();
        let input = args.iter().position(|arg| arg == "-i").unwrap();
        assert!(seek < input, "la recherche précède l'entrée : {args:?}");
        assert!(args.contains(&"3.500".to_owned()));
        assert!(args.contains(&"scale=96:-2,format=gray".to_owned()));
        assert!(args.contains(&"rawvideo".to_owned()));
        assert_eq!(args.last().unwrap(), "frame.gray");
    }

    #[test]
    fn la_commande_d_encode_un_jpeg_borne() {
        let args = build_poster_args(
            &PosterSource {
                input: Path::new("clip mp4.mp4"),
                width: Some(1920),
                height: Some(1080),
                duration_sec: Some(30.0),
            },
            15.0,
            Path::new("clip.cover.jpg"),
        );
        assert!(args.contains(&"scale=640:-2".to_owned()));
        assert!(args.contains(&"mjpeg".to_owned()));
        assert!(args.contains(&POSTER_QUALITY.to_string()));
        assert!(!args.contains(&"format=gray".to_owned()));
        assert_eq!(args.last().unwrap(), "clip.cover.jpg");
    }
}
