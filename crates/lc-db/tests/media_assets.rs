//! Tests du repository `media_assets` : CRUD, TTL, purge et cascades SQL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use chrono::Duration;
use lc_db::DbError;
use lc_domain::{MediaAssetId, MediaStatus};

#[tokio::test]
async fn crud_media_asset() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.media_assets();
    let now = common::now();
    let asset = common::sample_asset("hash-a", now);
    repo.insert(&asset).await.unwrap();

    let fetched = repo.get(asset.id).await.unwrap().expect("média inséré");
    assert_eq!(fetched, asset, "aller-retour SQL doit être fidèle");

    let by_hash = repo.get_by_source_hash("hash-a").await.unwrap().unwrap();
    assert_eq!(by_hash.id, asset.id);
    assert!(repo
        .get_by_source_hash("hash-inconnu")
        .await
        .unwrap()
        .is_none());
    assert!(repo.get(MediaAssetId::new_v4()).await.unwrap().is_none());

    assert!(repo
        .update_status(asset.id, MediaStatus::Failed, Some("codec non supporté"))
        .await
        .unwrap());
    let failed = repo.get(asset.id).await.unwrap().unwrap();
    assert_eq!(failed.status, MediaStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("codec non supporté"));

    // Statut repassé à READY : l'erreur est effacée.
    assert!(repo
        .update_status(asset.id, MediaStatus::Ready, None)
        .await
        .unwrap());
    let ready = repo.get(asset.id).await.unwrap().unwrap();
    assert_eq!(ready.status, MediaStatus::Ready);
    assert_eq!(ready.error, None);

    // Identifiant inconnu : `false`, pas d'erreur.
    assert!(!repo
        .update_status(MediaAssetId::new_v4(), MediaStatus::Ready, None)
        .await
        .unwrap());
}

#[tokio::test]
async fn source_hash_unique() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.media_assets();
    let now = common::now();

    let asset = common::sample_asset("hash-doublon", now);
    repo.insert(&asset).await.unwrap();

    let doublon = common::sample_asset("hash-doublon", now);
    let error = repo.insert(&doublon).await.unwrap_err();
    assert!(
        error.is_unique_violation(),
        "source_hash doit être UNIQUE : {error}"
    );
}

#[tokio::test]
async fn touche_et_listes_par_statut() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.media_assets();
    let now = common::now();

    let ready_old = common::sample_asset("ready-old", common::date(common::NOW_MS - 2_000));
    let ready_new = common::sample_asset("ready-new", common::date(common::NOW_MS - 1_000));
    let mut processing = common::sample_asset("processing", now);
    processing.status = MediaStatus::Processing;
    repo.insert(&ready_old).await.unwrap();
    repo.insert(&ready_new).await.unwrap();
    repo.insert(&processing).await.unwrap();

    let ready = repo.list_by_status(MediaStatus::Ready, 10).await.unwrap();
    assert_eq!(ready.len(), 2);
    assert_eq!(ready[0].id, ready_old.id, "tri par created_at croissant");
    let limited = repo.list_by_status(MediaStatus::Ready, 1).await.unwrap();
    assert_eq!(limited.len(), 1);
    let unlimited = repo.list_by_status(MediaStatus::Ready, 0).await.unwrap();
    assert_eq!(unlimited.len(), 2);

    let later = now + Duration::seconds(30);
    let expires = now + Duration::hours(2);
    assert!(repo
        .touch_last_accessed(ready_old.id, later, expires)
        .await
        .unwrap());
    let touched = repo.get(ready_old.id).await.unwrap().unwrap();
    assert_eq!(touched.last_accessed_at, later);
    assert_eq!(touched.expires_at, expires);
}

#[tokio::test]
async fn purge_expired_supprime_et_cascade() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");

    let mut expired = common::sample_asset("expire", now);
    expired.expires_at = common::date(common::NOW_MS - 1_000);
    let mut fresh = common::sample_asset("frais", now);
    fresh.expires_at = now + Duration::hours(1);
    db.media_assets().insert(&expired).await.unwrap();
    db.media_assets().insert(&fresh).await.unwrap();

    // Élément de meme board et job rattachés au média expiré.
    let meme = common::sample_meme(&guild, expired.id, now);
    db.meme_board().create(&meme).await.unwrap();
    let mut job = common::sample_job(&guild, now);
    job.media_asset_id = Some(expired.id);
    db.playback_jobs().create(&job).await.unwrap();

    let listed = db.media_assets().list_expired(now).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, expired.id);

    assert_eq!(db.media_assets().purge_expired(now).await.unwrap(), 1);
    assert!(db.media_assets().get(expired.id).await.unwrap().is_none());
    // Cascade `ON DELETE CASCADE` sur meme_board_items.
    assert!(db.meme_board().get(meme.id).await.unwrap().is_none());
    // `ON DELETE SET NULL` sur playback_jobs.media_asset_id.
    let job = db.playback_jobs().get(job.id).await.unwrap().unwrap();
    assert_eq!(job.media_asset_id, None);

    // Le média frais est intact et une seconde purge ne supprime rien.
    assert!(db.media_assets().get(fresh.id).await.unwrap().is_some());
    assert_eq!(db.media_assets().purge_expired(now).await.unwrap(), 0);
}

#[tokio::test]
async fn dates_ms_roundtrip_valeurs_limites() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.media_assets();

    for millis in [0_i64, 1_700_000_123_456, 4_102_444_800_000] {
        let timestamp = common::date(millis);
        let asset = common::sample_asset(&format!("hash-ms-{millis}"), timestamp);
        repo.insert(&asset).await.unwrap();

        let fetched = repo.get(asset.id).await.unwrap().unwrap();
        assert_eq!(fetched.created_at, timestamp);
        assert_eq!(fetched.created_at.timestamp_millis(), millis);
        assert_eq!(fetched.last_accessed_at, timestamp);
        assert_eq!(fetched.expires_at, timestamp);
    }
}

#[tokio::test]
async fn date_hors_plage_detectee_comme_corruption() {
    let (db, _dir) = common::temp_db().await;
    let id = MediaAssetId::new_v4();

    sqlx::query(
        "INSERT INTO media_assets ( \
            id, source_hash, source_url, kind, mime, storage_path, \
            last_accessed_at, expires_at, created_at \
         ) VALUES (?, ?, ?, 'IMAGE', 'image/png', 'storage/x.png', ?, 0, 0)",
    )
    .bind(id.to_canonical())
    .bind("corrupt-hash")
    .bind("https://example.test/corrupt.png")
    .bind(i64::MAX)
    .execute(db.pool())
    .await
    .unwrap();

    let error = db.media_assets().get(id).await.unwrap_err();
    match error {
        DbError::Corrupt { table, detail } => {
            assert_eq!(table, "media_assets");
            assert!(detail.contains("last_accessed_at"), "{detail}");
        }
        other => panic!("erreur inattendue : {other}"),
    }
}
