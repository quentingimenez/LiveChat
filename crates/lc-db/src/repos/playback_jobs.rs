//! Repository `playback_jobs` (file de lecture, scheduling et reprise).
//!
//! La sélection du prochain job est **atomique en une seule requête**
//! (`claim_next`) : le scheduler Phase 3 peut appeler `claim_next(now)` depuis
//! plusieurs workers/tasks sans jamais démarrer deux fois le même job ni deux
//! jobs en parallèle sur une même guilde.

use chrono::{DateTime, Utc};
use lc_domain::entities::PlaybackJob;
use lc_domain::enums::JobStatus;
use lc_domain::ids::{GuildId, MediaAssetId, PlaybackJobId};
use lc_domain::DomainError;
use sqlx::SqlitePool;

use crate::convert::{
    decode, decode_opt, from_bool, from_ms, from_opt_ms, to_bool, to_ms, to_opt_ms,
};
use crate::error::{DbError, DbResult};

const TABLE: &str = "playback_jobs";

/// Ligne SQLite de `playback_jobs`.
#[derive(Debug, sqlx::FromRow)]
struct PlaybackJobRow {
    id: String,
    guild_id: String,
    media_asset_id: Option<String>,
    text: Option<String>,
    show_text: i64,
    author_name: Option<String>,
    author_image: Option<String>,
    duration_sec: i64,
    priority: i64,
    status: String,
    resumes_after_job_id: Option<String>,
    resume_offset_sec: i64,
    remaining_ms_snapshot: Option<i64>,
    last_playback_state_at: Option<i64>,
    submission_date: i64,
    execution_date: i64,
    scheduled_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
}

impl PlaybackJobRow {
    fn into_entity(self) -> DbResult<PlaybackJob> {
        Ok(PlaybackJob {
            id: decode(TABLE, "id", &self.id, PlaybackJobId::parse)?,
            guild_id: decode(TABLE, "guild_id", &self.guild_id, GuildId::parse)?,
            media_asset_id: decode_opt(
                TABLE,
                "media_asset_id",
                self.media_asset_id.as_deref(),
                MediaAssetId::parse,
            )?,
            text: self.text,
            show_text: from_bool(self.show_text),
            author_name: self.author_name,
            author_image: self.author_image,
            duration_sec: self.duration_sec,
            priority: self.priority,
            status: decode(TABLE, "status", &self.status, JobStatus::from_db_str)?,
            resumes_after_job_id: decode_opt(
                TABLE,
                "resumes_after_job_id",
                self.resumes_after_job_id.as_deref(),
                PlaybackJobId::parse,
            )?,
            resume_offset_sec: self.resume_offset_sec,
            remaining_ms_snapshot: self.remaining_ms_snapshot,
            last_playback_state_at: from_opt_ms(
                TABLE,
                "last_playback_state_at",
                self.last_playback_state_at,
            )?,
            submission_date: from_ms(TABLE, "submission_date", self.submission_date)?,
            execution_date: from_ms(TABLE, "execution_date", self.execution_date)?,
            scheduled_at: from_ms(TABLE, "scheduled_at", self.scheduled_at)?,
            started_at: from_opt_ms(TABLE, "started_at", self.started_at)?,
            finished_at: from_opt_ms(TABLE, "finished_at", self.finished_at)?,
        })
    }
}

/// Repository des jobs de lecture.
#[derive(Clone, Debug)]
pub struct PlaybackJobsRepository {
    pool: SqlitePool,
}

