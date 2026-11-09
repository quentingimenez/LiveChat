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
pub(crate) fn build_play_payload(
    job: &PlaybackJob,
    media: Option<&MediaAsset>,
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
