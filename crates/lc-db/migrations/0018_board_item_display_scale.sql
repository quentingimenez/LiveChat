-- Migration 0018 — display scale chosen for an image board item.
--
--   * `media_board_items.display_scale_percent`: percentage of the image's
--     natural size shown on the overlays; NULL is 100 %, today's behaviour.
ALTER TABLE media_board_items ADD COLUMN display_scale_percent INTEGER
    CHECK (display_scale_percent IS NULL OR display_scale_percent BETWEEN 10 AND 1000);
