# Cross-guild media copy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Copy one or several board media from the active guild to another guild of the same server, through a context-menu entry and a new « Sélectionner » mode.

**Architecture:** A new server route `POST /overlay/media-board/import` is authenticated by the target guild's bearer token plus the source guild's token in `X-LiveChat-Source-Token`; it re-uses `complete_board_item` so the copy points to the same asset. A new Tauri command `board_import` sends both tokens (they never reach the web layer). The board gets a generic selection mode whose action bar hosts « Ajouter à une autre guilde… ».

**Tech Stack:** Rust (axum, sqlx via `lc-db`), Tauri 2 shell, vanilla JS web UI (`node --test` unit tests).

**Spec:** `docs/superpowers/specs/2026-11-16-cross-guild-media-copy-design.md`

## Global Constraints

- Code, comments, identifiers, commit messages in English; UI copy in French.
- Commit messages: one imperative line, no `Co-Authored-By`, no conventional-commit prefix required by the user (commitlint may require one: use `feat:`/`test:` if the hook refuses).
- Never commit or push without the user's go-ahead (the executor stops before each commit step and asks, unless the user said otherwise).
- Header name exactly `X-LiveChat-Source-Token`; route exactly `POST /overlay/media-board/import`; body `{"itemIds": [...]}`, 1 to 50 ids.
- Result statuses exactly `added | already | not_found | failed`.
- No backward compatibility for older overlays (user rule).
- Rust checks: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo nextest run --workspace --all-features --locked`.

## Review Focus

- Item id of a **third** guild (neither source nor target) passed in `itemIds` → `not_found`, never copied (Task 1 test `an_item_of_another_guild_is_not_found`).
- Source token revoked or of the same guild as the target → refused, nothing copied (Task 1 tests `invalid_source_token_is_refused`, `same_guild_is_refused`).
- Board quota reached half-way through a batch → earlier items `added`, the failing one `failed` with `media_ingestion_failed`, the next ones still tried (Task 1 test `quota_failure_does_not_stop_the_batch`).
- Selection holding ids deleted by another member meanwhile → dropped on board refresh, never sent (Task 3 test `pruneSelection drops ids no longer on the board`).
- Selection of more than 50 media → split into batches of 50, toast sums every batch (Task 3 tests `batches splits by 50`, `importSummary sums the results`).

---

### Task 1: Server route `POST /overlay/media-board/import`

**Files:**
- Modify: `apps/lc-server/src/http/overlay/auth.rs` (extract token authentication)
- Create: `apps/lc-server/src/http/overlay/media_import.rs`
- Create: `apps/lc-server/src/http/overlay/media_import_tests.rs`
- Modify: `apps/lc-server/src/http/overlay/mod.rs` (module, route next to `/overlay/media-board/items` ~l.122, route table in the module doc)
- Modify: `apps/lc-server/src/http/overlay/guild_tests.rs` (test helpers)

**Interfaces:**
- Produces (auth.rs): `pub(crate) async fn authenticate_token(db: &Db, token: &str, now: DateTime<Utc>) -> Result<AuthedOverlayClient, Response>` — the body of today's `from_request_parts` after the bearer is read (lookup by `hash_token`, `touch_last_seen`, member lookup); `from_request_parts` calls it. An unknown/revoked token returns `unauthorized()` as today.
- Produces (media_import.rs): `pub async fn import_items(State(state): State<AppState>, auth: AuthedOverlayClient, headers: HeaderMap, payload: Result<Json<ImportRequest>, JsonRejection>) -> Response`; `#[serde(rename_all = "camelCase")] pub struct ImportRequest { pub item_ids: Vec<String> }`; `pub const SOURCE_TOKEN_HEADER: &str = "x-livechat-source-token";`; `pub const MAX_IMPORT_ITEMS: usize = 50;`
- Consumes: `ensure_writable(&auth)`, `complete_board_item(&state, &auth, db, asset, title, message, now)` and `CompletedBoardItem`, `IngestionFailure { code, .. }` (media_board.rs); `assets::pin`, `db.media_board().set_artwork(..)`, `set_artwork_on_overlay(..)`; `state.admin_hub.broadcast_to_guild(..., ServerMessage::BoardChanged(BoardChangedPayload { change: BoardChange::Added, item_id }))`.
- Response JSON: `200 {"results":[{"sourceItemId":"…","status":"added","itemId":"…"}, {"sourceItemId":"…","status":"failed","error":"media_ingestion_failed","message":"…"}]}` — `itemId` present for `added` and `already`, `error` (and `message` when the failure has one) only for `failed`; results in request order after deduplication.
- Synchronous errors: `403 read_only_session` (via `ensure_writable`, before body), `403 forbidden` (no `Permission::AddMedia`), `400 invalid_payload` (bad JSON, 0 or >50 ids, an id that is not a UUID, same guild), `401 invalid_source_token` (header missing, empty or unknown/revoked).

