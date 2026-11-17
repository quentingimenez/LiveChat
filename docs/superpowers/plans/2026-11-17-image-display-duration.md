# Per-image display duration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let members choose how long each image stays on the overlays: per board item, and per send from the extension or Discord.

**Architecture:** A nullable `display_duration_sec` column on `media_board_items`, edited through the existing item `PATCH`. Send-time durations flow through the existing `/ingest` `durationSec` field and a new Discord option. One duration rule (chosen → guild `defaultMediaTime` → `DEFAULT_DURATION`, capped by `maxMediaTime`) applies to images and text-only jobs only.

**Tech Stack:** Rust (axum, sqlx/SQLite, serenity glue), TypeScript extension (vitest), vanilla JS overlay web app (`node --test`).

**Spec:** `docs/superpowers/specs/2026-11-16-image-display-duration-design.md`

## Global Constraints

- Branch `feat/image-display-duration` from `main`.
- Duration: integer seconds, `1..=600`. Constants `MIN_DISPLAY_DURATION_SEC = 1`, `MAX_DISPLAY_DURATION_SEC = 600` in `lc-domain` (Rust); same values as JS constants in the extension and the overlay.
- The chosen duration applies to images and text-only jobs; video and audio keep their real length.
- `maxMediaTime` always caps the result.
- No backward compatibility: `/ingest` `durationSec` no longer changes a video/audio job.
- Admin web console (`apps/lc-admin-web`): untouched.
- UI copy in French; code, comments, commit messages in English; no `Co-Authored-By` line.
- Commit steps run only after the user has asked for commits (project rule).

## Review Focus

- An image item whose media was replaced by a video (editor "replace") keeps a stored duration: the trigger must ignore it and use the video length.
- `PATCH` with `displayDurationSec` as `"8"`, `8.5`, `0`, `601`, or on a video item: `400 invalid_payload`, nothing written.
- A chosen duration above the guild's `maxMediaTime`: the played job lasts `maxMediaTime`.
- `/ingest` `saveToBoard` of a media already on the board, with a duration: the existing item's duration is unchanged.
- Extension field left empty, only spaces, `abc`, `0`, or `601`: nothing sent for empty/spaces; the other cases block the send with a French message.

---

### Task 1: Domain constant, entity field and DB column

**Files:**
- Modify: `crates/lc-domain/src/entities.rs` (field on `MediaBoardItem`, ~l. 137) and add the two constants next to it
- Create: `crates/lc-db/migrations/0015_board_item_display_duration.sql`
- Modify: `crates/lc-db/src/repos/media_board.rs` (row struct, `into_entity`, `create`, new setter)
- Modify: every `MediaBoardItem { … }` literal (compile errors list them: `apps/lc-server/src/media/board.rs`, `http/overlay/media_board.rs`, `media_import.rs`, test fixtures in `apps/lc-server`, `crates/lc-db/tests/common/mod.rs`, `crates/lc-playback/tests/engine_scenarios.rs`) with `display_duration_sec: None`
- Test: `crates/lc-db/tests/media_board.rs`

**Interfaces:**
- Produces: `lc_domain::entities::{MIN_DISPLAY_DURATION_SEC, MAX_DISPLAY_DURATION_SEC}: i64`; `MediaBoardItem.display_duration_sec: Option<i64>` (`#[serde(default)]`); `MediaBoardRepository::set_display_duration(&self, id: MediaBoardItemId, seconds: Option<i64>) -> DbResult<bool>` (`false` when no row).

- [ ] **Step 1: Write the failing tests** in `crates/lc-db/tests/media_board.rs`:
  - `display_duration_round_trips`: create an item with `display_duration_sec: Some(8)`, `get` → `Some(8)`; `set_display_duration(id, None)` → `true`, `get` → `None`; `set_display_duration(id, Some(600))` → `get` → `Some(600)`.
  - `display_duration_check_rejects_out_of_range`: `set_display_duration(id, Some(0))` and `Some(601)` both return `Err`.
- [ ] **Step 2: Run** `cargo test -p lc-db --test media_board display_duration` → fails to compile (no field/method).
- [ ] **Step 3: Implement.** Migration body:
  ```sql
  -- Migration 0015 — display duration chosen for an image board item.
  ALTER TABLE media_board_items ADD COLUMN display_duration_sec INTEGER
      CHECK (display_duration_sec IS NULL OR display_duration_sec BETWEEN 1 AND 600);
  ```
  Bind the field in `create`; `set_display_duration` is a single `UPDATE … SET display_duration_sec = ? WHERE id = ?`. Fix all literals.
