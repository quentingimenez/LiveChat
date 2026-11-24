//! Media edits applied with ffmpeg to an already normalized audio or video:
//! trimming and cutting, cropping, rotating, flipping, speed, volume, fades,
//! reversing, audio extraction and GIF export.
//!
//! The board sends an [`EditRecipe`]; [`EditRecipe::plan`] validates it against
//! the source and returns an [`EditPlan`], the only input of the pure argument
//! builder [`build_edit_args`]. [`apply_edit`] runs the result.
//!
//! Order of the picture operations: masks, annotations and colour adjustments
//! (on the source frame), crop (still on the source frame), rotation, then the
//! two flips (in the rotated frame). The board draws annotations in the same
//! order, so the editor preview matches the render.
//!
//! ffmpeg's `reverse` keeps every decoded frame of its input in memory, so a
//! video longer than [`REVERSE_PIECE_SEC`] is reversed piece by piece: each
//! piece is cut, framed and reversed by its own ffmpeg run, then the pieces are
//! joined in reverse order and the rest of the edit (speed, volume, fades,
//! size, format) is applied to the whole. The output of a video edit is
//! H.264 yuv420p + AAC in MP4, under the configured maximum height, so the
//! normalizer copies it instead of encoding it a second time.

use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use lc_domain::MediaKind;
use serde::Deserialize;

use crate::config::{MediaConfig, VideoEncoder};
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::process;
use crate::transcode::{has_nvenc, resolve_video_encoder, AUDIO_BITRATE};

/// Slowest accepted playback speed.
pub const MIN_SPEED: f64 = 0.25;
/// Fastest accepted playback speed.
pub const MAX_SPEED: f64 = 4.0;
/// Largest accepted linear gain.
pub const MAX_VOLUME: f64 = 3.0;
/// Longest accepted fade, in seconds.
pub const MAX_FADE_SEC: f64 = 10.0;
/// Shortest media an edit may produce, in source seconds kept.
pub const MIN_KEPT_SEC: f64 = 0.1;
/// Longest kept duration of a video reversed by a single ffmpeg run; a
/// longer one is reversed in pieces of at most this length.
pub const REVERSE_PIECE_SEC: f64 = 5.0;
/// Longest GIF an edit may produce, in output seconds.
pub const MAX_GIF_SEC: f64 = 60.0;
/// Most kept ranges accepted in one recipe.
pub const MAX_SEGMENTS: usize = 50;
/// Smallest crop side, as a fraction of the frame.
pub const MIN_CROP_FRACTION: f64 = 0.02;
/// A range may end this many seconds after the known duration (the stored
/// duration is rounded up to a second); it is clamped to the duration.
const DURATION_TOLERANCE_SEC: f64 = 1.0;
/// Ranges shorter than this are dropped after clamping.
const MIN_SEGMENT_SEC: f64 = 0.01;
/// Frame rate of an exported GIF.
const GIF_FPS: u32 = 12;
/// Widest exported GIF, in pixels.
const GIF_MAX_WIDTH: u32 = 480;
/// Most masks accepted in one recipe.
pub const MAX_MASKS: usize = 20;
/// Smallest mask side, as a fraction of the frame.
pub const MIN_MASK_FRACTION: f64 = 0.005;
/// Largest blur radius or pixel block, as a fraction of the frame width.
pub const MAX_MASK_STRENGTH: f64 = 0.25;
/// Smallest blur radius or pixel block, in source pixels.
pub const MIN_MASK_STRENGTH_PX: f64 = 2.0;
/// Largest annotation overlay accepted, in PNG bytes.
pub const MAX_OVERLAY_BYTES: usize = 4 * 1024 * 1024;
/// Largest accepted side of an annotation overlay, in pixels.
pub const MAX_OVERLAY_SIDE: u32 = 4096;
/// Largest accepted brightness, contrast or saturation, in percent.
pub const MAX_ADJUST_PERCENT: f64 = 400.0;
/// Largest accepted colour-adjustment blur, in board slider units.
pub const MAX_ADJUST_BLUR: f64 = 50.0;
/// Side the board's colour-adjustment blur radius is measured against.
const ADJUST_BLUR_UNIT_SIDE: f64 = 400.0;

/// What an edit produces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditOutput {
    /// Same kind as the source (video → MP4, audio → MP3).
    #[default]
    Same,
    /// The audio track of a video, as an MP3.
    Audio,
    /// An animated GIF made from a video.
    Gif,
}

/// Crop rectangle, as fractions of the source frame.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRect {
    /// Left edge (0..1).
    pub x: f64,
    /// Top edge (0..1).
    pub y: f64,
    /// Width (0..1).
    pub width: f64,
    /// Height (0..1).
    pub height: f64,
}

/// Rectangles hidden on the picture, as fractions of the source frame.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskRect {
    /// Left edge (0..1).
    pub x: f64,
    /// Top edge (0..1).
    pub y: f64,
    /// Width (0..1).
    pub width: f64,
    /// Height (0..1).
    pub height: f64,
    /// How the area is hidden.
    #[serde(default)]
    pub mode: MaskMode,
    /// Blur radius or pixel block, as a fraction of the frame width.
    pub strength: f64,
}

/// How a [`MaskRect`] hides its area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskMode {
    /// Blocks of the given size.
    #[default]
    Pixel,
    /// Blurred area.
    Blur,
}

/// Colour adjustments, as percentages where 100 keeps the colour unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EditAdjust {
    /// Brightness, in percent.
    pub brightness: f64,
    /// Contrast, in percent.
    pub contrast: f64,
    /// Saturation, in percent.
    pub saturate: f64,
    /// Blur radius, in the board's slider units (0..=10).
    pub blur: f64,
    /// Colour preset, applied after the sliders.
    pub preset: FilterPreset,
}

impl Default for EditAdjust {
    fn default() -> Self {
        Self {
            brightness: 100.0,
            contrast: 100.0,
            saturate: 100.0,
            blur: 0.0,
            preset: FilterPreset::None,
        }
    }
}

impl EditAdjust {
    /// `true` when the adjustments change nothing.
    pub fn is_neutral(&self) -> bool {
        (self.brightness - 100.0).abs() < 1e-6
            && (self.contrast - 100.0).abs() < 1e-6
            && (self.saturate - 100.0).abs() < 1e-6
            && self.blur.abs() < 1e-6
            && self.preset == FilterPreset::None
    }
}

/// Colour preset of the `Réglages` tool, like the board's CSS filters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterPreset {
    /// No preset.
    #[default]
    None,
    /// Grey levels.
    Mono,
    /// Sepia tones.
    Sepia,
    /// Stronger colours.
    Vivid,
    /// Saturated and contrasted.
    Fried,
    /// Negative.
    Invert,
}

/// Annotations drawn on the frame by the board, as a transparent PNG.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationOverlay {
    /// Standard base64 of a PNG with an alpha channel.
    pub png: String,
    /// Declared width, in pixels of the drawing.
    pub width: u32,
    /// Declared height, in pixels of the drawing.
    pub height: u32,
}

/// Edit requested by the board. Every field is optional; unknown fields are
/// ignored.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EditRecipe {
    /// Kept ranges `[start, end]` in source seconds, ascending and
    /// non-overlapping. Empty: the whole media.
    pub segments: Vec<[f64; 2]>,
    /// Playback speed (0.25..=4).
    pub speed: f64,
    /// Linear gain (0..=3).
    pub volume: f64,
    /// Video only: drop the audio track.
    pub mute: bool,
    /// Fade-in length in output seconds (sound, and picture for a video).
    pub fade_in_sec: f64,
    /// Fade-out length in output seconds.
    pub fade_out_sec: f64,
    /// Play backwards.
    pub reverse: bool,
    /// Video only: crop, applied before the rotation.
    pub crop: Option<CropRect>,
    /// Video only: clockwise rotation, 0, 90, 180 or 270.
    pub rotate: i64,
    /// Video only: mirror horizontally (after the rotation).
    pub flip_h: bool,
    /// Video only: mirror vertically (after the rotation).
    pub flip_v: bool,
    /// Video only: areas hidden on the source frame, before the crop.
    pub masks: Vec<MaskRect>,
    /// Video only: annotations drawn on the source frame, before the crop.
    pub overlay: Option<AnnotationOverlay>,
    /// Video only: colour adjustments, after the masks and the annotations.
    pub adjust: EditAdjust,
    /// What to produce.
    pub output: EditOutput,
}

impl Default for EditRecipe {
    fn default() -> Self {
        Self {
            segments: Vec::new(),
            speed: 1.0,
            volume: 1.0,
            mute: false,
            fade_in_sec: 0.0,
            fade_out_sec: 0.0,
            reverse: false,
            crop: None,
            rotate: 0,
            flip_h: false,
            flip_v: false,
            masks: Vec::new(),
            overlay: None,
            adjust: EditAdjust::default(),
            output: EditOutput::Same,
        }
    }
}

/// What is known of the media being edited.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditSource {
    /// Kind of the source asset.
    pub kind: MediaKind,
    /// Duration in seconds.
    pub duration_sec: f64,
    /// `true` when the file has an audio stream.
    pub has_audio: bool,
    /// Width of the picture, when known.
    pub width: Option<u32>,
    /// Height of the picture, when known.
    pub height: Option<u32>,
}

/// A mask placed on the source frame, in source pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskPlan {
    /// Left edge, in pixels.
    pub x: u32,
    /// Top edge, in pixels.
    pub y: u32,
    /// Width, in pixels (even, at least two).
    pub width: u32,
    /// Height, in pixels (even, at least two).
    pub height: u32,
    /// How the area is hidden.
    pub mode: MaskMode,
    /// Blur sigma or pixel block, in source pixels.
    pub strength: f64,
}

