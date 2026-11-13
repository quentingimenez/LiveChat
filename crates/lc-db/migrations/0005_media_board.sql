-- Migration 0005 — `meme_board_items` devient `media_board_items`.
--
-- Les migrations 0001..0004 sont checksummées par sqlx et restent inchangées :
-- le renommage de la table et de ses index se fait donc ici. Les index sont
-- recréés sous leur nom définitif (mêmes colonnes qu'en 0001).
ALTER TABLE meme_board_items RENAME TO media_board_items;

DROP INDEX idx_meme_board_items_guild_created;
DROP INDEX idx_meme_board_items_media_asset;

CREATE INDEX idx_media_board_items_guild_created
    ON media_board_items (guild_id, created_at);
CREATE INDEX idx_media_board_items_media_asset
    ON media_board_items (media_asset_id);
