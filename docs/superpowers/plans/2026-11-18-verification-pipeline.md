# Verification Pipeline Implementation Plan

> **For agentic workers:** Implement this plan task-by-task; steps use checkbox (`- [ ]`) syntax for tracking. Approved execution model: the lead delegates each task to one subagent (provider `opencode-go`, model `glm-5.3`, reasoning effort `max` — the only authorized route in this session; `deepseekV4.1 flash` does not exist there), reviews the returned diff, runs the verification battery, then commits. An adversarial review subagent re-reads every diff of a phase after Tasks 6, 9 and 13.

**Goal:** Make agent-generated code in this repo trustworthy by implementing the verification landscape of Cindy Sridharan's workflow diagram ([x.com/copyconstruct/status/2118539184654938154](https://x.com/copyconstruct/status/2118539184654938154), Nov 6 2026 — "what I necessarily require to trust agent generated code"): every technique family on the PRE-PRODUCTION → TESTING → DEPLOY/RELEASE → POST-RELEASE continuum maps to a concrete LiveChat command or CI gate. Concretely: property-based tests, fuzz targets, mutation testing, HTTP/WS contract tests, criterion benchmarks, a WebSocket load harness, a kill-and-recover resilience test, a canary deploy mode, server feature flags, a Prometheus `/metrics` endpoint, structured JSON logs, opt-in overlay telemetry, the `cargo xtask verify` battery, one nightly CI workflow, and docs + a `verify` skill that wire it all into the agent workflow.

**Source caveat:** the diagram could not be viewed directly (text-only model; the vision subagent was rejected by glm-5.3). It was reconstructed via WinRT OCR (3 passes) plus the tweet thread; uncertain labels are marked (?) in the spec. The mapping table in the spec is the authoritative scope, approved by the user on 2026-11-18 ("Pipeline complet adapté").

**Architecture:** New verification code is plain cargo tests and benches — they ride the existing `rust.yml` matrix unchanged. Heavy verification (fuzz, mutation, load) lives in one new nightly workflow, `.github/workflows/verification-nightly.yml` (SHA-pinned actions, `permissions: contents: read`, concurrency group, `cron '30 3 * * *'` + `workflow_dispatch`). The server gains a hand-rolled metrics registry (atomics, zero new dependencies) at `/metrics`; overlay→server telemetry is REST, so protocol v2 stays FROZEN. Canary deployment extends `scripts/release.ps1` + `ReleaseLib.psm1` (Windows PowerShell 5.1 and pwsh 7 compatible) with mocked-ssh tests; a live Unraid rehearsal only happens on the user's explicit go. Local entry point: `cargo xtask verify`.

