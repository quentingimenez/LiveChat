//! Assemblage d'une carte tweet contenant une vidéo.
//!
//! Quand le tweet (ou le tweet cité) porte une vidéo, la carte PNG sert de
//! fond : chaque flux vidéo est incrusté dans la zone arrondie réservée par
//! [`crate::tweet_card::render_tweet_card_with_slots`], puis l'ensemble est
//! encodé en MP4 (H.264/AAC) pour que l'overlay le lise comme n'importe quelle
//! vidéo.
//!
//! La durée du fichier est celle de la vidéo du tweet parent ; une vidéo citée
//! plus courte laisse sa zone revenir au fond sombre de la carte, et une vidéo
//! citée seule fixe la durée.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::MediaConfig;
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::process;
use crate::tweet_card::{build_video_mask_png, CardVideoSlot, CardVideoTarget, RenderedCard};

/// Débit de la piste audio conservée (identique aux autres assemblages).
const AUDIO_BITRATE: &str = "192k";

/// Zone vidéo à incruster dans la carte.
#[derive(Clone, Debug)]
struct CompositeSlot {
    /// Flux vidéo téléchargé.
    video: PathBuf,
    /// Masque PNG (blanc opaque dans la zone visible, noir ailleurs).
    mask: PathBuf,
    /// Géométrie de la zone dans la carte.
    slot: CardVideoSlot,
    /// Dernière zone incrustée : elle fixe la durée du fichier.
    primary: bool,
}

/// Compose la carte PNG et les vidéos en un MP4 prêt pour l'overlay.
///
/// `parent_video` est la vidéo du tweet, `quoted_video` celle du tweet cité.
/// Au moins l'une des deux doit être fournie ; sinon une erreur est renvoyée et
/// l'appelant se rabat sur la carte PNG.
pub async fn compose_tweet_card_video(
    config: &MediaConfig,
    temp_dir: &Path,
    card: &RenderedCard,
    parent_video: Option<&Path>,
    quoted_video: Option<&Path>,
) -> MediaResult<PathBuf> {
    // La zone vidéo du tweet cité est incrustée d'abord : la vidéo du tweet
    // parent reste la dernière, c'est elle qui coupe le fichier.
    let mut layers: Vec<(CardVideoTarget, &Path)> = Vec::new();
    if let Some(video) = quoted_video {
        layers.push((CardVideoTarget::Quoted, video));
    }
    if let Some(video) = parent_video {
        layers.push((CardVideoTarget::Parent, video));
    }
    if layers.is_empty() {
        return Err(MediaError::new(
            MediaErrorCode::TranscodeFailed,
            "carte tweet sans zone vidéo à composer",
        ));
    }
    let primary = layers
        .last()
        .map(|(target, _)| *target)
        .unwrap_or(CardVideoTarget::Parent);

    let background = temp_dir.join("tweet-card.png");
    fs::write(&background, &card.png).map_err(|error| {
        MediaError::from_io(
            MediaErrorCode::TranscodeFailed,
            "écriture du fond de la carte tweet",
            &error,
        )
    })?;

    let mut slots: Vec<CompositeSlot> = Vec::new();
    for (index, (target, video)) in layers.iter().enumerate() {
        let slot = *card
            .video_slots
            .iter()
            .find(|slot| slot.target == *target)
            .ok_or_else(|| {
                MediaError::new(
                    MediaErrorCode::TranscodeFailed,
                    format!("zone vidéo {target:?} absente de la carte"),
                )
            })?;
        let mask_path = temp_dir.join(format!("tweet-mask-{index}.png"));
        let mask = build_video_mask_png(slot.width, slot.height, slot.radius)?;
        fs::write(&mask_path, mask).map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::TranscodeFailed,
                "écriture du masque vidéo de la carte tweet",
                &error,
            )
        })?;
        slots.push(CompositeSlot {
            video: video.to_path_buf(),
            mask: mask_path,
            slot,
            primary: *target == primary,
        });
    }

    let output = temp_dir.join("tweet-card.mp4");
    let args = build_composite_args(&background, &slots, &output, config);
    process::run_command(
        &config.tools.ffmpeg,
        &args,
        config.download.timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await?;
    Ok(output)
}