- [ ] **Step 4: Run** `cargo test -p lc-db` and `cargo build --workspace --all-targets` → all pass / build.
- [ ] **Step 5: Commit** `Add a display duration column to board items`.

### Task 2: Board item API and board trigger

**Files:**
- Modify: `apps/lc-server/src/http/overlay/media_board.rs` (`update_item_payload` ~l. 880–974; item JSON ~l. 1203 and the list row reader ~l. 1282–1346)
- Modify: `apps/lc-server/src/ws/session.rs` (board trigger, ~l. 897)
- Modify: `apps/lc-server/src/http/overlay/media_import.rs` (copy keeps the duration)
- Test: `apps/lc-server/src/http/overlay/tests.rs`, `apps/lc-server/src/ws/tests.rs`, `apps/lc-server/src/http/overlay/media_import_tests.rs`

**Interfaces:**
- Consumes: Task 1 field, constants and `set_display_duration`.
- Produces: item JSON field `displayDurationSec: number | null` (list, PATCH response); PATCH accepts `displayDurationSec`.

- [ ] **Step 1: Write the failing tests:**
  - `tests.rs::media_board_patch_sets_and_clears_display_duration`: image item; PATCH `{"displayDurationSec": 8}` → 200, response `item.displayDurationSec == 8`, list shows `8`; PATCH `{"displayDurationSec": null}` → list shows `null`. A body with only this field is accepted.
  - `tests.rs::media_board_patch_rejects_bad_display_duration`: for each of `"8"`, `8.5`, `0`, `601` → 400 `invalid_payload`, item unchanged; on a video item `{"displayDurationSec": 8}` → 400 `invalid_payload`.
  - `ws/tests.rs::board_image_trigger_uses_item_duration`: image item with `Some(8)` → enqueued job `duration_sec == 8`.
  - `ws/tests.rs::board_image_trigger_falls_back_to_guild_default`: item `None`, guild `default_media_time = Some(12)` → `12`; no guild default → runtime `DEFAULT_DURATION`.
  - `ws/tests.rs::board_image_trigger_is_capped_by_max_media_time`: item `Some(30)`, guild `max_media_time = Some(10)` → played duration (the `play` payload `durationSec`) is `10`.
  - `ws/tests.rs::board_video_trigger_ignores_stored_duration`: item with `Some(8)` whose asset is a 20 s video → job `duration_sec == 20`.
  - `media_import_tests.rs::import_keeps_display_duration`: source item `Some(9)` → copy `Some(9)`.
- [ ] **Step 2: Run** `cargo test -p lc-server display_duration board_image_trigger board_video_trigger import_keeps` → failures.
- [ ] **Step 3: Implement.**
  - PATCH: `displayDurationSec` present → must be JSON `null` or an integer (`as_i64`, not a float) within the constants; else 400 `{"error":"invalid_payload","message":"displayDurationSec must be an integer between 1 and 600, or null"}`. It counts in the "at least one field" check. Non-null on a non-image asset (read the asset after the item lookup) → 400 `invalid_payload`. Write with `set_display_duration` before the existing `update`.
  - Trigger: for `MediaKind::Image`, `duration_sec = item.display_duration_sec.unwrap_or(guild default)` where the guild default is `hub.guild_defaults(&guild_id).await.1` (already guild `defaultMediaTime` → runtime default). Other kinds unchanged. The cap stays in `dispatch_timing`.
  - Import: copy `display_duration_sec` onto the new item.
- [ ] **Step 4: Run** `cargo test -p lc-server` → all pass.
- [ ] **Step 5: Commit** `Set a display duration on board images and use it when they play`.

### Task 3: Ingest duration rule and saveToBoard

**Files:**
- Modify: `apps/lc-server/src/media/jobs.rs` (`resolve_duration`)
- Modify: `apps/lc-server/src/media/board.rs` (`add_to_board` gains a parameter)
- Modify: `apps/lc-server/src/http/ingest/submit.rs` (range check, pass duration to `add_to_board`)
- Modify: `apps/lc-server/src/discord/adapters.rs` (caller; pass `request.display_duration_sec` once Task 4 adds it — in this task pass `None`)
- Test: `apps/lc-server/src/media/jobs.rs` tests, `apps/lc-server/src/media/board.rs` tests, `apps/lc-server/src/http/overlay/ingestion_tests.rs` or the ingest submit tests module

**Interfaces:**
- Consumes: Task 1 constants and field.
- Produces: `add_to_board(db, config, guild_id, asset, title, message, created_by_user_id, created_by_name, display_duration_sec: Option<i64>, now)` — the duration is stored only when `asset.kind == MediaKind::Image` and the item is new.

