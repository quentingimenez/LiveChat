//! Construction des payloads `play` et calcul des durées.
//!
//! Les règles reproduisent `buildOverlayPlayPayload` du legacy
//! (`server/src/components/messages/messagesWorker.ts:44-113`) et le calcul de
//! durée effective du scheduler : pour un média non-image reprit après une
//! préemption, `startOffsetSec` est renseigné et `durationSec` vaut la durée
//! restante.

use lc_domain::{Guild, MediaAsset, MediaKind as DomainMediaKind, PlaybackJob};
use lc_protocol::{
    AuthorPayload, MediaKind as ProtocolMediaKind, MediaPayload, PlayPayload, TextPayload,
};

use crate::config::PlaybackConfig;
use crate::MIN_SHORT_VIDEO_DISPLAY_SEC;

/// Durées d'un job au moment du dispatch (ou de la reprise).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlaybackTiming {
    /// Durée totale demandée par le job (après borne `max_media_time`).
    pub(crate) total_duration_sec: i64,
    /// Offset de départ dans le média (secondes, 0 = début).
    pub(crate) start_offset_sec: i64,
    /// Durée réellement jouée à partir du dispatch.
    pub(crate) play_duration_sec: i64,
}

/// Calcule les durées d'un job au moment de son dispatch.
///
/// A video shorter than [`MIN_SHORT_VIDEO_DISPLAY_SEC`] gets at least that
/// duration (before the guild cap).
///
/// Base : `job.duration_sec` s'il est positif, sinon la durée du média, sinon
/// la durée par défaut de la configuration. Le total est borné par
/// `guild.max_media_time` (s'il est positif) puis l'offset de reprise est
/// appliqué pour obtenir la durée jouée.
pub(crate) fn dispatch_timing(
    job: &PlaybackJob,
    media: Option<&MediaAsset>,
    guild: Option<&Guild>,
    config: &PlaybackConfig,
) -> PlaybackTiming {
    let media_duration = media
        .and_then(|asset| asset.duration_sec)
        .filter(|duration| *duration > 0);
    let mut total_duration_sec = if job.duration_sec > 0 {
        job.duration_sec
    } else {
        media_duration.unwrap_or(config.default_duration_sec)
    };
    // A very short video must be seen: its job lasts at least the minimum
    // display time (the overlay extends the clip until the job ends). A
    // resumed job keeps the duration fixed at its first dispatch.
    let is_short_video = media.is_some_and(|asset| asset.kind == DomainMediaKind::Video)
        && media_duration.is_some_and(|duration| duration < MIN_SHORT_VIDEO_DISPLAY_SEC);
    if is_short_video && job.resume_offset_sec <= 0 {
        total_duration_sec = total_duration_sec.max(MIN_SHORT_VIDEO_DISPLAY_SEC);
    }
    if let Some(max_media_time) = guild
        .and_then(|guild| guild.max_media_time)
        .filter(|max| *max > 0)
    {
        total_duration_sec = total_duration_sec.min(max_media_time);
    }
    let total_duration_sec = total_duration_sec.max(1);
    let start_offset_sec = job.resume_offset_sec.max(0).min(total_duration_sec - 1);
    PlaybackTiming {
        total_duration_sec,
        start_offset_sec,
        play_duration_sec: (total_duration_sec - start_offset_sec).max(1),
    }
}

/// Durées d'un job déjà dispatché, reconstruites depuis les colonnes persistées.
///
/// Après dispatch, `duration_sec` contient la durée réellement jouée et
/// `resume_offset_sec` l'offset cumulé dans le média : ces deux valeurs
/// suffisent à reconstruire le payload `currentJob`.
pub(crate) fn persisted_timing(job: &PlaybackJob) -> PlaybackTiming {
    let play_duration_sec = job.duration_sec.max(1);
    PlaybackTiming {
        total_duration_sec: play_duration_sec,
        start_offset_sec: job.resume_offset_sec.max(0),
        play_duration_sec,
    }
}

/// Construit le payload `play` v2 (spec `docs/rust/protocol-v2.md` §6.2).
///
/// L'URL média est **canonique et non signée** ; le hub la remplace par une URL
/// signée par client avant émission (§8). `media.startOffsetSec` n'est émis
/// que pour un média seekable (audio/vidéo) avec un offset positif.
/// `display_scale_percent` rides every kind's payload: the overlay sizes a
/// picture, the picture of a video, and the artwork of a sound alike.
pub(crate) fn build_play_payload(
    job: &PlaybackJob,
    media: Option<&MediaAsset>,
    artwork: Option<&MediaAsset>,
    display_scale_percent: Option<i64>,
    timing: PlaybackTiming,
    config: &PlaybackConfig,
) -> PlayPayload {
    let media_payload = media.map(|asset| {
        let seekable = matches!(asset.kind, DomainMediaKind::Audio | DomainMediaKind::Video);
        let offset_sec = if seekable { timing.start_offset_sec } else { 0 };
        MediaPayload {
            asset_id: asset.id.to_canonical(),
            url: canonical_media_url(config, asset, offset_sec),
            mime: asset.mime.clone(),
            kind: protocol_media_kind(asset.kind),
            duration_sec: asset
                .duration_sec
                .and_then(|duration| u64::try_from(duration).ok())
                .filter(|duration| *duration > 0),
            is_vertical: asset.is_vertical,
            start_offset_sec: if seekable && timing.start_offset_sec > 0 {
                u64::try_from(timing.start_offset_sec).ok()
            } else {
                None
            },
            artwork_asset_id: artwork
                .filter(|_| asset.kind == DomainMediaKind::Audio)
                .map(|picture| picture.id.to_canonical()),
            artwork_url: None,
            display_scale_percent,
        }
    });

    PlayPayload {
        job_id: job.id.to_canonical(),
        media: media_payload,
        text: TextPayload {
            value: job.text.clone().unwrap_or_default(),
            enabled: job.show_text,
        },
        author: AuthorPayload {
            name: job.author_name.clone().unwrap_or_default(),
            image: job.author_image.clone(),
            enabled: job.author_name.is_some(),
        },
        tweet_card: None,
        duration_sec: u64::try_from(timing.play_duration_sec).unwrap_or_default(),
        ..Default::default()
    }
}

