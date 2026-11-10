# Correctifs P0 — statut après audit 3.0.0

Date : 2026-11-10 (branche `feat/rust-rebuild`, baseline d'audit `e0a381f`, HEAD docs `b132e0c`).

Ce document clôt les 4 bloquants identifiés par l'audit multi-agents
(`docs/rust/audit/00-synthese.md`) et vérifiés par le vérificateur adversarial
(`docs/rust/audit/verification.md`). Les correctifs P1/P2/P3 restent au backlog
(`docs/rust/audit/backlog-corrections.md`).

## 1. Garde SSRF/LFI (AUD-COR-01)

Références : `AUD-A2-001`, `AUD-A2-002`, `AUD-A4-001`, `AUD-A4-002`,
`AUD-A4-003`, `AUD-A7-001`, `AUD-V-001`, `AUD-V-002`.

Implémentation : nouveau module `crates/lc-media/src/guard.rs`, propagé à
`http.rs` (client gardé, redirections manuelles), `download.rs`, `source.rs`,
`ingest.rs`, `resolve/*` ; pré-validation dans
`apps/lc-server/src/http/ingest/submit.rs` avant création d'asset.

- allowlist stricte `http`/`https` ; refus `file:`, `gtts:`, `ftp:`, `data:`,
  UNC `\\host\share`, `//host/share`, URL relatives et URL à identifiants ;
- résolution DNS avant connexion, refus loopback, RFC1918, link-local
  (`169.254/16`, `fe80::/10`), ULA `fc00::/7`, multicast, unspecified,
  broadcast, `0.0.0.0/8`, CGNAT `100.64/10`, documentation, benchmarking ;
- connexion à l'IP validée (client épinglé, `no_proxy`) → anti-rebinding ;
- redirections max 3 sauts, revalidation complète schéma + hôte + IP ;
- corps texte/erreur bornés (4 Mio / 8 Kio).

Preuves :
- `cargo test -p lc-media -p lc-server` : lc-media 112 unit + 7 offline +
  `tests/ssrf_guard.rs` 5 tests + 1 doctest, lc-server 234 passed / 1 ignored,
  0 échec ;
- probe runtime (serveur debug frais, base copiée, leurre HTTP loopback) :
  `file:///C:/Windows/win.ini` → `422 {"code":"UNSUPPORTED_SOURCE",
  "error":"media_ingestion_failed"}` ; `http://127.0.0.1:3521/x.mp4` → 422
  « adresse IP interne refusée (boucle locale) » **sans aucune connexion
  reçue par le leurre** ; redirection 302 → loopback refusée ; compteur
  `media_assets` inchangé (10 → 10).
- Test « refus avant connexion » : `TcpListener` sentinelle, `accept()` →
  `WouldBlock` (aucune connexion acceptée).

## 2. Ledger `_sqlx_migrations` du migrateur (AUD-COR-02)

Référence : `AUD-A5-001` (critique, confirmé V).

Implémentation : `xtask/src/migrate/ledger.rs` (DDL exacte sqlx, checksum
SHA-384 de `lc_db::MIGRATION_SQL`, `write`/`verify`/`compare_existing`) ;
écriture dans la cible après les 7 tables et avant le rename atomique ;
contrôle pré-rename et en dry-run ; `lc-server` démarre sans rejeu de
`0001_init`.

Preuves : `cargo test -p xtask` 47 passed / 1 ignored, dont l'oracle
« registre compatible boot et deuxième boot » (checksum identique à sqlx réel,
boot 1 et boot 2 via `lc_db::Db::open`, `/health` `db:"ok"` équivalent) et
`registre_sqlx_absent_ou_corrompu_detecte_avant_rename`.

## 3. `session_mode` sur les écritures du meme board (AUD-COR-03)

Référence : `AUD-V-003`.

Implémentation : `AuthedOverlayClient::session_mode()` / `is_read_only()`
(`apps/lc-server/src/http/overlay/auth.rs`) ; `ensure_writable()` en première
instruction de `create_item` / `update_item` / `delete_item` →
`403 {"error":"read_only_session"}` avant toute validation ; `GET` autorisés ;
mode issu de la base, identique en Bearer et en repli `?token=` du board.

Preuves : 4 tests `session_mode_tests` (Bearer + `?token=`, normal vs invite) ;
`cargo test -p lc-server` 234 passed / 1 ignored.

## 4. Gate CI/release (AUD-COR-04)

Références : `AUD-A7-002`, `AUD-A7-003`, baseline Lead (`cargo deny check` exit 7).

Implémentation :
- `deny.toml` : allowlist licences + `Apache-2.0 WITH LLVM-exception` et
  `CDLA-Permissive-2.0` ; 12 advisories ignorées avec justification
  individuelle (chaîne serenity → rustls 0.22.4, Tauri Linux zbus /
  proc-macro-error, SQLite bundlé), `unused-ignored-advisory=warn` ;
  path-deps workspace hors contrôles bans (`include-path-dependencies=false`,
  `allow-wildcard-paths=true`, duplicates=warn) ;
- `.github/workflows/rust.yml` réécrit : 5 jobs (fmt, clippy, test, deny,
  audit), actions épinglées SHA, `cargo audit` bloquant (13 `--ignore`
  justifiés), deps système Linux Tauri installées ;
- release dédupliquée : suppression des workflows Electron
  `overlay-release.yml` / `overlay-nightly.yml`, auto-update overlay
  explicitement désactivé pour 3.0.0 (`createUpdaterArtifacts:false`,
  pubkey placeholder) et documenté ; `server.yml` = tests legacy seulement,
  `server-image.yml` seul propriétaire des tags GHCR.

Preuves : `cargo deny check` → exit 0 (`advisories ok, bans ok, licenses ok,
sources ok`) ; `cargo audit` exit 0 en local (1 warning glib non bloquant) ;
5 YAML valides (`yaml.safe_load`), actions en SHA-40, 29 blocs `run:` validés
`bash -n`.

## Gates et vérifications finales

| Vérification | Résultat |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo test --workspace` | exit 0 (0 échec) |
| `cargo deny check` | exit 0 |
| Probe runtime garde SSRF/LFI | 422, 0 connexion, 0 asset créé |
| `git status` | 33 fichiers modifiés / 3 nouveaux, hors docs d'audit |

## Décision

Les 4 bloquants P0 sont corrigés et vérifiés localement. Le verdict de l'audit
reste inchangé pour le reste : **GO local/interne**, **NO-GO réseau large et
release publique** tant que la vague P1 n'est pas traitée (jetons dans les
URLs/logs, plafonds WS/anti-DoS, plafond peers, updater signé, anti-rebinding
renforcé, re-audit ciblé).
