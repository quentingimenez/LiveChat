-- Migration 0017 — real title of a media source.
--
--   * `media_assets.source_title`: title of the source publication (video
--     title, tweet text, file name of a direct link…), recorded when the
--     asset becomes READY. The board uses it as the default title of an
--     item added from a link when the member left the title empty.
ALTER TABLE media_assets ADD COLUMN source_title TEXT;
