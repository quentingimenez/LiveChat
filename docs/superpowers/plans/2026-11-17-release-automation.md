# Release automation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let Claude ship server, overlay and extension releases from one "go": a local PowerShell tool (`plan`, `bump`, `check`, `pending-tags`, `deploy-server`), a `/release` project skill, and GitHub workflows that check version consistency on PRs and tag + build on the merge of a `release/*` PR.

**Architecture:** All version logic lives in one PowerShell module `scripts/release/ReleaseLib.psm1` (Windows PowerShell 5.1 and `pwsh` 7 compatible), driven by the CLI `scripts/release.ps1`. CI runs the same module under `pwsh` on `ubuntu-latest`: `release-check.yml` runs `check` + the module's tests on PRs, `release.yml` runs `pending-tags` on push to `main`, pushes the tags and starts the existing build workflows with `gh workflow run --ref <tag>` (the only way a `GITHUB_TOKEN` action can trigger them). Deploying to Unraid stays local (SSH).

**Tech Stack:** PowerShell 5.1 / 7, git, GitHub Actions, `gh` CLI, bash on the Unraid host.

**Spec:** `docs/superpowers/specs/2026-11-17-release-automation-design.md`

## Global Constraints

- Work on branch `feat/release-automation` created from `main` in a new worktree (the main checkout has uncommitted work on `feat/cross-guild-copy`); copy the spec and this plan into it. No junction/link to `node_modules` of the main checkout.
- Code, comments, names, commit messages in English; commit messages one imperative line, no `Co-Authored-By`. Commit only once the user has said so (repo rule); each task's "Commit" step waits for that.
- Scripts must run under Windows PowerShell 5.1 (no `&&`, `||`, `??`, `?:`, no `-AsHashtable`, no ternary) **and** `pwsh` 7 on Linux (no Windows-only cmdlets/paths in the module; `Join-Path` with two arguments only).
- Pester is not usable (5.1 ships Pester 3.4): tests use a tiny harness, `scripts/release/tests/run-tests.ps1`, exit code ≠ 0 on failure.
- Files are rewritten byte-faithfully: keep BOM / no BOM and CRLF / LF of each file as found (`Cargo.toml` is UTF-8 **with BOM, CRLF**; `apps/lc-overlay/tauri.conf.json` CRLF no BOM; `extension/public/manifest.json` LF no BOM). Use `[IO.File]::ReadAllBytes` / `WriteAllBytes` with explicit `UTF8Encoding($bom)`, never `Set-Content`.
- Version format accepted by `bump`: `^\d+\.\d+\.\d+$` only. Tag prefixes: `server-v`, `overlay-v`, `extension-v`. Tags with a prerelease suffix (`-beta.N`) and other prefixes (`legacy-ts-2.0.2`, `extension-latest`, `next`) are ignored when looking for the last release.
- Component files:
  - server: `Cargo.toml` `[workspace.package] version`; `Cargo.lock` `[[package]]` entries of every workspace member whose `Cargo.toml` has `version.workspace = true` (today lc-db, lc-discord, lc-domain, lc-media, lc-playback, lc-protocol, lc-server, xtask).
  - overlay: `apps/lc-overlay/tauri.conf.json` top-level `"version"`, `apps/lc-overlay/Cargo.toml` `[package] version`, `Cargo.lock` entry `lc-overlay`. `tauri.beta.conf.json` is never touched.
  - extension: `extension/public/manifest.json` and `extension/package.json` top-level `"version"`, `extension/package-lock.json` top-level `"version"` and `packages[""].version`.