**Tech Stack:** Rust (MSRV 1.90), proptest + criterion (dev-deps), cargo-fuzz (excluded `fuzz/` crate, nightly), cargo-mutants (nightly CI), tokio-tungstenite (xtask/server-tests only), tracing-subscriber `json` feature, PowerShell 5.1/7, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-18-verification-pipeline-design.md` (committed with this plan in Task 0).

## Global Constraints

- Branch `feat/verification-pipeline` in a new worktree created from `origin/main` after `git fetch origin`. The main checkout (on `feat/cross-guild-copy`, dirty) is never touched; drafts of the spec and plan stay there untracked.
- English code, comments, commit messages. Commits follow `commitlint.config.js` (`@commitlint/config-conventional`): `type(scope)?: lowercase imperative subject`, one line, no `Co-Authored-By`. The plan approval (2026-11-18) is the single "go" for the 15 commits of this plan; pushing the branch and opening the PR waits for a final go. If `.husky/commit-msg` cannot run in the fresh worktree (no `node_modules`), commit with `--no-verify` — every message below already satisfies config-conventional.
- Protocol v2 is FROZEN (`docs/rust/protocol-v2.md`): no change to any `lc-protocol` wire type, constant or fixture; the 32 golden fixtures must pass unmodified. All server-facing additions are REST routes.
- MSRV 1.90: `proptest`, `criterion`, `tokio-tungstenite` are dev-deps or xtask-only and must compile on 1.90 (the `rust.yml` msrv job checks this). `fuzz/` is excluded from the workspace with its own `Cargo.lock` and nightly toolchain — invisible to the msrv/clippy/deny jobs.
- `deny.toml` and the `rust.yml` `cargo audit --ignore` list must stay in sync. `cargo xtask verify` gates advisories through `cargo deny check` (deny.toml is the single source of truth); plain `cargo audit` stays in `rust.yml` only.
- No new runtime dependencies for `lc-server`/`lc-overlay` except the `tracing-subscriber` `json` feature.
- Byte-faithful edits: keep each file's BOM and line endings (`Cargo.toml` UTF-8 BOM + CRLF; `apps/lc-overlay/tauri.conf.json` CRLF no BOM; `extension/public/manifest.json` LF no BOM).
- Anything PowerShell runs under Windows PowerShell 5.1 (no `&&`, `||`, `??`, `?:`, no `-AsHashtable`; `Join-Path` with two arguments) **and** pwsh 7 on Linux.
- `/metrics`, `/health` and the telemetry route expose no secrets (a test asserts the exposition never contains token/env values).
- Overlay telemetry is OFF by default on both ends (overlay local setting + server flag `overlay_telemetry`), frequency-capped, no personal data. Logs stay text unless `LIVECHAT_LOG_JSON=1`.
- Unraid (only if a live step is ever authorized): ssh `"C:/Windows/System32/OpenSSH/ssh.exe" -o BatchMode=yes root@192.0.2.59`; container `LiveChat`; canary container `LiveChat-canary` on port 3001 with its OWN empty data dir `/mnt/user/appdata/livechat-canary` — never the prod data dir; the main container is never stopped by the canary stage; never print container env/secrets.
- New CI is limited to `.github/workflows/verification-nightly.yml` (created Task 4, extended Tasks 5 and 7). Existing workflows (`rust.yml`, `server-image.yml`, `overlay-tauri.yml`, `extension.yml`, `release.yml`, `release-check.yml`) stay untouched.

## Review Focus

1. A test that cannot fail verifies nothing: every test-writing task below includes a sabotage step — break the invariant on purpose, watch the new test fail, revert.
2. Property tests must not weaken the golden fixtures: protocol wire compatibility is proven by `fixtures_roundtrip.rs` staying green with zero fixture edits.
3. `fuzz/` must stay excluded: a workspace leak shows up as the msrv job failing on nightly-only code.
4. `/metrics` uses atomics only (no locks on hot paths) and parses with a naive line parser in its test.
5. The canary path never stops the main container, always prints the rollback line on failure, and uses its own data dir.
6. Default behavior is byte-identical: text logs by default, no overlay network telemetry by default, 404 telemetry route when the flag is off.
7. Nightly jobs are advisory, not merge gates: the mutants baseline has slack (−1 pt) so nightlies don't turn into flaky blockers.
8. Load budgets are calibrated for a 2-vCPU runner: hello p95 < 250 ms, zero errors, no fd leak — the report JSON documents them.

## Out of Scope

- JS batteries (extension vitest, admin-web node tests, Playwright e2e): existing suites kept as-is.
- OTLP/OpenTelemetry export (P3, documented in `docs/rust/verification.md`).
- Any protocol v2 change, any live Unraid action without an explicit go, any new dependency beyond the listed dev-deps.

---

### Task 0: Worktree, spec and plan

**Files:**
- Create: `docs/superpowers/specs/2026-11-18-verification-pipeline-design.md`, `docs/superpowers/plans/2026-11-18-verification-pipeline.md` (copies of the drafts written on the main checkout).

**Steps:**
- [x] **Step 1:** `git fetch origin`; create the worktree from `origin/main`; rename the branch to `feat/verification-pipeline`; verify `git status` clean before adding.
- [x] **Step 2:** Copy both docs into the worktree, review the diff, commit `docs: add verification pipeline spec and plan`.

### Task 1: `cargo xtask verify`

**Files:**
- Modify: `xtask/src/main.rs`
- Create: `xtask/src/verify.rs`

**Interfaces:**
- `cargo xtask verify [--full]` runs, in order: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace --all-features --locked --no-fail-fast`, `cargo test --doc --workspace`, `cargo deny check`. Prints one `PASS`/`FAIL` line per stage plus a summary; exit code = number of failed stages. `--full` reserves hooks for the slow stages of Tasks 4–7 (documented in `--help`).