- [ ] **Step 1: Write the failing tests:**
  - `jobs.rs::image_uses_explicit_then_guild_default`: image + `duration_sec: Some(7)` → `7`; image + `None` + guild `default_media_time = Some(12)` → `12` (today: runtime default — this is the fix).
  - `jobs.rs::video_ignores_explicit_duration`: 20 s video + `Some(5)` → `20`.
  - `jobs.rs::text_only_keeps_explicit_duration`: no media + `Some(9)` → `9`.
  - `board.rs::add_to_board_stores_duration_for_new_image_only`: new image + `Some(8)` → item `Some(8)`; second call same asset + `Some(20)` → `created == false`, item still `Some(8)`; new video + `Some(8)` → `None`.
  - ingest test `ingest_rejects_duration_above_max`: `durationSec: 601` → 400 `invalid_payload`.
  - ingest test `ingest_save_to_board_stores_image_duration`: image URL + `saveToBoard: true` + `durationSec: 8` → created item `displayDurationSec == 8`.
- [ ] **Step 2: Run** `cargo test -p lc-server jobs:: board:: ingest_` → failures.
- [ ] **Step 3: Implement.** `resolve_duration` order: video/audio → media length, else runtime default; image → explicit → guild `default_media_time` → runtime default; no media → explicit → guild default → runtime default; then the `max_media_time` cap and `max(1)` as today. In `submit.rs`, right after normalization, `duration_sec > MAX_DISPLAY_DURATION_SEC` → `invalid_payload(Some("durationSec must be between 1 and 600"))` (non-numbers stay ignored, as today). Update the module doc of `jobs.rs` to the new order.
- [ ] **Step 4: Run** `cargo test -p lc-server` → all pass.
- [ ] **Step 5: Commit** `Apply chosen durations to images only and keep them when saving to the board`.

### Task 4: Discord `durée` option

**Files:**
- Modify: `crates/lc-discord/src/i18n.rs` (new strings), `crates/lc-discord/src/catalog.rs` (`SEND_OPTIONS`, `HIDE_SEND_OPTIONS`, `MEDIA_ADD_OPTIONS`: 5th option), `crates/lc-discord/src/inputs.rs` (`MediaSubmission.duration_sec: Option<i64>`), `crates/lc-discord/src/serenity_glue.rs` (`integer_option`, read index 4), `crates/lc-discord/src/handlers.rs` (`handle_media_command`, `handle_media_add`, `effective_duration`), `crates/lc-discord/src/context.rs` (`MediaAddRequest.display_duration_sec: Option<i64>`), `crates/lc-discord/src/testing.rs`, `apps/lc-server/src/discord/adapters.rs` (pass it to `add_to_board`)
- Test: `crates/lc-discord/tests/commands.rs`, `crates/lc-discord/src/serenity_glue.rs` tests

**Interfaces:**
- Consumes: Task 3 `add_to_board` signature; Task 1 constants.
- Produces: option name `("durée", "duration")`, description `("Durée d'affichage d'une image, en secondes (1 à 600)", "Image display time in seconds (1 to 600)")`, kind `OptionKind::Integer`, not required, at index 4 of the three commands.

- [ ] **Step 1: Write the failing tests** (`tests/commands.rs`, using the existing fake context):
  - `send_image_with_duration_enqueues_it`: `/send` image + `duration_sec: Some(8)` → enqueued `duration_sec == 8`.
  - `send_image_without_duration_uses_guild_default`: guild `default_media_time = Some(12)` → `12`.
  - `send_video_ignores_duration`: 20 s video + `Some(8)` → `20`.
  - `send_rejects_out_of_range_duration`: `Some(0)` and `Some(601)` → error reply (ephemeral), nothing enqueued, no ingestion.
  - `media_add_passes_duration`: `/media add` + `Some(8)` → recorded `MediaAddRequest.display_duration_sec == Some(8)`.
  - glue test: the registered payload of `/send` has a 5th option of type `4` named `durée` (FR).
- [ ] **Step 2: Run** `cargo test -p lc-discord` → failures.
- [ ] **Step 3: Implement.** Range check before any ingestion, with a new i18n error string `send_invalid_duration`: `("La durée doit être comprise entre 1 et 600 secondes.", "The duration must be between 1 and 600 seconds.")`. Pass `submission.duration_sec` as `explicit` to `effective_duration`; reorder `effective_duration` like Task 3 (image: explicit → guild default → config default; video/audio: media length; explicit ignored for them). Discord-side `min_value`/`max_value` are not declared (`OptionSpec` has no such fields); the handler check is the guard.
- [ ] **Step 4: Run** `cargo test -p lc-discord` and `cargo test -p lc-server discord` → all pass.
- [ ] **Step 5: Commit** `Add a duration option to the Discord send and media add commands`.