- Change paths (for `plan`): server = `crates/`, `apps/lc-server/`, `apps/lc-admin-web/`, `xtask/`, `deploy/`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`; overlay = `apps/lc-overlay/`, `crates/lc-protocol/`, `crates/lc-domain/`; extension = `extension/`. Shared crates = `crates/lc-protocol/`, `crates/lc-domain/`.
- Unraid: SSH `"C:/Windows/System32/OpenSSH/ssh.exe" -o BatchMode=yes root@192.0.2.59`; container `LiveChat`; template `/boot/config/plugins/dockerMan/templates-user/my-LiveChat.xml`; data `/mnt/user/appdata/livechat`; backups `/mnt/user/appdata/livechat-backups/before-<X.Y.Z>-<yyyyMMdd-HHmmss>/`; image `ghcr.io/quentingimenez/livechat-server:<X.Y.Z>`; update `/usr/local/emhttp/plugins/dynamix.docker.manager/scripts/update_container LiveChat` then **`docker start LiveChat`**. Never print container env/secrets.
- `/health` (`https://livechat.example.com/health`) JSON: `version`, `protocolVersion`, `db`, `discord` (`"ok"` lowercase), `status`.
- Extension: no Chrome Web Store publishing (job removed).

## Review Focus

1. A bump on `Cargo.toml` (BOM + CRLF) or a CRLF JSON file must change only the version bytes — a diff of the whole file means every following CI check and review is noise. → Task 2 test `Bump keeps BOM and line endings`.
2. Tags list contains prereleases and foreign prefixes (`server-v3.1.0-beta.3`, `legacy-ts-2.0.2`, `extension-latest`): last release must be the highest final semver, compared numerically (`3.10.0` > `3.9.0`). → Task 1 tests `Last tag ignores prereleases` and `Semver compares numerically`.
3. Extension has no `extension-v*` tag: `plan` must list its whole history and `bump`/`pending-tags` must not crash. → Task 3 test `Plan without tag lists whole history`.
4. Push to `main` that is not a `release/*` merge, or a re-run on a merge whose tag already exists: nothing is tagged twice, nothing is tagged without the go. → Task 3 test `Pending tags skips existing tag` + Task 5 gate on the PR head branch.
5. Deploy fails after the container was stopped (pull ok, `update_container` fails, or `/health` never reports the new version): the container must be running again and the rollback command printed. → Task 4 test `Deploy script restarts container on error`.

---

### Task 1: Component model, version reading and `check`

**Files:**
- Create: `scripts/release/ReleaseLib.psm1`
- Create: `scripts/release.ps1`
- Create: `scripts/release/tests/run-tests.ps1`, `scripts/release/tests/Lib.Tests.ps1`

**Interfaces:**
- Produces (module exports):
  - `Get-ReleaseComponents -> string[]` = `@('server','overlay','extension')`
  - `Get-ComponentVersionSources -Root <string> -Component <string> -> object[]`, each `[pscustomobject]@{ File = <repo-relative path with '/'>; Label = <string, e.g. 'Cargo.lock lc-server'>; Version = <string> }`
  - `Test-VersionConsistency -Root <string> -> string[]` (empty = consistent; one message per mismatch, naming file and both versions)
  - `Compare-SemVer -A <string> -B <string> -> int` (-1/0/1, numeric per part)
  - `Get-LastReleaseTag -Root <string> -Component <string> -> pscustomobject|$null` with `Tag`, `Version`
  - `Get-ServerWorkspaceMembers -Root <string> -> string[]` (package names of members with `version.workspace = true`)
- `scripts/release.ps1 <command> [args]`: `check` prints `OK` or the messages and exits 1.
- Test harness: `run-tests.ps1` dot-sources every `*.Tests.ps1` next to it; helpers `Test-Case -Name <string> -Body <scriptblock>`, `Assert-Equal -Expected -Actual -Message`, `Assert-True`; prints `PASS`/`FAIL <name>: <message>` and exits with the failure count. Helper `New-TestRepo -> string` copies the real version files into a temp dir (under `[IO.Path]::GetTempPath()`) and runs `git init` there.

