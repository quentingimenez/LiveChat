-- Migration 0015 — display duration chosen for an image board item.
--
--   * `media_board_items.display_duration_sec`: seconds an image item stays
--     on the overlays when it plays; NULL uses the guild default.
ALTER TABLE media_board_items ADD COLUMN display_duration_sec INTEGER
    CHECK (display_duration_sec IS NULL OR display_duration_sec BETWEEN 1 AND 600);