### Task 5: Extension duration field

**Files:**
- Modify: `extension/src/lib/ingestClient.ts` (`ComposeIngestRequest.durationSec?: number`, `IngestPayload.durationSec?: number`, `buildIngestPayload`)
- Modify: `extension/src/lib/messages.ts` (`SendComposeRequestMessage.durationSec?: number`, validation in `isSendComposeRequest`; `ComposeStateResponse.durationSec: number | null`)
- Modify: `extension/src/background/index.ts` (draft and send pass-through)
- Modify: `extension/src/content/composeModalTwitter.ts`, `composeModalTiktok.ts`, `composeModalYoutube.ts`, `extension/src/popup/popup.ts`, `extension/popup.html`
- Create: `extension/src/lib/duration.ts` with `parseDisplayDuration(raw: string): { ok: true; value: number | null } | { ok: false }`
- Test: `extension/src/lib/duration.test.ts`, `extension/src/lib/ingestClient.test.ts`, `extension/src/lib/messages.test.ts`

**Interfaces:**
- Consumes: server `/ingest` `durationSec` (Task 3).
- Produces: field label `Durée d'affichage des images (s)`, `<input type="number" min="1" max="600" step="1">`, empty by default.

- [ ] **Step 1: Write the failing tests:**
  - `duration.test.ts`: `''` and `'  '` → `{ ok: true, value: null }`; `'8'` → `{ ok: true, value: 8 }`; `'600'` → `600`; `'0'`, `'601'`, `'8.5'`, `'abc'` → `{ ok: false }`.
  - `ingestClient.test.ts`: compose request with `durationSec: 8` → payload has `durationSec: 8`; without it → payload has no `durationSec` key.
  - `messages.test.ts`: `isSendComposeRequest` rejects `durationSec: '8'` and accepts `8` or absent.
- [ ] **Step 2: Run** `npm test` in `extension/` → failures.
- [ ] **Step 3: Implement.** Each modal and the popup read the field through `parseDisplayDuration`; `{ ok: false }` blocks the send and shows `La durée doit être un nombre entier entre 1 et 600.` in the modal's existing error area. The draft keeps the value like `text`.
- [ ] **Step 4: Run** `npm test`, `npm run typecheck`, `npm run lint`, `npm run build` in `extension/` → all pass.
- [ ] **Step 5: Commit** `Let the extension send a display duration for images`.

### Task 6: Overlay board UI

**Files:**
- Modify: `apps/lc-overlay/web/app/core/format.js` (add `parseDisplayDuration(raw)` with the same contract as Task 5)
- Modify: `apps/lc-overlay/web/app/views/board.js` (`openEditDialog` ~l. 1287–1515; card badge ~l. 378; detail drawer "Format" ~l. 1166)
- Modify: `apps/lc-overlay/dev/board-mock.js` (items carry `displayDurationSec`; PATCH mock applies it with the server's validation)
- Test: `apps/lc-overlay/dev/tests/format.test.mjs`

**Interfaces:**
- Consumes: item JSON `displayDurationSec` and PATCH field (Task 2).

- [ ] **Step 1: Write the failing test** in `format.test.mjs`: same cases as Task 5's `duration.test.ts` for `parseDisplayDuration`.
- [ ] **Step 2: Run** `node --test apps/lc-overlay/dev/tests/` → failure.
- [ ] **Step 3: Implement.**
  - Edit dialog, image items only: label `Durée d'affichage (s)`, number input (1–600), value `item.displayDurationSec ?? ''`, placeholder `Par défaut (N s)` with N from `activeGuild()?.settings?.defaultMediaTime` when known, else `Par défaut`. Empty → `displayDurationSec: null` (fits the dialog's "Un champ vide efface la valeur."). Invalid → `notify('La durée doit être un nombre entier entre 1 et 600.', true)` and no request. Only sent when the value changed.
  - Card badge and detail drawer: for an image with `displayDurationSec`, show `formatDuration(displayDurationSec)` where a video shows its length.
- [ ] **Step 4: Run** `node --test apps/lc-overlay/dev/tests/` → all pass. Open the dev board mock (`apps/lc-overlay/dev/README.md`), set 8 s on an image, reopen the dialog → value kept; clear it → badge gone.
- [ ] **Step 5: Commit** `Edit and show the display duration of board images`.

### Task 7: Whole-branch verification

- [ ] **Step 1: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` → pass.
- [ ] **Step 2: Run** `npm test`, `npm run typecheck`, `npm run lint` in `extension/`, and `node --test apps/lc-overlay/dev/tests/` → pass.
- [ ] **Step 3: Report** what was not verified end to end (real Discord command registration, real overlay playback on a running server).