**Steps:**
- [ ] **Step 1:** Implement the subcommand (stage list, failure counting, clear output).
- [ ] **Step 2:** Run it on the clean worktree — expected: all stages `PASS`, exit 0.
- [ ] **Step 3:** Sabotage check: introduce a formatting error, re-run — expected `FAIL` on fmt, exit ≠ 0; revert.
- [ ] **Step 4:** Commit `feat(xtask): add a verify subcommand running the full battery`.

### Task 2: property-based tests, `lc-protocol`

**Files:**
- Modify: `crates/lc-protocol/Cargo.toml` (`[dev-dependencies] proptest`)
- Create: `crates/lc-protocol/tests/properties.rs`

**Properties (test names):**
- `version_strings_parse_and_compare` — `parse_simple_version` / `is_version_below` monotonic and total on generated `X.Y.Z` strings.
- `frame_size_guard_boundary` — `is_frame_too_large` flips exactly at `MAX_FRAME_BYTES` for generated payloads.
- `close_code_reason_total` — every generated `close_code` maps to a non-empty `close_reason` (documented exceptions only).
- `media_signature_input_deterministic` — same inputs, same signature string.
- `envelope_roundtrip` — generated `Envelope<ClientMessage>` / `Envelope<ServerMessage>` serialize→deserialize to an equal `serde_json::Value`, mirroring `fixtures_roundtrip.rs`.

**Steps:**
- [ ] **Step 1:** Write the strategies and tests.
- [ ] **Step 2:** Run `cargo nextest run -p lc-protocol` — expected green.
- [ ] **Step 3:** Sabotage: invert `is_version_below` — expected: the property fails; revert.
- [ ] **Step 4:** Confirm `fixtures_roundtrip.rs` stays green with zero fixture edits; run `cargo xtask verify`.
- [ ] **Step 5:** Commit `test(protocol): add property-based tests`.

### Task 3: property-based tests, `lc-media` + `lc-db`

**Files:**
- Modify: `crates/lc-media/Cargo.toml`, `crates/lc-db/Cargo.toml` (dev-dep `proptest`)
- Create: `crates/lc-media/tests/properties.rs`, `crates/lc-db/tests/properties.rs`

**Properties:**
- `ssrf_guard_rejects_private_literals` — across decimal/octal/hex/IPv6 encodings, private/loopback/link-local targets are always rejected (build on the `tests/ssrf_guard.rs` surface).
- `classify_never_panics` — arbitrary URL-ish strings never panic and return a consistent `MediaKind`.
- `ffmpeg_args_safe` — generated titles/paths yield arguments with no NUL, no leading `-` outside the allowlist.
- `queue_state_machine_invariants` — proptest state machine over queue ops (no duplicate ids, stable order, finished removed), on the `tests/invariants.rs` harness.

**Steps:**
- [ ] **Step 1:** Write the tests (reuse the existing harnesses; no production code change expected).
- [ ] **Step 2:** Run — expected green.
- [ ] **Step 3:** Sabotage: remove one guard branch from the SSRF guard — expected: the property fails; revert.
- [ ] **Step 4:** Run `cargo xtask verify`. Commit `test: add property-based tests for media classification and db queue invariants`.

### Task 4: fuzz targets + nightly workflow

**Files:**
- Create: `fuzz/Cargo.toml` (own workspace), `fuzz/fuzz_targets/protocol_frame.rs`, `fuzz/fuzz_targets/media_url.rs`, `fuzz/fuzz_targets/ingest_payload.rs`, `fuzz/corpus/protocol_frame/` seeded from `crates/lc-protocol/tests/fixtures/*.json`
- Modify: root `Cargo.toml` (`exclude = ["fuzz"]`), `.gitignore` (`fuzz/artifacts/`)
- Create: `.github/workflows/verification-nightly.yml` (job `fuzz`)

**Interfaces:**
- `protocol_frame(data: &[u8])` — raw bytes through the real frame decode path, no panic.
- `media_url(s: &str)` — URL string through classification + SSRF guard, no panic, private targets never pass.
- `ingest_payload(data: &[u8])` — ingest body parse, no panic.
- Nightly job `fuzz`: nightly toolchain, `cargo fuzz run <target> -- -max_total_time=45` per target, artifact upload on crash.

