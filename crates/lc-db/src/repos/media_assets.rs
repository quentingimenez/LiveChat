//! Repository `media_assets` (cache média et cycle de vie du pipeline).

use lc_domain::entities::MediaAsset;
use lc_domain::enums::{MediaKind, MediaStatus};
use lc_domain::ids::MediaAssetId;
use sqlx::SqlitePool;
use time::UtcDateTime;

use crate::convert::{decode, from_bool, from_ms, to_bool, to_ms};
use crate::error::DbResult;

const TABLE: &str = "media_assets";

/// `purge_expired` : supprime les assets expirés que rien ne référence
/// (média ou artwork d'un item de board, pochette d'un audio/média du board,
/// job actif).
///
/// Constante pour que le test de plan (`tests` en fin de module) exécute
/// exactement la requête du repository. Les index de 0022 évitent le scan de
/// `media_board_items` (`artwork_asset_id`), de `media_assets` par la
/// sous-requête des pochettes et celui déclenché par l'action
/// `ON DELETE SET NULL` des deux colonnes de référence.
const PURGE_EXPIRED_SQL: &str = "\
    DELETE FROM media_assets \
     WHERE expires_at <= ? \
       AND NOT EXISTS ( \
           SELECT 1 FROM media_board_items \
           WHERE media_board_items.media_asset_id = media_assets.id \
              OR media_board_items.artwork_asset_id = media_assets.id \
       ) \
       AND NOT EXISTS ( \
           SELECT 1 FROM media_assets AS cover_owner \
             JOIN media_board_items \
               ON media_board_items.media_asset_id = cover_owner.id \
            WHERE cover_owner.cover_asset_id = media_assets.id \
       ) \
       AND NOT EXISTS ( \
           SELECT 1 FROM playback_jobs \
           WHERE playback_jobs.media_asset_id = media_assets.id \
             AND playback_jobs.status IN ('PENDING', 'PLAYING') \
       ) \
     RETURNING *";

/// Exclusions communes à la sélection des candidats et aux suppressions.
///
/// Les sélections de candidats et les suppressions ciblées partagent ces trois
/// `NOT EXISTS` ; `purge_expired` porte la même condition (avec `RETURNING *`)
/// dans sa propre constante, couverte par un test dédié. Un asset épinglé au
/// board (média ou artwork), utilisé comme pochette ou référencé par un job
/// actif **entre** la sélection et le DELETE n'est donc pas supprimé (TOCTOU).
///
/// L'index `media_asset_id` du board vient de la migration 0005 ; ceux de 0022
/// couvrent `artwork_asset_id` du board et `cover_asset_id`. Ensemble, ils
/// gardent le plan sur des `SEARCH` — un `NOT IN` non corrélé forçait un scan
/// d'index de `media_board_items`.
///
/// `$prefix` porte le verbe et le début du `WHERE`, `$filter` la condition
/// propre à l'appelant : `concat!` exige des littéraux.
macro_rules! purge_sql {
    ($prefix:literal, $filter:literal) => {
        concat!(
            $prefix,
            $filter,
            " \
               AND NOT EXISTS ( \
                   SELECT 1 FROM media_board_items \
                   WHERE media_board_items.media_asset_id = media_assets.id \
                      OR media_board_items.artwork_asset_id = media_assets.id \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM media_assets AS cover_owner \
                     JOIN media_board_items \
                       ON media_board_items.media_asset_id = cover_owner.id \
                    WHERE cover_owner.cover_asset_id = media_assets.id \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM playback_jobs \
                    WHERE playback_jobs.media_asset_id = media_assets.id \
                      AND playback_jobs.status IN ('PENDING', 'PLAYING') \
               )",
        )
    };
}

/// Candidats de purge d'un statut terminal (`FAILED`/`PROCESSING`), périmés
/// selon `last_accessed_at`.
const PURGE_STALE_STATUS_CANDIDATES_SQL: &str = purge_sql!(
    "SELECT id, source_hash, storage_path FROM media_assets WHERE ",
    "status = ? AND last_accessed_at <= ?"
);

/// Candidats `READY` expirés (`expires_at <= ?`).
const PURGE_EXPIRED_READY_CANDIDATES_SQL: &str = purge_sql!(
    "SELECT id, source_hash, storage_path FROM media_assets WHERE ",
    "status = 'READY' AND expires_at <= ?"
);

