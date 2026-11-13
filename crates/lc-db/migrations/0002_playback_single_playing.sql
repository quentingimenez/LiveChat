-- Migration 0002 — invariant « un seul job PLAYING par guilde ».
--
-- AUD-A5-004 : l'unicité n'était garantie que par le SQL de claim ; l'appel
-- `set_status(id, Playing)` pouvait créer un second job PLAYING pour la même
-- guilde (double lecture). SQLite n'a pas de contrainte conditionnelle
-- déclarative, mais un index unique partiel fait respecter l'invariant par le
-- moteur, y compris pour les écritures qui ne passent pas par le claim.
--
-- Comportement après un arrêt brutal : un job resté PLAYING le reste (le
-- watchdog du moteur le libère après sa période de grâce) ; l'index empêche
-- simplement qu'un second job passe PLAYING pour la même guilde entre-temps.
-- Aucun « recovery » de masse n'est fait ici : la reprise est pilotée par le
-- moteur (watchdog + snapshots `playback.state`).
--
-- Avant de créer l'index, on normalise d'éventuels doublons hérités du legacy :
-- le job PLAYING le plus récent (started_at le plus grand, départage sur l'id)
-- est conservé ; les autres repassent PENDING pour être rejouables.

WITH ranked AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY guild_id
            ORDER BY COALESCE(started_at, -1) DESC, id DESC
        ) AS rank_in_guild
    FROM playback_jobs
    WHERE status = 'PLAYING'
)
UPDATE playback_jobs
SET status = 'PENDING',
    started_at = NULL,
    finished_at = NULL
WHERE id IN (SELECT id FROM ranked WHERE rank_in_guild > 1);

CREATE UNIQUE INDEX idx_playback_jobs_single_playing
    ON playback_jobs (guild_id)
    WHERE status = 'PLAYING';
