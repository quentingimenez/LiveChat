# Copy board media to another guild — design

Date: 2026-11-16

## Intent

A member who belongs to several guilds wants to take media from one guild's
board (source, A) and add it to another guild's board (target, B) without
pasting the link or uploading the file again. Two entry points:

- one media at a time, from the card's context menu;
- several at once, through a new multi-selection on the board.

## Goals

- Copy is instant: the new item in B points to the same `media_assets` row as
  the item in A (no download, no transcode). Works for link imports, uploads and
  editor results alike.
- The server proves both sides: the caller can read A's board and may add media
  to B.
- Clear per-item outcome: added, already on B's board, failed (quota, gone…).

## Non-goals

- Keeping boards in sync (later edits or deletions in A do not affect B).
- Copying favorites or keyboard shortcuts (personal, per guild).
- Copying between guilds hosted on different servers.
- Any Discord command for this.

## Anti-goals

- A client of B being able to copy an asset from a guild it has no token for.
- Overlay tokens reaching the web layer.

## Approaches considered

1. **One request carrying both tokens — chosen.** The Rust shell calls B's
   board with B's bearer token and A's token in a second header. The server
   authenticates both, checks the items belong to A and the B client may add
   media, then creates the items. One round trip, tokens stay in Rust.
2. Signed share ticket from A, then presented to B: two calls and a signature
   scheme for the same security as (1).
3. Re-import from the original link: fails for uploads and edited media and
   downloads everything again. Rejected.

## Design

### Server (`apps/lc-server`)

New route `POST /overlay/media-board/import` (module `media_board.rs` or a new
`media_import.rs`), authenticated by the usual bearer = **target** client.

- Header `X-LiveChat-Source-Token`: bearer token of the **source** overlay
  client. Authenticated with the same lookup as the bearer (not revoked).
- Body: `{"itemIds": ["<uuid>", ...]}`, 1 to 50 ids, deduplicated.
- Checks, in order:
  - target session not read-only (`403 read_only_session`), target client has
    `Permission::AddMedia` (`403` with the existing permission error code);
  - source token valid (`401 invalid_source_token`), source guild ≠ target
    guild (`400 invalid_payload`); the source client may be read-only (reading
    is enough);
  - each item must exist and belong to the source guild, else that item's
    result is `not_found` (no leak of other guilds' items).
- Per item: asset must be `READY`, then `media::board::add_to_board` on the
  target guild with the source item's title and message, author = the target
  caller (`board_author_name`, creator member recorded like `create_item`).
  When a new item is created, copy `artwork_asset_id` (pinned like the main
  asset) and `artwork_on_overlay`.
- Synchronous answer `200 {"results": [{"sourceItemId", "status", "itemId"?,
  "error"?}]}` with `status` ∈ `added | already | not_found | failed`
  (`failed` carries the error code, e.g. quota exceeded). Items are processed
  in order; a quota error on one item does not stop the others.
- For each `added`, broadcast `board.changed {change:"added"}` to B's overlays
  (same payload as today's ingestion path).

### Overlay shell (`apps/lc-overlay/src`)

New Tauri command `board_import` (allowed for the board window only, like
`guild_api`): payload `{sourceGuildId, targetGuildId, itemIds}`. It looks up both
memberships, refuses if their `server_url` differ, and sends the request above
with B's token as bearer and A's token in `X-LiveChat-Source-Token`. Returns the
same `ApiResponse` shape as `guild_api`; handles `401` on the target like
`reconnect_on_unauthorized`.

### Board UI (`apps/lc-overlay/web/app`)

- Context menu of a card: « Ajouter à une autre guilde… » when at least one
  other guild is eligible.
- Selection mode, a general board feature (not specific to the copy):
  - a « Sélectionner » button in the board toolbar (`board.js` toolbar, next
    to « Ajouter un média ») enters the mode; it becomes « Terminer »;
  - in the mode, a click on a card toggles it instead of opening/playing it,
    Shift+click selects a range in the visible order; each card shows a
    checkbox; « Tout sélectionner » selects the visible (filtered) cards;
  - an action bar at the bottom: « N sélectionnés » followed by the actions,
    then « Annuler ». First action: « Ajouter à une autre guilde… ». The bar
    is a list of actions so more can be added later (bulk favorites,
    bulk delete…) without reworking the mode;
  - Escape, « Terminer » or changing guild leaves the mode and clears the
    selection; a board refresh drops ids that no longer exist.
- Guild picker dialog: list of the other guilds (icon + name) on the same
  server; one choice, then « Ajouter ».
- Result toast: « 3 médias ajoutés à <guilde> », plus « 1 déjà présent » /
  « 1 impossible (quota) » when relevant.

## Builder's calls (not discussed, proposed)

- Author of the copy is the person who copies it, not the original author.
- Title, message and picture are copied; favorites and shortcuts are not.
- Guild picker lists every other guild on the same server; the server is the
  authority on rights, and a refusal shows as an error toast.
- Limit of 50 items per request; the UI splits a bigger selection into batches.
- Bulk favorites and other bulk actions are out of this change; the action bar
  is built to receive them next.

## Tests

- Server: route tests next to `media_board` tests — added / already /
  not_found (item of a third guild) / read-only target / missing or invalid
  source token / same guild / quota failure on one item / artwork copied /
  `board.changed` broadcast.
- Shell: unit test that differing `server_url` is refused.
- Web: `dev/tests` unit tests for range selection and result-toast wording.
