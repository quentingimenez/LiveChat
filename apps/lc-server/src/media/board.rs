//! Board média persistant : ajout d'un média ingéré au board d'une guilde.
//!
//! Reproduit l'ajout au board média du legacy :
//!
//! - un média déjà présent sur le board de la guilde n'est pas dupliqué : il est
//!   ré-épinglé (expiration très lointaine) et renvoyé avec `created = false` ;
//! - sinon, le budget `MEDIA_BOARD_MAX_TOTAL_MB` de la guilde est vérifié avant
//!   toute écriture (`BOARD_STORAGE_LIMIT_REACHED` en cas de dépassement) ;
//! - le média est épinglé puis l'élément est créé ; une course sur la contrainte
//!   d'unicité `(guild_id, media_asset_id)` est rattrapée en relisant la ligne.

use chrono::{DateTime, Utc};
use lc_db::{Db, DbError};
use lc_domain::entities::{MediaAsset, MediaBoardItem};
use lc_domain::ids::{GuildId, MediaAssetId, MediaBoardItemId};
use lc_domain::time::datetime_to_ms;
use lc_media::config::MediaConfig;

use crate::media::assets::pinned_expiry;
use crate::media::errors::{MediaServiceError, MediaServiceResult};
use crate::media::quotas;

/// Longueurs maximales héritées du legacy (troncature, pas de rejet).
const MAX_TITLE_CHARS: usize = 240;
const MAX_MESSAGE_CHARS: usize = 500;
const MAX_AUTHOR_NAME_CHARS: usize = 120;

/// Résultat d'un ajout au board média.
#[derive(Debug, Clone)]
pub struct BoardAddResult {
    /// `true` si une nouvelle ligne a été créée, `false` si le média y était déjà.
    pub created: bool,
    /// Élément de board correspondant (existant ou nouvellement créé).
    pub item: MediaBoardItem,
}

/// Ajoute (ou retrouve) un média sur le board média d'une guilde.
///
/// L'asset doit déjà être `READY` ; l'appelant a vérifié le média et les
/// permissions. `title`, `message`, `created_by_*` sont optionnels et tronqués
/// aux limites legacy.
#[allow(clippy::too_many_arguments)]
pub async fn add_to_board(
    db: &Db,
    config: &MediaConfig,
    guild_id: &GuildId,
    asset: &MediaAsset,
    title: Option<&str>,
    message: Option<&str>,
    created_by_user_id: Option<&str>,
    created_by_name: Option<&str>,
    now: DateTime<Utc>,
) -> MediaServiceResult<BoardAddResult> {
    let repository = db.media_board();

    if let Some(existing) = repository
        .get_by_guild_and_asset(guild_id, asset.id)
        .await?
    {
        pin_asset(db, asset.id, now).await?;
        return Ok(BoardAddResult {
            created: false,
            item: existing,
        });
    }

    // Le budget du board est vérifié avant l'épinglage : un dépassement ne doit
    // laisser aucun effet de bord.
    quotas::ensure_board_capacity(db, config, guild_id, asset.size_bytes).await?;

    pin_asset(db, asset.id, now).await?;

    let item = MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: asset.id,
        title: normalize(title, MAX_TITLE_CHARS),
        message: normalize(message, MAX_MESSAGE_CHARS),
        created_by_discord_user_id: normalize_optional(created_by_user_id),
        created_by_name: normalize(created_by_name, MAX_AUTHOR_NAME_CHARS),
        created_at: now,
    };

    match repository.create(&item).await {
        Ok(()) => Ok(BoardAddResult {
            created: true,
            item,
        }),
        Err(error) if error.is_unique_violation() => {
            // Course entre deux ajouts concurrents : la ligne existe désormais.
            let existing = repository
                .get_by_guild_and_asset(guild_id, asset.id)
                .await?
                .ok_or_else(|| {
                    MediaServiceError::unavailable(
                        "élément de board média introuvable après conflit d'unicité",
                    )
                })?;
            pin_asset(db, asset.id, now).await?;
            Ok(BoardAddResult {
                created: false,
                item: existing,
            })
        }
        Err(error) => Err(error.into()),
    }
}

/// Épingle un asset prêt : expiration lointaine et dernier accès à maintenant.
async fn pin_asset(db: &Db, asset_id: MediaAssetId, now: DateTime<Utc>) -> MediaServiceResult<()> {
    let pinned_until = datetime_to_ms(pinned_expiry(now));
    let now_ms = datetime_to_ms(now);

    // Même condition que le legacy : ne touche pas un asset non prêt ni un asset
    // dont l'expiration est déjà plus lointaine.
    sqlx::query(
        "UPDATE media_assets \
            SET expires_at = ?, last_accessed_at = ? \
          WHERE id = ? AND status = 'READY' AND expires_at < ?",
    )
    .bind(pinned_until)
    .bind(now_ms)
    .bind(asset_id.to_canonical())
    .bind(pinned_until)
    .execute(db.pool())
    .await
    .map_err(DbError::from)?;

    Ok(())
}

