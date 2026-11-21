-- Migration 0019 — file de lecture FIFO, sans préemption, avec source.
--
-- La préemption (un média du board de priorité 100 coupait la lecture en
-- cours, qui reprenait ensuite à l'offset) a été retirée : deux overlays ou
-- deux médias ne doivent plus s'interrompre, la file est servie dans l'ordre
-- d'arrivée. Cela supprime `priority`, la chaîne `resumes_after_job_id` et le
-- snapshot `remaining_ms_snapshot` qui n'existaient que pour la reprise après
-- interruption.
--
-- `source` remplace le diagnostic implicite : chaque job indique son origine
-- (board, ingestion, overlay, Discord) pour les logs et la file exposée.
--
-- SQLite : les index qui mentionnent une colonne supprimée doivent être
-- retirés avant le DROP COLUMN.

DROP INDEX IF EXISTS idx_playback_jobs_guild_status_prio_sub;
DROP INDEX IF EXISTS idx_playback_jobs_guild_status_resume;

ALTER TABLE playback_jobs DROP COLUMN priority;
ALTER TABLE playback_jobs DROP COLUMN resumes_after_job_id;
ALTER TABLE playback_jobs DROP COLUMN remaining_ms_snapshot;

-- Les lignes antérieures n'ont pas d'origine connue : `UNKNOWN` plutôt qu'une
-- supposition.
ALTER TABLE playback_jobs ADD COLUMN source TEXT NOT NULL DEFAULT 'UNKNOWN';

CREATE INDEX idx_playback_jobs_guild_status_sub
    ON playback_jobs (guild_id, status, submission_date);