/// Construit les arguments ffmpeg d'assemblage (fonction pure, testable).
///
/// Entrée 0 : fond (carte PNG en boucle). Puis, pour chaque zone : le flux
/// vidéo et son masque. Le fond boucle indéfiniment ; la dernière zone
/// incrustée est déclarée `shortest` pour que le fichier s'arrête avec elle.
fn build_composite_args(
    background: &Path,
    slots: &[CompositeSlot],
    output: &Path,
    config: &MediaConfig,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-v".to_string(),
        "error".to_string(),
        "-loop".to_string(),
        "1".to_string(),
        "-framerate".to_string(),
        "30".to_string(),
        "-i".to_string(),
        background.display().to_string(),
    ];
    for slot in slots {
        args.push("-i".to_string());
        args.push(slot.video.display().to_string());
        // Le masque reste une image unique : `alphamerge` répète sa dernière
        // image et s'arrête avec la vidéo. Boucler le masque rendrait le flux
        // infini, et le fichier ne se terminerait jamais.
        args.push("-i".to_string());
        args.push(slot.mask.display().to_string());
    }

    let mut filters: Vec<String> = Vec::new();
    let mut base = "[0:v]".to_string();
    for (index, slot) in slots.iter().enumerate() {
        let video_index = 1 + index * 2;
        let mask_index = 2 + index * 2;
        let width = slot.slot.width;
        let height = slot.slot.height;
        // La vidéo remplit la zone (recadrage centré), le masque arrondi décide
        // de ce qui reste visible.
        filters.push(format!(
            "[{video_index}:v]scale={width}:{height}:force_original_aspect_ratio=increase,\
             crop={width}:{height},setsar=1,fps=30,format=rgba[v{index}]"
        ));
        filters.push(format!("[{mask_index}:v]format=gray[m{index}]"));
        filters.push(format!("[v{index}][m{index}]alphamerge[o{index}]"));
        let output_label = format!("[b{}]", index + 1);
        let options = if slot.primary {
            // Coupe le fichier avec la vidéo principale.
            ":shortest=1".to_string()
        } else {
            // Une vidéo citée plus courte rend sa zone au fond de la carte.
            ":eof_action=pass".to_string()
        };
        filters.push(format!(
            "{base}[o{index}]overlay={}:{}{options}{output_label}",
            slot.slot.x, slot.slot.y
        ));
        base = output_label;
    }
    args.push("-filter_complex".to_string());
    args.push(filters.join(";"));
    args.push("-map".to_string());
    args.push(base);

    // Le son du tweet est conservé ; la vidéo citée reste muette.
    if let Some(parent) = slots.iter().position(|slot| slot.primary) {
        let audio_index = 1 + parent * 2;
        args.push("-map".to_string());
        args.push(format!("{audio_index}:a:0?"));
        args.push("-c:a".to_string());
        args.push("aac".to_string());
        args.push("-b:a".to_string());
        args.push(AUDIO_BITRATE.to_string());
    } else {
        args.push("-an".to_string());
    }

    args.push("-c:v".to_string());
    args.push("libx264".to_string());
    args.push("-preset".to_string());
    args.push(config.video.cpu_preset.clone());
    args.push("-pix_fmt".to_string());
    args.push("yuv420p".to_string());
    args.push("-movflags".to_string());
    args.push("+faststart".to_string());
    args.push(output.display().to_string());
    args
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn slot(target: CardVideoTarget, primary: bool) -> CompositeSlot {
        CompositeSlot {
            video: PathBuf::from("clip.mp4"),
            mask: PathBuf::from("mask.png"),
            slot: CardVideoSlot {
                x: 48,
                y: 640,
                width: 540,
                height: 960,
                radius: 14,
                target,
            },
            primary,
        }
    }

    fn config() -> MediaConfig {
        MediaConfig::default()
    }

    #[test]
    fn builds_the_composite_command_for_a_tweet_video() {
        let args = build_composite_args(
            Path::new("card.png"),
            &[slot(CardVideoTarget::Parent, true)],
            Path::new("card.mp4"),
            &config(),
        );
        assert_eq!(args.first().map(String::as_str), Some("-y"));
        let filter = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .map(|index| args[index + 1].clone())
            .expect("filtre présent");
        assert!(filter.contains("[1:v]scale=540:960:force_original_aspect_ratio=increase"));
        assert!(filter.contains("crop=540:960"));
        assert!(filter.contains("[2:v]format=gray"));
        assert!(filter.contains("alphamerge[o0]"));
        assert!(filter.contains("[0:v][o0]overlay=48:640:shortest=1[b1]"));
        assert!(args.contains(&"1:a:0?".to_string()));
        assert!(args.contains(&"libx264".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("card.mp4"));
    }

    #[test]
    fn lets_the_quoted_video_restore_the_card_background() {
        let args = build_composite_args(
            Path::new("card.png"),
            &[
                slot(CardVideoTarget::Quoted, false),
                slot(CardVideoTarget::Parent, true),
            ],
            Path::new("card.mp4"),
            &config(),
        );
        let filter = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .map(|index| args[index + 1].clone())
            .expect("filtre présent");
        // Vidéo citée : fond rendu au bout, pas de coupe du fichier.
        assert!(filter.contains("[0:v][o0]overlay=48:640:eof_action=pass[b1]"));
        // Vidéo parente : coupe le fichier.
        assert!(filter.contains("[b1][o1]overlay=48:640:shortest=1[b2]"));
        // Entrées : fond, vidéo citée + masque, vidéo parente + masque.
        assert!(args.contains(&"3:a:0?".to_string()));
    }

    #[test]
    fn refuses_a_card_without_video_zone() {
        let result = futures_lite_block_on(compose_without_slots());
        assert!(result.is_err());
    }

    async fn compose_without_slots() -> MediaResult<PathBuf> {
        let config = config();
        let directory = std::env::temp_dir().join("lc-tweet-card-video-test");
        let card = RenderedCard {
            png: Vec::new(),
            video_slots: Vec::new(),
        };
        compose_tweet_card_video(&config, &directory, &card, None, None).await
    }

    /// Exécute une future sans dépendre d'un runtime tokio complet.
    fn futures_lite_block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }
}
