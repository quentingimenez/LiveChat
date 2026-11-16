-- Migration 0013 — overlay client information for the admin console.
--
-- `overlay_version` is the `appVersion` last sent in `hello`, `platform` the
-- operating system (`windows`, `macos`, `linux`). Both are NULL until a client
-- sends them, and the admin console exposes them as `overlayVersion` /
-- `platform`.
ALTER TABLE overlay_clients ADD COLUMN overlay_version TEXT;

ALTER TABLE overlay_clients ADD COLUMN platform TEXT;