/// Supprime une ligne candidate terminale par identifiant, seulement si elle
/// n'a pas été épinglée (média ou artwork du board, pochette, job actif) et si
/// son état est toujours celui observé à la sélection : un statut ré-armé ou un
/// `last_accessed_at` plus récent entre la sélection et la suppression annule
/// le DELETE.
const PURGE_DELETE_CANDIDATE_SQL: &str = purge_sql!(
    "DELETE FROM media_assets WHERE ",
    "id = ? AND status = ? AND last_accessed_at <= ?"
);

/// Supprime une ligne candidate `READY` si elle est encore expirée et toujours
/// non épinglée : une ingestion concurrente peut avoir réarmé `expires_at`, et
/// un item de board peut avoir été créé, entre la sélection et la suppression.
const PURGE_DELETE_EXPIRED_READY_SQL: &str = purge_sql!(
    "DELETE FROM media_assets WHERE ",
    "id = ? AND status = 'READY' AND expires_at <= ?"
);

/// Chemins persistés non vides, utilisés pour compter les références de
/// fichiers de la purge runtime.
const STORAGE_PATHS_SQL: &str = "SELECT storage_path FROM media_assets WHERE storage_path != ''";

/// Ligne SQLite de `media_assets` (dates en ms, booléens en 0/1).
#[derive(Debug, sqlx::FromRow)]
struct MediaAssetRow {
    id: String,
    source_hash: String,
    source_url: String,
    source_title: Option<String>,
    kind: String,
    mime: String,
    duration_sec: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
    is_vertical: i64,
    storage_path: String,
    size_bytes: i64,
    status: String,
    error: Option<String>,
    last_accessed_at: i64,
    expires_at: i64,
    created_at: i64,
}

impl MediaAssetRow {
    fn into_entity(self) -> DbResult<MediaAsset> {
        Ok(MediaAsset {
            id: decode(TABLE, "id", &self.id, MediaAssetId::parse)?,
            source_hash: self.source_hash,
            source_url: self.source_url,
            source_title: self.source_title,
            kind: decode(TABLE, "kind", &self.kind, MediaKind::from_db_str)?,
            mime: self.mime,
            duration_sec: self.duration_sec,
            width: self.width,
            height: self.height,
            is_vertical: from_bool(self.is_vertical),
            storage_path: self.storage_path,
            size_bytes: self.size_bytes,
            status: decode(TABLE, "status", &self.status, MediaStatus::from_db_str)?,
            error: self.error,
            last_accessed_at: from_ms(TABLE, "last_accessed_at", self.last_accessed_at)?,
            expires_at: from_ms(TABLE, "expires_at", self.expires_at)?,
            created_at: from_ms(TABLE, "created_at", self.created_at)?,
        })
    }
}

/// Ligne candidate à la purge runtime.
///
/// Seuls les champs nécessaires au worker sont exposés : l'identifiant pour
/// supprimer la ligne, l'empreinte pour sérialiser avec une ingestion
/// concurrente, et le chemin pour supprimer le fichier si c'était sa dernière
/// référence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgeCandidate {
    /// Ligne à supprimer, identifiant **brut** tel que stocké en base.
    ///
    /// L'identifiant n'est ni reparsé ni canonicalisé : la purge doit pouvoir
    /// supprimer les lignes hors contrat (UUID non canonique, identifiant
    /// legacy) comme le faisait le worker historique, et non échouer sur elles.
    pub id: String,
    /// Empreinte de contenu (verrou `lock_source_hash`).
    pub source_hash: String,
    /// Variante persistée, vide si la ligne ne référence aucun fichier.
    pub storage_path: String,
}

/// Ligne SQLite d'un candidat de purge.
#[derive(Debug, sqlx::FromRow)]
struct PurgeCandidateRow {
    id: String,
    source_hash: String,
    storage_path: String,
}

impl PurgeCandidateRow {
    fn into_candidate(self) -> PurgeCandidate {
        PurgeCandidate {
            id: self.id,
            source_hash: self.source_hash,
            storage_path: self.storage_path,
        }
    }
}

/// Repository des médias ingérés.
#[derive(Clone, Debug)]
pub struct MediaAssetsRepository {
    pool: SqlitePool,
}

