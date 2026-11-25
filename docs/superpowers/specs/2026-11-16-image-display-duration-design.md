# Per-image display duration — design

Date: 2026-11-16

## Intent

Today every image shown on the overlays stays up for the same time. A member
wants to choose how long each image stays on screen: per image saved on the
media board, and per send from the browser extension or Discord.

## Current state

- Board trigger (`apps/lc-server/src/ws/session.rs`, ~l. 897): an image gets
  `duration_sec = 0`; the engine then falls back to the server-wide
  `DEFAULT_DURATION` frozen at engine creation
  (`crates/lc-playback/src/payload.rs`, `dispatch_timing`). The guild's
  `defaultMediaTime` is ignored on this path (bug), `maxMediaTime` caps.
- `/ingest` (`apps/lc-server/src/http/ingest/body.rs`) already accepts an
  optional `durationSec`, applied by `media/jobs.rs::resolve_duration` to any
  kind of media. The extension never sends it. `saveToBoard` stores the item
  without playing it.
- Discord `/send` (and its hidden variant) and `/media add` have no duration
  option; `handlers.rs::effective_duration` already takes an `explicit`
  argument, always `None`.
- `media_board_items` has no duration column.

## Goals

- A board item that is an image can carry its own display duration, set and
  cleared from the board's edit panel.
- The extension's send dialog and Discord's `/send` and `/media add` accept an
  optional duration. It is used for that send, or stored on the board item
  when the send saves to the board.
- An image without a chosen duration uses the guild's `defaultMediaTime`, then
  the server `DEFAULT_DURATION` (fixes the board path).

## Non-goals

- Durations for video and audio: they keep their real length.
- The admin web console (`apps/lc-admin-web`): no display or edit of the new
  field.
- Fractional seconds.

## Anti-goals

- A chosen duration must never bypass the guild's `maxMediaTime`.
- A duration chosen in one guild must not leak to another guild that shares
  the same media asset.

## Approaches

- **A. Duration on the board item + optional duration at send time (chosen).**
  Covers both the board and one-off sends; stored per guild, so no leak.
- B. Duration on the media asset: assets are deduplicated and shared across
  guilds, so a change in one guild would apply to all. Rejected.
- C. Send-time duration only: board images would all stay on one duration,
  which misses the original request. Rejected.

## Design

### Data

- Migration `0015_board_item_display_duration.sql`:
  `ALTER TABLE media_board_items ADD COLUMN display_duration_sec INTEGER
  CHECK (display_duration_sec IS NULL OR display_duration_sec BETWEEN 1 AND 600);`
- `MediaBoardItem.display_duration_sec: Option<i64>` (`#[serde(default)]`),
  read and written by the `lc-db` media board repository.
- Valid range everywhere: integer seconds, 1 to 600 (builder's call on the
  constant name; one shared constant per crate is fine).

### Duration rule

The chosen duration applies to images and to text-only jobs. For video and
audio it is ignored (the media length wins). Order for an image:
chosen duration → guild `defaultMediaTime` → `DEFAULT_DURATION`, then capped
by `maxMediaTime`.

This changes `/ingest`: `durationSec` no longer shortens or extends a video or
an audio clip (no backward compatibility kept, per project rule).

### Server

- Board trigger (`ws/session.rs`): for an image item, resolve the job
  duration with the rule above (item value, then guild default, then runtime
  default) instead of `0`. Video/audio unchanged.
- `PATCH /overlay/media-board/items/{id}`: new field `displayDurationSec`
  (integer 1–600, or `null` to clear). Same right as editing the title.
  Rejected with `invalid_payload` when out of range, not an integer, or the
  item is not an image. It joins `title`/`message`/`artworkOnOverlay` in the
  "at least one field" check.
- Item JSON (`media_board.rs` list/update responses, `board.changed` consumers
  unaffected): add `displayDurationSec`.
- `/ingest`: `durationSec` validated to 1–600 (rounded up as today; out of
  range → `invalid_payload`). With `saveToBoard` and an image, it is stored on
  the new item; if the item already exists, its duration is left unchanged.
- Cross-guild import (`media_import.rs`): copy `display_duration_sec` to the
  copy.
- `jobs.rs::resolve_duration`: explicit duration ignored for video/audio.

### Discord (`crates/lc-discord`)

- New optional integer option `durée` / `duration` (min 1, max 600) on
  `/send`, its hidden variant, and `/media add`; FR/EN strings in `i18n.rs`,
  declared in `catalog.rs`.
- `/send`: passed as `explicit` to `effective_duration`, which ignores it for
  video/audio.
- `/media add`: stored on the new item when the media is an image.
- Given on a video or audio: ignored silently.

### Extension (`extension/`)

- Each compose dialog (Twitter, TikTok, YouTube) gets an optional number
  field "Durée d'affichage des images (s)", 1–600, empty by default.
- `ingestClient.ts` sends `durationSec` only when filled.
- The field is shown on the three dialogs (the media kind is unknown before
  ingestion).

### Overlay app (`apps/lc-overlay/web`)

- Edit panel of an image item: a "Durée d'affichage" seconds field with a
  placeholder showing the guild default, and a way to clear it (back to the
  default). Saved through the existing item `PATCH`.
- Image cards on the grid and in the detail drawer show the chosen duration
  when set (e.g. `8 s`), using the existing duration badge.
- Duration filter/sort: unchanged (images still have no media duration).
- `dev/board-mock.js` follows the new field.

## Tests

- `lc-db`: migration applies; field round-trips; CHECK rejects 0 and 601.
- Server: board trigger uses item duration, then guild default, then runtime
  default, capped by `maxMediaTime`; PATCH validation (range, null, non-image);
  ingest stores the duration with `saveToBoard`; ingest ignores `durationSec`
  for a video; cross-guild copy keeps it.
- Discord: option parsed and passed; ignored for video; `/media add` stores it.
- Extension: `ingestClient` sends `durationSec` only when set (existing test
  file).
- Overlay: the existing `node --test` suite stays green; the edit panel is
  checked in the dev board mock.

## Left to the builder

- Exact constant names, helper placement, label wording in the UI (French,
  short).

## Branch

New branch `feat/image-display-duration` from `main`.