/// URL canonique du média, non signée.
fn canonical_media_url(config: &PlaybackConfig, asset: &MediaAsset, offset_sec: i64) -> String {
    let base = config.api_base_url.trim_end_matches('/');
    let asset_id = asset.id.to_canonical();
    if offset_sec > 0 {
        format!("{base}/overlay/media/{asset_id}?startOffsetSec={offset_sec}#t={offset_sec}")
    } else {
        format!("{base}/overlay/media/{asset_id}")
    }
}

/// Conversion `MediaKind` domaine → protocole.
fn protocol_media_kind(kind: DomainMediaKind) -> ProtocolMediaKind {
    match kind {
        DomainMediaKind::Image => ProtocolMediaKind::Image,
        DomainMediaKind::Audio => ProtocolMediaKind::Audio,
        DomainMediaKind::Video => ProtocolMediaKind::Video,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lc_domain::{
        ms_to_datetime, GuildId, JobSource, JobStatus, MediaAssetId, MediaStatus, PlaybackJobId,
    };

    const TEST_NOW_MS: i64 = 1_700_000_000_000;

    fn test_now() -> time::UtcDateTime {
        ms_to_datetime(TEST_NOW_MS).expect("horodatage de test valide")
    }

    fn playback_job(duration_sec: i64, resume_offset_sec: i64) -> PlaybackJob {
        PlaybackJob {
            id: PlaybackJobId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").expect("snowflake de test"),
            media_asset_id: None,
            text: Some("bonjour".to_owned()),
            show_text: true,
            author_name: Some("Auteur".to_owned()),
            author_image: Some("https://example.test/avatar.png".to_owned()),
            duration_sec,
            source: JobSource::Board,
            status: JobStatus::Pending,
            resume_offset_sec,
            last_playback_state_at: None,
            submission_date: test_now(),
            execution_date: test_now(),
            scheduled_at: test_now(),
            started_at: None,
            finished_at: None,
        }
    }

    fn media_asset(kind: DomainMediaKind, duration_sec: Option<i64>) -> MediaAsset {
        let mime = match kind {
            DomainMediaKind::Image => "image/png",
            DomainMediaKind::Audio => "audio/mpeg",
            DomainMediaKind::Video => "video/mp4",
        };
        MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: "hash-payload-test".to_owned(),
            source_url: "https://example.test/media".to_owned(),
            source_title: None,
            kind,
            mime: mime.to_owned(),
            duration_sec,
            width: None,
            height: None,
            is_vertical: false,
            storage_path: "hash.bin".to_owned(),
            size_bytes: 1,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: test_now(),
            expires_at: test_now(),
            created_at: test_now(),
        }
    }

    fn guild_with_max(max_media_time: Option<i64>) -> Guild {
        Guild {
            id: GuildId::parse("123456789012345678").expect("snowflake de test"),
            name: Some("Guilde de test".to_owned()),
            busy_until: None,
            default_media_time: None,
            max_media_time,
            display_media_full: false,
        }
    }

    fn playback_timing(
        total_duration_sec: i64,
        start_offset_sec: i64,
        play_duration_sec: i64,
    ) -> PlaybackTiming {
        PlaybackTiming {
            total_duration_sec,
            start_offset_sec,
            play_duration_sec,
        }
    }

    #[test]
    fn dispatch_timing_ignores_a_zero_media_duration() {
        let config = PlaybackConfig::default();
        let job = playback_job(0, 0);
        let zero_duration_video = media_asset(DomainMediaKind::Video, Some(0));

        assert_eq!(
            dispatch_timing(&job, Some(&zero_duration_video), None, &config),
            playback_timing(30, 0, 30),
            "une durée média nulle ne doit pas remplacer la durée par défaut"
        );
    }

    #[test]
    fn dispatch_timing_stretches_only_videos_shorter_than_the_minimum() {
        let config = PlaybackConfig::default();
        let job = playback_job(1, 0);
        let short_video = media_asset(DomainMediaKind::Video, Some(2));
        let boundary_video = media_asset(DomainMediaKind::Video, Some(MIN_SHORT_VIDEO_DISPLAY_SEC));

        assert_eq!(
            dispatch_timing(&job, Some(&short_video), None, &config),
            playback_timing(MIN_SHORT_VIDEO_DISPLAY_SEC, 0, MIN_SHORT_VIDEO_DISPLAY_SEC)
        );
        assert_eq!(
            dispatch_timing(&job, Some(&boundary_video), None, &config),
            playback_timing(1, 0, 1),
            "un média de la durée minimale n'est pas « court » (borne stricte)"
        );
    }

    #[test]
    fn dispatch_timing_ignores_a_non_positive_max_media_time() {
        let config = PlaybackConfig::default();
        let job = playback_job(30, 0);

        assert_eq!(
            dispatch_timing(&job, None, Some(&guild_with_max(Some(0))), &config),
            playback_timing(30, 0, 30),
            "un max_media_time nul ne borne pas la lecture"
        );
        assert_eq!(
            dispatch_timing(&job, None, Some(&guild_with_max(Some(10))), &config),
            playback_timing(10, 0, 10)
        );
    }

    #[test]
    fn dispatch_timing_applies_the_resume_offset_to_the_remaining_duration() {
        let config = PlaybackConfig::default();
        let resumed = playback_job(30, 10);
        let at_the_end = playback_job(30, 100);

        assert_eq!(
            dispatch_timing(&resumed, None, None, &config),
            playback_timing(30, 10, 20)
        );
        assert_eq!(
            dispatch_timing(&at_the_end, None, None, &config),
            playback_timing(30, 29, 1),
            "l'offset de reprise est borné à la seconde qui précède la fin"
        );
    }

    #[test]
    fn build_play_payload_pins_the_protocol_fields() {
        let job = playback_job(10, 0);
        let audio = media_asset(DomainMediaKind::Audio, Some(12));
        let artwork = media_asset(DomainMediaKind::Image, None);
        let config = PlaybackConfig::default().with_api_base_url("http://127.0.0.1:3000/");

        let payload = build_play_payload(
            &job,
            Some(&audio),
            Some(&artwork),
            Some(75),
            playback_timing(10, 0, 10),
            &config,
        );

        assert_eq!(payload.job_id, job.id.to_canonical());
        assert_eq!(payload.duration_sec, 10);
        assert_eq!(payload.text.value, "bonjour");
        assert!(payload.text.enabled);
        assert_eq!(payload.author.name, "Auteur");
        assert_eq!(
            payload.author.image.as_deref(),
            Some("https://example.test/avatar.png")
        );
        assert!(payload.author.enabled);
        assert!(payload.tweet_card.is_none());
        assert!(payload.extra.is_empty());

        let media_payload = payload.media.expect("média présent");
        assert_eq!(media_payload.asset_id, audio.id.to_canonical());
        assert_eq!(
            media_payload.url,
            format!(
                "http://127.0.0.1:3000/overlay/media/{}",
                audio.id.to_canonical()
            )
        );
        assert_eq!(media_payload.mime, "audio/mpeg");
        assert_eq!(media_payload.kind, ProtocolMediaKind::Audio);
        assert_eq!(media_payload.duration_sec, Some(12));
        assert_eq!(media_payload.start_offset_sec, None);
        assert_eq!(
            media_payload.artwork_asset_id,
            Some(artwork.id.to_canonical())
        );
        assert_eq!(media_payload.artwork_url, None);
        assert_eq!(media_payload.display_scale_percent, Some(75));
    }

    #[test]
    fn build_play_payload_omits_a_non_positive_media_duration() {
        let job = playback_job(0, 0);
        let audio = media_asset(DomainMediaKind::Audio, Some(0));

        let payload = build_play_payload(
            &job,
            Some(&audio),
            None,
            None,
            playback_timing(30, 0, 30),
            &PlaybackConfig::default(),
        );

        let media_payload = payload.media.expect("média présent");
        assert_eq!(media_payload.duration_sec, None);
    }

    #[test]
    fn build_play_payload_emits_the_offset_only_for_seekable_media() {
        let job = playback_job(20, 5);
        let audio = media_asset(DomainMediaKind::Audio, Some(20));
        let image = media_asset(DomainMediaKind::Image, None);
        let config = PlaybackConfig::default();
        let timing = playback_timing(20, 5, 15);

        let audio_payload = build_play_payload(&job, Some(&audio), None, None, timing, &config)
            .media
            .expect("média audio présent");
        assert_eq!(audio_payload.start_offset_sec, Some(5));
        assert_eq!(
            audio_payload.url,
            format!(
                "http://127.0.0.1:3000/overlay/media/{}?startOffsetSec=5#t=5",
                audio.id.to_canonical()
            )
        );

        let image_payload = build_play_payload(&job, Some(&image), None, None, timing, &config)
            .media
            .expect("média image présent");
        assert_eq!(image_payload.start_offset_sec, None);
        assert_eq!(
            image_payload.url,
            format!(
                "http://127.0.0.1:3000/overlay/media/{}",
                image.id.to_canonical()
            )
        );
    }
}
