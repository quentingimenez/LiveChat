-- Migration 0011 — per-guild media board quota.
--
-- `board_quota_mb` overrides `MEDIA_BOARD_MAX_TOTAL_MB` for one guild when it is
-- set; `NULL` keeps the server-wide limit. The column is purely additive and
-- existing rows keep the previous behaviour.

ALTER TABLE guilds ADD COLUMN board_quota_mb INTEGER
    CHECK (board_quota_mb IS NULL OR board_quota_mb > 0);
