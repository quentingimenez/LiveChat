//! Media edits applied with ffmpeg to an already normalized audio or video:
//! trimming and cutting, cropping, rotating, flipping, speed, volume, fades,
//! reversing, audio extraction and GIF export.
//!
//! The board sends an [`EditRecipe`]; [`EditRecipe::plan`] validates it against
//! the source and returns an [`EditPlan`], the only input of the pure argument
//! builder [`build_edit_args`]. [`apply_edit`] runs the result.
//!
//! Order of the picture operations: crop (on the source frame), rotation, then
//! the two flips (in the rotated frame). The output of a video edit is
//! H.264 yuv420p + AAC in MP4, under the configured maximum height, so the
//! normalizer copies it instead of encoding it a second time.

use std::path::Path;

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
/// Longest kept duration that may be reversed (`reverse` buffers every frame).
pub const MAX_REVERSE_SEC: f64 = 120.0;
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
            return Err("images are edited on the board".to_string());
        }
        let is_video = source.kind == MediaKind::Video;
        if !is_video && self.output != EditOutput::Same {
            return Err("this output is only available for a video".to_string());
        }
        if !is_video && self.mute {
            return Err("mute only applies to a video".to_string());
        }
        if self.output == EditOutput::Audio && self.mute {
            return Err("a muted video has no audio to extract".to_string());
        }
        let with_video = is_video && self.output != EditOutput::Audio;
        let wants_audio = match self.output {
            EditOutput::Gif => false,
            EditOutput::Audio => true,
            EditOutput::Same => !self.mute,
        };
        if !with_video && !source.has_audio {
            return Err("the media has no audio track".to_string());
        }
        let with_audio = wants_audio && source.has_audio;

        let speed = in_range("speed", self.speed, MIN_SPEED, MAX_SPEED)?;
        let volume = in_range("volume", self.volume, 0.0, MAX_VOLUME)?;
        let fade_in = in_range("fadeInSec", self.fade_in_sec, 0.0, MAX_FADE_SEC)?;
        let fade_out = in_range("fadeOutSec", self.fade_out_sec, 0.0, MAX_FADE_SEC)?;
        if !matches!(self.rotate, 0 | 90 | 180 | 270) {
            return Err("rotate must be 0, 90, 180 or 270".to_string());
        }

        let duration = source.duration_sec;
        if !duration.is_finite() || duration <= 0.0 {
            return Err("the media has no known duration".to_string());
        }
        let segments = plan_segments(&self.segments, duration)?;
        let kept = if segments.is_empty() {
            duration
        } else {
            segments.iter().map(|(start, end)| end - start).sum()
        };
        if kept < MIN_KEPT_SEC {
            return Err(format!("at least {MIN_KEPT_SEC} s must be kept"));
        }
        if self.reverse && kept > MAX_REVERSE_SEC {
            return Err(format!(
                "reverse is limited to {MAX_REVERSE_SEC} s of kept media"
            ));
        }
        let output_duration = kept / speed;
        if self.output == EditOutput::Gif && output_duration > MAX_GIF_SEC {
            return Err(format!("a GIF is limited to {MAX_GIF_SEC} s"));
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
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(value)
}

/// Validates and clamps the kept ranges. A single range covering the whole
/// media gives an empty list.
fn plan_segments(raw: &[[f64; 2]], duration: f64) -> Result<Vec<(f64, f64)>, String> {
    if raw.len() > MAX_SEGMENTS {
        return Err(format!("at most {MAX_SEGMENTS} segments are accepted"));
    }
    let mut segments: Vec<(f64, f64)> = Vec::with_capacity(raw.len());
    let mut previous_end = 0.0_f64;
    for (index, [start, end]) in raw.iter().copied().enumerate() {
        let number = index + 1;
        if !start.is_finite() || !end.is_finite() {
            return Err(format!("segment {number} is not a pair of numbers"));
        }
        if start < 0.0 || end <= start {
            return Err(format!("segment {number} must end after it starts"));
        }
        if start < previous_end {
            return Err("segments must be ascending and must not overlap".to_string());
        }
        if start >= duration || end > duration + DURATION_TOLERANCE_SEC {
            return Err(format!(
                "segment {number} is outside the media ({duration:.2} s)"
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
        return Err(format!("at least {MIN_KEPT_SEC} s must be kept"));
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
        return Err("crop must be made of numbers".to_string());
    }
    const EPSILON: f64 = 1e-6;
    if crop.x < -EPSILON
        || crop.y < -EPSILON
        || crop.x + crop.width > 1.0 + EPSILON
        || crop.y + crop.height > 1.0 + EPSILON
    {
        return Err("crop must stay inside the frame".to_string());
    }
    if crop.width < MIN_CROP_FRACTION || crop.height < MIN_CROP_FRACTION {
        return Err("crop must keep at least 2% of each side".to_string());
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
/// `encoder` and `preset` are only used for a video output (see
/// [`resolve_video_encoder`]).
pub fn build_edit_args(
    input: &Path,
    output: &Path,
    plan: &EditPlan,
    config: &MediaConfig,
    encoder: &str,
    preset: &str,
) -> Vec<String> {
    let mut args = vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-v".to_string(),
        "error".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-filter_complex".to_string(),
        build_filter_graph(plan, config.video.max_height),
    ];
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

/// Runs the edit and writes `output` (its extension must be
/// [`EditPlan::output_extension`]). A failed NVENC encode is tried again with
/// libx264, like the normalizer.
pub async fn apply_edit(
    input: &Path,
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
    let args = build_edit_args(input, output, plan, config, &encoder, &preset);
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
    let args = build_edit_args(input, output, plan, config, &encoder, &preset);
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
        }
    }

    fn audio(duration: f64) -> EditSource {
        EditSource {
            kind: MediaKind::Audio,
            duration_sec: duration,
            has_audio: true,
        }
    }

    fn recipe(json: &str) -> EditRecipe {
        serde_json::from_str(json).unwrap()
    }

    fn args_for(plan: &EditPlan) -> Vec<String> {
        build_edit_args(
            &PathBuf::from("in.mp4"),
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
        };
        assert_eq!(
            recipe("{}").plan(&image).unwrap_err(),
            "images are edited on the board"
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
    fn reverse_and_gif_have_a_duration_limit() {
        assert!(recipe(r#"{"reverse":true}"#).plan(&video(121.0)).is_err());
        assert!(recipe(r#"{"reverse":true,"segments":[[0,100]]}"#)
            .plan(&video(300.0))
            .is_ok());
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

    #[test]
    fn numbers_are_written_without_trailing_zeros() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(12.34567), "12.3457");
        assert_eq!(num(0.0), "0");
        assert_eq!(num(10.0), "10");
    }
}
