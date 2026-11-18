# Verification Pipeline Design

**Date:** 2026-11-18 · **Status:** approved by the user on 2026-11-18 (scope: "Pipeline complet adapté");
revised the same day (v2) after a review against the code and the user's decisions: no Prometheus,
a test Discord bot for the canary, execution by GLM 5.3 orchestrating DeepSeek workers.
**Plan:** `docs/superpowers/plans/2026-11-18-verification-pipeline.md`

## 1. Source and scope

Cindy Sridharan (@copyconstruct) published on Nov 6 2026 the workflow she requires to trust
agent-generated code ([x.com/copyconstruct/status/2118539184654938154](https://x.com/copyconstruct/status/2118539184654938154)):

> "A mix of the techniques shown here, in short. This has been my dream for nearly a decade now,
> and with agents I'm finally able to see this (partially) come true. But turns out this is what
> I necessarily require to trust agent generated code. It's not optional anymore, imo."

Context of the thread: in May 2026 she reported subtle bugs in merged AI-generated PRs that cost
days of manual verification; in Nov 2026 she states the problems are gone thanks to a
verification process that consumes ~70% of her daily token spend (only ~20-30% goes into creating
patches). @poteto asked what the process is; the diagram is the answer.

**Reconstruction caveat:** the attached image could not be viewed by the implementing model
(text-only; a vision subagent was rejected by glm-5.3). It was reconstructed via WinRT OCR
(3 passes: 3× grayscale, 3× grayscale inverted, 2× raw) plus the thread. The pipeline axis reads
PRE-PRODUCTION → TESTING → DEPLOY → RELEASE → POST-RELEASE, with these families:

- PRE-PRODUCTION: unit tests, functional tests, fuzz tests, property-based tests, formal(?),
  contract tests, mutation tests, smoke tests, performance tests, threat(?), audit(?).
- TESTING: integration tests, load tests, canary tests(?) ("CON VIGI"), chaos, shadowing,
  traffic (mirroring), tap/trace compare(?), uzlu$(?) tests.
- RELEASE / POST-RELEASE: feature flagging, logs, events, monitoring, traces, dynamic(?),
  real user monitoring / experience.

Labels marked (?) are OCR-uncertain. The mapping below — approved by the user — is the
authoritative scope, not the uncertain labels.

## 2. Mapping table (diagram family → LiveChat)

### PRE-PRODUCTION

| Family | Status before | LiveChat implementation |
|---|---|---|
| Unit / functional / integration tests | exists (nextest + doctests, Playwright e2e admin, vitest extension, overlay `node --test`) | kept as-is |
| Smoke tests | exists (`rust.yml` image job and `server-image.yml`: /health, console, login; `overlay-tauri.yml` NSIS install/run/uninstall) | kept as-is |
| Contract tests | partial (`lc-db/tests/schema_contract.rs`, 38 golden fixtures in `lc-protocol/tests/fixtures_roundtrip.rs`) | Task 6: exact `/health` key set, anonymous admin 401, WS HELLO version matrix through the real WebSocket |
| Property-based tests | absent | Task 2 (done) + 2b: protocol versions, frame guard, envelope round-trip, signature-input injectivity, total close mapping in `hub.rs`; Task 3: SSRF guard pure functions (ranges, IPv4-mapped IPv6, encoded hosts, redirects), yt-dlp argument order, one PLAYING job per guild |
| Fuzz tests | absent | Task 4: excluded `fuzz/` crate, targets `protocol_frame` (decode + round-trip), `media_url`, `html_parsers` (untrusted TikTok/Twitter HTML and JSON), seed test on stable, nightly CI |
| Mutation testing | absent | Task 1: `cargo xtask verify --mutants` mutates only the lines changed since `origin/main`; Task 5: nightly scores for `lc-protocol` and `lc-media/src/guard.rs` against a baseline |
| Performance tests | absent | Task 7: `cargo xtask loadtest` (WebSocket overlays + ingest bursts, budgets); criterion benches dropped (no CI comparison, microsecond decode) |
| Formal verification | absent | **adapted**: invariants as property tests + protocol v2 spec FROZEN (`docs/rust/protocol-v2.md`); no Kani (cost outweighs benefit here) |
| Threat/audit(?) | exists (cargo deny + cargo audit blocking, ignore lists synced deny.toml ↔ rust.yml) | kept; `cargo xtask verify` gates advisories via `cargo deny check` |

### TESTING

| Family | Status before | LiveChat implementation |
|---|---|---|
| Load tests | absent | Task 7: budgets hello p95 ≤ 250 ms, 0 WS/ingest errors, 0 disconnects, no fd growth (Linux), nightly job |
| Canary | absent (direct deploy) | Task 9: `release.ps1 deploy-server <X.Y.Z> -Canary` — `LiveChat-canary` on `127.0.0.1:3001`, a copy of the verified backup database, a TEST Discord bot from a host-only env file, health + Discord gate before the main container is updated |
| Chaos | absent | Task 8: abrupt kill while ingesting → restart → every confirmed row present, `PRAGMA integrity_check` ok, writes work; job statuses after restart reported (no startup recovery found) |
| Shadowing / traffic mirroring | not applicable (homelab, no prod traffic to mirror) | **adapted**: load harness + canary on a copy of the real database |
| Integration (overlay↔server) | partial (golden fixtures, e2e admin) | WS version matrix (Task 6); no new harness |

### DEPLOY / RELEASE

| Family | Status before | LiveChat implementation |
|---|---|---|
| Verified deployment | exists (`release.ps1` plan/bump/check/pending-tags/deploy-server, `release.yml`, `release-check.yml`, backup + health + Discord checks) | kept; Task 9 adds the canary gate; Task 10's skill wires the canary into the release flow |

### POST-RELEASE

| Family | Status before | LiveChat implementation |
|---|---|---|
| Feature flagging | runtime settings exist (`/admin/api/runtime-settings`, console, journaled) | **adapted**: kept; no env-flag system (its only consumer, overlay telemetry, was dropped) |
| Logs / events | text tracing (`EnvFilter`, `RUST_LOG`), admin journal | kept; JSON output out of scope (no log aggregator) |
| Monitoring | `/health` {status, version, protocolVersion, db, discord, uptimeSec, activeJobs} + admin console | **adapted**: kept, plus an external uptime probe on `/health` on the user's side (e.g. Uptime Kuma on Unraid); `/metrics` dropped (no Prometheus) |
| Traces | tracing spans | kept; OTLP out of scope |
| Real user monitoring | HELLO already stores each overlay's `appVersion` and `platform` | **adapted**: kept; opt-in overlay telemetry dropped |

## 3. Architecture decisions

1. **Plain tests ride the existing CI.** Property, contract and resilience tests are ordinary
   cargo tests: the `rust.yml` matrix (fmt, clippy, msrv 1.90, nextest ubuntu+windows,
   doctests, e2e, image smoke, deny, audit) runs them with zero workflow changes.
2. **Heavy verification is nightly-only.** One new workflow, `verification-nightly.yml`
   (SHA-pinned actions, `permissions: contents: read`, concurrency group, cron + dispatch, and a
   `pull_request` trigger limited to its own files so a PR can run it before it reaches `main`),
   with jobs `fuzz`, `mutants`, `load`. Nightlies never become merge gates.
3. **Protocol v2 stays FROZEN.** No wire change, no fixture change, no new route.
4. **No new runtime dependency.** New crates are dev-only (`proptest`, `tokio-tungstenite`),
   xtask-only (`futures-util`, `tokio-tungstenite`, internal crates), or excluded (`fuzz/`: own
   workspace, own `Cargo.lock`, nightly to run, `cargo check` on stable).
5. **The only production change** is a hardening found by the review: `build_ytdlp_args` puts
   `--` before the URL, so a URL can never be read as a yt-dlp option.
6. **Canary isolation.** The canary runs after the existing stop + verified backup, on a copy of
   that backup, with a test bot token from `/mnt/user/appdata/livechat-canary.env` (host-only,
   `chmod 600`, never read or printed by a script). It never mounts the production data dir,
   never receives the production Discord token or TikTok cookie, never stops or recreates the
   main container itself; every failure path removes it, restarts the main container on its
   previous image and prints the rollback line. Cost: the main container stays stopped while the
   canary is checked (≤ 90 s). Live rehearsal on Unraid only on the user's go.
7. **Mutation testing guards agent work.** `cargo xtask verify --mutants` mutates the lines a
   change touched and fails on any missed mutant: an agent must kill it with a test or justify it
   as equivalent. This is the cheapest direct answer to "do this change's tests catch anything?".
8. **The agent workflow is a deliverable.** Task 10 writes `docs/rust/verification.md` and the
   `verify` skill (spec → failing tests first → implement → `cargo xtask verify --offline` →
   `--mutants` → adversarial review → canary before deploy), mirroring the release skill.

## 4. Execution model (approved, revised 2026-11-18)

- One worktree `feat/verification-pipeline` from `origin/main`; the dirty main checkout
  (`feat/cross-guild-copy`) is untouched.
- Orchestrator: GLM 5.3 through DeepSeek Harness (opencode account `moi`). Workers:
  `opencode-go/deepseek-v4.1-flash#max`, one task per worker, self-contained brief (task section,
  global constraints, definition of done, allowed files).
- The orchestrator re-runs every acceptance command itself, checks the touched files, retries a
  worker once at most, and commits (conventional one-liners in English; the plan approval is the
  "go" for the plan's commits). Push and PR wait for the user's go.
- Adversarial review runs after Task 5 and after Task 9.
- Every test-writing task includes a sabotage step that breaks the product invariant (never the
  test harness), watches the named test fail, and reverts.

## 5. Success criteria

- `cargo xtask verify --offline` green in the worktree (Windows PowerShell 5.1);
  `cargo xtask verify --offline --mutants` with every missed product mutant killed or justified.
- Existing CI unchanged and green; `verification-nightly.yml` green on its PR run.
- The 38 protocol fixtures pass unmodified; `docs/rust/protocol-v2.md` untouched.
- No new runtime dependency; `cargo deny check` green; MSRV 1.90 check green.
- `cargo metadata` of the workspace never lists `livechat-fuzz`.
- Load harness budgets pass locally (20 clients, 20 s) and in the nightly job (50 clients, 60 s).
- Canary unit tests green under Windows PowerShell 5.1 locally and pwsh 7 in `release-check.yml`;
  no live Unraid action taken.
- Docs + skill committed; PR opened only after the user's final go.