/// A validated edit, ready for [`build_edit_args`].
#[derive(Clone, Debug, PartialEq)]
pub struct EditPlan {
    /// Kept ranges in source seconds. Empty: the whole media.
    pub segments: Vec<(f64, f64)>,
    /// Playback speed.
    pub speed: f64,
    /// Linear gain.
    pub volume: f64,
    /// Fade-in length in output seconds.
    pub fade_in_sec: f64,
    /// Fade-out length in output seconds.
    pub fade_out_sec: f64,
    /// Play backwards.
    pub reverse: bool,
    /// Crop, when it is not the whole frame.
    pub crop: Option<CropRect>,
    /// Clockwise rotation: 0, 90, 180 or 270.
    pub rotate: u16,
    /// Mirror horizontally.
    pub flip_h: bool,
    /// Mirror vertically.
    pub flip_v: bool,
    /// Masks, in source pixels, drawn before the crop.
    pub masks: Vec<MaskPlan>,
    /// `true` when an annotation overlay is drawn before the crop.
    pub overlay: bool,
    /// Colour adjustments, after the masks and the overlay.
    pub adjust: EditAdjust,
    /// Size of the source frame, when an annotation needs it.
    pub frame_size: Option<(u32, u32)>,
    /// What is produced.
    pub output: EditOutput,
    /// `true` when the output has a picture (MP4 or GIF).
    pub with_video: bool,
    /// `true` when the output has sound.
    pub with_audio: bool,
    /// Source seconds kept.
    pub kept_duration_sec: f64,
    /// Duration of the output (kept seconds divided by the speed).
    pub output_duration_sec: f64,
}

impl EditPlan {
    /// `true` when the picture is reversed in pieces (see [`reverse_pieces`]).
    pub fn reverses_in_pieces(&self) -> bool {
        self.reverse && self.with_video && self.kept_duration_sec > REVERSE_PIECE_SEC
    }

    /// Kept ranges in source seconds, the whole media included.
    fn kept_ranges(&self) -> Vec<(f64, f64)> {
        if self.segments.is_empty() {
            vec![(0.0, self.kept_duration_sec)]
        } else {
            self.segments.clone()
        }
    }

    /// The edit of one reversed piece: cut, crop, rotation, flips and reverse
    /// on `ranges`, in piece seconds (the piece starts at its first range).
    fn reversed_piece(&self, ranges: &[(f64, f64)]) -> EditPlan {
        let kept: f64 = ranges.iter().map(|(start, end)| end - start).sum();
        EditPlan {
            segments: ranges.to_vec(),
            speed: 1.0,
            volume: 1.0,
            fade_in_sec: 0.0,
            fade_out_sec: 0.0,
            reverse: true,
            output: EditOutput::Same,
            with_video: true,
            kept_duration_sec: kept,
            output_duration_sec: kept,
            ..self.clone()
        }
    }

    /// What is left to apply once the reversed pieces are joined.
    fn after_reversed_pieces(&self) -> EditPlan {
        EditPlan {
            segments: Vec::new(),
            reverse: false,
            crop: None,
            rotate: 0,
            flip_h: false,
            flip_v: false,
            // Masks, annotations and colour adjustments were burnt into the
            // pieces: applying them again would hide the picture twice.
            masks: Vec::new(),
            overlay: false,
            adjust: EditAdjust::default(),
            frame_size: None,
            ..self.clone()
        }
    }

    /// Kind of the media produced.
    pub fn output_kind(&self) -> MediaKind {
        if !self.with_video {
            MediaKind::Audio
        } else if self.output == EditOutput::Gif {
            MediaKind::Image
        } else {
            MediaKind::Video
        }
    }

    /// File extension of the media produced, without the dot.
    pub fn output_extension(&self) -> &'static str {
        match self.output_kind() {
            MediaKind::Audio => "mp3",
            MediaKind::Image => "gif",
            MediaKind::Video => "mp4",
        }
    }

    /// MIME type of the media produced.
    pub fn output_mime(&self) -> &'static str {
        match self.output_kind() {
            MediaKind::Audio => "audio/mpeg",
            MediaKind::Image => "image/gif",
            MediaKind::Video => "video/mp4",
        }
    }
}

impl EditRecipe {
    /// Validates the recipe against its source.
    ///
    /// The error is a sentence for the caller (`400 invalid_payload`).
    pub fn plan(&self, source: &EditSource) -> Result<EditPlan, String> {
        if source.kind == MediaKind::Image {
            return Err("Les images se modifient directement sur le board.".to_string());
        }
        let is_video = source.kind == MediaKind::Video;
        if !is_video && self.output != EditOutput::Same {
            return Err("Ce format de sortie n’est disponible que pour une vidéo.".to_string());
        }
        if !is_video && self.mute {
            return Err("Le mode muet ne s’applique qu’à une vidéo.".to_string());
        }
        if self.output == EditOutput::Audio && self.mute {
            return Err("Une vidéo mise en sourdine n’a pas d’audio à extraire.".to_string());
        }
        let with_video = is_video && self.output != EditOutput::Audio;
        let wants_audio = match self.output {
            EditOutput::Gif => false,
            EditOutput::Audio => true,
            EditOutput::Same => !self.mute,
        };
        if !with_video && !source.has_audio {
            return Err("Ce média n’a pas de piste audio.".to_string());
        }
        let with_audio = wants_audio && source.has_audio;

        let speed = in_range("speed", self.speed, MIN_SPEED, MAX_SPEED)?;
        let volume = in_range("volume", self.volume, 0.0, MAX_VOLUME)?;
        let fade_in = in_range("fadeInSec", self.fade_in_sec, 0.0, MAX_FADE_SEC)?;
        let fade_out = in_range("fadeOutSec", self.fade_out_sec, 0.0, MAX_FADE_SEC)?;
        if !matches!(self.rotate, 0 | 90 | 180 | 270) {
            return Err("La rotation doit être 0, 90, 180 ou 270 degrés.".to_string());
        }

        let duration = source.duration_sec;
        if !duration.is_finite() || duration <= 0.0 {
            return Err("La durée de ce média est inconnue.".to_string());
        }
        let segments = plan_segments(&self.segments, duration)?;
        let kept = if segments.is_empty() {
            duration
        } else {
            segments.iter().map(|(start, end)| end - start).sum()
        };
        if kept < MIN_KEPT_SEC {
            return Err(format!(
                "La sélection doit conserver au moins {MIN_KEPT_SEC} s."
            ));
        }
        let output_duration = kept / speed;
        if self.output == EditOutput::Gif && output_duration > MAX_GIF_SEC {
            return Err(format!("Un GIF est limité à {MAX_GIF_SEC} s."));
        }
        // Fades longer than the output share it.
        let (fade_in, fade_out) = if fade_in + fade_out > output_duration {
            let half = output_duration / 2.0;
            (fade_in.min(half), fade_out.min(half))
        } else {
            (fade_in, fade_out)
        };

        let crop = if with_video {
            plan_crop(self.crop.as_ref())?
        } else {
            None
        };
        let (masks, overlay, adjust, frame_size) = if with_video {
            plan_annotations(&self.masks, self.overlay.as_ref(), &self.adjust, source)?
        } else {
            (Vec::new(), false, EditAdjust::default(), None)
        };
        Ok(EditPlan {
            segments,
            speed,
            volume,
            fade_in_sec: fade_in,
            fade_out_sec: fade_out,
            reverse: self.reverse,
            crop,
            rotate: if with_video {
                u16::try_from(self.rotate).unwrap_or(0)
            } else {
                0
            },
            flip_h: with_video && self.flip_h,
            flip_v: with_video && self.flip_v,
            masks,
            overlay,
            adjust,
            frame_size,
            output: self.output,
            with_video,
            with_audio,
            kept_duration_sec: kept,
            output_duration_sec: output_duration,
        })
    }
}

/// Checks a finite number against an inclusive range.
fn in_range(name: &str, value: f64, min: f64, max: f64) -> Result<f64, String> {
    if !value.is_finite() || value < min || value > max {
        return Err(format!(
            "La valeur de {name} doit être comprise entre {min} et {max}."
        ));
    }
    Ok(value)
}

/// Validates and clamps the kept ranges. A single range covering the whole
/// media gives an empty list.
fn plan_segments(raw: &[[f64; 2]], duration: f64) -> Result<Vec<(f64, f64)>, String> {
    if raw.len() > MAX_SEGMENTS {
        return Err(format!("Au plus {MAX_SEGMENTS} segments sont acceptés."));
    }
    let mut segments: Vec<(f64, f64)> = Vec::with_capacity(raw.len());
    let mut previous_end = 0.0_f64;
    for (index, [start, end]) in raw.iter().copied().enumerate() {
        let number = index + 1;
        if !start.is_finite() || !end.is_finite() {
            return Err(format!(
                "Le segment {number} doit être une paire de nombres."
            ));
        }
        if start < 0.0 || end <= start {
            return Err(format!(
                "Le segment {number} doit se terminer après son début."
            ));
        }
        if start < previous_end {
            return Err(
                "Les segments doivent être croissants et ne pas se chevaucher.".to_string(),
            );
        }
        if start >= duration || end > duration + DURATION_TOLERANCE_SEC {
            return Err(format!(
                "Le segment {number} dépasse la durée du média ({duration:.2} s)."
            ));
        }
        let end = end.min(duration);
        previous_end = end;
        if end - start >= MIN_SEGMENT_SEC {
            segments.push((start, end));
        }
    }
    if raw.is_empty() {
        return Ok(segments);
    }
    if segments.is_empty() {
        return Err(format!(
            "La sélection doit conserver au moins {MIN_KEPT_SEC} s."
        ));
    }
    if let [(start, end)] = segments.as_slice() {
        if *start <= MIN_SEGMENT_SEC && *end >= duration - MIN_SEGMENT_SEC {
            return Ok(Vec::new());
        }
    }
    Ok(segments)
}

