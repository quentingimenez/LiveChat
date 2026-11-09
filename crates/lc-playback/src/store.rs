//! Accès SQLite complémentaire pour l'ordonnanceur.
//!
//! `lc-db` expose les opérations génériques (création, `claim_next` global,
//! `finish`, `set_playback_state`, `requeue`…) mais pas les primitives dont le
//! scheduler a besoin : liste des jobs `PLAYING` d'une guilde, claim ciblé,
//! suspension avec `resumes_after_job_id`, claim restreint à une guilde et
//! libération groupée. Ces requêtes restent isolées ici pour deux raisons :
//!
//! 1. `lc-playback` ne doit pas modifier `lc-db` (un autre agent en est
//!    propriétaire) ;
//! 2. elles réutilisent exactement les critères de `claim_next`
//!    (`crates/lc-db/src/repos/playback_jobs.rs:155-194`) afin de préserver
//!    l'invariant « au plus un `PLAYING` par guilde ».
//!
//! Écart à signaler : ces primitives mériteraient d'être promues dans
//! `lc-db` (elles seraient utiles au serveur), cf.
//! `docs/rust/lc-playback.md`.

use chrono::{DateTime, Utc};
use lc_db::{Db, DbError, DbResult};
use lc_domain::{datetime_to_ms, GuildId, PlaybackJob, PlaybackJobId};

/// Accès SQLite complémentaire, adossé au pool de `lc-db`.
pub(crate) struct PlaybackStore {
    db: Db,
}

impl PlaybackStore {
    pub(crate) fn new(db: Db) -> Self {
        Self { db }
    }

