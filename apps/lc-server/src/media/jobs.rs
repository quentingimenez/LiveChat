//! Création des jobs de lecture déclenchés par l'ingestion.
//!
//! Reproduit `createPlaybackJob` du legacy : la durée est résolue dans l'ordre
//! `durationSec` explicite → durée du média (sauf image) → `DEFAULT_DURATION`,
//! puis la guilde peut la réduire (`maxMediaTime`) ou fournir une durée par
//! défaut (`defaultMediaTime`). L'offset de départ d'une URL YouTube est encodé
//! dans un payload riche par la route : la durée du job est alors réduite
//! d'autant et `resumeOffsetSec` porte l'offset.
//!
//! Le job est persisté par ce module ; la notification de lecture est faite par
//! la route via [`crate::media::hook::notify_ingested`] (contrat Phase 4b :
//! `PlaybackEngine::notify`, jamais `enqueue` qui réinsère le job).

use chrono::{DateTime, Utc};
use lc_db::Db;
use lc_domain::entities::{MediaAsset, PlaybackJob};
use lc_domain::enums::{JobStatus, MediaKind};
use lc_domain::ids::{GuildId, PlaybackJobId};

use crate::media::errors::MediaServiceResult;

/// Spécification d'un job de lecture issu d'une requête d'ingestion.
#[derive(Debug, Clone)]
pub struct JobSpec<'a> {
    /// Guilde qui recevra le job.
    pub guild_id: GuildId,
    /// Média ingéré, si la requête en contient un.
    pub media: Option<&'a MediaAsset>,
    /// Texte à afficher (déjà construit par la route, payload riche compris).
    pub text: Option<&'a str>,
    /// Affichage du texte dans l'overlay.
    pub show_text: bool,
    /// Auteur affiché (défaut token-bound fourni par la route).
    pub author_name: Option<&'a str>,
    /// Image d'auteur affichée.
    pub author_image: Option<&'a str>,
    /// Durée explicite demandée par le client, en secondes.
    pub duration_sec: Option<i64>,
    /// Priorité du job (0 par défaut ; le meme board utilisera la priorité de
    /// l'engine en Phase 4b).
    pub priority: i64,
}

/// Crée et persiste un job `PENDING` pour la prochaine lecture de la guilde.
pub async fn create_for_ingest(
    db: &Db,
    default_duration: u64,
    spec: JobSpec<'_>,
    now: DateTime<Utc>,
) -> MediaServiceResult<PlaybackJob> {
    let guild = db.guilds().get(&spec.guild_id).await?;
    let default_duration = i64::try_from(default_duration).unwrap_or(i64::MAX);
    let is_image = spec
        .media
        .is_some_and(|asset| asset.kind == MediaKind::Image);

    // Offset encodé dans un payload riche `media` (route /ingest), uniquement
    // pour une vidéo/audio et sans reprise demandée.
    let embedded_offset = if spec.media.is_some() && !is_image {
        spec.text
            .and_then(lc_discord::rich_payload::media_start_offset_sec)
            .unwrap_or(0)
    } else {
        0
    };

    let mut duration = resolve_duration(&spec, is_image, default_duration, guild.as_ref());

    if spec.media.is_some() && !is_image && embedded_offset > 0 {
        duration = (duration - embedded_offset).max(1);
    }

    let job = PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id: spec.guild_id,
        media_asset_id: spec.media.map(|asset| asset.id),
        text: spec.text.map(str::to_owned),
        show_text: spec.show_text,
        author_name: spec.author_name.map(str::to_owned),
        author_image: spec.author_image.map(str::to_owned),
        duration_sec: duration,
        priority: spec.priority,
        status: JobStatus::Pending,
        resumes_after_job_id: None,
        resume_offset_sec: embedded_offset,
        remaining_ms_snapshot: None,
        last_playback_state_at: None,
        submission_date: now,
        execution_date: now,
        scheduled_at: now,
        started_at: None,
        finished_at: None,
    };

    db.playback_jobs().create(&job).await?;

    tracing::debug!(
        job_id = %job.id.to_canonical(),
        guild_id = %job.guild_id.as_str(),
        duration_sec = job.duration_sec,
        "job de lecture créé par l'ingestion"
    );

    Ok(job)
}

