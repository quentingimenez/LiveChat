-- Migration 0022 — index des références aux médias (audit performance B-11).
--
-- `media_assets.cover_asset_id` (pochette embarquée d'un audio, 0009) et
-- `media_board_items.artwork_asset_id` (image affichée pour un item, 0009) sont
-- des clés étrangères vers `media_assets` que SQLite n'indexe pas.
--   * `guild_references_asset` cherche `cover_asset_id = ?` dans une
--     sous-requête : sans index, elle scanne `media_assets` ;
--   * le comptage de références de `replace_media` (`media_asset_id = ? OR
--     artwork_asset_id = ?`) et le `NOT EXISTS` de `purge_expired` scannent
--     `media_board_items` : l'index de 0005 sur `media_asset_id` ne couvre pas
--     le second terme du `OR` ;
--   * les actions de FK `ON DELETE SET NULL` des deux colonnes scannent la
--     table enfant à chaque suppression d'asset, ce que le plan de
--     `purge_expired` montre aussi.
--
-- `IF NOT EXISTS` rend la migration sans effet si un index de même nom existe
-- déjà ; sqlx enregistre de toute façon sa version, une seule fois.
CREATE INDEX IF NOT EXISTS idx_media_assets_cover_asset
    ON media_assets (cover_asset_id);

CREATE INDEX IF NOT EXISTS idx_media_board_items_artwork_asset
    ON media_board_items (artwork_asset_id);