**Steps:**
- [ ] **Step 1:** Scaffold the excluded crate; wire targets to the real code paths (`libfuzzer-sys` pinned).
- [ ] **Step 2:** `cargo +nightly fuzz build` locally if a nightly toolchain exists (else CI-only, WSL note documented).
- [ ] **Step 3:** Create the workflow (SHA-pinned actions, `permissions: contents: read`, concurrency, `cron '30 3 * * *'` + `workflow_dispatch`).
- [ ] **Step 4:** Run `cargo xtask verify` (the workspace must not see `fuzz/`). Commit `test(fuzz): add fuzz targets for protocol frames, media URLs and ingest payloads`.

### Task 5: mutation testing nightly

**Files:**
- Create: `scripts/mutants-baseline.json`
- Modify: `.github/workflows/verification-nightly.yml` (job `mutants`)

**Interfaces:**
- Baseline shape: `{ "lc-protocol": { "caught_pct": <n> }, "lc-media-resolve": { "caught_pct": <n> } }`.
- Job `mutants`: download the pinned `cargo-mutants` release binary; `cargo mutants -p lc-protocol --in-place --max-test-time 60`; `lc-media` scoped to `src/resolve/` + the SSRF guard module (paths pinned in the job); parse the JSON output with `jq`, assert `caught_pct >= baseline - 1`, upload the JSON as an artifact; 20 min budget.

**Steps:**
- [ ] **Step 1:** Measure both scopes (locally or via a disposable `workflow_dispatch` run); record the baseline minus 1 pt slack.
- [ ] **Step 2:** Add the job; run the battery. Commit `ci: add nightly mutation testing with a per-crate score baseline`.

### Task 6: HTTP contracts + WS version matrix

**Files:**
- Create: `apps/lc-server/tests/contracts.rs`, `crates/lc-protocol/tests/version_matrix.rs`

**Interfaces:**
- Contracts (golden pattern of `lc-db/tests/schema_contract.rs`): `/health` JSON keys and types stable (`version`, `protocolVersion == 2`, `db`, `discord`, `status ok|degraded`); anonymous `/admin/api/overview` → 401 shape; the ingest route rejects malformed payloads with a 4xx (route + exact status discovered in Step 1 and pinned).
- Version matrix: `HELLO` envelopes with overlay version from `DEFAULT_MIN_OVERLAY_VERSION` (`3.0.0`) through `PROTOCOL_VERSION` are accepted by the server-side checks.

**Steps:**
- [ ] **Step 1:** Locate the router and the ingest route (read-only discovery); pin names in the tests.
- [ ] **Step 2:** Write the tests; run — expected green.
- [ ] **Step 3:** Sabotage: rename a `/health` field in a scratch edit — expected: the contract fails; revert.
- [ ] **Step 4:** Run `cargo xtask verify`. Commit `test: add HTTP API contracts and a WS version compatibility matrix`.
- [ ] **Step 5:** **Adversarial review subagent** re-reads the diffs of Tasks 1–6 (verification gaps, wrong scopes, weak properties).

### Task 7: benchmarks + load harness

**Files:**
- Modify: `crates/lc-protocol/Cargo.toml` (`[[bench]]` + dev-dep `criterion`), `xtask/Cargo.toml` (dev `tokio-tungstenite`, `futures-util`)
- Create: `crates/lc-protocol/benches/decode.rs`, `xtask/src/loadtest.rs`
- Modify: `.github/workflows/verification-nightly.yml` (job `load`)

**Interfaces:**
- `cargo bench -p lc-protocol` — criterion on Envelope encode/decode + version parse (local tool, not CI).
- `cargo xtask loadtest --url ws://127.0.0.1:<port> --clients 50 --duration 60s --ingest-rate 20 --report <path>` — spawns WS clients (HELLO + heartbeats), ingest bursts; writes `{"hello_p50_ms","hello_p95_ms","errors","frames_sent","overlays_peak"}`.
- Job `load`: build `lc-server` release, start with a temp dir, run the harness, `jq` assert `hello_p95_ms < 250 && errors == 0`, upload the report.

