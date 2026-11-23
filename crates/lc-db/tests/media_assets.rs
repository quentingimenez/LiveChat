//! Tests du repository `media_assets` : CRUD, TTL, purge et cascades SQL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use lc_db::DbError;
use lc_domain::{JobStatus, MediaAssetId, MediaKind, MediaStatus};
use time::Duration;

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
async fn purge_expired_protege_board_pochette_et_jobs_actifs() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");

    // Six assets expirés : épinglé au board, pochette de cet item, artwork du
    // même item, référencé par un job PENDING, orphelin, et référencé par un
    // job terminé (DONE).
    let mut pinned = common::sample_asset("pin", now);
    pinned.kind = MediaKind::Audio;
    pinned.mime = "audio/mpeg".to_owned();
    pinned.expires_at = common::date(common::NOW_MS - 1_000);
    let mut cover = common::sample_asset("pochette", now);
    cover.kind = MediaKind::Image;
    cover.mime = "image/jpeg".to_owned();
    cover.expires_at = common::date(common::NOW_MS - 1_000);
    let mut artwork = common::sample_asset("artwork", now);
    artwork.kind = MediaKind::Image;
    artwork.mime = "image/png".to_owned();
    artwork.expires_at = common::date(common::NOW_MS - 1_000);
    let mut active = common::sample_asset("actif", now);
    active.expires_at = common::date(common::NOW_MS - 1_000);
    let mut orphan = common::sample_asset("orphelin", now);
    orphan.expires_at = common::date(common::NOW_MS - 1_000);
    let mut done = common::sample_asset("termine", now);
    done.expires_at = common::date(common::NOW_MS - 1_000);
    for asset in [&pinned, &cover, &artwork, &active, &orphan, &done] {
        db.media_assets().insert(asset).await.unwrap();
    }

    let mut item = common::sample_board_item(&guild, pinned.id, now);
    item.artwork_asset_id = Some(artwork.id);
    item.artwork_on_overlay = true;
    db.media_board().create(&item).await.unwrap();
    db.media_assets()
        .set_cover(pinned.id, Some(cover.id))
        .await
        .unwrap();

    let mut pending_job = common::sample_job(&guild, now);
    pending_job.media_asset_id = Some(active.id);
    db.playback_jobs().create(&pending_job).await.unwrap();

    let mut done_job = common::sample_job(&guild, now);
    done_job.media_asset_id = Some(done.id);
    done_job.status = JobStatus::Done;
    done_job.finished_at = Some(now);
    db.playback_jobs().create(&done_job).await.unwrap();

    let listed = db.media_assets().list_expired(now).await.unwrap();
    assert_eq!(listed.len(), 6, "les six assets sont expirés");

    let purged = db.media_assets().purge_expired(now).await.unwrap();
    assert_eq!(
        purged.len(),
        2,
        "seuls les assets sans référence au board, en pochette ou dans un job actif sont purgés"
    );
    let purged_ids: Vec<_> = purged.iter().map(|asset| asset.id).collect();
    assert!(purged_ids.contains(&orphan.id), "asset orphelin purgé");
    assert!(purged_ids.contains(&done.id), "asset d'un job DONE purgé");
    assert!(
        purged.iter().all(|asset| !asset.storage_path.is_empty()),
        "les lignes supprimées sont retournées pour supprimer les fichiers"
    );

    // Les références protégées sont intactes.
    assert!(db.media_assets().get(pinned.id).await.unwrap().is_some());
    assert!(db.media_assets().get(cover.id).await.unwrap().is_some());
    assert_eq!(
        db.media_assets().cover_of(pinned.id).await.unwrap(),
        Some(cover.id),
        "la pochette de l'item de board reste référencée"
    );
    assert!(
        db.media_assets().get(artwork.id).await.unwrap().is_some(),
        "l'artwork de l'item de board reste référencé"
    );
    let stored_board = db.media_board().get(item.id).await.unwrap().unwrap();
    assert_eq!(
        stored_board.artwork_asset_id,
        Some(artwork.id),
        "la référence artwork de l'item de board est conservée"
    );
    assert!(db.media_assets().get(active.id).await.unwrap().is_some());
    let stored_pending = db
        .playback_jobs()
        .get(pending_job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored_pending.media_asset_id, Some(active.id));

    // Le job terminé survit, mais sa FK est passée à NULL (ON DELETE SET NULL).
    let stored_done = db.playback_jobs().get(done_job.id).await.unwrap().unwrap();
    assert_eq!(stored_done.media_asset_id, None);

    // Les assets encore référencés restent expirés : une seconde purge ne les
    // supprime toujours pas.
    let second = db.media_assets().purge_expired(now).await.unwrap();
    assert!(
        second.is_empty(),
        "aucun autre asset purgeable : {second:?}"
    );
    assert!(db.media_assets().get(pinned.id).await.unwrap().is_some());
    assert!(db.media_assets().get(cover.id).await.unwrap().is_some());
    assert!(db.media_assets().get(artwork.id).await.unwrap().is_some());
    assert!(db.media_assets().get(active.id).await.unwrap().is_some());
}