- [ ] **Step 1: Write the failing tests** in `Lib.Tests.ps1`:
  - `Reads versions of the real repo`: `Get-ComponentVersionSources` on the repo root returns, for server, a `Cargo.toml` source plus one `Cargo.lock` source per name in `Get-ServerWorkspaceMembers` (8 today, includes `lc-server`, `xtask`, excludes `lc-overlay`); for overlay 3 sources; for extension 4 sources.
  - `Consistent repo has no errors`: `Test-VersionConsistency` on a `New-TestRepo` copy returns `@()`.
  - `Detects a diverging file`: in the copy, change `extension/package.json` version to `9.9.9` → one message containing `extension/package.json` and `9.9.9`.
  - `Semver compares numerically`: `Compare-SemVer 3.10.0 3.9.0` = 1; `3.5.1` vs `3.5.1` = 0; `0.1.0` vs `0.2.0` = -1.
  - `Last tag ignores prereleases`: in a temp git repo with one commit and tags `server-v3.1.0-beta.3`, `server-v3.9.0`, `server-v3.10.0`, `legacy-ts-2.0.2`, `extension-latest` → server = `server-v3.10.0` / `3.10.0`; extension = `$null`.
- [ ] **Step 2: Run** `powershell -NoProfile -File scripts/release/tests/run-tests.ps1` — expected: failures (module missing).
- [ ] **Step 3: Implement** the exports above in `ReleaseLib.psm1` and the `check` command in `release.ps1` (`param([Parameter(Position=0)][string]$Command, [Parameter(ValueFromRemainingArguments)]$Rest)`; root = `git rev-parse --show-toplevel`). Read files as bytes → strip BOM → string; match versions with anchored regexes per the Global Constraints (TOML: first `version = "..."` inside the right table; JSON: the `"version"` key at top-level indentation and inside `"": {` for the lockfile).
- [ ] **Step 4: Run** the tests again — expected: all `PASS`, exit 0. Run `powershell -NoProfile -File scripts/release.ps1 check` — expected `OK`.
- [ ] **Step 5: Commit** `Add release version model and consistency check`.

### Task 2: `bump`

**Files:**
- Modify: `scripts/release/ReleaseLib.psm1`, `scripts/release.ps1`
- Create: `scripts/release/tests/Bump.Tests.ps1`

**Interfaces:**
- Consumes: Task 1 exports.
- Produces: `Set-ComponentVersion -Root <string> -Component <string> -Version <string> -> string[]` (repo-relative files written). Throws `invalid version` for a non `X.Y.Z` value and `version must be greater than <v>` when `Version` ≤ max(last release tag version, current file version). CLI: `release.ps1 bump <component> <X.Y.Z>`, then runs `check`.

- [ ] **Step 1: Write the failing tests** (all on `New-TestRepo` copies):
  - `Bump server writes every source`: bump server to `3.6.0` → every server source reads `3.6.0`, overlay sources unchanged (`lc-overlay` lock entry still its old version), `Test-VersionConsistency` empty.
  - `Bump keeps BOM and line endings`: after bumping server and overlay, `Cargo.toml` still starts with bytes `EF BB BF` and contains `\r\n` only (no lone `\n`); `tauri.conf.json` still CRLF without BOM; `manifest.json` still LF; byte length of each file differs from the original only by `len(new) - len(old)` per replaced occurrence.
  - `Bump extension updates the lockfile`: bump extension to `0.2.0` → `package-lock.json` top-level and `packages[""]` = `0.2.0`, and the dependency entries' versions are unchanged (count of `"version": "0.27.3"` identical before/after); `ConvertFrom-Json` still parses it.
  - `Bump refuses lower or equal`: server `3.5.1` → throws `greater`; `3.6` → throws `invalid version`.
- [ ] **Step 2: Run** the harness — expected: the 4 new tests fail.
- [ ] **Step 3: Implement** `Set-ComponentVersion`: regex replacement on the same anchors as Task 1, byte-faithful write (Global Constraints); `Cargo.lock` edited in place (only the `version = ` line right after `name = "<member>"`) — no `cargo` call.
- [ ] **Step 4: Run** the harness — expected: all `PASS`. Then, in the real worktree, run `powershell -NoProfile -File scripts/release.ps1 bump server 3.6.0`, then `cargo check -p lc-server --offline` (expected: lockfile accepted, no "lock file needs to be updated"), then `git checkout -- Cargo.toml Cargo.lock` to revert.
- [ ] **Step 5: Commit** `Add release version bump`.