**Steps:**
- [ ] **Step 1:** Write the bench; run `cargo bench -p lc-protocol` — expected output with sane numbers.
- [ ] **Step 2:** Write the harness; run it against a local dev server — expected: p95 < 250 ms, 0 errors.
- [ ] **Step 3:** Sabotage: halve the HELLO timeout — expected: the harness reports errors; revert.
- [ ] **Step 4:** Add the job; run the battery. Commit `feat(xtask): add a WebSocket load test harness with criterion benches`.

### Task 8: kill-and-recover

**Files:**
- Create: `apps/lc-server/tests/kill_recover.rs`

**Interfaces:**
- Spawn the real server binary (in-package integration test, `CARGO_BIN_EXE_<name>`; pin the real bin name in Step 1), temp data dir; drive traffic with a minimal WS client (dev-dep `tokio-tungstenite`); kill the child abruptly mid-write (`Child::kill()`, cross-platform); restart; assert `/health` `db ok`, queue rows intact, and `PRAGMA integrity_check` clean via `lc-db`. If Windows flakes in the matrix, gate with `#[cfg(unix)]` and document it.

**Steps:**
- [ ] **Step 1:** Write the test (the existing ubuntu matrix picks it up).
- [ ] **Step 2:** Sabotage: remove the restart step — expected: the health assertion fails; revert.
- [ ] **Step 3:** Run `cargo xtask verify`. Commit `test(server): add a kill-and-recover resilience test`.

### Task 9: canary deploy mode

**Files:**
- Modify: `scripts/release.ps1`, `scripts/release/ReleaseLib.psm1`, `.claude/skills/release/SKILL.md`
- Create: `scripts/release/tests/Canary.Tests.ps1`

**Interfaces:**
- `release.ps1 deploy-server <X.Y.Z> [-Canary]`:
  - pull the image; create `LiveChat-canary` from the template-derived config, port 3001, data dir `/mnt/user/appdata/livechat-canary` (fresh db, auto-migrate);
  - poll `http://localhost:3001/health` (ssh `curl`) until `version == X.Y.Z` and `status ok|degraded`, 90 s timeout;
  - on green → the existing deploy path (update_container LiveChat, docker start, `/health` poll);
  - on red → `docker stop`/`rm` the canary, print the rollback line, exit 1 — the main container is never stopped by the canary stage.
- Unit tests (mocked ssh/docker, harness `run-tests.ps1`, PS 5.1 + pwsh 7): `Canary creates its own container and data dir`, `Canary promotes the main container on green health`, `Canary cleans up and exits 1 on failed health`, `Canary never stops the main container`.

**Steps:**
- [ ] **Step 1:** Write the failing tests in the mocked harness.
- [ ] **Step 2:** Implement in `ReleaseLib.psm1` + CLI passthrough; run under both shells — expected all `PASS`.
- [ ] **Step 3:** Sabotage: make the health poll always fail — expected: cleanup path, exit 1, rollback line; revert.
- [ ] **Step 4:** Update the release skill with the canary section; run `cargo xtask verify`.
- [ ] **Step 5:** Commit `feat(release): add a canary deployment mode to deploy-server`. Live Unraid rehearsal: only on the user's explicit go, outside this PR.
- [ ] **Step 6:** **Adversarial review subagent** re-reads the diffs of Tasks 7–9.

### Task 10: server feature flags

**Files:**
- Create: `apps/lc-server/src/flags.rs`
- Modify: the router file (GET `/admin/api/flags` behind the existing admin auth)
- Create: `apps/lc-server/tests/flags.rs`

**Interfaces:**
- `Flags::from_env()` parses `LIVECHAT_FLAGS` (comma list, trimmed); unknown names log a warning and are ignored; empty/absent → all off. `Flags::overlay_telemetry() -> bool`.
- `/admin/api/flags` → `{"flags":[{"name":…,"enabled":…}]}`; anonymous → 401 like the other admin routes.

**Steps:**
- [ ] **Step 1:** Write the failing tests (parse + route).
- [ ] **Step 2:** Implement; sabotage: let an unknown flag be silently accepted — expected: the warning test fails; revert.
- [ ] **Step 3:** Run `cargo xtask verify`. Commit `feat(server): add env-driven feature flags with an admin listing`.