#[tokio::test]
async fn suppressions_conditionnelles_ne_suppriment_pas_un_asset_epingle_entre_temps() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");
    let stale_since = common::date(common::NOW_MS - 10_000);
    let expired = common::date(common::NOW_MS - 1_000);
    let repo = db.media_assets();

    // Trois candidats FAILED, sélectionnés avant tout épinglage.
    let mut board_pinned = common::sample_asset("board-pin", now);
    board_pinned.status = MediaStatus::Failed;
    board_pinned.last_accessed_at = stale_since;
    let mut cover_pinned = common::sample_asset("cover-pin", now);
    cover_pinned.status = MediaStatus::Failed;
    cover_pinned.last_accessed_at = stale_since;
    let mut job_pinned = common::sample_asset("job-pin", now);
    job_pinned.status = MediaStatus::Failed;
    job_pinned.last_accessed_at = stale_since;

    // Supports non expirés des références créées après la sélection.
    let mut cover_owner = common::sample_asset("cover-owner", now);
    cover_owner.expires_at = now + Duration::hours(1);
    let mut artwork_owner = common::sample_asset("artwork-owner", now);
    artwork_owner.expires_at = now + Duration::hours(1);
    let mut ready_artwork = common::sample_asset("ready-artwork", now);
    ready_artwork.expires_at = expired;

    for asset in [
        &board_pinned,
        &cover_pinned,
        &job_pinned,
        &cover_owner,
        &artwork_owner,
        &ready_artwork,
    ] {
        repo.insert(asset).await.unwrap();
    }

    let stale = repo
        .list_stale_purge_candidates(MediaStatus::Failed, now + Duration::seconds(1))
        .await
        .unwrap();
    assert_eq!(stale.len(), 3, "trois candidats FAILED : {stale:?}");
    let ready = repo.list_expired_ready_purge_candidates(now).await.unwrap();
    assert!(
        ready
            .iter()
            .any(|candidate| candidate.id == ready_artwork.id.to_canonical()),
        "l'artwork expiré est candidat avant épinglage : {ready:?}"
    );

    // Épinglage survenu entre la sélection et la suppression.
    let board_item = common::sample_board_item(&guild, board_pinned.id, now);
    db.media_board().create(&board_item).await.unwrap();

    let cover_item = common::sample_board_item(&guild, cover_owner.id, now);
    db.media_board().create(&cover_item).await.unwrap();
    repo.set_cover(cover_owner.id, Some(cover_pinned.id))
        .await
        .unwrap();

    let mut pending_job = common::sample_job(&guild, now);
    pending_job.media_asset_id = Some(job_pinned.id);
    db.playback_jobs().create(&pending_job).await.unwrap();

    let mut artwork_item = common::sample_board_item(&guild, artwork_owner.id, now);
    artwork_item.artwork_asset_id = Some(ready_artwork.id);
    artwork_item.artwork_on_overlay = true;
    db.media_board().create(&artwork_item).await.unwrap();

    for (label, id) in [
        ("média du board", board_pinned.id),
        ("pochette d'un item", cover_pinned.id),
        ("job actif", job_pinned.id),
    ] {
        assert!(
            !repo
                .delete_purge_candidate(&id.to_canonical())
                .await
                .unwrap(),
            "un asset devenu {label} entre la sélection et le DELETE ne doit pas être supprimé"
        );
        assert!(
            repo.get(id).await.unwrap().is_some(),
            "l'asset {label} reste en base"
        );
    }

    assert!(
        !repo
            .delete_expired_ready_candidate(&ready_artwork.id.to_canonical(), now)
            .await
            .unwrap(),
        "un artwork épinglé entre la sélection et le DELETE ne doit pas être supprimé"
    );
    assert!(
        repo.get(ready_artwork.id).await.unwrap().is_some(),
        "l'artwork épinglé reste en base"
    );
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
        assert_eq!(lc_domain::datetime_to_ms(fetched.created_at), millis);
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
