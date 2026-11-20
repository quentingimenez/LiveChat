//! Création des jobs de lecture déclenchés par l'ingestion.
//!
//! Reproduit `createPlaybackJob` du legacy : la durée est résolue dans l'ordre
//! - video/audio: the media length, else `DEFAULT_DURATION` (a requested
//!   `durationSec` is ignored);
//! - image or text only: the requested `durationSec`, else the guild
//!   `defaultMediaTime`, else `DEFAULT_DURATION`;
//! - tweet card image sans durée demandée : durée calculée à partir de la
//!   quantité de texte (voir [`super::display_duration`]) ;
//!
//! then the guild cap (`maxMediaTime`) applies. L'offset de départ d'une URL YouTube est encodé
//! dans un payload riche par la route : la durée du job est alors réduite
//! d'autant et `resumeOffsetSec` porte l'offset.
//!
//! Le job est persisté par ce module ; la notification de lecture est faite par
//! la route via [`crate::media::hook::notify_ingested`] (contrat Phase 4b :
//! `PlaybackEngine::notify`, jamais `enqueue` qui réinsère le job).

use lc_db::Db;
use lc_domain::entities::{MediaAsset, PlaybackJob};
use lc_domain::enums::{JobStatus, MediaKind};
use lc_domain::ids::{GuildId, PlaybackJobId};
use time::UtcDateTime;

use crate::media::display_duration::tweet_card_duration_sec;
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
    /// Priorité du job (0 par défaut ; le board média utilisera la priorité de
    /// l'engine en Phase 4b).
    pub priority: i64,
}

/// Crée et persiste un job `PENDING` pour la prochaine lecture de la guilde.
pub async fn create_for_ingest(
    db: &Db,
    default_duration: u64,
    spec: JobSpec<'_>,
    now: UtcDateTime,
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

/// Resolves the job duration (see the module doc), then applies the guild
/// cap.
fn resolve_duration(
    spec: &JobSpec<'_>,
    is_image: bool,
    default_duration: i64,
    guild: Option<&lc_domain::entities::Guild>,
) -> i64 {
    let guild_default = guild
        .and_then(|guild| guild.default_media_time)
        .unwrap_or(default_duration);
    let mut duration = match spec.media {
        Some(asset) if !is_image => asset.duration_sec.unwrap_or(default_duration),
        Some(asset) => spec
            .duration_sec
            .or_else(|| tweet_card_duration_sec(asset, guild_default))
            .unwrap_or(guild_default),
        None => spec.duration_sec.unwrap_or(guild_default),
    };

    if let Some(max) = guild.and_then(|guild| guild.max_media_time) {
        duration = duration.min(max);
    }

    duration.max(1)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use lc_db::DbConfig;
    use lc_domain::entities::Guild;
    use lc_domain::enums::{JobStatus, MediaKind, MediaStatus};
    use lc_domain::ids::MediaAssetId;
    use time::Duration;

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Db,
        guild_id: GuildId,
        now: UtcDateTime,
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
            now: UtcDateTime::now(),
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
        insert_asset(
            fixture,
            kind,
            duration_sec,
            "https://example.com/video",
            None,
        )
        .await
    }

    async fn insert_asset(
        fixture: &Fixture,
        kind: MediaKind,
        duration_sec: Option<i64>,
        source_url: &str,
        source_title: Option<&str>,
    ) -> MediaAsset {
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: format!("hash-{}", MediaAssetId::new_v4().to_canonical()),
            source_url: source_url.into(),
            source_title: source_title.map(str::to_owned),
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

    /// Job duration for `media` (or text only) with `explicit` requested.
    async fn duration_for(
        fixture: &Fixture,
        media: Option<&MediaAsset>,
        explicit: Option<i64>,
    ) -> i64 {
        duration_for_with_default(fixture, media, explicit, 30).await
    }

    async fn duration_for_with_default(
        fixture: &Fixture,
        media: Option<&MediaAsset>,
        explicit: Option<i64>,
        default_duration: u64,
    ) -> i64 {
        create_for_ingest(
            &fixture.db,
            default_duration,
            JobSpec {
                guild_id: fixture.guild_id.clone(),
                media,
                text: media.is_none().then_some("bonjour"),
                show_text: media.is_none(),
                author_name: None,
                author_image: None,
                duration_sec: explicit,
                priority: 0,
            },
            fixture.now,
        )
        .await
        .expect("job")
        .duration_sec
    }

    #[tokio::test]
    async fn video_ignores_explicit_duration() {
        let fixture = fixture().await;
        insert_guild(&fixture, None, None).await;
        let media = insert_media(&fixture, MediaKind::Video, Some(20)).await;
        assert_eq!(duration_for(&fixture, Some(&media), Some(5)).await, 20);
    }

    #[tokio::test]
    async fn image_uses_explicit_then_guild_default() {
        let fixture = fixture().await;
        insert_guild(&fixture, Some(12), None).await;
        let media = insert_media(&fixture, MediaKind::Image, None).await;
        assert_eq!(duration_for(&fixture, Some(&media), Some(7)).await, 7);
        assert_eq!(duration_for(&fixture, Some(&media), None).await, 12);
    }

    #[tokio::test]
    async fn text_only_keeps_explicit_duration() {
        let fixture = fixture().await;
        insert_guild(&fixture, None, None).await;
        assert_eq!(duration_for(&fixture, None, Some(9)).await, 9);
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
    async fn tweet_card_duration_follows_its_text_length() {
        let fixture = fixture().await;
        insert_guild(&fixture, None, None).await;
        let tweet_url = "https://x.com/jeanne/status/1234567890";
        let short =
            insert_asset(&fixture, MediaKind::Image, None, tweet_url, Some("Bonjour")).await;
        let long_text = "a".repeat(120);
        let long = insert_asset(
            &fixture,
            MediaKind::Image,
            None,
            "https://x.com/jeanne/status/9876543210",
            Some(&long_text),
        )
        .await;

        assert_eq!(
            duration_for_with_default(&fixture, Some(&short), None, 5).await,
            6
        );
        assert_eq!(
            duration_for_with_default(&fixture, Some(&long), None, 5).await,
            15
        );
        // A requested duration still has priority over the text-based one.
        assert_eq!(duration_for(&fixture, Some(&long), Some(9)).await, 9);
    }

    #[tokio::test]
    async fn tweet_card_duration_respects_the_guild_default_and_cap() {
        let fixture = fixture().await;
        insert_guild(&fixture, Some(12), Some(18)).await;
        let tweet_url = "https://x.com/jeanne/status/1234567890";
        let short =
            insert_asset(&fixture, MediaKind::Image, None, tweet_url, Some("Bonjour")).await;
        let long_text = "a".repeat(240);
        let long = insert_asset(
            &fixture,
            MediaKind::Image,
            None,
            "https://x.com/jeanne/status/9876543210",
            Some(&long_text),
        )
        .await;

        // The guild default is a floor: 6 s computed, but 12 s configured.
        assert_eq!(
            duration_for_with_default(&fixture, Some(&short), None, 5).await,
            12
        );
        // The guild cap still wins over the text-based duration (30 s > 18 s).
        assert_eq!(
            duration_for_with_default(&fixture, Some(&long), None, 5).await,
            18
        );
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
