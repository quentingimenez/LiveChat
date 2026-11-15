-- Migration 0009 — personal favorites and pictures for audio media.
--
--   * `media_assets.cover_asset_id`: the embedded cover of an audio file
--     (album art), stored as its own image asset;
--   * `media_board_items.artwork_asset_id`: picture shown for an audio item
--     on the board (the cover by default, or an image chosen by a member);
--     `artwork_on_overlay` also shows it on overlays while the audio plays;
--   * `media_board_favorites`: each member's favorites, per board item.
ALTER TABLE media_assets ADD COLUMN cover_asset_id TEXT
    REFERENCES media_assets (id) ON DELETE SET NULL;

ALTER TABLE media_board_items ADD COLUMN artwork_asset_id TEXT
    REFERENCES media_assets (id) ON DELETE SET NULL;
ALTER TABLE media_board_items ADD COLUMN artwork_on_overlay INTEGER NOT NULL DEFAULT 0
    CHECK (artwork_on_overlay IN (0, 1));

CREATE TABLE media_board_favorites (
    member_id  TEXT    NOT NULL REFERENCES guild_members (id) ON DELETE CASCADE,
    item_id    TEXT    NOT NULL REFERENCES media_board_items (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (member_id, item_id)
) STRICT;

CREATE INDEX idx_media_board_favorites_item ON media_board_favorites (item_id);