Order in the handler: `ensure_writable` → `AddMedia` → parse body → source header → `authenticate_token` (map its rejection to `401 invalid_source_token`) → same-guild check → per item. Per item: `db.media_board().get(id)`; missing or `guild_id != source guild` → `not_found`; asset not `is_ready()` → `failed` / `media_not_ready`; otherwise `complete_board_item` with the source title/message; `created:false` → `already`; `Err(failure)` → `failed` with `failure.code` and `failure.message`; `created:true` → copy the source item's `artwork_asset_id` (pin it first; a pin or set failure is logged with `tracing::warn!`, the item stays `added`) and `artwork_on_overlay`, then broadcast `Added`.

- [ ] **Step 1: Add test helpers in `guild_tests.rs`**

`Harness::second_guild(&self, name: &str) -> GuildId` (creates a standalone guild in the same db), `Harness::invite_for(&self, guild_id: &GuildId, role: GuildRole) -> String` (`server_invite` becomes a call to it with `self.guild_id`), `Harness::call_with_headers(&self, method, uri, token: Option<&str>, headers: &[(&str, &str)], body: Option<Value>) -> (StatusCode, Value)` (`call_app` gains the headers slice; `call` passes `&[]`).

- [ ] **Step 2: Write the failing tests in `media_import_tests.rs`** (register with `#[cfg(test)] mod media_import_tests;` in `mod.rs`)

Fixtures: guild A = `harness.guild_id`, guild B = `second_guild("Les Thons")`; tokens from `join(invite_for(.., Member))`; assets/items built like `media_edit_tests.rs::asset` / `item`, with an `item_in(harness, guild_id, asset)` variant. Helper `import(harness, target_token, source_token: Option<&str>, ids: Value) -> (StatusCode, Value)`.

```rust
#[tokio::test] async fn copies_an_item_to_the_target_board()
// status 200; results[0] == {"sourceItemId": a_item.id, "status": "added", "itemId": <new id>}
// db.media_board().get_by_guild_and_asset(&b, asset.id) is Some, title "Clip", message "aïe",
// created_by_name == B member display name; B's broadcast recorder saw BoardChange::Added once.
#[tokio::test] async fn an_item_already_on_the_target_board_is_already()
// status "already", itemId == existing B item id, no broadcast.
#[tokio::test] async fn an_item_of_another_guild_is_not_found()
// item in a third guild C → status "not_found"; nothing created in B.
#[tokio::test] async fn invalid_source_token_is_refused()
// header missing → 401 {"error":"invalid_source_token"}; header "nope" → same.
#[tokio::test] async fn same_guild_is_refused()
// source token of B with target B → 400 invalid_payload.
#[tokio::test] async fn a_guest_target_is_refused_before_the_body()
// guest token of B, body "not json" → 403 read_only_session.
#[tokio::test] async fn a_guest_source_may_be_read()
// guest token of A as source → 200, status "added".
#[tokio::test] async fn more_than_fifty_ids_are_refused()
// 51 ids → 400 invalid_payload; [] → 400 invalid_payload.
#[tokio::test] async fn quota_failure_does_not_stop_the_batch()
// config media_board_max_total_mb so only 1 of 3 assets fits (test_config in guild_tests.rs;
// assets sized accordingly); results: added, failed{"error":"media_ingestion_failed"}, 3rd also tried.
#[tokio::test] async fn artwork_is_copied_with_the_item()
// source item with artwork_asset_id = image asset and artwork_on_overlay = true →
// new B item has the same artwork_asset_id and artwork_on_overlay == true.
```

For the broadcast assertion reuse the recorder pattern of `media_upload_tests.rs` (`change_count(guild_id, BoardChange::Added)`); if the harness router cannot take a recorder, assert through `GET /overlay/media-board/items` on B instead and drop the broadcast assertion (say so in the task report).

- [ ] **Step 3: Run them, expect failures**

Run: `cargo nextest run -p lc-server media_import`
Expected: compile error / 404 on the route.

- [ ] **Step 4: Extract `authenticate_token` in `auth.rs`, add `media_import.rs`, register the route `post(media_import::import_items)` at `/overlay/media-board/import` and its line in the route tables of `mod.rs` and `media_board.rs` docs**

- [ ] **Step 5: Run the tests and the crate suite**