### Task 3: `plan` and `pending-tags`

**Files:**
- Modify: `scripts/release/ReleaseLib.psm1`, `scripts/release.ps1`
- Create: `scripts/release/tests/Plan.Tests.ps1`

**Interfaces:**
- Consumes: Task 1 (`Get-LastReleaseTag`, `Get-ComponentVersionSources`).
- Produces:
  - `Get-ComponentChanges -Root <string> -Component <string> -> pscustomobject` with `Component`, `LastTag` (string or `$null`), `CurrentVersion`, `Commits` (`object[]` of `@{ Sha; Subject }`, newest first, from `git log <LastTag>..HEAD -- <paths>` or whole history when no tag), `Files` (`string[]`, `git diff --name-only <LastTag>..HEAD -- <paths>`; when no tag, files touched by the listed commits), `SharedCratesChanged` (bool; overlay only: shared crates changed since the last **server** tag, else `$false`).
  - `Get-PendingReleaseTags -Root <string> -BaseRef <string> -> object[]` of `@{ Component; Version; Tag }`: components whose version read at `HEAD` differs from the version read at `BaseRef` (via `git show <ref>:<file>`, first source file of the component) **and** whose tag does not exist yet.
  - CLI: `release.ps1 plan [-Json]` (text: one block per component with last tag, current version, commit count, commit list, flag line `shared crates changed since server-vX` when true; `-Json` = `ConvertTo-Json -Depth 5` of the array); `release.ps1 pending-tags <baseRef> [-Json]` (text: `<component> <tag>` per line).

- [ ] **Step 1: Write the failing tests** on a temp git repo built by `New-TestRepo` (initial commit, tag `server-v3.5.1` and `overlay-v3.5.0` on it):
  - `Plan lists commits since tag`: add a commit touching `apps/lc-overlay/web/x.js` → overlay `Commits.Count` = 1, `Files` contains that path; server `Commits.Count` = 0.
  - `Plan without tag lists whole history`: extension (no tag) → `LastTag` `$null`, `Commits.Count` = 2 after one extra commit touching `extension/src/a.ts`.
  - `Shared crates flag`: commit touching `crates/lc-protocol/src/lib.rs` → overlay `SharedCratesChanged` = `$true`.
  - `Pending tags after bump`: `$base = HEAD`; bump overlay to `3.6.0` and commit → `Get-PendingReleaseTags -BaseRef $base` returns exactly `overlay` / `3.6.0` / `overlay-v3.6.0`.
  - `Pending tags skips existing tag`: then create tag `overlay-v3.6.0` → returns `@()`.
  - `No bump no tag`: a commit that changes no version file → `@()`.
- [ ] **Step 2: Run** the harness — expected: new tests fail.
- [ ] **Step 3: Implement** the two functions and the CLI commands (call git with `-C $Root`; check `$LASTEXITCODE`).
- [ ] **Step 4: Run** the harness — all `PASS`; then `powershell -NoProfile -File scripts/release.ps1 plan` on the real repo — expected: server since `server-v3.5.1`, overlay since `overlay-v3.5.0`, extension with no tag.
- [ ] **Step 5: Commit** `Add release plan and pending tag detection`.

### Task 4: `deploy-server`

**Files:**
- Modify: `scripts/release/ReleaseLib.psm1`, `scripts/release.ps1`
- Create: `scripts/release/tests/Deploy.Tests.ps1`

**Interfaces:**
- Produces:
  - `Get-ServerDeployScript -Version <string> -Stamp <string> -> string` (bash, LF line endings, starts with `set -euo pipefail`).
  - `Invoke-ServerDeploy -Version <string> [-SkipBackup]` : pipes the script to `ssh ... bash -s` (stdin, not argument), then polls `https://livechat.example.com/health` every 5 s up to 120 s until `version -eq $Version`, `db -eq 'ok'`, `discord -eq 'ok'`; then runs the remote checks (below). Throws on any failure after printing the rollback line.
  - CLI: `release.ps1 deploy-server <X.Y.Z>`.
