-- Migration 0020 — ordre explicite de la file de lecture.
--
-- La file se vidait dans l'ordre d'arrivée (`submission_date ASC`, `id ASC`).
-- `queue_position` permet de réordonner les jobs en attente depuis l'overlay
-- et la console admin : l'ordre affiché et l'ordre de lecture sont désormais
-- `queue_position ASC`, puis `submission_date ASC`, `id ASC` pour départager
-- deux jobs de même position (créations concurrentes).
--
-- Les lignes existantes gardent la valeur par défaut 0 : leur ordre relatif
-- reste donc celui de `submission_date`, et un nouveau job reçoit
-- `MAX(queue_position) + 1` pour passer derrière ceux déjà en file.

ALTER TABLE playback_jobs ADD COLUMN queue_position INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_playback_jobs_guild_status_queue
    ON playback_jobs (guild_id, status, queue_position);