/// Résout la durée du job selon la priorité legacy, puis applique les limites
/// de la guilde.
fn resolve_duration(
    spec: &JobSpec<'_>,
    is_image: bool,
    default_duration: i64,
    guild: Option<&lc_domain::entities::Guild>,
) -> i64 {
    let explicit = spec.duration_sec;
    let media_duration = if is_image {
        None
    } else {
        spec.media.and_then(|asset| asset.duration_sec)
    };
    let fallback = if spec.media.is_some() {
        Some(default_duration.max(1))
    } else {
        None
    };

    let mut duration = explicit.or(media_duration).or(fallback).unwrap_or_else(|| {
        guild
            .and_then(|guild| guild.default_media_time)
            .unwrap_or(default_duration)
    });

    if let Some(max) = guild.and_then(|guild| guild.max_media_time) {
        duration = duration.min(max);
    }

    duration.max(1)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use chrono::Duration;
    use lc_db::DbConfig;
    use lc_domain::entities::Guild;
    use lc_domain::enums::{JobStatus, MediaKind, MediaStatus};
    use lc_domain::ids::MediaAssetId;

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Db,
        guild_id: GuildId,
        now: DateTime<Utc>,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let db = Db::open(dir.path().join("test.db"), &DbConfig::default())
            .await
            .expect("base de test");
        Fixture {
            _dir: dir,
            db,
            guild_id: GuildId::parse("123456789012345678").expect("guilde"),
            now: Utc::now(),
        }
    }

    async fn insert_guild(
        fixture: &Fixture,
        default_media_time: Option<i64>,
        max_media_time: Option<i64>,
    ) {
        let guild = Guild {
            id: fixture.guild_id.clone(),
            name: Some("guilde de test".into()),
            busy_until: None,
            default_media_time,
            max_media_time,
            display_media_full: false,
        };
        fixture
            .db
            .guilds()
            .upsert(&guild)
            .await
            .expect("guilde insérée");
    }

    async fn insert_media(
        fixture: &Fixture,
        kind: MediaKind,
        duration_sec: Option<i64>,
    ) -> MediaAsset {
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: format!("hash-{}", MediaAssetId::new_v4().to_canonical()),
            source_url: "https://example.com/video".into(),
            kind,
            mime: "video/mp4".into(),
            duration_sec,
            width: None,
            height: None,
            is_vertical: false,
            storage_path: "/tmp/media.mp4".into(),
            size_bytes: 10,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: fixture.now,
            expires_at: fixture.now + Duration::hours(1),
            created_at: fixture.now,
        };
        fixture
            .db
            .media_assets()
            .insert(&asset)
            .await
            .expect("asset inséré");
        asset
    }

    #[tokio::test]
    async fn text_only_job_uses_default_duration() {
        let fixture = fixture().await;
        insert_guild(&fixture, None, None).await;

        let job = create_for_ingest(
            &fixture.db,
            30,
            JobSpec {
                guild_id: fixture.guild_id.clone(),
                media: None,
                text: Some("bonjour"),
                show_text: true,
                author_name: Some("auteur"),
                author_image: None,
                duration_sec: None,
                priority: 0,
            },
            fixture.now,
        )
        .await
        .expect("job");

        assert_eq!(job.duration_sec, 30);
        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.resume_offset_sec, 0);
        assert_eq!(job.execution_date, fixture.now);
    }

    #[tokio::test]
    async fn media_duration_is_clamped_by_guild_max() {
        let fixture = fixture().await;
        insert_guild(&fixture, Some(10), Some(8)).await;
        let media = insert_media(&fixture, MediaKind::Video, Some(20)).await;

        let job = create_for_ingest(
            &fixture.db,
            30,
            JobSpec {
                guild_id: fixture.guild_id.clone(),
                media: Some(&media),
                text: None,
                show_text: false,
                author_name: None,
                author_image: None,
                duration_sec: None,
                priority: 0,
            },
            fixture.now,
        )
        .await
        .expect("job");

        assert_eq!(job.duration_sec, 8);
        assert_eq!(job.media_asset_id, Some(media.id));
    }

    #[tokio::test]
    async fn explicit_duration_wins_over_media_duration() {
        let fixture = fixture().await;
        insert_guild(&fixture, None, None).await;
        let media = insert_media(&fixture, MediaKind::Video, Some(20)).await;

        let job = create_for_ingest(
            &fixture.db,
            30,
            JobSpec {
                guild_id: fixture.guild_id.clone(),
                media: Some(&media),
                text: None,
                show_text: false,
                author_name: None,
                author_image: None,
                duration_sec: Some(7),
                priority: 0,
            },
            fixture.now,
        )
        .await
        .expect("job");

        assert_eq!(job.duration_sec, 7);
    }

    #[tokio::test]
    async fn images_ignore_the_asset_duration() {
        let fixture = fixture().await;
        insert_guild(&fixture, None, None).await;
        let media = insert_media(&fixture, MediaKind::Image, Some(20)).await;

        let job = create_for_ingest(
            &fixture.db,
            30,
            JobSpec {
                guild_id: fixture.guild_id.clone(),
                media: Some(&media),
                text: None,
                show_text: false,
                author_name: None,
                author_image: None,
                duration_sec: None,
                priority: 0,
            },
            fixture.now,
        )
        .await
        .expect("job");

        assert_eq!(job.duration_sec, 30);
    }

    #[tokio::test]
    async fn rich_payload_offset_shortens_the_job_duration() {
        let fixture = fixture().await;
        insert_guild(&fixture, None, None).await;
        let media = insert_media(&fixture, MediaKind::Video, Some(20)).await;
        let text = lc_discord::rich_payload::build_media_text_payload(Some("caption"), Some(5))
            .expect("payload riche");

        let job = create_for_ingest(
            &fixture.db,
            30,
            JobSpec {
                guild_id: fixture.guild_id.clone(),
                media: Some(&media),
                text: Some(&text),
                show_text: true,
                author_name: None,
                author_image: None,
                duration_sec: None,
                priority: 0,
            },
            fixture.now,
        )
        .await
        .expect("job");

        assert_eq!(job.duration_sec, 15);
        assert_eq!(job.resume_offset_sec, 5);
    }
}