/// Validates the crop. The whole frame gives `None`.
fn plan_crop(crop: Option<&CropRect>) -> Result<Option<CropRect>, String> {
    let Some(crop) = crop else {
        return Ok(None);
    };
    let values = [crop.x, crop.y, crop.width, crop.height];
    if values.iter().any(|value| !value.is_finite()) {
        return Err("Le cadrage doit être composé de nombres.".to_string());
    }
    const EPSILON: f64 = 1e-6;
    if crop.x < -EPSILON
        || crop.y < -EPSILON
        || crop.x + crop.width > 1.0 + EPSILON
        || crop.y + crop.height > 1.0 + EPSILON
    {
        return Err("Le cadrage doit rester dans l’image.".to_string());
    }
    if crop.width < MIN_CROP_FRACTION || crop.height < MIN_CROP_FRACTION {
        return Err("Le cadrage doit conserver au moins 2 % de chaque côté.".to_string());
    }
    let x = crop.x.clamp(0.0, 1.0);
    let y = crop.y.clamp(0.0, 1.0);
    let width = crop.width.min(1.0 - x);
    let height = crop.height.min(1.0 - y);
    if x <= EPSILON && y <= EPSILON && width >= 1.0 - EPSILON && height >= 1.0 - EPSILON {
        return Ok(None);
    }
    Ok(Some(CropRect {
        x,
        y,
        width,
        height,
    }))
}

/// What the annotations bring to a plan once validated: the masks in source
/// pixels, whether an overlay is burnt in, the colour adjustments and the size
/// of the source frame.
type PlannedAnnotations = (Vec<MaskPlan>, bool, EditAdjust, Option<(u32, u32)>);

/// Validates the annotations: masks, overlay and colour adjustments. They need
/// the size of the source frame; a recipe without annotation keeps its frame
/// size unknown.
fn plan_annotations(
    raw_masks: &[MaskRect],
    overlay: Option<&AnnotationOverlay>,
    adjust: &EditAdjust,
    source: &EditSource,
) -> Result<PlannedAnnotations, String> {
    if raw_masks.is_empty() && overlay.is_none() && adjust.is_neutral() {
        return Ok((Vec::new(), false, EditAdjust::default(), None));
    }
    let (Some(width), Some(height)) = (source.width, source.height) else {
        return Err("La taille de l’image est inconnue.".to_string());
    };
    if width < 2 || height < 2 {
        return Err("L’image est trop petite pour être annotée.".to_string());
    }
    Ok((
        plan_masks(raw_masks, width, height)?,
        plan_overlay(overlay, width, height)?,
        plan_adjust(*adjust)?,
        Some((width, height)),
    ))
}

/// Validates the masks and turns their fractions into source pixels.
fn plan_masks(raw: &[MaskRect], width: u32, height: u32) -> Result<Vec<MaskPlan>, String> {
    if raw.len() > MAX_MASKS {
        return Err(format!("Au plus {MAX_MASKS} masques sont acceptés."));
    }
    let frame_width = f64::from(width);
    let frame_height = f64::from(height);
    let mut masks = Vec::with_capacity(raw.len());
    for (index, mask) in raw.iter().enumerate() {
        let number = index + 1;
        let values = [mask.x, mask.y, mask.width, mask.height, mask.strength];
        if values.iter().any(|value| !value.is_finite()) {
            return Err(format!("Le masque {number} doit être composé de nombres."));
        }
        const EPSILON: f64 = 1e-6;
        if mask.x < -EPSILON
            || mask.y < -EPSILON
            || mask.x + mask.width > 1.0 + EPSILON
            || mask.y + mask.height > 1.0 + EPSILON
        {
            return Err(format!("Le masque {number} doit rester dans l’image."));
        }
        if mask.width < MIN_MASK_FRACTION || mask.height < MIN_MASK_FRACTION {
            return Err(format!(
                "Le masque {number} doit conserver au moins {:.1} % de chaque côté.",
                MIN_MASK_FRACTION * 100.0
            ));
        }
        if mask.strength < 0.0 || mask.strength > MAX_MASK_STRENGTH {
            return Err(format!(
                "L’intensité du masque {number} doit être comprise entre 0 et {MAX_MASK_STRENGTH}."
            ));
        }
        // Fractions become pixels: the far edges are rounded too, so a mask
        // never grows by a pixel to catch a floating-point artefact.
        let x = (mask.x.max(0.0) * frame_width).round() as u32;
        let y = (mask.y.max(0.0) * frame_height).round() as u32;
        let right = ((mask.x + mask.width).min(1.0) * frame_width).round() as u32;
        let bottom = ((mask.y + mask.height).min(1.0) * frame_height).round() as u32;
        let x = x.min(width - 2);
        let y = y.min(height - 2);
        let mask_width = right.saturating_sub(x).clamp(2, width - x);
        let mask_height = bottom.saturating_sub(y).clamp(2, height - y);
        masks.push(MaskPlan {
            x,
            y,
            width: mask_width,
            height: mask_height,
            mode: mask.mode,
            strength: (mask.strength * frame_width).max(MIN_MASK_STRENGTH_PX),
        });
    }
    Ok(masks)
}

/// Validates the colour adjustments and returns them normalised.
fn plan_adjust(adjust: EditAdjust) -> Result<EditAdjust, String> {
    Ok(EditAdjust {
        brightness: in_range("brightness", adjust.brightness, 0.0, MAX_ADJUST_PERCENT)?,
        contrast: in_range("contrast", adjust.contrast, 0.0, MAX_ADJUST_PERCENT)?,
        saturate: in_range("saturate", adjust.saturate, 0.0, MAX_ADJUST_PERCENT)?,
        blur: in_range("blur", adjust.blur, 0.0, MAX_ADJUST_BLUR)?,
        preset: adjust.preset,
    })
}

/// Validates the annotation overlay: a PNG of the declared size, as base64.
fn plan_overlay(
    overlay: Option<&AnnotationOverlay>,
    width: u32,
    height: u32,
) -> Result<bool, String> {
    let Some(overlay) = overlay else {
        return Ok(false);
    };
    if overlay.width != width || overlay.height != height {
        return Err("La couche d’annotation doit avoir la taille de l’image.".to_string());
    }
    if overlay.width > MAX_OVERLAY_SIDE || overlay.height > MAX_OVERLAY_SIDE {
        return Err(format!(
            "La couche d’annotation est limitée à {MAX_OVERLAY_SIDE} px de côté."
        ));
    }
    let bytes = decode_overlay(&overlay.png)?;
    if bytes.len() > MAX_OVERLAY_BYTES {
        return Err(format!(
            "La couche d’annotation est limitée à {} Mio.",
            MAX_OVERLAY_BYTES / (1024 * 1024)
        ));
    }
    let (png_width, png_height) = png_size(&bytes)?;
    if png_width != overlay.width || png_height != overlay.height {
        return Err(
            "La taille déclarée de la couche d’annotation ne correspond pas à son image."
                .to_string(),
        );
    }
    Ok(true)
}

/// Decodes the base64 PNG of an annotation overlay.
pub(crate) fn decode_overlay(png: &str) -> Result<Vec<u8>, String> {
    STANDARD
        .decode(png)
        .map_err(|_| "La couche d’annotation n’est pas un base64 valide.".to_string())
}

/// Size declared by the `IHDR` chunk of a PNG, without decoding the picture.
fn png_size(bytes: &[u8]) -> Result<(u32, u32), String> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return Err("La couche d’annotation n’est pas un PNG.".to_string());
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Ok((width, height))
}