impl MediaAssetsRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insère un média (échec `UNIQUE` si `source_hash` est déjà présent).
    pub async fn insert(&self, asset: &MediaAsset) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO media_assets ( \
                id, source_hash, source_url, source_title, kind, mime, duration_sec, width, \
                height, is_vertical, storage_path, size_bytes, status, error, last_accessed_at, \
                expires_at, created_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(asset.id.to_canonical())
        .bind(asset.source_hash.as_str())
        .bind(asset.source_url.as_str())
        .bind(asset.source_title.as_deref())
        .bind(asset.kind.as_db_str())
        .bind(asset.mime.as_str())
        .bind(asset.duration_sec)
        .bind(asset.width)
        .bind(asset.height)
        .bind(to_bool(asset.is_vertical))
        .bind(asset.storage_path.as_str())
        .bind(asset.size_bytes)
        .bind(asset.status.as_db_str())
        .bind(asset.error.as_deref())
        .bind(to_ms(asset.last_accessed_at))
        .bind(to_ms(asset.expires_at))
        .bind(to_ms(asset.created_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Récupère un média par identifiant.
    pub async fn get(&self, id: MediaAssetId) -> DbResult<Option<MediaAsset>> {
        let row = sqlx::query_as::<_, MediaAssetRow>("SELECT * FROM media_assets WHERE id = ?")
            .bind(id.to_canonical())
            .fetch_optional(&self.pool)
            .await?;
        row.map(MediaAssetRow::into_entity).transpose()
    }

    /// Récupère un média par empreinte de contenu (clé de dédoublonnage).
    pub async fn get_by_source_hash(&self, source_hash: &str) -> DbResult<Option<MediaAsset>> {
        let row =
            sqlx::query_as::<_, MediaAssetRow>("SELECT * FROM media_assets WHERE source_hash = ?")
                .bind(source_hash)
                .fetch_optional(&self.pool)
                .await?;
        row.map(MediaAssetRow::into_entity).transpose()
    }

    /// Met à jour l'état du pipeline et le détail d'erreur associé.
    ///
    /// Retourne `false` si aucun média ne porte cet identifiant.
    pub async fn update_status(
        &self,
        id: MediaAssetId,
        status: MediaStatus,
        error: Option<&str>,
    ) -> DbResult<bool> {
        let result = sqlx::query("UPDATE media_assets SET status = ?, error = ? WHERE id = ?")
            .bind(status.as_db_str())
            .bind(error)
            .bind(id.to_canonical())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Met à jour le TTL glissant (`last_accessed_at` + `expires_at` calculé
    /// par l'appelant).
    pub async fn touch_last_accessed(
        &self,
        id: MediaAssetId,
        last_accessed_at: UtcDateTime,
        expires_at: UtcDateTime,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            "UPDATE media_assets SET last_accessed_at = ?, expires_at = ? WHERE id = ?",
        )
        .bind(to_ms(last_accessed_at))
        .bind(to_ms(expires_at))
        .bind(id.to_canonical())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Liste les médias expirés à l'instant `now`, les plus anciens d'abord.
    pub async fn list_expired(&self, now: UtcDateTime) -> DbResult<Vec<MediaAsset>> {
        let rows = sqlx::query_as::<_, MediaAssetRow>(
            "SELECT * FROM media_assets WHERE expires_at <= ? ORDER BY expires_at ASC, id ASC",
        )
        .bind(to_ms(now))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(MediaAssetRow::into_entity).collect()
    }

    /// Records the embedded cover (an image asset) of an audio asset.
    pub async fn set_cover(&self, id: MediaAssetId, cover: Option<MediaAssetId>) -> DbResult<()> {
        sqlx::query("UPDATE media_assets SET cover_asset_id = ? WHERE id = ?")
            .bind(cover.map(MediaAssetId::to_canonical))
            .bind(id.to_canonical())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Embedded cover recorded for an asset.
    pub async fn cover_of(&self, id: MediaAssetId) -> DbResult<Option<MediaAssetId>> {
        let raw: Option<Option<String>> =
            sqlx::query_scalar("SELECT cover_asset_id FROM media_assets WHERE id = ?")
                .bind(id.to_canonical())
                .fetch_optional(&self.pool)
                .await?;
        crate::convert::decode_opt(
            "media_assets",
            "cover_asset_id",
            raw.flatten().as_deref(),
            MediaAssetId::parse,
        )
    }

    /// Supprime les médias expirés **non référencés**.
    ///
    /// Un média expiré est conservé (et donc non retourné) s'il est encore
    /// épinglé comme média ou artwork d'un item de board, utilisé comme pochette,
    /// ou référencé par un job `PENDING`/`PLAYING` :
    /// la purge ne doit jamais casser un média visible ou une lecture en cours.
    /// Les autres sont supprimés ; les jobs terminés qui le référençaient
    /// passent `media_asset_id` à `NULL` (`ON DELETE SET NULL`).
    ///
    /// Retourne les lignes supprimées : l'appelant (worker de purge, service
    /// média) utilise `storage_path` pour supprimer les fichiers correspondants,
    /// que la couche SQL ne connaît pas.
    ///
    /// AUD-A5-007 : l'ancienne version `DELETE … WHERE expires_at <= ?` pouvait
    /// supprimer en cascade des `media_board_items` épinglés.
    pub async fn purge_expired(&self, now: UtcDateTime) -> DbResult<Vec<MediaAsset>> {
        let rows = sqlx::query_as::<_, MediaAssetRow>(PURGE_EXPIRED_SQL)
            .bind(to_ms(now))
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(MediaAssetRow::into_entity).collect()
    }

    /// Liste les candidats de purge d'un statut terminal (`FAILED` ou
    /// `PROCESSING`) dont `last_accessed_at <= stale_before`.
    ///
    /// Un asset épinglé sur le board (média, artwork ou pochette) ou référencé
    /// par un job `PENDING`/`PLAYING` n'est jamais candidat : la purge runtime
    /// ne doit pas casser un média visible ni une lecture en cours.
    pub async fn list_stale_purge_candidates(
        &self,
        status: MediaStatus,
        stale_before: UtcDateTime,
    ) -> DbResult<Vec<PurgeCandidate>> {
        let rows = sqlx::query_as::<_, PurgeCandidateRow>(PURGE_STALE_STATUS_CANDIDATES_SQL)
            .bind(status.as_db_str())
            .bind(to_ms(stale_before))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(PurgeCandidateRow::into_candidate)
            .collect())
    }

    /// Liste les assets `READY` expirés à `now`, hors board média/artwork,
    /// pochette et jobs actifs.
    pub async fn list_expired_ready_purge_candidates(
        &self,
        now: UtcDateTime,
    ) -> DbResult<Vec<PurgeCandidate>> {
        let rows = sqlx::query_as::<_, PurgeCandidateRow>(PURGE_EXPIRED_READY_CANDIDATES_SQL)
            .bind(to_ms(now))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(PurgeCandidateRow::into_candidate)
            .collect())
    }

    /// Supprime une ligne candidate terminale par identifiant brut, tel que lu
    /// en base.
    ///
    /// Retourne `false` si la ligne a déjà disparu (course avec une autre
    /// passe ou une ingestion), si elle a été épinglée entre la sélection et la
    /// suppression (exclusions média/artwork du board, pochette et job actif
    /// rejouées dans le DELETE), ou si son `status`/`last_accessed_at` a été
    /// ré-armé depuis la sélection.
    pub async fn delete_purge_candidate(
        &self,
        id: &str,
        status: MediaStatus,
        stale_before: UtcDateTime,
    ) -> DbResult<bool> {
        let result = sqlx::query(PURGE_DELETE_CANDIDATE_SQL)
            .bind(id)
            .bind(status.as_db_str())
            .bind(to_ms(stale_before))
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Supprime une ligne candidate `READY` seulement si elle est encore
    /// expirée à `now` et toujours non épinglée.
    ///
    /// Retourne `false` si une ingestion concurrente a réarmé `expires_at`, si
    /// un item de board (média ou artwork), une pochette ou un job actif l'a
    /// épinglée entre-temps, ou si la ligne a disparu : le fichier ne doit
    /// alors pas être touché.
    pub async fn delete_expired_ready_candidate(
        &self,
        id: &str,
        now: UtcDateTime,
    ) -> DbResult<bool> {
        let result = sqlx::query(PURGE_DELETE_EXPIRED_READY_SQL)
            .bind(id)
            .bind(to_ms(now))
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Chemins de stockage non vides de tous les assets.
    ///
    /// La purge s'en sert pour compter les références de fichiers : deux lignes
    /// peuvent partager la même variante (AUD-V2-S-004). La lecture porte
    /// volontairement sur toute la colonne, aucun index ne peut restreindre
    /// l'ensemble des fichiers encore référencés.
    pub async fn storage_paths(&self) -> DbResult<Vec<String>> {
        let paths = sqlx::query_scalar(STORAGE_PATHS_SQL)
            .fetch_all(&self.pool)
            .await?;
        Ok(paths)
    }

    /// Liste les médias d'un état donné, les plus anciens d'abord.
    ///
    /// `limit <= 0` signifie « sans limite ».
    pub async fn list_by_status(
        &self,
        status: MediaStatus,
        limit: i64,
    ) -> DbResult<Vec<MediaAsset>> {
        let rows = if limit > 0 {
            sqlx::query_as::<_, MediaAssetRow>(
                "SELECT * FROM media_assets WHERE status = ? ORDER BY created_at ASC, id ASC LIMIT ?",
            )
            .bind(status.as_db_str())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, MediaAssetRow>(
                "SELECT * FROM media_assets WHERE status = ? ORDER BY created_at ASC, id ASC",
            )
            .bind(status.as_db_str())
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(MediaAssetRow::into_entity).collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use time::{Duration, UtcDateTime};

    use super::*;
    use crate::{Db, DbConfig};

    /// Un asset prêt avec un titre source, persisté puis relu.
    #[tokio::test]
    async fn source_title_round_trips() {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let db = Db::open(dir.path().join("test.db"), &DbConfig::default())
            .await
            .expect("base de test");
        let now = UtcDateTime::now();
        let mut asset = sample_asset(now);
        asset.source_title = Some("Chat qui tombe".to_owned());
        db.media_assets().insert(&asset).await.expect("insertion");

        let stored = db
            .media_assets()
            .get(asset.id)
            .await
            .expect("lecture par id")
            .expect("asset présent");
        assert_eq!(stored.source_title.as_deref(), Some("Chat qui tombe"));

        let by_hash = db
            .media_assets()
            .get_by_source_hash(&asset.source_hash)
            .await
            .expect("lecture par hash")
            .expect("asset présent");
        assert_eq!(by_hash.source_title, asset.source_title);

        // `NULL` reste `NULL` (assets antérieurs à la colonne).
        let mut untitled = sample_asset(now + Duration::hours(1));
        untitled.source_title = None;
        db.media_assets()
            .insert(&untitled)
            .await
            .expect("insertion");
        let stored = db
            .media_assets()
            .get(untitled.id)
            .await
            .expect("lecture")
            .expect("asset présent");
        assert_eq!(stored.source_title, None);
    }

    /// La purge ne scanne ni `media_board_items` (artwork) ni `media_assets`
    /// (sous-requête des pochettes et action de FK `cover_asset_id`) : les index
    /// de 0022 sont utilisés. Seule l'action de FK de `playback_jobs` parcourt
    /// cette table.
    #[tokio::test]
    async fn purge_expired_ne_scanne_pas_les_tables_de_references() {
        let db = Db::open_in_memory(&DbConfig::default())
            .await
            .expect("base mémoire");
        let plan = plan(&db, PURGE_EXPIRED_SQL, &[Param::Int(0)]).await;

        assert_aucun_scan_hors("purge_expired", &plan, &["playback_jobs"]);
        assert!(
            plan.iter()
                .any(|line| line.contains("idx_media_assets_cover_asset")),
            "la FK cover_asset_id doit passer par idx_media_assets_cover_asset : {plan:?}"
        );
        assert!(
            plan.iter()
                .any(|line| line.contains("idx_media_board_items_artwork_asset")),
            "la FK artwork_asset_id doit passer par idx_media_board_items_artwork_asset : {plan:?}"
        );
    }

    /// Suit la régression qui a motivé le helper : un plan peut nommer l'alias
    /// (`SCAN a`, alias de `media_assets`) au lieu de la table, et doit être
    /// rejeté quand même ; l'ancien garde-fou ne testait que l'alias `b`.
    #[test]
    #[should_panic(expected = "scanne `a`")]
    fn le_garde_fou_rejette_un_scan_par_alias() {
        let lignes = vec!["SCAN a".to_owned()];
        assert_aucun_scan_hors("une requête de purge", &lignes, &["playback_jobs"]);
    }

    /// Paramètre lié d'un plan : les requêtes de purge mêlent texte (statut,
    /// identifiant) et entier (échéance en ms).
    enum Param<'a> {
        Text(&'a str),
        Int(i64),
    }

    async fn memory_db() -> Db {
        Db::open_in_memory(&DbConfig::default())
            .await
            .expect("base mémoire")
    }

    /// Plan SQLite de `sql`, une ligne par étape, avec ses paramètres liés.
    async fn plan(db: &Db, sql: &str, params: &[Param<'_>]) -> Vec<String> {
        let statement = format!("EXPLAIN QUERY PLAN {sql}");
        let mut query = sqlx::query(&statement);
        for param in params {
            query = match param {
                Param::Text(value) => query.bind(*value),
                Param::Int(value) => query.bind(*value),
            };
        }
        let rows = query
            .fetch_all(db.pool())
            .await
            .expect("EXPLAIN QUERY PLAN");
        rows.iter()
            .map(|row| sqlx::Row::get::<String, _>(row, "detail"))
            .collect()
    }

    /// Table parcourue par une étape `SCAN`, `None` pour les `SEARCH`.
    fn table_scannee(line: &str) -> Option<&str> {
        line.strip_prefix("SCAN ")
            .and_then(|reste| reste.split_whitespace().next())
    }

    /// Rejette toute lecture séquentielle non explicitement autorisée.
    ///
    /// SQLite affiche tantôt le nom de la table, tantôt son alias (`SCAN a`) :
    /// l'autorisation porte donc sur la cible telle qu'affichée, et un alias
    /// inconnu est rejeté. Les seules exceptions admises sont `playback_jobs`
    /// (action de FK `ON DELETE SET NULL` des suppressions) et l'énumération
    /// volontaire de `storage_paths` dans `media_assets`.
    fn assert_aucun_scan_hors(context: &str, lines: &[String], autorisees: &[&str]) {
        for line in lines {
            let Some(table) = table_scannee(line) else {
                continue;
            };
            assert!(
                autorisees.contains(&table),
                "{context} scanne `{table}` : seuls les scans de {autorisees:?} sont autorisés ; \
                 plan : {lines:?}"
            );
        }
    }

    /// Les candidats `FAILED`/`PROCESSING` passent par l'index de statut et par
    /// les deux index du board : aucun `NOT IN` ne doit revenir dans le SQL.
    #[tokio::test]
    async fn purge_stale_candidates_ne_scannent_pas_les_tables_de_references() {
        let db = memory_db().await;
        let lines = plan(
            &db,
            PURGE_STALE_STATUS_CANDIDATES_SQL,
            &[Param::Text(MediaStatus::Failed.as_db_str()), Param::Int(0)],
        )
        .await;

        assert_aucun_scan_hors("les candidats FAILED", &lines, &[]);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_assets_status")),
            "le statut doit être cherché par idx_media_assets_status : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_board_items_media_asset")),
            "le board média doit passer par idx_media_board_items_media_asset : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_board_items_artwork_asset")),
            "l'artwork doit passer par idx_media_board_items_artwork_asset : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_assets_cover_asset")),
            "la pochette doit passer par idx_media_assets_cover_asset : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_playback_jobs_status_finished")),
            "les jobs actifs doivent passer par idx_playback_jobs_status_finished : {lines:?}"
        );
    }

    /// Même garantie pour les `READY` expirés : le filtre `expires_at` reste
    /// un `SEARCH`, jamais un scan de `media_assets`.
    #[tokio::test]
    async fn purge_expired_ready_candidates_ne_scannent_pas_les_tables_de_references() {
        let db = memory_db().await;
        let lines = plan(&db, PURGE_EXPIRED_READY_CANDIDATES_SQL, &[Param::Int(0)]).await;

        assert_aucun_scan_hors("les candidats READY expirés", &lines, &[]);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_board_items_media_asset")),
            "le board média doit passer par idx_media_board_items_media_asset : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_board_items_artwork_asset")),
            "l'artwork doit passer par idx_media_board_items_artwork_asset : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_assets_cover_asset")),
            "la pochette doit passer par idx_media_assets_cover_asset : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_playback_jobs_status_finished")),
            "les jobs actifs doivent passer par idx_playback_jobs_status_finished : {lines:?}"
        );
    }

    /// La suppression d'un candidat passe par la clé primaire ; les exclusions
    /// rejouées (média ou artwork du board, pochette, job actif) et les actions
    /// de FK (`cover_asset_id`, `artwork_asset_id`, board média) restent des `SEARCH`.
    #[tokio::test]
    async fn suppression_d_un_candidat_ne_scanne_pas_les_tables_de_references() {
        let db = memory_db().await;
        let asset = MediaAssetId::new_v4().to_canonical();
        let lines = plan(
            &db,
            PURGE_DELETE_CANDIDATE_SQL,
            &[
                Param::Text(&asset),
                Param::Text(MediaStatus::Failed.as_db_str()),
                Param::Int(0),
            ],
        )
        .await;

        assert_aucun_scan_hors("la suppression d'un candidat", &lines, &["playback_jobs"]);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("sqlite_autoindex_media_assets_1")),
            "la ligne doit être supprimée par la clé primaire : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_board_items_media_asset")),
            "l'exclusion/FK board doit passer par idx_media_board_items_media_asset : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_board_items_artwork_asset")),
            "l'exclusion/FK artwork doit passer par idx_media_board_items_artwork_asset : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_assets_cover_asset")),
            "l'exclusion/FK cover doit passer par idx_media_assets_cover_asset : {lines:?}"
        );
    }

    /// La suppression conditionnelle des `READY` expirés suit le même plan que
    /// la suppression simple : clé primaire, exclusions rejouées et index de FK.
    #[tokio::test]
    async fn suppression_d_un_ready_expire_ne_scanne_pas_les_tables_de_references() {
        let db = memory_db().await;
        let asset = MediaAssetId::new_v4().to_canonical();
        let lines = plan(
            &db,
            PURGE_DELETE_EXPIRED_READY_SQL,
            &[Param::Text(&asset), Param::Int(0)],
        )
        .await;

        assert_aucun_scan_hors(
            "la suppression d'un READY expiré",
            &lines,
            &["playback_jobs"],
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("sqlite_autoindex_media_assets_1")),
            "la ligne doit être supprimée par la clé primaire : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_board_items_media_asset")),
            "l'exclusion/FK board doit passer par idx_media_board_items_media_asset : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_board_items_artwork_asset")),
            "l'exclusion/FK artwork doit passer par idx_media_board_items_artwork_asset : {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("idx_media_assets_cover_asset")),
            "l'exclusion/FK cover doit passer par idx_media_assets_cover_asset : {lines:?}"
        );
    }

    /// Une ré-ingestion qui ré-arme le statut ou `last_accessed_at` entre la
    /// sélection stale et le DELETE ne doit pas être supprimée.
    #[tokio::test]
    async fn suppression_stale_ne_supprime_pas_une_ligne_rearmee() {
        let db = memory_db().await;
        let now = UtcDateTime::now();
        let stale_before = now - Duration::seconds(10);
        let mut status_rearmed = sample_asset(now);
        status_rearmed.status = MediaStatus::Failed;
        status_rearmed.last_accessed_at = stale_before - Duration::minutes(1);
        let mut touched = sample_asset(now + Duration::minutes(1));
        touched.status = MediaStatus::Failed;
        touched.last_accessed_at = stale_before - Duration::minutes(1);
        for asset in [&status_rearmed, &touched] {
            db.media_assets().insert(asset).await.expect("insertion");
        }

        let candidates = db
            .media_assets()
            .list_stale_purge_candidates(MediaStatus::Failed, stale_before)
            .await
            .expect("candidats");
        assert_eq!(candidates.len(), 2, "candidats : {candidates:?}");

        // Ré-armement entre la sélection et le DELETE.
        db.media_assets()
            .update_status(status_rearmed.id, MediaStatus::Processing, None)
            .await
            .expect("statut ré-armé");
        db.media_assets()
            .touch_last_accessed(touched.id, now, now + Duration::hours(1))
            .await
            .expect("last_accessed_at ré-armé");

        for (label, id) in [
            ("statut ré-armé", &status_rearmed.id),
            ("last_accessed_at ré-armé", &touched.id),
        ] {
            assert!(
                !db.media_assets()
                    .delete_purge_candidate(&id.to_canonical(), MediaStatus::Failed, stale_before)
                    .await
                    .expect("suppression"),
                "un asset dont le {label} entre la sélection et le DELETE ne doit pas être supprimé"
            );
            assert!(
                db.media_assets().get(*id).await.expect("lecture").is_some(),
                "l'asset dont le {label} reste en base"
            );
        }
    }

    /// La purge supprime l'identifiant tel qu'il est stocké : une ligne hors
    /// contrat (UUID majuscule, identifiant legacy non UUID) doit rester
    /// supprimable, comme avant le refactor.
    #[tokio::test]
    async fn la_purge_supprime_l_identifiant_brut_hors_contrat() {
        let db = memory_db().await;
        let now = UtcDateTime::now();
        let stale_before = now + Duration::hours(2);
        let mut uppercase = sample_asset(now);
        uppercase.status = MediaStatus::Failed;
        uppercase.last_accessed_at = now - Duration::minutes(5);
        let mut legacy = sample_asset(now + Duration::minutes(1));
        legacy.status = MediaStatus::Failed;
        legacy.last_accessed_at = now - Duration::minutes(5);
        for asset in [&uppercase, &legacy] {
            db.media_assets().insert(asset).await.expect("insertion");
        }

        let uppercase_raw = uppercase.id.to_string().to_uppercase();
        let legacy_raw = "legacy-media-id".to_owned();
        for (raw_id, canonical_id) in [
            (&uppercase_raw, uppercase.id.to_canonical().to_owned()),
            (&legacy_raw, legacy.id.to_canonical().to_owned()),
        ] {
            sqlx::query("UPDATE media_assets SET id = ? WHERE id = ?")
                .bind(raw_id)
                .bind(&canonical_id)
                .execute(db.pool())
                .await
                .expect("identifiant hors contrat");
        }

        let candidates = db
            .media_assets()
            .list_stale_purge_candidates(MediaStatus::Failed, stale_before)
            .await
            .expect("candidats");
        assert_eq!(candidates.len(), 2, "candidats : {candidates:?}");
        let ids: Vec<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
        assert!(
            ids.contains(&uppercase_raw.as_str()),
            "l'UUID majuscule doit être candidat : {ids:?}"
        );
        assert!(
            ids.contains(&legacy_raw.as_str()),
            "l'identifiant legacy doit être candidat : {ids:?}"
        );

        for candidate in &candidates {
            assert!(
                db.media_assets()
                    .delete_purge_candidate(&candidate.id, MediaStatus::Failed, stale_before)
                    .await
                    .expect("suppression"),
                "l'identifiant brut `{}` doit être supprimé",
                candidate.id
            );
        }
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_assets")
            .fetch_one(db.pool())
            .await
            .expect("comptage");
        assert_eq!(remaining, 0, "les deux lignes hors contrat sont supprimées");
    }

    /// La purge lit tous les chemins de `media_assets` pour compter les
    /// références de fichiers : c'est la seule lecture séquentielle attendue,
    /// et elle ne touche aucune table de références.
    #[tokio::test]
    async fn storage_paths_ne_lit_que_media_assets() {
        let db = memory_db().await;
        let lines = plan(&db, STORAGE_PATHS_SQL, &[]).await;

        assert_eq!(
            lines.len(),
            1,
            "la lecture des chemins doit rester une seule étape : {lines:?}"
        );
        assert!(
            lines[0].starts_with("SCAN media_assets"),
            "les chemins sont lus séquentiellement dans media_assets : {lines:?}"
        );
    }

    fn sample_asset(now: UtcDateTime) -> MediaAsset {
        MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: format!("hash-{}", MediaAssetId::new_v4()),
            source_url: "https://example.com/clip.mp4".to_owned(),
            source_title: None,
            kind: MediaKind::Video,
            mime: "video/mp4".to_owned(),
            duration_sec: Some(12),
            width: None,
            height: None,
            is_vertical: false,
            storage_path: "/tmp/clip.mp4".to_owned(),
            size_bytes: 4_096,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: now,
            expires_at: now + Duration::hours(1),
            created_at: now,
        }
    }
}