impl PlaybackJobsRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insère un job dans la file.
    pub async fn create(&self, job: &PlaybackJob) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO playback_jobs ( \
                id, guild_id, media_asset_id, text, show_text, author_name, author_image, \
                duration_sec, priority, status, resumes_after_job_id, resume_offset_sec, \
                remaining_ms_snapshot, last_playback_state_at, submission_date, execution_date, \
                scheduled_at, started_at, finished_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(job.id.to_canonical())
        .bind(job.guild_id.as_str())
        .bind(job.media_asset_id.map(|id| id.to_canonical()))
        .bind(job.text.as_deref())
        .bind(to_bool(job.show_text))
        .bind(job.author_name.as_deref())
        .bind(job.author_image.as_deref())
        .bind(job.duration_sec)
        .bind(job.priority)
        .bind(job.status.as_db_str())
        .bind(job.resumes_after_job_id.map(|id| id.to_canonical()))
        .bind(job.resume_offset_sec)
        .bind(job.remaining_ms_snapshot)
        .bind(to_opt_ms(job.last_playback_state_at))
        .bind(to_ms(job.submission_date))
        .bind(to_ms(job.execution_date))
        .bind(to_ms(job.scheduled_at))
        .bind(to_opt_ms(job.started_at))
        .bind(to_opt_ms(job.finished_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Récupère un job par identifiant.
    pub async fn get(&self, id: PlaybackJobId) -> DbResult<Option<PlaybackJob>> {
        let row = sqlx::query_as::<_, PlaybackJobRow>("SELECT * FROM playback_jobs WHERE id = ?")
            .bind(id.to_canonical())
            .fetch_optional(&self.pool)
            .await?;
        row.map(PlaybackJobRow::into_entity).transpose()
    }

    /// Réclame le prochain job éligible et le passe `PLAYING` en une seule
    /// requête atomique.
    ///
    /// Critères de sélection (parité scheduler legacy) :
    /// - `status = 'PENDING'`, `scheduled_at <= now`, `finished_at IS NULL` ;
    /// - un job de reprise (`resumes_after_job_id`) n'est éligible que si son
    ///   parent n'est plus `PENDING`/`PLAYING` (ou n'existe plus) ;
    /// - aucun autre job `PLAYING` pour la même guilde (invariant « au plus un
    ///   job en lecture par guilde ») ;
    /// - `guilds.busy_until` absent ou `<= now` (verrou anti-collision legacy) ;
    /// - ordre `priority DESC`, `submission_date ASC`, `id ASC`.
    ///
    /// Retourne `None` si aucun job n'est éligible. Sûr en concurrence : la
    /// sélection et la mise à jour `PENDING → PLAYING` sont la même écriture
    /// SQLite (le perdant voit `None` ou un autre job).
    pub async fn claim_next(&self, now: DateTime<Utc>) -> DbResult<Option<PlaybackJob>> {
        let row = sqlx::query_as::<_, PlaybackJobRow>(
            "UPDATE playback_jobs \
             SET status = 'PLAYING', started_at = ? \
             WHERE id = ( \
                 SELECT j.id FROM playback_jobs j \
                 WHERE j.status = 'PENDING' \
                   AND j.scheduled_at <= ? \
                   AND j.finished_at IS NULL \
                   AND ( \
                       j.resumes_after_job_id IS NULL \
                       OR NOT EXISTS ( \
                           SELECT 1 FROM playback_jobs p \
                           WHERE p.id = j.resumes_after_job_id \
                             AND p.status IN ('PENDING', 'PLAYING') \
                       ) \
                   ) \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM playback_jobs busy \
                       WHERE busy.guild_id = j.guild_id AND busy.status = 'PLAYING' \
                   ) \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM guilds g \
                       WHERE g.id = j.guild_id \
                         AND g.busy_until IS NOT NULL \
                         AND g.busy_until > ? \
                   ) \
                 ORDER BY j.priority DESC, j.submission_date ASC, j.id ASC \
                 LIMIT 1 \
             ) \
             AND status = 'PENDING' \
             RETURNING *",
        )
        .bind(to_ms(now))
        .bind(to_ms(now))
        .bind(to_ms(now))
        .fetch_optional(&self.pool)
        .await?;
        row.map(PlaybackJobRow::into_entity).transpose()
    }

    /// Change le statut d'un job sans toucher aux dates de cycle de vie.
    ///
    /// Retourne `false` si le job n'existe pas.
    pub async fn set_status(&self, id: PlaybackJobId, status: JobStatus) -> DbResult<bool> {
        let result = sqlx::query("UPDATE playback_jobs SET status = ? WHERE id = ?")
            .bind(status.as_db_str())
            .bind(id.to_canonical())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Termine un job : statut terminal (`DONE`/`FAILED`) + `finished_at`.
    ///
    /// `Err(DomainError::Validation)` si `status` n'est pas terminal.
    pub async fn finish(
        &self,
        id: PlaybackJobId,
        status: JobStatus,
        finished_at: DateTime<Utc>,
    ) -> DbResult<bool> {
        if !status.is_terminal() {
            return Err(DbError::Domain(DomainError::validation(
                "finish exige un statut DONE ou FAILED",
            )));
        }
        let result =
            sqlx::query("UPDATE playback_jobs SET status = ?, finished_at = ? WHERE id = ?")
                .bind(status.as_db_str())
                .bind(to_ms(finished_at))
                .bind(id.to_canonical())
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Liste les jobs en attente d'une guilde, dans l'ordre du scheduler
    /// (`priority DESC`, `submission_date ASC`, `id ASC`).
    pub async fn list_pending_by_guild(&self, guild_id: &GuildId) -> DbResult<Vec<PlaybackJob>> {
        let rows = sqlx::query_as::<_, PlaybackJobRow>(
            "SELECT * FROM playback_jobs WHERE guild_id = ? AND status = 'PENDING' \
             ORDER BY priority DESC, submission_date ASC, id ASC",
        )
        .bind(guild_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(PlaybackJobRow::into_entity).collect()
    }

    /// Compte les jobs d'un statut donné (toutes guildes).
    pub async fn count_by_status(&self, status: JobStatus) -> DbResult<i64> {
        let count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM playback_jobs WHERE status = ?")
                .bind(status.as_db_str())
                .fetch_one(&self.pool)
                .await?;
        Ok(count)
    }

    /// Supprime les jobs terminés `DONE`/`FAILED` dont `finished_at` est
    /// antérieur ou égal à `cutoff`.
    ///
    /// Retourne le nombre de jobs supprimés.
    pub async fn purge_done_before(&self, cutoff: DateTime<Utc>) -> DbResult<u64> {
        let result = sqlx::query(
            "DELETE FROM playback_jobs \
             WHERE status IN ('DONE', 'FAILED') \
               AND finished_at IS NOT NULL \
               AND finished_at <= ?",
        )
        .bind(to_ms(cutoff))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Persiste l'état de lecture reçu de l'overlay (`playback.state`).
    ///
    /// Met à jour l'offset de reprise, le snapshot du temps restant et la date
    /// du dernier état ; le statut et `busy_until` de la guilde sont gérés par
    /// l'appelant (service).
    pub async fn set_playback_state(
        &self,
        id: PlaybackJobId,
        resume_offset_sec: i64,
        remaining_ms_snapshot: Option<i64>,
        last_playback_state_at: DateTime<Utc>,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            "UPDATE playback_jobs \
             SET resume_offset_sec = ?, remaining_ms_snapshot = ?, last_playback_state_at = ? \
             WHERE id = ?",
        )
        .bind(resume_offset_sec)
        .bind(remaining_ms_snapshot)
        .bind(to_ms(last_playback_state_at))
        .bind(id.to_canonical())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Remet un job en attente de lecture (`PENDING`), avec une nouvelle date
    /// d'éligibilité.
    ///
    /// Efface `started_at`/`finished_at` ; conserve la priorité, les données
    /// auteur et le lien de reprise. Utile pour relancer un job échoué ou
    /// rejouer le dernier média.
    pub async fn requeue(&self, id: PlaybackJobId, scheduled_at: DateTime<Utc>) -> DbResult<bool> {
        let result = sqlx::query(
            "UPDATE playback_jobs \
             SET status = 'PENDING', started_at = NULL, finished_at = NULL, scheduled_at = ? \
             WHERE id = ?",
        )
        .bind(to_ms(scheduled_at))
        .bind(id.to_canonical())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