/// Formats a number for a filter graph: at most four decimals, no trailing
/// zeros.
fn num(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// `atempo` factors reaching `speed`; each stays inside 0.5..=2.
fn atempo_chain(speed: f64) -> Vec<String> {
    let mut factors = Vec::new();
    let mut remaining = speed;
    while remaining > 2.0 {
        factors.push(2.0);
        remaining /= 2.0;
    }
    while remaining < 0.5 {
        factors.push(0.5);
        remaining /= 0.5;
    }
    if (remaining - 1.0).abs() > 1e-6 {
        factors.push(remaining);
    }
    factors
        .into_iter()
        .map(|factor| format!("atempo={}", num(factor)))
        .collect()
}

/// Picture filters applied after the cut.
fn video_filters(plan: &EditPlan, max_height: u32) -> Vec<String> {
    let mut filters = Vec::new();
    if let Some(crop) = &plan.crop {
        filters.push(format!(
            "crop=w='max(2,trunc(iw*{w}/2)*2)':h='max(2,trunc(ih*{h}/2)*2)':x='trunc(iw*{x})':y='trunc(ih*{y})'",
            w = num(crop.width),
            h = num(crop.height),
            x = num(crop.x),
            y = num(crop.y),
        ));
    }
    match plan.rotate {
        90 => filters.push("transpose=1".to_string()),
        180 => {
            filters.push("hflip".to_string());
            filters.push("vflip".to_string());
        }
        270 => filters.push("transpose=2".to_string()),
        _ => {}
    }
    if plan.flip_h {
        filters.push("hflip".to_string());
    }
    if plan.flip_v {
        filters.push("vflip".to_string());
    }
    if plan.reverse {
        filters.push("reverse".to_string());
    }
    if (plan.speed - 1.0).abs() > 1e-6 {
        filters.push(format!("setpts=PTS/{}", num(plan.speed)));
    }
    if plan.fade_in_sec > 0.0 {
        filters.push(format!("fade=t=in:st=0:d={}", num(plan.fade_in_sec)));
    }
    if plan.fade_out_sec > 0.0 {
        let start = (plan.output_duration_sec - plan.fade_out_sec).max(0.0);
        filters.push(format!(
            "fade=t=out:st={}:d={}",
            num(start),
            num(plan.fade_out_sec)
        ));
    }
    if plan.output == EditOutput::Gif {
        filters.push(format!("fps={GIF_FPS}"));
        filters.push(format!(
            "scale=w='min({GIF_MAX_WIDTH},iw)':h=-1:flags=lanczos"
        ));
    } else {
        // Even sides under the maximum height: what the normalizer copies.
        let max_height = max_height.max(2);
        filters.push(format!(
            "scale=w='trunc(iw*min(1,{max_height}/ih)/2)*2':h='trunc(min(ih,{max_height})/2)*2'"
        ));
        filters.push("setsar=1".to_string());
    }
    filters
}

/// Sound filters applied after the cut.
fn audio_filters(plan: &EditPlan) -> Vec<String> {
    let mut filters = Vec::new();
    if plan.reverse {
        filters.push("areverse".to_string());
    }
    filters.extend(atempo_chain(plan.speed));
    if (plan.volume - 1.0).abs() > 1e-6 {
        filters.push(format!("volume={}", num(plan.volume)));
    }
    if plan.fade_in_sec > 0.0 {
        filters.push(format!("afade=t=in:st=0:d={}", num(plan.fade_in_sec)));
    }
    if plan.fade_out_sec > 0.0 {
        let start = (plan.output_duration_sec - plan.fade_out_sec).max(0.0);
        filters.push(format!(
            "afade=t=out:st={}:d={}",
            num(start),
            num(plan.fade_out_sec)
        ));
    }
    if filters.is_empty() {
        filters.push("anull".to_string());
    }
    filters
}

/// `crop` filter of a mask rectangle, in source pixels.
fn crop_filter(mask: &MaskPlan) -> String {
    format!(
        "crop=w={}:h={}:x={}:y={}",
        mask.width, mask.height, mask.x, mask.y
    )
}

/// Pixel mask: the frame is pixelized by blocks aligned on its top-left corner,
/// then cropped to the mask. `pixelize` averages every plane, so the blocks
/// keep their colours; a `scale` round trip drops the chroma of the smallest
/// sizes and turns the blocks grey.
fn pixelated_region(mask: &MaskPlan) -> String {
    let block = mask.strength.round().clamp(1.0, 1024.0) as u32;
    format!("pixelize=w={block}:h={block},{}", crop_filter(mask))
}

/// Statements drawing the masks, the annotations and the colour adjustments on
/// the source frame, and the label of their output. Input 1 is the annotation
/// overlay, when the plan has one.
fn annotation_stage(plan: &EditPlan, video_in: &str) -> (Vec<String>, String) {
    let mut statements: Vec<String> = Vec::new();
    let mut chain = video_in.to_string();

    if !plan.masks.is_empty() {
        let copies: String = (0..plan.masks.len())
            .map(|index| format!("[mc{index}]"))
            .collect();
        let outputs = plan.masks.len() + 1;
        statements.push(format!("[{chain}]split={outputs}{copies}[mbase]"));
        chain = "mbase".to_string();
        for (index, mask) in plan.masks.iter().enumerate() {
            // The region is read from its own copy, so it is not blurred or
            // pixelated twice by the masks that came before.
            let region = match mask.mode {
                MaskMode::Pixel => pixelated_region(mask),
                MaskMode::Blur => {
                    format!("gblur=sigma={},{}", num(mask.strength), crop_filter(mask))
                }
            };
            statements.push(format!("[mc{index}]{region}[mr{index}]"));
            statements.push(format!(
                "[{chain}][mr{index}]overlay={}:{}[mn{index}]",
                mask.x, mask.y
            ));
            chain = format!("mn{index}");
        }
    }

    if plan.overlay {
        if let Some((width, height)) = plan.frame_size {
            statements.push(format!(
                "[1:v]scale=w={width}:h={height}:flags=lanczos,format=rgba,setsar=1[ov]"
            ));
            statements.push(format!("[{chain}][ov]overlay=0:0[annotated]"));
            chain = "annotated".to_string();
        }
    }

    if !plan.adjust.is_neutral() {
        let filters = adjust_filters(&plan.adjust, plan.frame_size);
        if !filters.is_empty() {
            statements.push(format!("[{chain}]{filters}[adjusted]"));
            chain = "adjusted".to_string();
        }
    }

    (statements, chain)
}

/// Coefficients of `sepia(0.9)`, the preset of the board (identity blended at
/// 10 % with the usual sepia matrix).
const SEPIA_COEFFICIENTS: &str =
    "rr=0.4537:rg=0.6921:rb=0.1701:gr=0.3141:gg=0.7174:gb=0.1512:br=0.2448:bg=0.4806:bb=0.2179";

/// Filters applying the colour adjustments of the board, in its order: the
/// sliders, then the preset, then the blur.
fn adjust_filters(adjust: &EditAdjust, frame: Option<(u32, u32)>) -> String {
    let mut brightness = adjust.brightness / 100.0;
    let mut contrast = adjust.contrast / 100.0;
    let mut saturation = adjust.saturate / 100.0;
    match adjust.preset {
        FilterPreset::Vivid => {
            saturation *= 1.6;
            contrast *= 1.1;
        }
        FilterPreset::Fried => {
            saturation *= 3.0;
            contrast *= 1.8;
            brightness *= 1.1;
        }
        FilterPreset::None | FilterPreset::Mono | FilterPreset::Sepia | FilterPreset::Invert => {}
    }
    let mut filters: Vec<String> = Vec::new();
    if (brightness - 1.0).abs() > 1e-6
        || (contrast - 1.0).abs() > 1e-6
        || (saturation - 1.0).abs() > 1e-6
    {
        filters.push(format!(
            "eq=brightness={}:contrast={}:saturation={}",
            num((brightness - 1.0).clamp(-1.0, 1.0)),
            num(contrast),
            num(saturation)
        ));
    }
    match adjust.preset {
        FilterPreset::Mono => filters.push("hue=s=0".to_string()),
        FilterPreset::Sepia => filters.push(format!("colorchannelmixer={SEPIA_COEFFICIENTS}")),
        FilterPreset::Invert => filters.push("negate".to_string()),
        FilterPreset::None | FilterPreset::Vivid | FilterPreset::Fried => {}
    }
    if let Some((width, height)) = frame {
        let sigma = adjust.blur * f64::from(width.max(height)) / ADJUST_BLUR_UNIT_SIDE;
        if sigma > 0.0 {
            filters.push(format!("gblur=sigma={}", num(sigma)));
        }
    }
    filters.join(",")
}

/// The `-filter_complex` graph of a plan. Its outputs are `[v]` (picture, when
/// the plan has one) and `[a]` (sound, when the plan has one).
pub fn build_filter_graph(plan: &EditPlan, max_height: u32) -> String {
    let mut graph: Vec<String> = Vec::new();
    let count = plan.segments.len();
    let mut video_in = "0:v:0".to_string();
    let mut audio_in = "0:a:0".to_string();

    if count == 1 {
        let (start, end) = plan.segments[0];
        if plan.with_video {
            graph.push(format!(
                "[0:v:0]trim=start={}:end={},setpts=PTS-STARTPTS[vc]",
                num(start),
                num(end)
            ));
            video_in = "vc".to_string();
        }
        if plan.with_audio {
            graph.push(format!(
                "[0:a:0]atrim=start={}:end={},asetpts=PTS-STARTPTS[ac]",
                num(start),
                num(end)
            ));
            audio_in = "ac".to_string();
        }
    } else if count > 1 {
        if plan.with_video {
            let outputs: String = (0..count).map(|index| format!("[vs{index}]")).collect();
            graph.push(format!("[0:v:0]split={count}{outputs}"));
        }
        if plan.with_audio {
            let outputs: String = (0..count).map(|index| format!("[as{index}]")).collect();
            graph.push(format!("[0:a:0]asplit={count}{outputs}"));
        }
        let mut concat_inputs = String::new();
        for (index, (start, end)) in plan.segments.iter().enumerate() {
            if plan.with_video {
                graph.push(format!(
                    "[vs{index}]trim=start={}:end={},setpts=PTS-STARTPTS[vt{index}]",
                    num(*start),
                    num(*end)
                ));
                concat_inputs.push_str(&format!("[vt{index}]"));
            }
            if plan.with_audio {
                graph.push(format!(
                    "[as{index}]atrim=start={}:end={},asetpts=PTS-STARTPTS[at{index}]",
                    num(*start),
                    num(*end)
                ));
                concat_inputs.push_str(&format!("[at{index}]"));
            }
        }
        let video_count = u8::from(plan.with_video);
        let audio_count = u8::from(plan.with_audio);
        let mut concat_outputs = String::new();
        if plan.with_video {
            concat_outputs.push_str("[vc]");
            video_in = "vc".to_string();
        }
        if plan.with_audio {
            concat_outputs.push_str("[ac]");
            audio_in = "ac".to_string();
        }
        graph.push(format!(
            "{concat_inputs}concat=n={count}:v={video_count}:a={audio_count}{concat_outputs}"
        ));
    }

    if plan.with_video {
        let (statements, video_in) = annotation_stage(plan, &video_in);
        graph.extend(statements);
        let filters = video_filters(plan, max_height).join(",");
        if plan.output == EditOutput::Gif {
            graph.push(format!("[{video_in}]{filters},split[g0][g1]"));
            graph.push("[g0]palettegen[gp]".to_string());
            graph.push("[g1][gp]paletteuse[v]".to_string());
        } else {
            graph.push(format!("[{video_in}]{filters}[v]"));
        }
    }
    if plan.with_audio {
        graph.push(format!("[{audio_in}]{}[a]", audio_filters(plan).join(",")));
    }
    graph.join(";")
}

/// ffmpeg arguments applying `plan` to `input`.
///
/// `overlay` is the PNG of the annotations, when the plan draws some: it is
/// read as a second input whose single frame `overlay` repeats. `encoder` and
/// `preset` are only used for a video output (see [`resolve_video_encoder`]).
pub fn build_edit_args(
    input: &Path,
    overlay: Option<&Path>,
    output: &Path,
    plan: &EditPlan,
    config: &MediaConfig,
    encoder: &str,
    preset: &str,
) -> Vec<String> {
    edit_args(&[], input, overlay, output, plan, config, encoder, preset)
}

/// [`build_edit_args`] with options placed before `-i` (an input format).
fn edit_args(
    input_options: &[&str],
    input: &Path,
    overlay: Option<&Path>,
    output: &Path,
    plan: &EditPlan,
    config: &MediaConfig,
    encoder: &str,
    preset: &str,
) -> Vec<String> {
    let mut args: Vec<String> = ["-y", "-hide_banner", "-v", "error"]
        .iter()
        .chain(input_options)
        .map(|arg| (*arg).to_string())
        .collect();
    args.extend(["-i".to_string(), input.display().to_string()]);
    if let Some(overlay) = overlay {
        // A single frame: `overlay` repeats it, and looping the image would
        // keep ffmpeg reading forever (`-loop 1` never reaches EOF).
        args.extend(["-i".to_string(), overlay.display().to_string()]);
    }
    args.extend([
        "-filter_complex".to_string(),
        build_filter_graph(plan, config.video.max_height),
    ]);
    if plan.with_video {
        args.push("-map".to_string());
        args.push("[v]".to_string());
    }
    if plan.with_audio {
        args.push("-map".to_string());
        args.push("[a]".to_string());
    } else {
        args.push("-an".to_string());
    }
    match plan.output_kind() {
        MediaKind::Video => {
            args.extend(["-c:v", encoder, "-preset", preset].map(str::to_string));
            if encoder == crate::transcode::NVENC_ENCODER {
                args.extend(crate::transcode::nvenc_quality_args());
            }
            args.extend(["-pix_fmt", "yuv420p"].map(str::to_string));
            if plan.with_audio {
                args.extend(["-c:a", "aac", "-b:a", AUDIO_BITRATE].map(str::to_string));
            }
            args.extend(["-movflags", "+faststart"].map(str::to_string));
        }
        MediaKind::Audio => {
            args.extend(["-vn", "-c:a", "libmp3lame", "-b:a", AUDIO_BITRATE].map(str::to_string));
        }
        MediaKind::Image => {
            args.extend(["-loop", "0"].map(str::to_string));
        }
    }
    args.push(output.display().to_string());
    args
}

/// Splits the kept ranges into pieces of equal kept length, at most
/// `piece_sec` each, in the order of the reversed result: the end of the media
/// first. The ranges inside a piece stay ascending.
pub fn reverse_pieces(ranges: &[(f64, f64)], piece_sec: f64) -> Vec<Vec<(f64, f64)>> {
    let kept: f64 = ranges.iter().map(|(start, end)| end - start).sum();
    if kept <= 0.0 || piece_sec <= 0.0 {
        return Vec::new();
    }
    // Equal pieces: no sliver of a few frames at the start of the media.
    let count = ((kept / piece_sec) - 1e-9).ceil().max(1.0) as usize;
    let size = kept / count as f64;
    let mut pieces: Vec<Vec<(f64, f64)>> = Vec::with_capacity(count);
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut room = size;
    for &(start, end) in ranges.iter().rev() {
        let mut end = end;
        while end - start > 1e-6 {
            // The last piece takes everything left (rounding).
            let last = pieces.len() + 1 >= count;
            let from = if last { start } else { (end - room).max(start) };
            current.insert(0, (from, end));
            room -= end - from;
            end = from;
            if !last && room <= 1e-6 {
                pieces.push(std::mem::take(&mut current));
                room = size;
            }
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// ffmpeg arguments writing one reversed piece of `plan` (see
/// [`reverse_pieces`]) to `output`, an MKV kept for the final pass: H.264 at
/// a near-lossless quality and PCM sound, so the join adds no gap.
pub fn build_reverse_piece_args(
    input: &Path,
    overlay: Option<&Path>,
    output: &Path,
    plan: &EditPlan,
    piece: &[(f64, f64)],
    max_height: u32,
) -> Vec<String> {
    let seek = piece.first().map_or(0.0, |range| range.0);
    let until = piece.last().map_or(0.0, |range| range.1);
    let ranges: Vec<(f64, f64)> = piece
        .iter()
        .map(|(start, end)| (start - seek, end - seek))
        .collect();
    let piece_plan = plan.reversed_piece(&ranges);
    let mut args: Vec<String> = ["-y", "-hide_banner", "-v", "error", "-ss"]
        .map(str::to_string)
        .to_vec();
    args.extend([
        num(seek),
        "-t".to_string(),
        num(until - seek),
        "-i".to_string(),
        input.display().to_string(),
    ]);
    if let Some(overlay) = overlay {
        args.extend(["-i".to_string(), overlay.display().to_string()]);
    }
    args.extend([
        "-filter_complex".to_string(),
        build_filter_graph(&piece_plan, max_height),
        "-map".to_string(),
        "[v]".to_string(),
    ]);
    if piece_plan.with_audio {
        args.extend(["-map", "[a]", "-c:a", "pcm_s16le"].map(str::to_string));
    } else {
        args.push("-an".to_string());
    }
    args.extend(
        [
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "14", "-pix_fmt", "yuv420p",
        ]
        .map(str::to_string),
    );
    args.push(output.display().to_string());
    args
}

/// Concat list joining the reversed pieces, named relative to the list.
fn reverse_concat_list(names: &[String]) -> String {
    let mut list = String::from("ffconcat version 1.0\n");
    for name in names {
        list.push_str(&format!("file '{name}'\n"));
    }
    list
}

/// Runs the edit and writes `output` (its extension must be
/// [`EditPlan::output_extension`]). A failed NVENC encode is tried again with
/// libx264, like the normalizer.
///
/// A long reversed video is cut into pieces written next to `output`; they
/// are removed afterwards.
pub async fn apply_edit(
    input: &Path,
    overlay: Option<&Path>,
    output: &Path,
    plan: &EditPlan,
    config: &MediaConfig,
) -> MediaResult<()> {
    if !plan.reverses_in_pieces() {
        return encode_edit(&[], input, overlay, output, plan, config).await;
    }
    let directory = output.parent().unwrap_or_else(|| Path::new("."));
    let pieces = reverse_pieces(&plan.kept_ranges(), REVERSE_PIECE_SEC);
    let names: Vec<String> = (0..pieces.len())
        .map(|index| format!("reverse-piece-{index}.mkv"))
        .collect();
    let list_path = directory.join("reverse-pieces.ffconcat");
    let result = async {
        for (piece, name) in pieces.iter().zip(&names) {
            let args = build_reverse_piece_args(
                input,
                overlay,
                &directory.join(name),
                plan,
                piece,
                config.video.max_height,
            );
            process::run_command(
                &config.tools.ffmpeg,
                &args,
                config.download.timeout,
                MediaErrorCode::TranscodeFailed,
            )
            .await?;
        }
        std::fs::write(&list_path, reverse_concat_list(&names)).map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::TranscodeFailed,
                "écriture de la liste des morceaux inversés",
                &error,
            )
        })?;
        encode_edit(
            &["-f", "concat", "-safe", "0"],
            &list_path,
            None,
            output,
            &plan.after_reversed_pieces(),
            config,
        )
        .await
    }
    .await;
    for path in names
        .iter()
        .map(|name| directory.join(name))
        .chain([list_path])
    {
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%error, path = %path.display(), "morceau inversé non supprimé");
            }
        }
    }
    result
}

