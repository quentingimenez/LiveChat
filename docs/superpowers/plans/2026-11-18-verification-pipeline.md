# Verification Pipeline Implementation Plan (v2)

> **For agentic workers:** this plan is executed by an orchestrator (GLM 5.3 through
> DeepSeek Harness, opencode account `moi`) that hands each task to one worker
> (`opencode-go/deepseek-v4.1-flash#max`). Read "Execution model" before anything else.
> Steps use checkbox (`- [ ]`) syntax. v2 (2026-11-18) replaces v1 after a review against
> the code: every name, path and command below was checked in the worktree.

**Goal:** make agent-generated code in this repo trustworthy by mapping the verification
landscape of Cindy Sridharan's workflow diagram
([x.com/copyconstruct/status/2118539184654938154](https://x.com/copyconstruct/status/2118539184654938154),
Nov 6 2026) onto concrete LiveChat commands and CI gates: `cargo xtask verify` (with mutation
testing of the changed lines), property-based tests, fuzz targets, nightly mutation scores,
HTTP/WS contract tests, a WebSocket load harness, a kill-and-recover test, a canary deploy
mode with a test Discord bot, one nightly workflow, and docs + a `verify` skill.

**Source caveat:** the diagram was reconstructed by OCR plus the tweet thread; uncertain
labels are marked (?) in the spec. The spec's mapping table is the authoritative scope.

**Architecture:** new verification code is plain cargo tests: the `rust.yml` matrix runs it
unchanged. Heavy runs (fuzz, mutants, load) live in one new workflow,
`.github/workflows/verification-nightly.yml`. No new server route, no protocol change, no new
runtime dependency. Canary deployment extends `scripts/release.ps1` + `ReleaseLib.psm1`.

**Tech Stack:** Rust (MSRV 1.90), proptest, cargo-fuzz (excluded `fuzz/` crate),
cargo-mutants 27, cargo-nextest, cargo-deny, tokio-tungstenite, Windows PowerShell 5.1 /
pwsh 7, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-18-verification-pipeline-design.md`.

## Status on 2026-11-18 (v2 written)

| Task | State |
|---|---|
| 0 worktree, spec, plan v1 | committed `632caa8` |
| 0b dependencies | done in the worktree by Claude, NOT committed (see Task 0b) |
| 1 `cargo xtask verify` | draft NOT committed: `xtask/src/verify.rs`, `xtask/src/main.rs` |
| 2 protocol property tests | committed `5823233`; Task 2b fixes two weak tests |
| 3 to 10 | to do |

v1 Tasks 10 (feature flags), 11 (`/metrics`), 12 (JSON logs) and 13 (overlay telemetry)
and the criterion benches are **dropped** (user decision 2026-11-18: no Prometheus; the
server already stores each overlay's `appVersion` and `platform` from HELLO; runtime
settings already exist at `/admin/api/runtime-settings`).

## Execution model

- Worktree (all commands run from here, in Windows PowerShell 5.1: no `&&`, `||`, `?:`,
  `??`): `C:\Users\Streamer\code\LiveChat\.agents\worktrees\verification-pipeline`,
  branch `feat/verification-pipeline`.
- One task = one worker run. The worker gets a brief that contains, copied verbatim: the
  task section, the "Global Constraints" section, the "Definition of done" section and the
  allowed-files list. The worker never sees this conversation.
- Workers never commit, push, install tools, delete files outside their task, or edit a
  file outside the task's allowed list.
- After a worker returns, the orchestrator itself re-runs every acceptance command of the
  task and reads `git status --short` + `git diff --stat`. A worker's "all green" is never
  enough. One retry with a corrected brief; if it fails again, stop and report to the user.
- Commits: the orchestrator commits each task once its acceptance passes, with the exact
  message given in the task (user approval of 2026-11-18 covers these commits). Never push,
  never open the PR: both wait for the user's explicit go.
- Parallel runs: at most 2 workers, only for the pairs marked "parallel-safe", each on its
  own files. Two workers compiling at once share `target/` and wait on its lock: expected.
- Adversarial reviews: Review A after Task 5, Review B after Task 9 (see their sections).
- Known traps on this machine:
  - set `TMP`/`TEMP` outside the worktree before launching a DSH worker, or its shell
    breaks and it codes blind;
  - after a DSH run, tests failing with "Accès refusé" in `%TEMP%` come from a
    low-integrity label: check `icacls` before touching code;
  - pwsh 7 is not installed locally: release tooling tests run under Windows PowerShell
    5.1 locally and under pwsh 7 in `release-check.yml`;
  - a condensed test count printed by a wrapper can be wrong: read the nextest
    `Summary` line itself.

## Global Constraints

- English code, comments, test names, commit messages. Commits follow
  `commitlint.config.js`: `type(scope): lowercase imperative subject`, header ≤ 100
  characters, no `Co-Authored-By` line, no "Generated with" line.
- Protocol v2 is FROZEN (`docs/rust/protocol-v2.md`): no change to any `lc-protocol` type,
  constant or function, no edit of the 38 fixtures in `crates/lc-protocol/tests/fixtures/`.
  A sabotage step may break `lc-protocol` temporarily; it is always reverted.
- Dependencies: only the ones Task 0b already declared (`proptest`, `tokio-tungstenite`,
  `futures-util`, `lc-domain`, `lc-protocol` for xtask; `libfuzzer-sys`, `serde_json` in
  `fuzz/`). If a task seems to need another one, stop and report: do not add it.
- Offline: every cargo command takes `--locked --offline` (or `cargo xtask verify
  --offline`); `cargo deny --offline check`. All crates are already in the local cache.
- MSRV 1.90: new code must compile with `cargo +1.90.0 check --all-targets` on the touched
  crates (toolchain 1.90.0 is installed).
- Workspace lints deny `unwrap_used`, `expect_used`, `panic`. New test files start with
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`, like
  `crates/lc-db/tests/invariants.rs:7`.
- Production code changes only where a task says so (Task 3: yt-dlp `--`; Task 9: release
  scripts; Task 2b: test module of `hub.rs`). If a new test fails on unmodified production
  code: do not change the production code, do not weaken the test. Stop the task and
  report the failing input (it is a finding for the user).
- No network, DNS, ffmpeg or yt-dlp in property tests, fuzz targets or mutation scopes.
- Byte-faithful edits: the working tree uses CRLF (git `autocrlf=true`); keep the UTF-8 BOM
  of the root `Cargo.toml`. Never rewrite a whole file to change a few lines.
- PowerShell code runs under Windows PowerShell 5.1 and pwsh 7 (no `&&`, `||`, `??`, `?:`,
  no `-AsHashtable`; `Join-Path` with two arguments; remote bash scripts ASCII only, LF).
- No live Unraid action of any kind. No secret in any file, brief or log.
- CI: only `.github/workflows/verification-nightly.yml` is new. `rust.yml`,
  `server-image.yml`, `overlay-tauri.yml`, `extension.yml`, `release.yml` and
  `release-check.yml` stay untouched. Actions are pinned by the SHAs already used in
  `rust.yml` (`actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4`,
  `dtolnay/rust-toolchain@b725eda525362e7cf34af9a7243cac1e3f01a7ec`,
  `Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2`,
  `taiki-e/install-action@cfdb7ed4ccc7c7dce37759eae89dc5674b469b39 # v2`) and
  `actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4`.

## Definition of done (every task)

1. `git status --short` lists only the task's allowed files.
2. Every acceptance command of the task gives the stated result.
3. The sabotage step was run: the named test failed with the sabotage, then passed after
   the revert, and `git diff` shows no trace of the sabotage.
4. `cargo xtask verify --offline` prints five `PASS` lines and exits 0 (from Task 1 on).
5. `cargo xtask verify --offline --mutants` was run (from Task 1 on). Every missed mutant in
   `crates/` or `apps/` is either killed by a new test or listed in the report with the
   reason it is equivalent. Missed mutants in `xtask/` are listed, not blocking.
6. The worker's report lists: files changed, each command with its exit code, the sabotage
   evidence (failing test name + assertion message), and open questions.

## Review Focus

1. A test that cannot fail verifies nothing: no test may check a copy of the code written
   inside the test; every sabotage breaks the product invariant, never the test harness.
2. Property, fuzz and mutation scopes stay pure: no DNS (`validate_target` is excluded), no
   network, no ffmpeg/yt-dlp process.
3. `fuzz/` stays outside the workspace: `cargo metadata` of the root never lists
   `livechat-fuzz`, and the msrv/clippy/deny jobs never compile it.
4. The canary never receives the production Discord token or cookies, never mounts the
   production data dir, and every failure path restarts the main container on its
   previous image.
5. Default behavior is unchanged: no new HTTP route, no protocol change, the only
   production change is the yt-dlp `--` separator.

---

### Task 0b: verification tooling dependencies (done, to commit)

Done by Claude on 2026-11-18 and verified, not committed. Files:

- `Cargo.toml`: `exclude = ["spikes", "fuzz"]`; `[workspace.dependencies]` gains
  `proptest = "1"` and `tokio-tungstenite = "0.29"`.
- `crates/lc-protocol/Cargo.toml`, `crates/lc-db/Cargo.toml`, `crates/lc-media/Cargo.toml`:
  dev-dependency `proptest` from the workspace.
- `apps/lc-server/Cargo.toml`: dev-dependency `tokio-tungstenite` from the workspace.
- `xtask/Cargo.toml`: dependencies `futures-util`, `lc-domain`, `lc-protocol`,
  `tokio-tungstenite` from the workspace.
- `Cargo.lock` (+6 lines, dependency edges only).
- `fuzz/Cargo.toml` (own `[workspace]`, `libfuzzer-sys 0.4` with `link_libfuzzer`
  re-exposed as the default feature), `fuzz/Cargo.lock`, three placeholder targets in
  `fuzz/fuzz_targets/` (`protocol_frame.rs`, `media_url.rs`, `html_parsers.rs`).
- `.gitignore`: `/fuzz/target/`, `/fuzz/artifacts/`, `/fuzz/corpus/`, `/fuzz/coverage/`.

Verified (all exit 0): `cargo check --workspace --all-targets --all-features --locked
--offline`; `cargo +1.90.0 check -p lc-protocol -p lc-db -p lc-media -p xtask -p lc-server
--all-targets --locked --offline`; `cargo nextest run -p lc-protocol --locked --offline`
(29 passed); `cargo deny check` and `cargo deny --offline check` (advisories, bans,
licenses, sources ok); `cargo check --manifest-path fuzz/Cargo.toml --locked --offline`
with and without `--no-default-features`.

- [ ] **Commit 1** (only the files above):
  `build: add verification tooling dependencies`
- [ ] **Commit 2** (`docs/superpowers/plans/2026-11-18-verification-pipeline.md` and
  `docs/superpowers/specs/2026-11-18-verification-pipeline-design.md`):
  `docs: revise the verification pipeline plan for glm and deepseek execution`

### Task 1: `cargo xtask verify`

**Files:** modify `xtask/src/verify.rs` and `xtask/src/main.rs` (uncommitted draft already
present: keep its structure, `Stage`, `Verdict`, one line per stage, summary line, exit code
= number of failed stages).

**Interfaces:**
- CLI: `cargo xtask verify [--offline] [--mutants]`. `--full` is removed. An unknown flag is
  a `CliError::UnknownArgument`, like the other subcommands.
- `pub struct Options { pub offline: bool, pub mutants: bool }`,
  `pub fn execute(options: Options) -> u8`.
- `pub(crate) fn stage_argv(stage: &str, offline: bool) -> Vec<&'static str>`: the cargo
  arguments of a stage, so they are unit-testable. Exact values (copied from `rust.yml`;
  `[--offline]` present only when `offline` is true):
  - `fmt`: `fmt --all -- --check`
  - `clippy`: `clippy --workspace --all-targets --all-features --locked [--offline] -- -D warnings`
  - `tests`: `nextest run --workspace --all-features --locked [--offline] --no-fail-fast`
  - `doctests`: `test --workspace --doc --locked [--offline]`
  - `deny`: `deny [--offline] check`
- A missing tool (`cargo nextest --version`, `cargo deny --version`,
  `cargo mutants --version` fails) makes its stage `FAIL (<tool> not installed)`. No
  fallback, no `SKIP`: remove `Verdict::Skip`.
- Stage `mutants` (only with `--mutants`):
  1. `git merge-base HEAD origin/main` gives `<base>`;
  2. `git diff <base> -- *.rs` (the pathspec is passed as an argument, no shell) is
     written to `target/verify/mutants.diff` (create the directory);
  3. empty diff → `PASS mutants (no Rust change since origin/main)`;
  4. otherwise run `cargo mutants --in-place --in-diff target/verify/mutants.diff
     --test-tool nextest --output target/verify`; exit 0 → `PASS`, anything else →
     `FAIL mutants (see target/verify/mutants.out/missed.txt)`.
  `--in-place` is required: the copy mode skips the gitignored `apps/lc-admin-web/dist`
  and `node_modules`, so `lc-server`'s build script would call npm. Never interrupt a
  `--mutants` run; if one is interrupted, `git diff` and restore the mutated file.
- `cargo xtask verify --help` (and `cargo xtask help`) list both flags.

**Steps:**
- [ ] **Step 1:** Unit tests first, in the existing test module of `xtask/src/main.rs` (or
  a new `#[cfg(test)] mod tests` in `verify.rs`):
  `verify_parses_offline_and_mutants` (`["verify", "--offline", "--mutants"]` → both true),
  `verify_rejects_unknown_flag` (`["verify", "--full"]` → `UnknownArgument`),
  `stage_argv_mirrors_rust_yml` (asserts the five exact lists above, offline and not).
  Run `cargo nextest run -p xtask --locked --offline` → the new tests fail.
- [ ] **Step 2:** Implement. Run `cargo nextest run -p xtask --locked --offline` → all pass.
- [ ] **Step 3:** `cargo xtask verify --offline` → `PASS fmt`, `PASS clippy`, `PASS tests`,
  `PASS doctests`, `PASS deny`, `verify: 5 passed, 0 failed`, `$LASTEXITCODE` = 0.
- [ ] **Step 4:** Sabotage: append the line `fn  sabotage ( ) { }` to `xtask/src/verify.rs`;
  `cargo xtask verify --offline` → `FAIL fmt`, exit code ≥ 1. Revert.
- [ ] **Step 5:** `cargo xtask verify --offline --mutants`; put the missed-mutant list in
  the report (xtask: not blocking).
- [ ] **Step 6:** Commit `feat(xtask): add a verify subcommand mirroring the ci battery`.

### Task 2b: fix the two weak protocol properties

Task 2 (`5823233`) is good except two tests that cannot fail:
`close_code_reason_total` checks `close_reason_for_code`, a copy of the server mapping
written inside the test file, and `media_signature_input_deterministic` checks that
`format!` is deterministic.

**Files:** modify `crates/lc-protocol/tests/properties.rs` and the test module of
`apps/lc-server/src/ws/hub.rs` (`#[cfg(test)] mod tests` at line 1130). No other change.

**Steps:**
- [ ] **Step 1:** In `properties.rs`, delete `close_reason_for_code`, `close_code_values`
  and `close_code_reason_total` (and the doc comment that describes them).
- [ ] **Step 2:** In `hub.rs` tests, add `close_reason_for_code_is_total_and_non_empty`
  (loop over every `code` in `0..=u16::MAX`: `!close_reason_for_code(code).is_empty()`) and
  `documented_close_codes_map_to_their_reason` (the eight pairs: `NORMAL` →
  `CLIENT_SHUTDOWN`, `GOING_AWAY` → `SERVER_SHUTDOWN`, `UNAUTHORIZED` → `INVALID_TOKEN`,
  `REVOKED` → `REVOKED`, `OUTDATED_PROTOCOL` → `OUTDATED_PROTOCOL`, `GUILD_DELETED` →
  `GUILD_DELETED`, `FRAME_TOO_LARGE` → `FRAME_TOO_LARGE`, `RATE_LIMITED` → `RATE_LIMITED`;
  any other code → `INTERNAL_ERROR`). Check these pairs against the real function at
  `hub.rs:1024` first; if they differ, the function wins: report the difference.
- [ ] **Step 3:** Replace `media_signature_input_deterministic` with
  `media_signature_input_is_injective_without_separator`: two generated tuples
  `(asset_id, client_id, exp)`, ids from `"[a-z0-9-]{1,40}"`, `exp` any `u64`; if the tuples
  differ, the two inputs differ. Add a plain `#[test]`
  `separator_in_ids_makes_inputs_ambiguous` asserting that `("a|b", "c", 1)` and
  `("a", "b|c", 1)` give the same input (documents why ids must never contain `|`; frozen,
  not fixed).
- [ ] **Step 4:** Run `cargo nextest run -p lc-protocol -p lc-server --locked --offline` →
  all pass.
- [ ] **Step 5:** Sabotage A: make one arm of `close_reason_for_code` return `""` → the
  total test fails; revert. Sabotage B: change `media_signature_input` to
  `format!("{asset_id}{client_id}{exp_sec}")` → the injectivity property fails; revert
  (frozen crate: the revert is mandatory, check `git diff crates/lc-protocol/src`).
- [ ] **Step 6:** Commit `test: replace protocol properties that could not fail`.

### Task 3: properties for the SSRF guard, yt-dlp arguments and the job queue

Parallel-safe with Task 9.

**Files:** create `crates/lc-media/tests/properties.rs`, `crates/lc-db/tests/properties.rs`;
modify `crates/lc-media/src/download.rs` (one production line + its unit tests).

**Interfaces (all real, checked):**
- `lc_media::guard::ip_block_reason(ip: IpAddr) -> Option<&'static str>` (pure),
  `lc_media::guard::parse_media_url(raw: &str) -> MediaResult<Url>` (pure),
  `lc_media::guard::resolve_redirect(current: &Url, location: &str) -> MediaResult<Url>`
  (pure). Never call `validate_target` (async, DNS).
- `lc_media::download::build_ytdlp_args(config: &MediaConfig, url: &str, temp_dir: &Path,
  options: &YtdlpOptions) -> Vec<String>`; `MediaConfig::default()` exists; build
  `YtdlpOptions` like the unit tests at `download.rs:1057-1090`. If `YtdlpOptions` cannot be
  built outside the crate, put that property in the `#[cfg(test)]` module of
  `download.rs` instead (proptest is a dev-dependency of lc-media).
- `lc_db` playback jobs: `db.playback_jobs().create(&job)`, `.claim_next(now)`,
  `.set_status(id, status)` (setting `Playing` while the guild already has one returns
  `Err(DbError::Domain(_))`), `.finish(id, JobStatus::Done | JobStatus::Failed, now)`.
  Helpers in `crates/lc-db/tests/common/mod.rs`: `memory_db()`, `guild(&str)`,
  `sample_job(&GuildId, now)`, `now()`. Include them with `mod common;`.
- `url::Host` is usable from lc-media integration tests (`url` is a dependency).

**Properties (exact names):**
- `blocked_ipv4_ranges_are_always_rejected`: addresses generated inside each of
  `0.0.0.0/8`, `10.0.0.0/8`, `100.64.0.0/10`, `127.0.0.0/8`, `169.254.0.0/16`,
  `172.16.0.0/12`, `192.0.2.0/24`, `192.168.0.0/16`, `198.18.0.0/15`, `198.51.100.0/24`,
  `203.0.113.0/24`, `224.0.0.0/4`, `240.0.0.0/4` → `ip_block_reason(V4(ip)).is_some()`.
- `blocked_ipv6_ranges_are_always_rejected`: `::1`, `::`, `fc00::/7`, `fe80::/10`,
  `ff00::/8`, `2001:db8::/32` → `is_some()`.
- `ipv4_mapped_ipv6_has_the_ipv4_verdict`: for any `Ipv4Addr v`,
  `ip_block_reason(V4(v)).is_some() == ip_block_reason(V6(v.to_ipv6_mapped())).is_some()`.
- `encoded_blocked_ipv4_hosts_parse_to_the_blocked_ip`: for a blocked IPv4 from the
  generator above, each encoding (dotted decimal, one decimal `u32`, `0x` hex `u32`, dotted
  octal with a leading `0` per octet, dotted `0x` hex) in `http://{enc}/x`:
  `parse_media_url` is `Ok(url)`, `url.host() == Some(Host::Ipv4(ip))`, and
  `ip_block_reason(V4(ip)).is_some()`.
- `parse_media_url_never_panics_and_only_accepts_http`: any string (`any::<String>()` and
  `"\\PC{0,200}"`); `Ok(url)` implies scheme `http` or `https`, a host, empty username, no
  password.
- `redirects_stay_http_or_https`: `resolve_redirect(&Url::parse("https://example.com/a/b")?,
  location)` for any `location`; `Ok(next)` implies scheme `http` or `https`.
- `ytdlp_url_comes_last_after_double_dash`: for any url string (including strings starting
  with `-`), the last argument is the url and the one before is `--`.
- `at_most_one_playing_job_per_guild` (lc-db): sequences of 1 to 40 operations over two
  guilds, each `Create { guild }`, `ClaimNext`, `SetStatus { pick, Pending | Playing }`,
  `Finish { pick, Done | Failed }` (`pick` indexes the ids created so far; skip when none).
  After each operation, `SELECT guild_id, COUNT(*) FROM playback_jobs WHERE
  status = 'PLAYING' GROUP BY guild_id` returns only counts ≤ 1. The only accepted error is
  `DbError::Domain(_)` from `set_status(_, Playing)`; any other error fails the case. Each
  case builds a current-thread tokio runtime (`Builder::new_current_thread().enable_all()`)
  and a fresh `memory_db()`; `ProptestConfig { cases: 64, .. }`.

**Steps:**
- [ ] **Step 1:** Write all properties. Run `cargo nextest run -p lc-media -p lc-db --locked
  --offline` → every property passes except `ytdlp_url_comes_last_after_double_dash`
  (today the url is pushed without `--`, `download.rs:448`).
- [ ] **Step 2:** Production change: in `build_ytdlp_args`, push `"--"` right before the url.
  The cookie insertion in `download_with_ytdlp` (`download.rs:595`) inserts before `-o`, so
  it stays before `--`. Update the unit tests of `download.rs` that compare whole argument
  vectors. Run the same command → all pass.
- [ ] **Step 3 (optional evidence):** if `yt-dlp` is on PATH:
  `yt-dlp --ignore-config --simulate --print id -- https://www.youtube.com/watch?v=jNQXAC9IVRw`
  prints `jNQXAC9IVRw`. Otherwise report "`--` not checked against a real yt-dlp".
- [ ] **Step 4:** Sabotage A: delete the CGNAT branch of `ipv4_block_reason` →
  `blocked_ipv4_ranges_are_always_rejected` fails on a `100.64.x.x` address; revert.
  Sabotage B: run `sqlx::query("DROP INDEX idx_playback_jobs_single_playing")` right after
  `memory_db()` in the property → it fails with a shrunk case of two `Playing`; revert.
- [ ] **Step 5:** `cargo +1.90.0 check -p lc-media -p lc-db --all-targets --locked
  --offline` → exit 0. Definition of done.
- [ ] **Step 6:** Commit
  `test: add properties for the ssrf guard, yt-dlp arguments and job queue`.

### Task 4: fuzz targets and the nightly workflow

**Files:** create `fuzz/src/lib.rs`, `.github/workflows/verification-nightly.yml`; replace
the bodies of `fuzz/fuzz_targets/protocol_frame.rs`, `media_url.rs`, `html_parsers.rs`.

**Interfaces:**
- `fuzz/src/lib.rs` (crate `livechat_fuzz`) exposes `pub fn protocol_frame(data: &[u8])`,
  `pub fn media_url(data: &[u8])`, `pub fn html_parsers(data: &[u8])`. Each target body is
  one call: `fuzz_target!(|data: &[u8]| { livechat_fuzz::protocol_frame(data); });`.
- `protocol_frame`: if `data` is UTF-8, for `M` in `ClientMessage` and `ServerMessage`: if
  `serde_json::from_str::<Envelope<M>>(text)` is `Ok(envelope)`, then
  `v1 = serde_json::to_value(&envelope)`, `back: Envelope<M> = serde_json::from_value(v1)`
  must be `Ok`, and `serde_json::to_value(&back) == v1` (`assert_eq!`).
- `media_url`: if UTF-8: `parse_media_url(text)`; on `Ok(url)`, call
  `resolve_redirect(&url, text)`, and `Ok(next)` must have scheme `http` or `https`.
- `html_parsers`: `let s = String::from_utf8_lossy(data);` then call, ignoring results:
  `lc_media::resolve::tiktok::parse_page_html(&s, "https://www.tiktok.com/@a/video/1")`,
  `tiktok::photo_slides_from_raw_html(&s)`, `tiktok::extract_item_id(&s)`,
  `lc_media::resolve::twitter::sanitize_card_html(&s)`, `lc_media::html::attributes(&s)`,
  `html::meta_content(&s, "og:video")`, `html::script_json(&s, "__NEXT_DATA__")`,
  `html::find_ci(&s, "<script", n)` with `n = data.first().copied().unwrap_or(0) as usize`;
  then if `serde_json::from_str::<serde_json::Value>(&s)` is `Ok(v)`:
  `twitter::tweet_card_content(&v)`, `twitter::collect_variants(&v)`,
  `twitter::collect_photos(&v)`. These parse untrusted third-party HTML/JSON with byte
  indices: a panic on a char boundary is the bug class we look for.
- `fuzz/src/lib.rs` has a `#[cfg(test)] mod tests` that feeds every seed through the three
  functions: the 38 files of `crates/lc-protocol/tests/fixtures/` into `protocol_frame`, the
  files of `crates/lc-media/tests/fixtures/` (`tiktok_video.html`, `tiktok_photo.html`,
  `twitter_syndication*.json`, …) into `html_parsers`, and the urls
  `https://www.tiktok.com/@a/video/1`, `http://0x7f.1/`, `http://[::ffff:127.0.0.1]/`,
  `https://youtu.be/abc` into `media_url`. Locally:
  `cargo test --manifest-path fuzz/Cargo.toml --no-default-features --lib --locked --offline`.
- Workflow `verification-nightly.yml`: `name: verification-nightly`; triggers `schedule`
  (`cron: '30 3 * * *'`), `workflow_dispatch`, and `pull_request` with `paths:
  [".github/workflows/verification-nightly.yml", "fuzz/**", "scripts/mutants-baseline.json"]`
  (`schedule`/`workflow_dispatch` only run once the file is on `main`; the PR trigger runs it
  on the PR). `permissions: contents: read`; `concurrency: group:
  verification-nightly-${{ github.ref }}`, `cancel-in-progress: true`.
  Job `fuzz`: `ubuntu-latest`, `timeout-minutes: 30`, `strategy.fail-fast: false`, matrix
  `target: [protocol_frame, media_url, html_parsers]`; steps: checkout; toolchain
  `nightly`; rust-cache with `workspaces: fuzz -> target`; install `cargo-fuzz` with
  taiki-e (fallback if the tool is unknown: `cargo install cargo-fuzz --locked`); seed
  `fuzz/corpus/<target>/` (copy the fixtures above; write the four urls as one file each);
  `cargo fuzz run ${{ matrix.target }} -- -max_total_time=45`; on failure upload
  `fuzz/artifacts/${{ matrix.target }}/`.

**Steps:**
- [ ] **Step 1:** Write `fuzz/src/lib.rs` with its seed test. Run the local `cargo test`
  above → pass.
- [ ] **Step 2:** Sabotage: in `protocol_frame`, compare against `v1` with its `"id"` field
  replaced by `"x"` → the seed test fails on a fixture; revert.
- [ ] **Step 3:** Wire the three targets; `cargo check --manifest-path fuzz/Cargo.toml
  --locked --offline` → exit 0.
- [ ] **Step 4:** Write the workflow. If `actionlint` is on PATH, run it on the file;
  otherwise report "workflow not validated locally".
- [ ] **Step 5:** Workspace isolation: `cargo metadata --no-deps --format-version 1
  --offline` does not contain `livechat-fuzz`. Definition of done.
- [ ] **Step 6:** Commit
  `test(fuzz): add fuzz targets for protocol frames, media urls and html parsers`.

### Task 5: nightly mutation scores

**Files:** create `scripts/mutants-baseline.json`; modify
`.github/workflows/verification-nightly.yml` (job `mutants`).

**Interfaces:**
- Scopes:
  - `lc-protocol`: `cargo mutants -p lc-protocol --test-tool nextest -j 2 --output
    target/mutants/lc-protocol`;
  - `lc-media-guard`: `cargo mutants -p lc-media --file crates/lc-media/src/guard.rs
    --test-tool nextest -j 2 --output target/mutants/lc-media-guard -- -E 'kind(lib) |
    binary(ssrf_guard) | binary(properties)'` (the filter keeps the ffmpeg and network
    test binaries out; if cargo-mutants rejects the `--` arguments, report it and use
    `--cargo-test-arg` instead).
- Score: from `<output>/mutants.out/outcomes.json`,
  `caught_pct = floor(100 * caught / (caught + missed + timeout))` (`unviable` excluded).
  Check the key names in the produced file first and report them.
- Baseline file: `{ "lc-protocol": { "caught_pct": <measured - 1> }, "lc-media-guard":
  { "caught_pct": <measured - 1> } }`.
- Job `mutants`: `ubuntu-latest`, `timeout-minutes: 60`; checkout, stable toolchain,
  rust-cache, install `cargo-mutants,nextest` with taiki-e; run both scopes; compare each
  score with `jq` against the baseline (fail if `caught_pct < baseline`); upload both
  `mutants.out` directories (always). Advisory: a red nightly is not a merge gate.

**Steps:**
- [ ] **Step 1:** Measure both scopes locally (long: run in the background, do not
  interrupt). Read the scores with PowerShell
  (`(Get-Content <file> -Raw | ConvertFrom-Json)`), write the baseline.
- [ ] **Step 2:** List every missed mutant of `guard.rs` in the report (they are SSRF
  guard branches no test checks: candidates for Task 3 follow-up tests).
- [ ] **Step 3:** Add the job. Definition of done (no sabotage: the score threshold is the
  check; report the measured numbers instead).
- [ ] **Step 4:** Commit `ci: add nightly mutation testing with per-scope score baselines`.

### Review A (after Task 5)

A separate worker run, read-only. Brief: read `git log --oneline origin/main..HEAD` and
`git diff origin/main...HEAD`; report, with `file:line`, (1) tests that cannot fail or test
a copy of the code, (2) sabotage steps that broke the harness instead of the product,
(3) DNS, network or process use in properties, fuzz or mutation scopes, (4) files touched
outside the allowed lists, any fixture or `protocol-v2.md` edit, (5) properties whose
generators avoid the interesting inputs. For each finding, propose the test that would
catch it. The orchestrator fixes confirmed findings (commit
`test: address verification review findings`) or lists them for the user.

### Task 6: HTTP contracts and the WebSocket version matrix

Parallel-safe with Task 8 (different files; both compile lc-server).

**Files:** create `apps/lc-server/tests/contracts.rs`; modify the test file
`apps/lc-server/src/ws/tests.rs` (add one test, no helper change).

**Interfaces:**
- Test server for `contracts.rs`: `Config::from_pairs([("API_URL", "http://127.0.0.1:3000"),
  ("NODE_ENV", "test")])` (pairs of `String`), `Db::open(dir.join("contract.sqlite"),
  &DbConfig::default())`, `AppState::new(Arc::new(config), Some(db), Shutdown::new().0)`,
  `let app = lc_server::router(state);`, served on `127.0.0.1:0` with
  `axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())`
  (as `apps/lc-server/src/lib.rs:248`), queried with `reqwest`.
- `/health` contract (`apps/lc-server/src/http/health.rs:107`): HTTP 200; the body's key
  set is exactly `{"status", "version", "protocolVersion", "db", "discord", "uptimeSec",
  "activeJobs"}`; `status` ∈ {`ok`, `degraded`}; `version == env!("CARGO_PKG_VERSION")`;
  `protocolVersion == 2`; `db == "ok"`; `discord == "disabled"` (no Discord config);
  `uptimeSec` is an integer ≥ 0; `activeJobs == 0`.
- Without a database (`None`): `db == "disabled"`, `activeJobs` is `null`, `status == "ok"`.
- Anonymous `GET /admin/api/overview` → 401; the exact JSON body is recorded once from a
  real run and pinned as a literal.
- WS matrix in `ws/tests.rs`: `hello_version_matrix_against_min_3_0_0`, with
  `start_server()` (its `MIN_OVERLAY_VERSION` is `3.0.0`), `seed_guild`,
  `seed_overlay_client(..., TOKEN, OverlaySessionMode::Normal)`, `connect`, `send_hello`.
  Cases: `2.9.9`, `3`, `garbage` → `expect_close(&mut socket, close_code::OUTDATED_PROTOCOL)`;
  `3.0.0`, `3.0.0-beta.1` (frozen rule: suffixes ignored), `3.8.0`, `10.0.0` (numeric, not
  string comparison) → the first text frame has `"type": "welcome"`. One connection per
  case.

**Steps:**
- [ ] **Step 1:** Write `contracts.rs` (record the 401 body, then pin it) and the matrix.
  Run `cargo nextest run -p lc-server --locked --offline` → all pass.
- [ ] **Step 2:** Sabotage A: rename `uptimeSec` to `uptime` in the serde attribute of
  `health.rs` → the key-set contract fails; revert. Sabotage B: negate the
  `is_version_below` condition at `ws/session.rs:195` → the matrix fails; revert.
- [ ] **Step 3:** Definition of done. Commit
  `test(server): add http contracts and a websocket version matrix`.

### Task 7: WebSocket load harness

**Files:** create `xtask/src/loadtest.rs`; modify `xtask/src/main.rs` (subcommand) and
`.github/workflows/verification-nightly.yml` (job `load`).

**Interfaces:**
- `cargo xtask loadtest [--server-bin <path>] [--clients 50] [--duration-secs 60]
  [--ingest-rate 20] [--max-hello-p95-ms 250] [--report <path>]`. Default server binary:
  `target/debug/lc-server` + `std::env::consts::EXE_SUFFIX`.
- Setup: temp dir; seed `<tmp>/livechat.sqlite` with `lc_db::Db::open` before starting the
  server: one guild (as `seed_guild`, `ws/tests.rs:312`), `--clients` overlay clients with
  tokens `load-overlay-{i:03}` (as `seed_overlay_client`, `ws/tests.rs:326`), one guild
  member + one ingest client with token `load-ingest-token` (as `insert_client`,
  `apps/lc-server/src/http/ingest/submit.rs:351`). Token hashes are lowercase hex SHA-256
  (`format!("{:x}", Sha256::digest(token))`), like `hash_token` in the server.
- Server process env: `PORT` (a free port: bind `127.0.0.1:0`, read it, drop the listener),
  `API_URL=http://127.0.0.1:<port>`, `DATABASE_URL=file:<tmp>/livechat.sqlite`,
  `MEDIA_STORAGE_DIR=<tmp>/media`, `NODE_ENV=development` (loopback is exempt from the rate
  limiter, `apps/lc-server/src/http/rate_limit.rs`), `RUST_LOG=warn`. Wait for `/health`
  `db == "ok"` (30 s, else exit 1).
- Load: each client connects to `ws://127.0.0.1:<port>/ws`, sends HELLO
  (`{"token", "appVersion": "3.0.0", "platform": "windows"}`, envelope as `envelope()` in
  `ws/tests.rs:144`, `v` = `lc_protocol::PROTOCOL_VERSION`), measures the time to the first
  `welcome` frame, then sends a `heartbeat` every `lc_protocol::HEARTBEAT_INTERVAL_MS`
  (envelope as `ws/tests.rs:589`) and keeps reading frames. In parallel, `POST /ingest`
  with `Authorization: Bearer load-ingest-token` and body `{"text": "load <n>"}` at
  `--ingest-rate` per second (201 expected).
- Report JSON keys: `clients`, `duration_secs`, `hello_p50_ms`, `hello_p95_ms`,
  `hello_max_ms`, `ws_errors`, `disconnects`, `ingest_sent`, `ingest_created`,
  `ingest_rate_limited`, `ingest_errors`, `server_alive`, `fd_start`, `fd_end` (Linux:
  entries of `/proc/<pid>/fd` 5 s after the last HELLO and at the end; `null` elsewhere).
- Budgets, one `PASS`/`FAIL` line each, exit 0 only if all pass: `hello_p95_ms <=
  --max-hello-p95-ms`, `ws_errors == 0`, `disconnects == 0`, `ingest_errors == 0`,
  `ingest_rate_limited == 0`, `server_alive`, and on Linux `fd_end <= fd_start + 10`. The
  server is killed at the end in every case.
- `fn percentile(sorted_ms: &[u64], p: f64) -> u64` (nearest-rank, empty → 0) with unit
  tests on known vectors.
- Job `load`: `ubuntu-latest`, `timeout-minutes: 30`; checkout, stable toolchain,
  rust-cache; `cargo build -p lc-server --release --locked` with env
  `LC_ADMIN_WEB_BUILD: "0"`; `cargo run -p xtask --release --locked -- loadtest
  --server-bin target/release/lc-server --report target/load-report.json`; upload the
  report (always).

**Steps:**
- [ ] **Step 1:** Unit tests for `percentile` and the report keys; implement.
- [ ] **Step 2:** `cargo build -p lc-server --locked --offline`, then `cargo xtask loadtest
  --clients 20 --duration-secs 20 --report target/load-report.json` → all budgets `PASS`,
  exit 0; paste the report in the task report.
- [ ] **Step 3:** Sabotage: add `tokio::time::sleep(std::time::Duration::from_millis(300))
  .await;` right before the WELCOME is sent (`ws/session.rs:588`), rebuild, rerun → `FAIL
  hello_p95_ms`, exit 1; revert and rebuild.
- [ ] **Step 4:** Add the job. Definition of done. Commit
  `feat(xtask): add a websocket load test harness`.

### Task 8: kill-and-recover

Parallel-safe with Task 6.

**Files:** create `apps/lc-server/tests/kill_recover.rs` only.

**Interfaces:**
- Binary: `env!("CARGO_BIN_EXE_lc-server")`, started with `tokio::process::Command` and the
  same env as Task 7 (free port, `file:` database in a temp dir, `NODE_ENV=development`).
  A guard struct kills the child on drop (no orphan process on panic).
- Seed before the first start: one guild member + one ingest client (recipe of
  `submit.rs:351`, token `kill-ingest-token`).
- Scenario: a task posts `{"text": "kill <n>"}` to `/ingest` in a loop and records every
  `n` that got a 201; after at least 30 confirmed posts, kill the server abruptly
  (`child.kill().await`), stop the poster, restart the server on the same data dir, wait
  for `/health` `db == "ok"` (30 s).
- Assertions: (1) every confirmed text is in `SELECT text FROM playback_jobs` (read through a
  second `Db::open` on the same file, WAL allows it); (2) `PRAGMA integrity_check` returns
  `ok`; (3) one more post after the restart gets 201.
- Characterization, not asserted: print with `eprintln!` the statuses of the jobs after the
  restart and put them in the report. A job left `PLAYING` forever would be a finding for
  the user (no startup recovery was found in the code); do not fix it here.

**Steps:**
- [ ] **Step 1:** Write the test; `cargo nextest run -p lc-server --test kill_recover
  --locked --offline` → pass (Windows: `kill` is `TerminateProcess`, expected to work).
- [ ] **Step 2:** Sabotage: delete `<db>-wal` between the kill and the restart → assertion
  (1) fails (committed rows lost); revert.
- [ ] **Step 3:** Definition of done. Commit `test(server): add a kill-and-recover test`.

### Task 9: canary deployment with a test Discord bot

Parallel-safe with Task 3.

**Files:** modify `scripts/release/ReleaseLib.psm1`, `scripts/release.ps1`,
`.claude/skills/release/SKILL.md`; create `scripts/release/tests/Canary.Tests.ps1` (picked up
by `run-tests.ps1`, which loads every `*.Tests.ps1`).

**Interfaces:**
- `Get-ServerDeployScript -Version <X.Y.Z> -Stamp <yyyyMMdd-HHmmss> [-Canary]`: without
  `-Canary` the script is byte-identical to today's. With `-Canary`:
  - new variables at the top: `CANARY_DIR='/mnt/user/appdata/livechat-canary'`,
    `CANARY_ENV='/mnt/user/appdata/livechat-canary.env'`, `CANARY=0`;
  - right after `[ -n "$PREV_IMAGE" ]`: `[ -f "$CANARY_ENV" ]` (fails before anything is
    stopped);
  - right after the `cmp` loop (backup verified, main container stopped), these lines:
    ```
    docker rm -f LiveChat-canary >/dev/null 2>&1 || true
    rm -rf "$CANARY_DIR"
    mkdir -p "$CANARY_DIR/media"
    cp -a "$BACKUP"/livechat.sqlite* "$CANARY_DIR"/
    chown -R 10001:10001 "$CANARY_DIR"
    CANARY=1
    docker run -d --name LiveChat-canary -p 127.0.0.1:3001:3000 -v "$CANARY_DIR":/app/data --env-file "$CANARY_ENV" -e API_URL=http://127.0.0.1:3001 "$IMAGE" >/dev/null
    H=''
    for i in $(seq 1 45); do H="$(curl -fsS --max-time 2 http://127.0.0.1:3001/health 2>/dev/null || true)"; if grep -q "\"version\":\"$VERSION\"" <<<"$H" && grep -q '"db":"ok"' <<<"$H" && grep -q '"discord":"ok"' <<<"$H"; then break; fi; sleep 2; done
    grep -q "\"version\":\"$VERSION\"" <<<"$H"
    grep -q '"db":"ok"' <<<"$H"
    grep -q '"discord":"ok"' <<<"$H"
    [ "$(docker logs LiveChat-canary 2>&1 | grep -c 'Discord pr' || true)" -ge 1 ]
    docker rm -f LiveChat-canary >/dev/null
    CANARY=0
    echo "CANARY OK $VERSION"
    ```
  - in `on_error`, first: `if [ "$CANARY" = 1 ]; then echo "CANARY FAILED" >&2; docker rm -f
    LiveChat-canary >/dev/null 2>&1 || true; fi`. The rest of the trap is unchanged: the
    template is not modified yet (`UPDATING=0`), so `docker start LiveChat` restarts the
    previous image, and the ROLLBACK line is printed.
- `Invoke-ServerDeploy -Version <X.Y.Z> [-Canary]` passes `-Canary` through;
  `release.ps1 deploy-server <X.Y.Z> [-Canary]` adds `[switch]$Canary` to its `param` block.
- The env file is created once by the user on the host (never by a script, never printed):
  `DISCORD_TOKEN` and `DISCORD_CLIENT_ID` of the TEST bot, a canary-only `ADMIN_UI_TOKEN`
  and `MEDIA_URL_SECRET`; `chmod 600`. The canary runs on a copy of the verified backup, so
  it proves the real migration of the real database without touching it. Cost: the main
  container stays stopped during the canary (≤ 90 s of health polling).

**Tests (`Canary.Tests.ps1`, style of `Deploy.Tests.ps1`: `Test-Case`, `Assert-True`,
`Get-StepIndex`):**
- `Canary script order`: `[ -f "$CANARY_ENV" ]` < `docker stop -t 20 LiveChat` < `cmp ` <
  `docker run -d --name LiveChat-canary` < `CANARY OK` < `sed -i "s|<Repository>` <
  `"$UPDATE" LiveChat` (search each step after the previous one, as `Deploy script order`).
- `Canary uses its own data dir and the test bot env file`: contains `-v
  "$CANARY_DIR":/app/data`, `--env-file "$CANARY_ENV"`, `-p 127.0.0.1:3001:3000`; the
  `docker run` line does not contain `$DATA`; the script contains none of `DISCORD_TOKEN`,
  `TIKTOK_COOKIE`, `docker inspect`, `Config.Env`.
- `Canary never stops or recreates the main container`: between `CANARY=1` and `CANARY=0`
  there is no `docker stop`, no `"$UPDATE"`, and every `docker` command names
  `LiveChat-canary`.
- `Canary failure removes the canary and restarts the main container`: the `on_error` body
  contains `docker rm -f LiveChat-canary` and still `docker start LiveChat`.
- `Deploy script without canary is unchanged`: the default script does not contain
  `canary` (case-insensitive); the existing `Deploy.Tests.ps1` stays green unmodified.
- `Canary script is ASCII only and has no CR`.

**Steps:**
- [ ] **Step 1:** Write the tests; run `powershell -NoProfile -File
  scripts/release/tests/run-tests.ps1` → the new tests fail, the old ones pass.
- [ ] **Step 2:** Implement; rerun → all pass.
- [ ] **Step 3:** Sabotage: move `docker rm -f LiveChat-canary >/dev/null` (success path)
  after `"$UPDATE" LiveChat` → `Canary script order` fails; revert.
- [ ] **Step 4:** Release skill: add a "Canary" section (one-time setup: test bot in the
  Discord developer portal, invite it to a test guild, create the env file, check that
  port 3001 is free with `ss -ltn | grep ':3001 '`; usage; what it proves; extra downtime;
  failure output). No live run: a rehearsal on Unraid waits for the user's explicit go.
- [ ] **Step 5:** Definition of done (`cargo xtask verify --offline` too, unaffected).
  Commit `feat(release): add a canary deployment mode with a test discord bot`.

### Review B (after Task 9)

Same brief as Review A, on the commits of Tasks 6 to 9, plus: (6) the canary script can
never mount `/mnt/user/appdata/livechat`, read or print a secret, or leave the main
container stopped on any failure path; (7) the load harness and the kill test always kill
their server process.

### Task 10: docs, verify skill, final battery

**Files:** create `docs/rust/verification.md`, `.claude/skills/verify/SKILL.md`; modify
`README.md` (one section with a link).

**Interfaces:**
- `docs/rust/verification.md`: the phase → technique → command → gate table of the spec;
  the adaptations (formal → property tests + frozen spec; shadowing/mirroring → load
  harness + canary; monitoring → `/health` + admin console + an external uptime probe on
  the user's side; feature flags → the existing runtime settings; real-user monitoring →
  HELLO `appVersion`/`platform` already stored; `/metrics`, JSON logs, OTLP → out of scope);
  the nightly workflow map; how to run everything locally. First line links
  `docs/rust/audit/verification.md` and says that file is the past audit, not this pipeline.
- `SKILL.md` (frontmatter like `.claude/skills/release/SKILL.md`): trigger phrases
  ("verify", "vérifie", "verification battery", "is this change trustworthy"); workflow for
  agent-written changes: spec → failing tests first → implement → `cargo xtask verify
  --offline` → `cargo xtask verify --offline --mutants` (kill or justify every missed
  mutant) → adversarial review run → `release.ps1 deploy-server <X.Y.Z> -Canary` before any
  deploy.

**Steps:**
- [ ] **Step 1:** Write the docs and the skill.
- [ ] **Step 2:** Final battery, each must pass: `cargo xtask verify --offline --mutants`;
  `powershell -NoProfile -File scripts/release/tests/run-tests.ps1`;
  `cargo test --manifest-path fuzz/Cargo.toml --no-default-features --lib --locked
  --offline`; `cargo xtask loadtest --clients 20 --duration-secs 20`.
- [ ] **Step 3:** Commit `docs: document the verification pipeline and add a verify skill`.
  Report to the user. Push and PR only after the user's go.
