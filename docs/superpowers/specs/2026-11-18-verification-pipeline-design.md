# Verification Pipeline Design

**Date:** 2026-11-18 · **Status:** approved by the user on 2026-11-18 (scope: "Pipeline complet adapté")
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
| Unit / functional / integration tests | exists (nextest + doctests, 23 crate test files, Playwright e2e admin, vitest extension) | kept as-is |
| Smoke tests | exists (server-image.yml smoke: /health, console, non-root uid 10001, ffmpeg/yt-dlp, SQLite write, SIGTERM ≤ 20 s; overlay-tauri.yml NSIS install/run/uninstall) | kept as-is |
| Contract tests | partial (`lc-db/tests/schema_contract.rs`, 32 golden fixtures in `lc-protocol/tests/fixtures_roundtrip.rs`) | Task 6: HTTP contracts (/health, admin auth, ingest 4xx) + WS version matrix `3.0.0..PROTOCOL_VERSION` |
| Property-based tests | absent | Tasks 2-3: proptest on lc-protocol (versions, frame guard, close codes, signature determinism, envelope roundtrip), lc-media (SSRF guard, classification, ffmpeg args), lc-db (queue state machine) |
| Fuzz tests | absent | Task 4: excluded `fuzz/` crate, targets `protocol_frame`, `media_url`, `ingest_payload`, nightly CI |
| Mutation testing | absent | Task 5: cargo-mutants nightly, baseline per scope (lc-protocol, lc-media/resolve), jq threshold |
| Performance tests | absent | Task 7: criterion benches + `xtask loadtest` (50 WS overlays + ingest bursts) |
| Formal verification | absent | **adapted**: invariants as property tests + protocol v2 spec FROZEN (`docs/rust/protocol-v2.md`); no Kani (cost outweighs benefit here), documented |
| Threat/audit(?) | exists (cargo deny + cargo audit blocking, ignore lists synced deny.toml ↔ rust.yml) | kept; `cargo xtask verify` gates advisories via `cargo deny check` |

### TESTING

| Family | Status before | LiveChat implementation |
|---|---|---|
| Load tests | absent | Task 7: `cargo xtask loadtest`, budgets hello p95 < 250 ms / 0 errors / no fd leak, nightly job |
| Canary | absent (direct deploy) | Task 9: `release.ps1 deploy-server <X.Y.Z> -Canary` — `LiveChat-canary` on port 3001, own data dir, health-gated promotion, rollback line, never stops the main container |
| Chaos | absent | Task 8: kill-and-recover test (abrupt server kill mid-SQLite-write → restart → `/health` db ok + `PRAGMA integrity_check` clean + queue intact) |
| Shadowing / traffic mirroring | not applicable (homelab, no prod traffic to mirror) | **adapted**: replaced by the load harness (golden-scenario replay) + canary; documented |
| Integration (overlay↔server) | partial (golden fixtures, e2e admin) | contract matrix Task 6; no new harness |

### DEPLOY / RELEASE

| Family | Status before | LiveChat implementation |
|---|---|---|
| Verified deployment | exists (`release.ps1` plan/bump/check/pending-tags/deploy-server, `release.yml`, `release-check.yml`, Unraid backup+health procedure) | kept; Task 9 adds the canary gate; Task 14's skill wires the canary checklist into the release flow |

### POST-RELEASE

| Family | Status before | LiveChat implementation |
|---|---|---|
| Feature flagging | absent | Task 10: `LIVECHAT_FLAGS` env, `Flags::overlay_telemetry()`, `/admin/api/flags` listing |
| Logs | text tracing (`EnvFilter`, RUST_LOG) | Task 12: optional `LIVECHAT_LOG_JSON=1` JSON formatter; default unchanged; media events carry `media_id`, `stage`, `duration_ms`, `outcome` |
| Events | partial (admin journal) | Task 12 normalized media event fields; journal keeps admin-action semantics |
| Monitoring | partial (`/health` {version, protocolVersion, db, discord, status}) | Task 11: `/metrics` Prometheus text endpoint, hand-rolled atomics registry (zero new deps) |
| Traces | partial (tracing spans) | kept; OTLP export documented as out of scope (P3) |
| Real user monitoring | absent | Task 13: opt-in overlay telemetry (overlay setting + server flag, both default OFF), `POST /telemetry/overlay`, ≥ 1 h throttle, no personal data, counted in `/metrics` |

