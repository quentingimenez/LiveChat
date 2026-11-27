-- Migration 0024 — distinguer un retrait de file d'un échec de lecture.
--
-- Retirer un job en attente le passe `FAILED` : c'est la seule transition
-- `PENDING → terminal` admise par la machine d'état (`JobStatus::can_transition_to`).
-- La base ne distinguait donc pas un média retiré par un modérateur d'un média
-- qui n'a pas pu jouer, et l'historique affichait « Échec » dans les deux cas.
--
-- `removed_at` (et son auteur) marque les lignes retirées ; une ligne `FAILED`
-- sans `removed_at` reste un vrai échec de lecture. `requeue` efface ces deux
-- colonnes : un job relancé repart de zéro.

ALTER TABLE playback_jobs ADD COLUMN removed_at INTEGER;

ALTER TABLE playback_jobs ADD COLUMN removed_by TEXT;