/// One ffmpeg run of `plan`, with the NVENC fallback.
async fn encode_edit(
    input_options: &[&str],
    input: &Path,
    overlay: Option<&Path>,
    output: &Path,
    plan: &EditPlan,
    config: &MediaConfig,
) -> MediaResult<()> {
    let timeout = config.download.timeout;
    let nvenc_available = plan.output_kind() == MediaKind::Video
        && match config.video.encoder {
            VideoEncoder::LibX264 => false,
            VideoEncoder::H264Nvenc | VideoEncoder::Auto => {
                has_nvenc(&config.tools.ffmpeg, timeout).await
            }
        };
    let (encoder, preset) = resolve_video_encoder(&config.video, nvenc_available);
    let args = edit_args(
        input_options,
        input,
        overlay,
        output,
        plan,
        config,
        &encoder,
        &preset,
    );
    let first = process::run_command(
        &config.tools.ffmpeg,
        &args,
        timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await;
    let Err(error) = first else {
        return Ok(());
    };
    if encoder != "h264_nvenc" {
        return Err(error);
    }
    tracing::warn!(%error, "échec NVENC pendant une édition, nouvelle tentative en libx264");
    let (encoder, preset) = resolve_video_encoder(&config.video, false);
    let args = edit_args(
        input_options,
        input,
        overlay,
        output,
        plan,
        config,
        &encoder,
        &preset,
    );
    process::run_command(
        &config.tools.ffmpeg,
        &args,
        timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await
    .map(|_| ())
}

/// Media error for a recipe refused once the file has been probed.
pub fn recipe_error(message: impl Into<String>) -> MediaError {
    MediaError::new(MediaErrorCode::InvalidMedia, message)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn video(duration: f64) -> EditSource {
        EditSource {
            kind: MediaKind::Video,
            duration_sec: duration,
            has_audio: true,
            width: Some(320),
            height: Some(240),
        }
    }

    fn audio(duration: f64) -> EditSource {
        EditSource {
            kind: MediaKind::Audio,
            duration_sec: duration,
            has_audio: true,
            width: None,
            height: None,
        }
    }

    fn recipe(json: &str) -> EditRecipe {
        serde_json::from_str(json).unwrap()
    }

    fn args_for(plan: &EditPlan) -> Vec<String> {
        build_edit_args(
            &PathBuf::from("in.mp4"),
            None,
            &PathBuf::from(format!("out.{}", plan.output_extension())),
            plan,
            &MediaConfig::default(),
            "libx264",
            "ultrafast",
        )
    }

    fn graph_of(args: &[String]) -> &str {
        let index = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .unwrap();
        &args[index + 1]
    }

    #[test]
    fn empty_recipe_is_the_neutral_edit() {
        let parsed = recipe("{}");
        assert_eq!(parsed, EditRecipe::default());
        let plan = parsed.plan(&video(12.5)).unwrap();
        assert!(plan.segments.is_empty());
        assert!(plan.with_video && plan.with_audio);
        assert_eq!(plan.output_kind(), MediaKind::Video);
        assert_eq!(plan.output_extension(), "mp4");
        assert!((plan.output_duration_sec - 12.5).abs() < 1e-9);
    }

    #[test]
    fn recipe_reads_camel_case_and_ignores_unknown_fields() {
        let parsed = recipe(
            r#"{"segments":[[1,2.5],[4,6]],"speed":2,"volume":0.5,"mute":false,
                "fadeInSec":0.5,"fadeOutSec":1,"reverse":true,
                "crop":{"x":0.1,"y":0.2,"width":0.5,"height":0.6},
                "rotate":90,"flipH":true,"flipV":false,"output":"gif","later":1}"#,
        );
        assert_eq!(parsed.segments, vec![[1.0, 2.5], [4.0, 6.0]]);
        assert_eq!(parsed.output, EditOutput::Gif);
        assert_eq!(parsed.rotate, 90);
        assert!(parsed.flip_h && parsed.reverse);
        assert_eq!(parsed.crop.unwrap().width, 0.5);
        assert!(serde_json::from_str::<EditRecipe>(r#"{"output":"webm"}"#).is_err());
        assert!(serde_json::from_str::<EditRecipe>(r#"{"segments":[[1]]}"#).is_err());
    }

    #[test]
    fn images_and_incompatible_outputs_are_refused() {
        let image = EditSource {
            kind: MediaKind::Image,
            duration_sec: 0.0,
            has_audio: false,
            width: Some(320),
            height: Some(240),
        };
        assert_eq!(
            recipe("{}").plan(&image).unwrap_err(),
            "Les images se modifient directement sur le board."
        );
        assert!(recipe(r#"{"output":"gif"}"#).plan(&audio(10.0)).is_err());
        assert!(recipe(r#"{"output":"audio"}"#).plan(&audio(10.0)).is_err());
        assert!(recipe(r#"{"mute":true}"#).plan(&audio(10.0)).is_err());
        assert!(recipe(r#"{"output":"audio","mute":true}"#)
            .plan(&video(10.0))
            .is_err());
        let silent = EditSource {
            has_audio: false,
            ..video(10.0)
        };
        assert!(recipe(r#"{"output":"audio"}"#).plan(&silent).is_err());
        let plan = recipe("{}").plan(&silent).unwrap();
        assert!(plan.with_video && !plan.with_audio);
    }

    #[test]
    fn numbers_out_of_range_are_refused() {
        for json in [
            r#"{"speed":0.1}"#,
            r#"{"speed":5}"#,
            r#"{"volume":-1}"#,
            r#"{"volume":3.5}"#,
            r#"{"fadeInSec":11}"#,
            r#"{"fadeOutSec":-0.5}"#,
            r#"{"rotate":45}"#,
            r#"{"crop":{"x":0.5,"y":0,"width":0.6,"height":1}}"#,
            r#"{"crop":{"x":0,"y":0,"width":0.01,"height":1}}"#,
            r#"{"crop":{"x":-0.2,"y":0,"width":0.5,"height":1}}"#,
        ] {
            assert!(recipe(json).plan(&video(10.0)).is_err(), "{json}");
        }
        let nan = EditRecipe {
            speed: f64::NAN,
            ..EditRecipe::default()
        };
        assert!(nan.plan(&video(10.0)).is_err());
    }

    #[test]
    fn segments_are_checked_and_clamped() {
        // A small overshoot of the (rounded) duration is clamped.
        let plan = recipe(r#"{"segments":[[1,3],[8,10.4]]}"#)
            .plan(&video(10.0))
            .unwrap();
        assert_eq!(plan.segments, vec![(1.0, 3.0), (8.0, 10.0)]);
        assert!((plan.kept_duration_sec - 4.0).abs() < 1e-9);

        // The whole media, given as one range, is not a cut.
        let whole = recipe(r#"{"segments":[[0,10.2]]}"#)
            .plan(&video(10.0))
            .unwrap();
        assert!(whole.segments.is_empty());

        for json in [
            r#"{"segments":[[3,1]]}"#,
            r#"{"segments":[[-1,2]]}"#,
            r#"{"segments":[[0,4],[3,6]]}"#,
            r#"{"segments":[[5,6],[1,2]]}"#,
            r#"{"segments":[[2,13]]}"#,
            r#"{"segments":[[10,10.5]]}"#,
            r#"{"segments":[[1,1.05]]}"#,
        ] {
            assert!(recipe(json).plan(&video(10.0)).is_err(), "{json}");
        }
        let too_many = EditRecipe {
            segments: (0..=MAX_SEGMENTS)
                .map(|index| [index as f64, index as f64 + 0.5])
                .collect(),
            ..EditRecipe::default()
        };
        assert!(too_many.plan(&video(100.0)).is_err());
    }

    #[test]
    fn reverse_has_no_duration_limit_and_gif_has_one() {
        let long = recipe(r#"{"reverse":true}"#).plan(&video(3600.0)).unwrap();
        assert!(long.reverses_in_pieces());
        let short = recipe(r#"{"reverse":true,"segments":[[0,5]]}"#)
            .plan(&video(3600.0))
            .unwrap();
        assert!(!short.reverses_in_pieces());
        let sound = recipe(r#"{"reverse":true}"#).plan(&audio(3600.0)).unwrap();
        assert!(!sound.reverses_in_pieces());
        let extracted = recipe(r#"{"reverse":true,"output":"audio"}"#)
            .plan(&video(3600.0))
            .unwrap();
        assert!(!extracted.reverses_in_pieces());
        assert!(recipe(r#"{"output":"gif"}"#).plan(&video(61.0)).is_err());
        assert!(recipe(r#"{"output":"gif","speed":2}"#)
            .plan(&video(100.0))
            .is_ok());
    }

    #[test]
    fn fades_are_shared_when_longer_than_the_output() {
        let plan = recipe(r#"{"segments":[[0,2]],"fadeInSec":3,"fadeOutSec":0.5}"#)
            .plan(&audio(10.0))
            .unwrap();
        assert!((plan.fade_in_sec - 1.0).abs() < 1e-9);
        assert!((plan.fade_out_sec - 0.5).abs() < 1e-9);
    }

    #[test]
    fn picture_operations_are_ignored_for_audio() {
        let plan =
            recipe(r#"{"rotate":90,"flipH":true,"crop":{"x":0,"y":0,"width":0.5,"height":0.5}}"#)
                .plan(&audio(10.0))
                .unwrap();
        assert_eq!(plan.rotate, 0);
        assert!(!plan.flip_h && plan.crop.is_none() && !plan.with_video);
        let full = recipe(r#"{"crop":{"x":0,"y":0,"width":1,"height":1}}"#)
            .plan(&video(10.0))
            .unwrap();
        assert!(full.crop.is_none());
    }

    #[test]
    fn atempo_stays_inside_its_range() {
        assert!(atempo_chain(1.0).is_empty());
        assert_eq!(atempo_chain(2.0), vec!["atempo=2"]);
        assert_eq!(atempo_chain(4.0), vec!["atempo=2", "atempo=2"]);
        assert_eq!(atempo_chain(0.25), vec!["atempo=0.5", "atempo=0.5"]);
        assert_eq!(atempo_chain(3.0), vec!["atempo=2", "atempo=1.5"]);
        assert_eq!(atempo_chain(0.4), vec!["atempo=0.5", "atempo=0.8"]);
    }

    #[test]
    fn neutral_video_edit_encodes_what_the_normalizer_copies() {
        let plan = recipe("{}").plan(&video(10.0)).unwrap();
        let args = args_for(&plan);
        let graph = graph_of(&args);
        assert_eq!(
            graph,
            "[0:v:0]scale=w='trunc(iw*min(1,1080/ih)/2)*2':h='trunc(min(ih,1080)/2)*2',setsar=1[v];[0:a:0]anull[a]"
        );
        let joined = args.join(" ");
        assert!(joined.contains("-map [v] -map [a]"));
        assert!(joined.contains("-c:v libx264 -preset ultrafast -pix_fmt yuv420p"));
        assert!(joined.contains("-c:a aac -b:a 192k -movflags +faststart"));
        assert_eq!(args.last().unwrap(), "out.mp4");
        assert_eq!(
            &args[..6],
            ["-y", "-hide_banner", "-v", "error", "-i", "in.mp4"]
        );
    }

    #[test]
    fn single_range_is_trimmed_without_concat() {
        let plan = recipe(r#"{"segments":[[1.5,4]]}"#)
            .plan(&video(10.0))
            .unwrap();
        let args = args_for(&plan);
        let graph = graph_of(&args);
        assert!(graph.starts_with(
            "[0:v:0]trim=start=1.5:end=4,setpts=PTS-STARTPTS[vc];[0:a:0]atrim=start=1.5:end=4,asetpts=PTS-STARTPTS[ac];"
        ));
        assert!(!graph.contains("concat"));
        assert!(graph.contains("[vc]scale="));
        assert!(graph.ends_with("[ac]anull[a]"));
    }

    #[test]
    fn several_ranges_are_split_trimmed_and_concatenated() {
        let plan = recipe(r#"{"segments":[[0,2],[4,6],[8,9.5]]}"#)
            .plan(&video(10.0))
            .unwrap();
        let args = args_for(&plan);
        let graph = graph_of(&args);
        assert!(graph.contains("[0:v:0]split=3[vs0][vs1][vs2]"));
        assert!(graph.contains("[0:a:0]asplit=3[as0][as1][as2]"));
        assert!(graph.contains("[vs1]trim=start=4:end=6,setpts=PTS-STARTPTS[vt1]"));
        assert!(graph.contains("[as2]atrim=start=8:end=9.5,asetpts=PTS-STARTPTS[at2]"));
        assert!(graph.contains("[vt0][at0][vt1][at1][vt2][at2]concat=n=3:v=1:a=1[vc][ac]"));
    }

    #[test]
    fn picture_and_sound_filters_follow_the_documented_order() {
        let plan = recipe(
            r#"{"segments":[[0,8]],"speed":2,"volume":1.5,"fadeInSec":0.5,"fadeOutSec":1,
                "reverse":true,"crop":{"x":0.1,"y":0.2,"width":0.5,"height":0.6},
                "rotate":90,"flipH":true,"flipV":true}"#,
        )
        .plan(&video(10.0))
        .unwrap();
        assert!((plan.output_duration_sec - 4.0).abs() < 1e-9);
        let args = args_for(&plan);
        let graph = graph_of(&args);
        assert!(graph.contains(
            "[vc]crop=w='max(2,trunc(iw*0.5/2)*2)':h='max(2,trunc(ih*0.6/2)*2)':x='trunc(iw*0.1)':y='trunc(ih*0.2)',transpose=1,hflip,vflip,reverse,setpts=PTS/2,fade=t=in:st=0:d=0.5,fade=t=out:st=3:d=1,scale="
        ));
        assert!(graph.contains(
            "[ac]areverse,atempo=2,volume=1.5,afade=t=in:st=0:d=0.5,afade=t=out:st=3:d=1[a]"
        ));
        let turned = recipe(r#"{"rotate":180}"#).plan(&video(10.0)).unwrap();
        assert!(graph_of(&args_for(&turned)).starts_with("[0:v:0]hflip,vflip,scale="));
        let left = recipe(r#"{"rotate":270}"#).plan(&video(10.0)).unwrap();
        assert!(graph_of(&args_for(&left)).starts_with("[0:v:0]transpose=2,scale="));
    }

    #[test]
    fn muted_or_silent_video_has_no_audio_chain() {
        for plan in [
            recipe(r#"{"mute":true,"segments":[[0,2],[3,4]]}"#)
                .plan(&video(10.0))
                .unwrap(),
            recipe(r#"{"segments":[[0,2],[3,4]]}"#)
                .plan(&EditSource {
                    has_audio: false,
                    ..video(10.0)
                })
                .unwrap(),
        ] {
            let args = args_for(&plan);
            let graph = graph_of(&args);
            assert!(!graph.contains("0:a"), "{graph}");
            assert!(graph.contains("[vt0][vt1]concat=n=2:v=1:a=0[vc]"));
            assert!(args.contains(&"-an".to_string()));
            assert!(!args.contains(&"aac".to_string()));
            assert!(!args.contains(&"[a]".to_string()));
        }
    }

    #[test]
    fn audio_edits_and_extraction_produce_mp3() {
        let plan = recipe(r#"{"segments":[[1,3]],"speed":0.5}"#)
            .plan(&audio(10.0))
            .unwrap();
        assert_eq!(plan.output_extension(), "mp3");
        let args = args_for(&plan);
        let graph = graph_of(&args);
        assert_eq!(
            graph,
            "[0:a:0]atrim=start=1:end=3,asetpts=PTS-STARTPTS[ac];[ac]atempo=0.5[a]"
        );
        assert!(args
            .join(" ")
            .ends_with("-map [a] -vn -c:a libmp3lame -b:a 192k out.mp3"));

        let extracted = recipe(r#"{"output":"audio","rotate":90}"#)
            .plan(&video(10.0))
            .unwrap();
        assert_eq!(extracted.output_kind(), MediaKind::Audio);
        let args = args_for(&extracted);
        assert_eq!(graph_of(&args), "[0:a:0]anull[a]");
        assert!(!args.contains(&"[v]".to_string()));
    }

    #[test]
    fn gif_uses_a_palette_and_no_sound() {
        let plan = recipe(r#"{"output":"gif","segments":[[1,3]],"volume":2}"#)
            .plan(&video(10.0))
            .unwrap();
        assert_eq!(plan.output_kind(), MediaKind::Image);
        assert_eq!(plan.output_mime(), "image/gif");
        let args = args_for(&plan);
        let graph = graph_of(&args);
        assert!(graph.contains(
            "[vc]fps=12,scale=w='min(480,iw)':h=-1:flags=lanczos,split[g0][g1];[g0]palettegen[gp];[g1][gp]paletteuse[v]"
        ));
        assert!(!graph.contains("atrim"));
        assert!(args.join(" ").ends_with("-map [v] -an -loop 0 out.gif"));
    }

    fn total(pieces: &[Vec<(f64, f64)>]) -> f64 {
        pieces
            .iter()
            .flatten()
            .map(|(start, end)| end - start)
            .sum()
    }

    #[test]
    fn reverse_pieces_start_from_the_end_in_equal_parts() {
        let pieces = reverse_pieces(&[(0.0, 12.0)], 5.0);
        assert_eq!(
            pieces,
            vec![vec![(8.0, 12.0)], vec![(4.0, 8.0)], vec![(0.0, 4.0)]]
        );
        assert_eq!(reverse_pieces(&[(1.0, 4.0)], 5.0), vec![vec![(1.0, 4.0)]]);
        assert_eq!(
            reverse_pieces(&[(0.0, 10.0)], 5.0),
            vec![vec![(5.0, 10.0)], vec![(0.0, 5.0)]]
        );
        assert!(reverse_pieces(&[], 5.0).is_empty());
    }

    #[test]
    fn reverse_pieces_span_the_cuts_and_keep_everything() {
        let ranges = [(0.0, 3.0), (10.0, 14.0), (20.0, 23.3)];
        let pieces = reverse_pieces(&ranges, 5.0);
        assert_eq!(pieces.len(), 3);
        assert!((total(&pieces) - 10.3).abs() < 1e-9);
        for piece in &pieces {
            let kept: f64 = piece.iter().map(|(start, end)| end - start).sum();
            assert!(kept <= 5.0 + 1e-9, "{piece:?}");
            assert!(piece.windows(2).all(|pair| pair[0].1 <= pair[1].0));
        }
        assert_eq!(pieces[0].last().unwrap().1, 23.3);
        assert_eq!(pieces[2].first().unwrap().0, 0.0);
        // Ascending pieces once put back in source order, without overlap.
        let flat: Vec<(f64, f64)> = pieces.iter().rev().flatten().copied().collect();
        assert!(flat.windows(2).all(|pair| pair[0].1 <= pair[1].0 + 1e-9));
        // A long media: pieces of at most 5 s.
        let long = reverse_pieces(&[(0.0, 3600.0)], 5.0);
        assert_eq!(long.len(), 720);
        assert!((total(&long) - 3600.0).abs() < 1e-6);
    }

    #[test]
    fn a_reversed_piece_is_framed_reversed_and_joined_later() {
        let plan = recipe(
            r#"{"segments":[[2,7],[8,20]],"speed":2,"volume":2,"fadeInSec":1,
                "reverse":true,"crop":{"x":0,"y":0,"width":0.5,"height":0.5},"rotate":90}"#,
        )
        .plan(&video(30.0))
        .unwrap();
        assert!(plan.reverses_in_pieces());
        let pieces = reverse_pieces(&plan.kept_ranges(), REVERSE_PIECE_SEC);
        assert_eq!(pieces.len(), 4);
        // The piece that spans the cut: [6.25, 7] then [8, 11.5].
        assert_eq!(pieces[2], vec![(6.25, 7.0), (8.0, 11.5)]);
        let args = build_reverse_piece_args(
            &PathBuf::from("in.mp4"),
            None,
            &PathBuf::from("piece.mkv"),
            &plan,
            &pieces[2],
            720,
        );
        let joined = args.join(" ");
        assert!(joined.starts_with("-y -hide_banner -v error -ss 6.25 -t 5.25 -i in.mp4"));
        let graph = graph_of(&args);
        assert!(graph.contains("trim=start=0:end=0.75,"), "{graph}");
        assert!(graph.contains("trim=start=1.75:end=5.25,"), "{graph}");
        assert!(graph.contains("transpose=1,reverse,scale="), "{graph}");
        assert!(graph.contains("[ac]areverse[a]"), "{graph}");
        assert!(!graph.contains("setpts=PTS/2") && !graph.contains("fade"));
        assert!(joined.ends_with(
            "-c:a pcm_s16le -c:v libx264 -preset veryfast -crf 14 -pix_fmt yuv420p piece.mkv"
        ));

        let rest = plan.after_reversed_pieces();
        let args = edit_args(
            &["-f", "concat", "-safe", "0"],
            &PathBuf::from("pieces.ffconcat"),
            None,
            &PathBuf::from("out.mp4"),
            &rest,
            &MediaConfig::default(),
            "libx264",
            "ultrafast",
        );
        assert!(args
            .join(" ")
            .starts_with("-y -hide_banner -v error -f concat -safe 0 -i pieces.ffconcat"));
        let graph = graph_of(&args);
        assert!(!graph.contains("trim") && !graph.contains("reverse") && !graph.contains("crop"));
        assert!(graph.contains("setpts=PTS/2,fade=t=in:st=0:d=1"), "{graph}");
        assert!(
            graph.contains("atempo=2,volume=2,afade=t=in:st=0:d=1"),
            "{graph}"
        );
        assert_eq!(
            reverse_concat_list(&["a.mkv".to_string(), "b.mkv".to_string()]),
            "ffconcat version 1.0\nfile 'a.mkv'\nfile 'b.mkv'\n"
        );
    }

    #[test]
    fn numbers_are_written_without_trailing_zeros() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(12.34567), "12.3457");
        assert_eq!(num(0.0), "0");
        assert_eq!(num(10.0), "10");
    }

    /// Base64 of a PNG header declaring `width` and `height`: enough for the
    /// recipe check, which never decodes the picture.
    fn overlay_png(width: u32, height: u32) -> String {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine as _;

        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        STANDARD.encode(bytes)
    }

    fn annotated(json: &str) -> EditPlan {
        recipe(json).plan(&video(10.0)).unwrap()
    }

    #[test]
    fn mask_fractions_become_source_pixels() {
        let plan = annotated(
            r#"{"masks":[{"x":0.1,"y":0.2,"width":0.5,"height":0.25,
                "mode":"pixel","strength":0.05}]}"#,
        );
        assert_eq!(plan.frame_size, Some((320, 240)));
        assert_eq!(plan.masks.len(), 1);
        let mask = plan.masks[0];
        assert_eq!((mask.x, mask.y), (32, 48));
        assert_eq!((mask.width, mask.height), (160, 60));
        assert!((mask.strength - 16.0).abs() < 1e-9);
        assert_eq!(mask.mode, MaskMode::Pixel);
        // The mode is optional and defaults to pixel blocks.
        let default =
            annotated(r#"{"masks":[{"x":0,"y":0,"width":0.5,"height":0.5,"strength":0.01}]}"#);
        assert_eq!(default.masks[0].mode, MaskMode::Pixel);
        // A blur mask keeps its sigma, never smaller than two pixels.
        let blur = annotated(
            r#"{"masks":[{"x":0,"y":0,"width":0.5,"height":0.5,"mode":"blur","strength":0.001}]}"#,
        );
        assert_eq!(blur.masks[0].mode, MaskMode::Blur);
        assert!((blur.masks[0].strength - MIN_MASK_STRENGTH_PX).abs() < 1e-9);
        // A mask ending on the frame edge stays inside it, two pixels wide.
        let corner = annotated(
            r#"{"masks":[{"x":0.995,"y":0.995,"width":0.005,"height":0.005,"strength":0.1}]}"#,
        );
        let mask = corner.masks[0];
        assert_eq!((mask.x, mask.y), (318, 238));
        assert_eq!((mask.width, mask.height), (2, 2));
    }

    #[test]
    fn masks_are_checked() {
        for json in [
            // Outside the frame.
            r#"{"masks":[{"x":0.6,"y":0,"width":0.5,"height":1,"strength":0.1}]}"#,
            // Smaller than half a percent of a side.
            r#"{"masks":[{"x":0,"y":0,"width":0.001,"height":0.5,"strength":0.1}]}"#,
            // Stronger than a quarter of the frame width.
            r#"{"masks":[{"x":0,"y":0,"width":0.5,"height":0.5,"strength":0.5}]}"#,
        ] {
            assert!(recipe(json).plan(&video(10.0)).is_err(), "{json}");
        }
        // An unknown hiding mode is refused before the recipe exists.
        assert!(serde_json::from_str::<EditRecipe>(
            r#"{"masks":[{"x":0,"y":0,"width":0.5,"height":0.5,"strength":0.1,"mode":"black"}]}"#
        )
        .is_err());
        let mut mask_recipe =
            recipe(r#"{"masks":[{"x":0,"y":0,"width":0.5,"height":0.5,"strength":0.1}]}"#);
        mask_recipe.masks[0].strength = f64::NAN;
        assert_eq!(
            mask_recipe.plan(&video(10.0)).unwrap_err(),
            "Le masque 1 doit être composé de nombres."
        );
        let too_many = format!(
            r#"{{"masks":[{}]}}"#,
            (0..=MAX_MASKS)
                .map(|_| r#"{"x":0,"y":0,"width":0.1,"height":0.1,"strength":0.1}"#)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert_eq!(
            recipe(&too_many).plan(&video(10.0)).unwrap_err(),
            format!("Au plus {MAX_MASKS} masques sont acceptés.")
        );
        // A video whose picture size is unknown cannot be annotated.
        let unknown = EditSource {
            width: None,
            height: None,
            ..video(10.0)
        };
        assert_eq!(
            recipe(r#"{"adjust":{"brightness":120}}"#)
                .plan(&unknown)
                .unwrap_err(),
            "La taille de l’image est inconnue."
        );
        // An audio edit ignores them entirely.
        let sound = recipe(
            r#"{"output":"audio","masks":[{"x":0,"y":0,"width":0.5,"height":0.5,"strength":0.1}]}"#,
        )
        .plan(&video(10.0))
        .unwrap();
        assert!(sound.masks.is_empty() && sound.frame_size.is_none());
    }

    #[test]
    fn overlay_must_be_a_png_of_the_frame_size() {
        let overlay = |png: String, width: u32, height: u32| {
            format!(r#"{{"overlay":{{"png":"{png}","width":{width},"height":{height}}}}}"#)
        };
        let plan = annotated(&overlay(overlay_png(320, 240), 320, 240));
        assert!(plan.overlay);
        assert_eq!(plan.frame_size, Some((320, 240)));
        assert_eq!(
            recipe(&overlay(overlay_png(320, 240), 640, 480))
                .plan(&video(10.0))
                .unwrap_err(),
            "La couche d’annotation doit avoir la taille de l’image."
        );
        assert_eq!(
            recipe(&overlay(overlay_png(160, 240), 320, 240))
                .plan(&video(10.0))
                .unwrap_err(),
            "La taille déclarée de la couche d’annotation ne correspond pas à son image."
        );
        assert_eq!(
            recipe(&overlay("bm90IGEgcG5n".to_string(), 320, 240))
                .plan(&video(10.0))
                .unwrap_err(),
            "La couche d’annotation n’est pas un PNG."
        );
        assert_eq!(
            recipe(&overlay("é".to_string(), 320, 240))
                .plan(&video(10.0))
                .unwrap_err(),
            "La couche d’annotation n’est pas un base64 valide."
        );
    }

    #[test]
    fn colour_adjustments_are_validated_and_ordered() {
        let plan = annotated(r#"{"adjust":{"brightness":120,"contrast":150,"saturate":80}}"#);
        assert!(!plan.adjust.is_neutral());
        let args = args_for(&plan);
        let graph = graph_of(&args);
        assert!(
            graph.contains("[0:v:0]eq=brightness=0.2:contrast=1.5:saturation=0.8[adjusted]"),
            "{graph}"
        );
        // The board's presets are folded exactly like its CSS filters.
        let vivid = graph_of(&args_for(&annotated(r#"{"adjust":{"preset":"vivid"}}"#))).to_owned();
        assert!(
            vivid.contains("eq=brightness=0:contrast=1.1:saturation=1.6"),
            "{vivid}"
        );
        let mono = graph_of(&args_for(&annotated(r#"{"adjust":{"preset":"mono"}}"#))).to_owned();
        assert!(mono.contains("hue=s=0"), "{mono}");
        let sepia = graph_of(&args_for(&annotated(r#"{"adjust":{"preset":"sepia"}}"#))).to_owned();
        assert!(
            sepia.contains("colorchannelmixer=rr=0.4537:rg=0.6921:rb=0.1701"),
            "{sepia}"
        );
        let invert =
            graph_of(&args_for(&annotated(r#"{"adjust":{"preset":"invert"}}"#))).to_owned();
        assert!(invert.contains("negate"), "{invert}");
        // The blur radius follows the frame size, like the board's slider.
        let blurred = graph_of(&args_for(&annotated(r#"{"adjust":{"blur":2}}"#))).to_owned();
        assert!(blurred.contains("gblur=sigma=1.6"), "{blurred}");
        for json in [
            r#"{"adjust":{"brightness":500}}"#,
            r#"{"adjust":{"contrast":-1}}"#,
            r#"{"adjust":{"saturate":401}}"#,
            r#"{"adjust":{"blur":51}}"#,
        ] {
            assert!(recipe(json).plan(&video(10.0)).is_err(), "{json}");
        }
        // An unknown preset is refused before the recipe exists.
        assert!(serde_json::from_str::<EditRecipe>(r#"{"adjust":{"preset":"vintage"}}"#).is_err());
        // A neutral adjustment is not an annotation: it needs no frame size.
        let neutral = EditSource {
            width: None,
            height: None,
            ..video(10.0)
        };
        assert!(recipe(r#"{"adjust":{"brightness":100}}"#)
            .plan(&neutral)
            .is_ok());
    }

    #[test]
    fn masks_and_overlay_are_drawn_before_the_crop() {
        let overlay = overlay_png(320, 240);
        let plan = annotated(&format!(
            r#"{{"masks":[{{"x":0.1,"y":0.2,"width":0.5,"height":0.25,"strength":0.05}},
                {{"x":0.6,"y":0.1,"width":0.2,"height":0.2,"mode":"blur","strength":0.02}}],
                "overlay":{{"png":"{overlay}","width":320,"height":240}},
                "adjust":{{"brightness":120}},
                "crop":{{"x":0.1,"y":0,"width":0.5,"height":1}},
                "rotate":90}}"#
        ));
        let args = build_edit_args(
            &PathBuf::from("in.mp4"),
            Some(Path::new("out.png")),
            &PathBuf::from("out.mp4"),
            &plan,
            &MediaConfig::default(),
            "libx264",
            "ultrafast",
        );
        let graph = graph_of(&args);
        // One copy of the frame per mask, then one overlay per mask, in order.
        assert!(graph.contains("[0:v:0]split=3[mc0][mc1][mbase]"), "{graph}");
        assert!(
            graph.contains("[mc0]pixelize=w=16:h=16,crop=w=160:h=60:x=32:y=48[mr0]"),
            "{graph}"
        );
        assert!(
            graph.contains("[mc1]gblur=sigma=6.4,crop=w=64:h=48:x=192:y=24[mr1]"),
            "{graph}"
        );
        assert!(graph.contains("[mbase][mr0]overlay=32:48[mn0]"), "{graph}");
        assert!(graph.contains("[mn0][mr1]overlay=192:24[mn1]"), "{graph}");
        assert!(graph.contains("[mn1][ov]overlay=0:0[annotated]"), "{graph}");
        assert!(graph.contains("[1:v]scale=w=320:h=240"), "{graph}");
        assert!(graph.contains("[annotated]eq="), "{graph}");
        // The geometry comes after the annotations: crop, rotation, flips.
        let annotations_at = graph.find("gblur").unwrap();
        let crop_at = graph.find("crop=w='max(2").unwrap();
        let rotate_at = graph.find("transpose=1").unwrap();
        assert!(annotations_at < crop_at && crop_at < rotate_at, "{graph}");
        // The overlay is a second input, a single frame repeated by `overlay`.
        let joined = args.join(" ");
        assert!(joined.contains("-i in.mp4 -i out.png"), "{joined}");
    }

    #[test]
    fn reversed_pieces_burn_the_annotations_once() {
        let plan = annotated(
            r#"{"reverse":true,"masks":[{"x":0.5,"y":0.5,"width":0.25,"height":0.25,"strength":0.05}],
                "adjust":{"preset":"mono"}}"#,
        );
        assert!(plan.reverses_in_pieces());
        let pieces = reverse_pieces(&plan.kept_ranges(), REVERSE_PIECE_SEC);
        let piece_args = build_reverse_piece_args(
            &PathBuf::from("in.mp4"),
            Some(Path::new("annotations.png")),
            &PathBuf::from("piece.mkv"),
            &plan,
            &pieces[0],
            1080,
        );
        let piece_graph = graph_of(&piece_args);
        assert!(piece_graph.contains("overlay=160:120"), "{piece_graph}");
        assert!(piece_graph.contains("hue=s=0"), "{piece_graph}");
        assert!(piece_args.join(" ").contains("-i annotations.png"));
        // The join pass must not draw them a second time.
        let rest = plan.after_reversed_pieces();
        assert!(rest.masks.is_empty() && !rest.overlay && rest.adjust.is_neutral());
        assert!(rest.frame_size.is_none());
        let join_args = args_for(&rest);
        let join_graph = graph_of(&join_args);
        assert!(
            !join_graph.contains("gblur") && !join_graph.contains("hue"),
            "{join_graph}"
        );
        assert!(!join_args.join(" ").contains("-loop 1"));
    }
}