## 3. Architecture decisions

1. **Plain tests ride the existing CI.** Property/contract/resilience tests are ordinary cargo
   tests: the `rust.yml` matrix (fmt, clippy, msrv 1.90, nextest ubuntu+windows, doctests, e2e,
   image smoke, deny, audit) runs them with zero workflow changes.
2. **Heavy verification is nightly-only.** One new workflow, `verification-nightly.yml`
   (SHA-pinned actions, `permissions: contents: read`, concurrency group, cron + dispatch),
   with three jobs: `fuzz` (nightly toolchain, 45 s per target, crash artifacts), `mutants`
   (pinned cargo-mutants, 20 min budget, baseline − 1 pt slack, advisory), `load` (release build,
   `xtask loadtest`, jq budget asserts). Nightlies never become merge gates.
3. **Protocol v2 stays FROZEN.** No wire change, no fixture change; every server-facing addition
   is a REST route (`/metrics`, `/telemetry/overlay`, `/admin/api/flags`).
4. **Zero new runtime dependencies** except the `tracing-subscriber` `json` feature. `/metrics`
   is hand-rolled atomics; flags are hand-rolled. New deps are dev-only (proptest, criterion,
   tokio-tungstenite) or excluded (`fuzz/`, own Cargo.lock, nightly toolchain — invisible to
   msrv/clippy/deny).
5. **Canary isolation.** `LiveChat-canary` runs on port 3001 with its own empty data dir
   (`/mnt/user/appdata/livechat-canary`): it proves the image boots, migrates and serves; the
   prod data dir is never mounted twice. The canary stage never stops the main container; every
   failure path prints the rollback line. Live rehearsal on Unraid only on the user's go.
6. **Privacy defaults.** Overlay telemetry OFF on both ends; logs stay text unless opted in;
   `/metrics` and `/health` expose no secrets (test-enforced).
7. **The agent workflow is a deliverable.** Task 14 writes `docs/rust/verification.md` (the
   phase→technique→command→gate map, including the adapted families) and the `verify` skill
   (spec → failing tests first → implement → `cargo xtask verify` → adversarial review →
   canary before deploy), mirroring the release skill.

## 4. Execution model (approved)

- One worktree `feat/verification-pipeline` from `origin/main`; the dirty main checkout
  (`feat/cross-guild-copy`) is untouched.
- One task = one subagent (provider `opencode-go`, model `glm-5.3`, reasoning effort `max` —
  the only authorized route; `deepseekV4.1 flash` does not exist in this session), with a
  self-contained prompt: repo rules, files, interfaces, steps.
- The lead reviews every diff, runs the battery after every task, and commits (conventional
  one-liners in English; the plan approval is the single "go" for the 15 commits).
- Adversarial review subagents re-read phase diffs after Tasks 6, 9 and 13.
- Every test-writing task includes a sabotage step (break the invariant, watch the test fail,
  revert) — the tweet's core point: the verification itself must be verified.

## 5. Success criteria

- `cargo xtask verify` green on Windows PowerShell 5.1 in the worktree.
- Existing CI unchanged and green; `verification-nightly.yml` valid and green on main after merge.
- The 32 protocol fixtures pass unmodified; `docs/rust/protocol-v2.md` untouched.
- No new runtime dependency except the `tracing-subscriber` `json` feature; deny/audit green.
- Overlay telemetry: default OFF end-to-end; no new network calls by default.
- `/metrics` parses; exposition contains no secret values.
- Canary unit tests green under PS 5.1 and pwsh 7; no live Unraid action taken.
- Docs + skill committed; PR opened only after the user's final go.