- Remote script steps, in this order: `docker pull <image>:<v>`; `docker stop -t 20 LiveChat`; set `STOPPED=1`; `trap` on ERR that runs `docker start LiveChat` when `STOPPED=1` and prints the rollback command; `mkdir` backup dir; `cp -a livechat.sqlite*` and the template into it; `cmp` each copy; `sed -i` the `<Repository>` line of the template to `ghcr.io/quentingimenez/livechat-server:<v>`; `update_container LiveChat`; `docker start LiveChat`; print `DEPLOYED <v> BACKUP <dir>`. Rollback line printed = `sed` back to the previous image tag (read from the template before editing, kept in `PREV_IMAGE`) + `update_container` + `docker start` + `cp` of the backup.
- Remote checks after health: `docker logs --since 5m LiveChat 2>&1 | grep -c 'Discord prêt'` ≥ 1; `/admin/` refused from outside: `curl -sk -o /dev/null -w '%{http_code}' --resolve livechat.example.com:4433:127.0.0.1 https://livechat.example.com:4433/admin/` = `403` (source 172.17.0.1 is not an allowed IP).

- [ ] **Step 1: Write the failing tests** on `Get-ServerDeployScript -Version 3.6.0 -Stamp 20261117-120000`:
  - `Deploy script order`: indices of `docker pull`, `docker stop`, `cmp`, `<Repository>`, `update_container`, `docker start LiveChat` are strictly increasing; contains `livechat-backups/before-3.6.0-20261117-120000`.
  - `Deploy script restarts container on error`: contains a `trap` whose handler contains `docker start LiveChat` and `PREV_IMAGE`.
  - `Deploy script has no CR and no secrets`: no `\r`; does not contain `docker inspect` or `Config.Env`.
  - `Deploy refuses invalid version`: `Get-ServerDeployScript -Version 'latest'` throws.
- [ ] **Step 2: Run** the harness — new tests fail.
- [ ] **Step 3: Implement** both functions and the CLI command.
- [ ] **Step 4: Run** the harness — all `PASS`. Then a read-only smoke on the host: `"C:/Windows/System32/OpenSSH/ssh.exe" -o BatchMode=yes root@192.0.2.59 "grep '<Repository>' /boot/config/plugins/dockerMan/templates-user/my-LiveChat.xml"` matches the `sed` pattern the script uses, and the `--resolve` curl above returns `403`. Do **not** run a real deploy in this task.
- [ ] **Step 5: Commit** `Add Unraid server deploy to the release tool`.

### Task 5: GitHub workflows

**Files:**
- Create: `.github/workflows/release-check.yml`, `.github/workflows/release.yml`
- Modify: `.github/workflows/extension.yml` (remove job `publish-webstore`; add in `build` a step, on `refs/tags/extension-v*` only, that fails when the tag differs from `extension/public/manifest.json` version)

**Interfaces:**
- Consumes: `release.ps1 check`, `release.ps1 pending-tags <baseRef> -Json`, the test harness.
- `release-check.yml`: on `pull_request` (all paths) and `push` to `main`; `ubuntu-latest`; `actions/checkout` pinned by SHA like the other workflows (reuse `11d5960a326750d5838078e36cf38b85af677262 # v4`) with `fetch-depth: 0`; steps `pwsh -NoProfile -File scripts/release.ps1 check` and `pwsh -NoProfile -File scripts/release/tests/run-tests.ps1` (needs `git config --global user.email/name` for temp repos).
- `release.yml`: on `push` to `main`; `permissions: contents: write, actions: write`; `concurrency: release-main`; checkout `fetch-depth: 0`. Steps:
  1. Gate: `head=$(gh api repos/$GITHUB_REPOSITORY/commits/$GITHUB_SHA/pulls --jq '.[0].head.ref')`; continue only if it starts with `release/` (else log `not a release merge` and exit 0).
  2. `pending=$(pwsh -NoProfile -File scripts/release.ps1 pending-tags "$GITHUB_SHA^1" -Json)`.
  3. For each entry: `git tag -a <tag> -m "<Component> <version>" $GITHUB_SHA`, `git push origin <tag>` (bot identity `github-actions[bot]`).
  4. Dispatch: server → `gh workflow run server-image.yml --ref <tag> -f version=<v> -f update_latest=true`; overlay → `gh workflow run overlay-tauri.yml --ref <tag>`; extension → `gh workflow run extension.yml --ref <tag>`. `GH_TOKEN: ${{ github.token }}`.