Run: `cargo nextest run -p lc-server`
Expected: all pass, including existing auth tests.

- [ ] **Step 6: fmt + clippy** (commands in Global Constraints) — clean.

- [ ] **Step 7: Commit** (after the user's go-ahead) — `Add a route copying board items from another guild`

---

### Task 2: Overlay shell command `board_import`

**Files:**
- Modify: `apps/lc-overlay/src/net/api.rs` (extra header)
- Modify: `apps/lc-overlay/src/commands.rs` (command, allowed list ~l.70-97, tests module)
- Modify: `apps/lc-overlay/src/lib.rs:220` (register in `generate_handler!`)

**Interfaces:**
- Produces (api.rs): `pub async fn request_with_headers(membership: &Membership, method: &str, path: &str, body: Option<Value>, headers: &[(&str, &str)]) -> Result<ApiResponse, String>`; `request` becomes a call to it with `&[]`.
- Produces (commands.rs): `#[tauri::command] pub async fn board_import(window, app, state, payload: Value) -> Result<api::ApiResponse, String>`; payload `{"sourceGuildId": String, "targetGuildId": String, "itemIds": [String]}` parsed into `#[serde(rename_all = "camelCase", default)] struct BoardImportRequest { source_guild_id: Option<String>, target_guild_id: Option<String>, item_ids: Vec<String> }`.
- Produces: `fn same_server(a: &Membership, b: &Membership) -> bool` — compares `server_url` trimmed, without trailing `/`, case-insensitive.
- Consumes: `ensure_allowed(&window, "board_import")`, `state.config_snapshot().membership(&id)`, `reconnect_on_unauthorized(&app, &target_guild_id, &response)`.

Behavior: unknown guild → `Err("guilde inconnue")`; source == target → `Err("même guilde")`; `!same_server` → `Err("serveurs différents")`; else POST `/overlay/media-board/import` with target membership, body `{"itemIds": item_ids}` and header `("X-LiveChat-Source-Token", source.client_token)`. `board_import` is added to the **board window** allowed list only (same list as `guild_api`).

- [ ] **Step 1: Failing unit tests in `commands.rs` tests module**

```rust
#[test] fn same_server_ignores_case_and_trailing_slash()
// "https://LC.example.test/" vs "https://lc.example.test" → true
#[test] fn same_server_refuses_another_host()
// "https://a.test" vs "https://b.test" → false
```

- [ ] **Step 2: Run** `cargo nextest run -p lc-overlay same_server` — fails to compile.

- [ ] **Step 3: Implement `request_with_headers`, `same_server`, `board_import`, allowed-list entry, handler registration.**

- [ ] **Step 4: Run** `cargo nextest run -p lc-overlay` — all pass; then fmt + clippy clean.

- [ ] **Step 5: Commit** (after go-ahead) — `Add a shell command importing board items from another guild`

---

### Task 3: Web helpers `core/selection.js`

**Files:**
- Create: `apps/lc-overlay/web/app/core/selection.js`
- Create: `apps/lc-overlay/dev/tests/selection.test.mjs`

**Interfaces (all pure, exported):**
- `toggleId(selected: Set<string>, id: string) -> Set<string>` (new set).
- `selectRange(selected: Set<string>, orderedIds: string[], anchorId: string|null, id: string) -> Set<string>` — adds every id between anchor and id (inclusive) in `orderedIds`; no anchor or anchor not visible → behaves like adding `id`.
- `pruneSelection(selected: Set<string>, existingIds: Iterable<string>) -> Set<string>`.
- `importTargets(guilds, activeGuildId) -> guild[]` — other guilds with same `membership.serverUrl` (same normalization as Task 2: trim, no trailing `/`, lowercase), not `membership.guestMode`, not `membership.role === 'guest'`.
- `batches(ids: string[], size = 50) -> string[][]`.
- `importSummary(results: {status, error?}[], guildName: string) -> {message: string, isError: boolean}`.

Exact copy for `importSummary` (French, pluralised with `s` on `média`/`ajouté`):
- `« 1 média ajouté à Les Thons. »`, `« 3 médias ajoutés à Les Thons. »`
- appended parts, joined by ` · `: `1 déjà présent` / `2 déjà présents`, `1 introuvable` / `2 introuvables`, `1 impossible` / `2 impossibles`.
- nothing added: message starts `Aucun média ajouté à Les Thons` followed by ` : ` and the parts; `isError` true when nothing was added and at least one is `failed` or `not_found`.

- [ ] **Step 1: Failing tests** in `selection.test.mjs` (`node:test`, style of `board-filters.test.mjs`):
  `toggleId adds then removes`, `selectRange selects between anchor and id both ways`, `selectRange without anchor adds the id`, `pruneSelection drops ids no longer on the board`, `importTargets keeps other guilds on the same server only`, `importTargets skips guest memberships`, `batches splits by 50` (120 ids → lengths `[50, 50, 20]`), `importSummary sums the results` (2 added + 1 already + 1 failed → `« 2 médias ajoutés à Les Thons. · 1 déjà présent · 1 impossible »`, isError false), `importSummary with nothing added is an error`.

- [ ] **Step 2: Run** `node --test apps/lc-overlay/dev/tests/selection.test.mjs` — fails (module missing).

- [ ] **Step 3: Implement `core/selection.js`.**

- [ ] **Step 4: Run** `node --test apps/lc-overlay/dev/tests/` — all pass.

- [ ] **Step 5: Commit** (after go-ahead) — `Add board selection and import helpers`

---

### Task 4: Board UI — selection mode, action bar, guild picker, context menu

**Files:**
- Modify: `apps/lc-overlay/web/app/core/state.js` (state fields)
- Modify: `apps/lc-overlay/web/app/views/board.js` (toolbar ~l.478-515, `buildCard` ~l.340, `openCardMenu` ~l.61, keyboard handling, board refresh)
- Create: `apps/lc-overlay/web/app/views/import-media.js` (guild picker + import run)
- Modify: `apps/lc-overlay/web/app/styles/board.css` (checkbox, selected card, action bar)
- Modify: `apps/lc-overlay/dev/board-mock.js` (mock `board_import` so the mock board can exercise the flow)

**Interfaces:**
- State: `state.selecting: boolean`, `state.selection: Set<string>`, `state.selectionAnchor: string|null`.
- board.js exports `enterSelection()`, `leaveSelection()`; leaving clears selection and anchor. Leaving happens on Escape (when no dialog/menu is open), « Terminer », « Annuler », and active-guild change.
- import-media.js exports `openImportDialog(itemIds: string[])`: lists `importTargets(state.guilds, state.activeGuildId)` (icon via the rail's `guildFace` style + name), one choice, button « Ajouter »; runs `batches(itemIds)` sequentially through `invoke('board_import', { payload: { sourceGuildId, targetGuildId, itemIds } })` (via `core/ipc.js`), collects `body.results`, shows `importSummary` with `notify(message, isError)`; a non-ok response shows `describeError(body.error)`; success leaves selection mode.
- Consumes: Task 3 helpers; `openDialog` (`core/ui.js`), `notify` (`core/toast.js`), `can('addMedia')` is **not** required on the source (rights are the target's).

Behavior:
- Toolbar: « Sélectionner » button (`btn is-quiet`) before « Recharger », shown when the board has items; in the mode its label is « Terminer ».
- In the mode: click on a card's thumb toggles it (Shift+click → `selectRange` over `visibleItems()` ids, anchor = last clicked); double-click does not play; hover drawer is not opened; the play/fav buttons are hidden; each card shows a checkbox (`.card.is-selecting`, `.card.is-selected`).
- Action bar fixed at the bottom of the board: `N sélectionné(s)` · « Tout sélectionner » (visible ids) · « Ajouter à une autre guilde… » (disabled when selection empty or `importTargets` empty, `title` explains « Aucune autre guilde sur ce serveur ») · « Annuler ». The actions are built from an array `SELECTION_ACTIONS` so later bulk actions plug in.
- Context menu: « Ajouter à une autre guilde… » when `importTargets` is non-empty; in selection mode, on a selected card, it acts on the whole selection.
- On items reload / `board.changed`: `state.selection = pruneSelection(...)`.

- [ ] **Step 1: Implement state, board.js changes, import-media.js, CSS, mock.**

- [ ] **Step 2: Run** `node --test apps/lc-overlay/dev/tests/` — all pass (existing board tests unaffected).

- [ ] **Step 3: Verify in the mock board** (`apps/lc-overlay/dev/board-mock.html` in the built-in browser): enter mode, Shift-select a range, select all, open the picker, run the mocked import, see the toast, mode left; Escape leaves the mode; context-menu entry present. Screenshot as proof.

- [ ] **Step 4: Commit** (after go-ahead) — `Add a selection mode to the board and copy media to another guild`

---

### Final check

- [ ] `cargo fmt --all -- --check`, clippy, `cargo nextest run --workspace --all-features --locked`, `node --test apps/lc-overlay/dev/tests/` — all green.
- [ ] Not verifiable here: a real two-guild copy against a deployed server with the packaged overlay; state it in the report.