### Task 11: Prometheus `/metrics`

**Files:**
- Create: `apps/lc-server/src/metrics.rs`
- Modify: the router file + the connection/queue/media call sites (instrument hooks)
- Create: `apps/lc-server/tests/metrics.rs`

**Interfaces:**
- Registry of static atomics; `metrics::exposition() -> String` in Prometheus text format; `GET /metrics` (public like `/health`).
- Series: `lc_server_info{version,protocol_version}`, `lc_overlays_connected`, `lc_ws_frames_total{direction}`, `lc_queue_depth`, `lc_media_events_total{stage,outcome}`, `lc_overlay_telemetry_total{state}`.

**Steps:**
- [ ] **Step 1:** Implement the registry + route + hooks.
- [ ] **Step 2:** Tests: `Exposition parses line by line`, `Counters move`, `No secrets in exposition`.
- [ ] **Step 3:** Sabotage: skip an increment — expected: the counter test fails; revert.
- [ ] **Step 4:** Run `cargo xtask verify`. Commit `feat(server): add a Prometheus /metrics endpoint`.

### Task 12: JSON logs + media events

**Files:**
- Modify: `apps/lc-server/src/lib.rs` (log init), `apps/lc-server/Cargo.toml` (`tracing-subscriber` feature `json`), the media pipeline call sites (structured fields)

**Interfaces:**
- `LIVECHAT_LOG_JSON=1` → `.json()` formatter; default output unchanged.
- Media pipeline events carry `media_id`, `stage`, `duration_ms`, `outcome`.

**Steps:**
- [ ] **Step 1:** Write the failing tests (captured writer: JSON lines parse; default mode unchanged).
- [ ] **Step 2:** Implement; sabotage: flip the default branch — expected: the default-mode test fails; revert.
- [ ] **Step 3:** Run `cargo xtask verify`. Commit `feat(server): add structured JSON log output and normalized media events`.

### Task 13: opt-in overlay telemetry

**Files:**
- Modify: `apps/lc-overlay/` (settings toggle in the web UI + the send client)
- Modify: the server router (flag-gated route)
- Tests on both sides

**Interfaces:**
- Overlay: local setting `telemetry` (default off); when on, `POST /telemetry/overlay` with `{overlayVersion, updaterState, lastError}` on updater checks and on app errors, throttled ≥ 1 h, no personal data.
- Server: the route 404s unless `Flags::overlay_telemetry`; accepts, counts into the Task 11 metrics; the admin journal keeps admin-action semantics (no telemetry entries).

**Steps:**
- [ ] **Step 1:** Server route + tests (404 off / 200 on / schema / throttle).
- [ ] **Step 2:** Overlay setting + client + web unit tests with mocked fetch (the repo's dev mock harness).
- [ ] **Step 3:** Confirm: protocol fixtures untouched; default = zero new network calls.
- [ ] **Step 4:** Run `cargo xtask verify`. Commit `feat(overlay): add opt-in overlay telemetry surfaced in metrics`.
- [ ] **Step 5:** **Adversarial review subagent** re-reads the diffs of Tasks 10–13.

### Task 14: docs, skill, final battery, PR prep

**Files:**
- Create: `docs/rust/verification.md`, `.claude/skills/verify/SKILL.md`
- Modify: `README.md` (section linking the landscape)

**Interfaces:**
- `docs/rust/verification.md`: the phase→technique→command→gate table; documented adaptations (formal → property tests + frozen spec; shadowing/traffic mirroring → load harness + canary; OTLP → out of scope); the nightly workflow map; how to run everything locally.
- `SKILL.md` (frontmatter like the release skill): trigger phrases; workflow for agent-implemented changes: spec → failing tests first → implement → `cargo xtask verify` → adversarial review → canary checklist before any deploy.

**Steps:**
- [ ] **Step 1:** Write the docs and the skill.
- [ ] **Step 2:** Full battery + the release tooling tests (`scripts/release/tests/run-tests.ps1`) green.
- [ ] **Step 3:** Commit `docs: document the verification landscape and add a verify skill`; report; open the PR after the user's go.