- [ ] **Step 1: Write** the three workflow changes.
- [ ] **Step 2: Validate syntax**: `npx --yes @action-validator/cli` is network-dependent — instead run `node -e "require('js-yaml')..."` only if `js-yaml` is in `node_modules`; otherwise `python -c "import yaml,sys;[yaml.safe_load(open(f)) for f in sys.argv[1:]]" <files>` — expected: no error. Check that `overlay-tauri.yml` and `extension.yml` release jobs use `startsWith(github.ref, 'refs/tags/...')` (true for a dispatch on a tag ref) and that `server-image.yml` dispatch path reads `inputs.version` (already the case: no change).
- [ ] **Step 3: Run** `pwsh` is not installed locally: run `powershell -NoProfile -File scripts/release.ps1 pending-tags HEAD~1 -Json` — expected `[]` (or empty) on the work branch.
- [ ] **Step 4: Commit** `Tag and build releases from merged release branches`.
- Not verifiable before merge: the real `release.yml` run (only on a `release/*` merge) and `pwsh` on Linux (verified by `release-check.yml` on this branch's PR).

### Task 6: `/release` skill and docs

**Files:**
- Create: `.claude/skills/release/SKILL.md` (frontmatter `name: release`, `description:` "Use when the user asks to release, ship or publish a new version of the LiveChat server, overlay or extension")
- Modify: `docs/rust/overlay-release.md` §7 (point to the new flow, drop the manual `gh release create` as the normal path), `README.md` (one "Releasing" section linking the skill and `scripts/release.ps1`)

**Content the skill must carry (French prose, commands in code blocks):**
- Flow from the spec's "Déroulé d'une release" (plan → proposal → go → branch `release/<summary>` from `main`, worktree if the tree is dirty → `bump` each → `check` → project tests (`cargo test --workspace`, extension `npm run ci:check` if extension bumped) → commit `Release <component> <version>[ and ...]` → push → PR with notes → merge when `gh pr checks` is green → wait for `release.yml` and the build runs (`gh run list`, `gh run watch`) → `deploy-server` → overlay `gh release edit overlay-vX --draft=false --latest` → report).
- Version rules verbatim from the spec (patch/minor/major, extension 0.x, highest wins), plus: a commit that only bumps another component's version does not count; flag `SharedCratesChanged` → propose a server release too; raising `DEFAULT_MIN_OVERLAY_VERSION` = major and announce the few-minute cut.
- The proposal format: per component `current → new (level) — reason`, then commit summary; wait for "go". The go covers commit, push, merge, tags, publish, deploy of that release only.
- Failure rules: red PR CI → fix on the release branch; red build after tag → no deploy, no publish, tell the user; deploy failure → script already restarted the container, report the printed rollback, overlay stays draft; never publish the overlay before the server health check passed.
- Never mark an extension or beta release Latest.

- [ ] **Step 1: Write** the skill and doc changes.
- [ ] **Step 2: Check** the skill is listed: start a new Claude Code session in the worktree, or verify the frontmatter parses (`---` block with `name` and `description`).
- [ ] **Step 3: Run** the full harness and `release.ps1 check` once more — all `PASS`, `OK`.
- [ ] **Step 4: Commit** `Add the release skill and document the release flow`.

### Final verification

- [ ] Whole harness under Windows PowerShell 5.1 green; `release.ps1 plan` output reviewed.
- [ ] Open the PR; `release-check.yml` green on Linux `pwsh` (proves 7.x compatibility).
- [ ] Not verifiable until the first real release: `release.yml` gate + dispatch, `deploy-server` end to end. The first release run with the skill is the end-to-end test; say so in the PR.