    /// Guildes ayant au moins un job `PENDING` ou `PLAYING`.
    pub(crate) async fn active_guild_ids(&self) -> DbResult<Vec<GuildId>> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT guild_id FROM playback_jobs \
             WHERE status IN ('PENDING', 'PLAYING') ORDER BY guild_id",
        )
        .fetch_all(self.db.pool())
        .await?;
        rows.into_iter()
            .map(|raw| GuildId::parse(&raw).map_err(DbError::from))
            .collect()
    }

    /// Jobs `PLAYING` d'une guilde, du plus ancien au plus récent.
    pub(crate) async fn playing_jobs(&self, guild_id: &GuildId) -> DbResult<Vec<PlaybackJob>> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM playback_jobs \
             WHERE guild_id = ? AND status = 'PLAYING' \
             ORDER BY started_at ASC, id ASC",
        )
        .bind(guild_id.as_str())
        .fetch_all(self.db.pool())
        .await?;
        let mut jobs = Vec::with_capacity(rows.len());
        for raw in rows {
            let job_id = PlaybackJobId::parse(&raw).map_err(DbError::from)?;
            if let Some(job) = self.db.playback_jobs().get(job_id).await? {
                jobs.push(job);
            }
        }
        Ok(jobs)
    }

    /// Claim ciblé d'un job précis (préemption/reprise), avec les mêmes gardes
    /// que `claim_next`.
    pub(crate) async fn claim_specific(
        &self,
        job_id: PlaybackJobId,
        guild_id: &GuildId,
        now: DateTime<Utc>,
    ) -> DbResult<Option<PlaybackJob>> {
        let now_ms = datetime_to_ms(now);
        let claimed: Option<String> = sqlx::query_scalar(
            "UPDATE playback_jobs SET status = 'PLAYING', started_at = ? \
             WHERE id = ( \
                 SELECT j.id FROM playback_jobs j \
                 WHERE j.id = ? AND j.guild_id = ? AND j.status = 'PENDING' \
                   AND j.finished_at IS NULL AND j.scheduled_at <= ? \
                   AND (j.resumes_after_job_id IS NULL OR NOT EXISTS ( \
                       SELECT 1 FROM playback_jobs p \
                       WHERE p.id = j.resumes_after_job_id AND p.status IN ('PENDING', 'PLAYING'))) \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM playback_jobs busy \
                       WHERE busy.guild_id = j.guild_id AND busy.status = 'PLAYING') \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM guilds g \
                       WHERE g.id = j.guild_id AND g.busy_until IS NOT NULL AND g.busy_until > ?) \
                 LIMIT 1) \
             AND status = 'PENDING' \
             RETURNING id",
        )
        .bind(now_ms)
        .bind(job_id.to_canonical())
        .bind(guild_id.as_str())
        .bind(now_ms)
        .bind(now_ms)
        .fetch_optional(self.db.pool())
        .await?;
        match claimed {
            Some(raw) => {
                let claimed_id = PlaybackJobId::parse(&raw).map_err(DbError::from)?;
                self.db.playback_jobs().get(claimed_id).await
            }
            None => Ok(None),
        }
    }

    /// Claim du prochain job d'une guilde, avec l'ordre de priorité legacy.
    pub(crate) async fn claim_next_for_guild(
        &self,
        guild_id: &GuildId,
        now: DateTime<Utc>,
    ) -> DbResult<Option<PlaybackJob>> {
        let now_ms = datetime_to_ms(now);
        let claimed: Option<String> = sqlx::query_scalar(
            "UPDATE playback_jobs SET status = 'PLAYING', started_at = ? \
             WHERE id = ( \
                 SELECT j.id FROM playback_jobs j \
                 WHERE j.guild_id = ? AND j.status = 'PENDING' \
                   AND j.finished_at IS NULL AND j.scheduled_at <= ? \
                   AND (j.resumes_after_job_id IS NULL OR NOT EXISTS ( \
                       SELECT 1 FROM playback_jobs p \
                       WHERE p.id = j.resumes_after_job_id AND p.status IN ('PENDING', 'PLAYING'))) \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM playback_jobs busy \
                       WHERE busy.guild_id = j.guild_id AND busy.status = 'PLAYING') \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM guilds g \
                       WHERE g.id = j.guild_id AND g.busy_until IS NOT NULL AND g.busy_until > ?) \
                 ORDER BY j.priority DESC, j.submission_date ASC, j.id ASC \
                 LIMIT 1) \
             AND status = 'PENDING' \
             RETURNING id",
        )
        .bind(now_ms)
        .bind(guild_id.as_str())
        .bind(now_ms)
        .bind(now_ms)
        .fetch_optional(self.db.pool())
        .await?;
        match claimed {
            Some(raw) => {
                let claimed_id = PlaybackJobId::parse(&raw).map_err(DbError::from)?;
                self.db.playback_jobs().get(claimed_id).await
            }
            None => Ok(None),
        }
    }

    /// Enregistre les durées effectives d'un job qui vient d'être dispatché.
    pub(crate) async fn mark_dispatched(
        &self,
        job_id: PlaybackJobId,
        duration_sec: i64,
        resume_offset_sec: i64,
        now: DateTime<Utc>,
    ) -> DbResult<bool> {
        let now_ms = datetime_to_ms(now);
        let result = sqlx::query(
            "UPDATE playback_jobs SET duration_sec = ?, resume_offset_sec = ?, \
             resumes_after_job_id = NULL, execution_date = ?, scheduled_at = ?, \
             remaining_ms_snapshot = NULL, last_playback_state_at = NULL \
             WHERE id = ? AND status = 'PLAYING'",
        )
        .bind(duration_sec)
        .bind(resume_offset_sec)
        .bind(now_ms)
        .bind(now_ms)
        .bind(job_id.to_canonical())
        .execute(self.db.pool())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Suspend un job `PLAYING` pour préemption (retour `PENDING` + reprise).
    pub(crate) async fn suspend_playing(
        &self,
        job_id: PlaybackJobId,
        guild_id: &GuildId,
        preempting: Option<PlaybackJobId>,
        resume_offset_sec: i64,
        remaining_ms: i64,
        now: DateTime<Utc>,
    ) -> DbResult<bool> {
        let now_ms = datetime_to_ms(now);
        let result = sqlx::query(
            "UPDATE playback_jobs SET status = 'PENDING', started_at = NULL, \
             finished_at = NULL, resumes_after_job_id = ?, resume_offset_sec = ?, \
             remaining_ms_snapshot = ?, last_playback_state_at = NULL, \
             execution_date = ?, scheduled_at = ? \
             WHERE id = ? AND guild_id = ? AND status = 'PLAYING'",
        )
        .bind(preempting.map(|job_id| job_id.to_canonical()))
        .bind(resume_offset_sec)
        .bind(remaining_ms)
        .bind(now_ms)
        .bind(now_ms)
        .bind(job_id.to_canonical())
        .bind(guild_id.as_str())
        .execute(self.db.pool())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Libère (statut `DONE`) les jobs `PLAYING` d'une guilde, éventuellement
    /// un seul. Renvoie les identifiants effectivement libérés.
    pub(crate) async fn release_playing(
        &self,
        guild_id: &GuildId,
        target: Option<PlaybackJobId>,
        now: DateTime<Utc>,
    ) -> DbResult<Vec<PlaybackJobId>> {
        let rows: Vec<String> = match target {
            Some(job_id) => {
                sqlx::query_scalar(
                    "SELECT id FROM playback_jobs \
                     WHERE guild_id = ? AND status = 'PLAYING' AND id = ? \
                     ORDER BY started_at ASC, id ASC",
                )
                .bind(guild_id.as_str())
                .bind(job_id.to_canonical())
                .fetch_all(self.db.pool())
                .await?
            }
            None => {
                sqlx::query_scalar(
                    "SELECT id FROM playback_jobs \
                     WHERE guild_id = ? AND status = 'PLAYING' \
                     ORDER BY started_at ASC, id ASC",
                )
                .bind(guild_id.as_str())
                .fetch_all(self.db.pool())
                .await?
            }
        };
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let now_ms = datetime_to_ms(now);
        let mut released = Vec::with_capacity(rows.len());
        for raw in &rows {
            sqlx::query(
                "UPDATE playback_jobs SET status = 'DONE', finished_at = ?, \
                 remaining_ms_snapshot = NULL, last_playback_state_at = NULL \
                 WHERE id = ? AND status = 'PLAYING'",
            )
            .bind(now_ms)
            .bind(raw)
            .execute(self.db.pool())
            .await?;
            released.push(PlaybackJobId::parse(raw).map_err(DbError::from)?);
        }
        Ok(released)
    }
}