/// Trim + troncature ; `None` si vide.
fn normalize(value: Option<&str>, max_chars: usize) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(max_chars).collect())
}

/// Identifiant Discord optionnel (pas de troncature, chaîne vide → `None`).
fn normalize_optional(value: Option<&str>) -> Option<String> {
    normalize(value, usize::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use chrono::Duration;
    use lc_db::DbConfig;
    use lc_domain::enums::MediaKind;
    use lc_domain::ids::MediaAssetId;
    use lc_media::error::MediaErrorCode;

    use super::*;
    use crate::media::errors::MediaServiceError;

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Db,
        config: MediaConfig,
        now: DateTime<Utc>,
    }

    async fn fixture(board_max_bytes: u64) -> Fixture {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let db = Db::open(dir.path().join("test.db"), &DbConfig::default())
            .await
            .expect("base de test");
        let mut config = MediaConfig::default();
        config.storage.directory = dir.path().join("media");
        config.storage.board_max_total_bytes = board_max_bytes;
        Fixture {
            _dir: dir,
            db,
            config,
            now: Utc::now(),
        }
    }

    async fn insert_asset(fixture: &Fixture, hash: &str, size_bytes: i64) -> MediaAsset {
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: hash.to_owned(),
            source_url: format!("https://example.com/{hash}"),
            kind: MediaKind::Video,
            mime: "video/mp4".into(),
            duration_sec: Some(5),
            width: None,
            height: None,
            is_vertical: false,
            storage_path: format!("/tmp/{hash}.mp4"),
            size_bytes,
            status: lc_domain::enums::MediaStatus::Ready,
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
    async fn adds_and_pins_an_asset_once() {
        let fixture = fixture(1_000).await;
        let guild = GuildId::parse("123456789012345678").expect("guilde");
        let asset = insert_asset(&fixture, "hash-board", 10).await;

        let first = add_to_board(
            &fixture.db,
            &fixture.config,
            &guild,
            &asset,
            Some("  un titre  "),
            Some("message"),
            Some("42"),
            Some("auteur"),
            fixture.now,
        )
        .await
        .expect("ajout");

        assert!(first.created);
        assert_eq!(first.item.title.as_deref(), Some("un titre"));
        assert_eq!(first.item.created_by_discord_user_id.as_deref(), Some("42"));

        let pinned = fixture
            .db
            .media_assets()
            .get(asset.id)
            .await
            .expect("lecture")
            .expect("asset présent");
        assert!(pinned.expires_at > fixture.now + Duration::days(36_000));

        let second = add_to_board(
            &fixture.db,
            &fixture.config,
            &guild,
            &asset,
            None,
            None,
            None,
            None,
            fixture.now,
        )
        .await
        .expect("second ajout");
        assert!(!second.created);
        assert_eq!(second.item.id, first.item.id);
    }

    #[tokio::test]
    async fn rejects_an_asset_that_exceeds_the_board_budget() {
        let fixture = fixture(10).await;
        let guild = GuildId::parse("123456789012345678").expect("guilde");
        let asset = insert_asset(&fixture, "hash-too-big", 20).await;

        let error = add_to_board(
            &fixture.db,
            &fixture.config,
            &guild,
            &asset,
            None,
            None,
            None,
            None,
            fixture.now,
        )
        .await
        .expect_err("budget dépassé");

        let media_error = match error {
            MediaServiceError::Media(inner) => inner,
            other => panic!("erreur inattendue : {other}"),
        };
        assert_eq!(media_error.code(), MediaErrorCode::BoardStorageLimitReached);
        assert!(fixture
            .db
            .media_board()
            .list_by_guild(&guild)
            .await
            .expect("liste")
            .is_empty());
    }

    #[tokio::test]
    async fn db_errors_are_reported_as_database_errors() {
        // La base est fermée : l'ajout doit remonter une erreur de base, pas un
        // panic.
        let fixture = fixture(1_000).await;
        let guild = GuildId::parse("123456789012345678").expect("guilde");
        let asset = insert_asset(&fixture, "hash-closed", 1).await;
        fixture.db.clone().close().await;

        let error = add_to_board(
            &fixture.db,
            &fixture.config,
            &guild,
            &asset,
            None,
            None,
            None,
            None,
            fixture.now,
        )
        .await
        .expect_err("base fermée");
        assert!(matches!(error, MediaServiceError::Database(_)));
    }
}
