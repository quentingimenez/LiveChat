# Correctifs P1 — AUD-COR-05 à AUD-COR-11 (2026-11-10)

Suite de l'audit 3.0.0 et de la vague P0 (`p0-correctifs.md`). Objectif : fermer les
7 correctifs majeurs avant toute exposition réseau large ou release publique.

- **Baseline** : `ab97b1c` (P0 clos), branche `feat/rust-rebuild`.
- **Méthode** : 5 lanes parallèles, write scopes stricts, aucun commit d'agent,
  Lead propriétaire de `app.rs`/`lib.rs`/`Cargo.toml` et de l'intégration.
- **Validation** : gates workspace + probes runtime rejouées par le Lead
  (`.phase0/p1/probe.py`, `.phase0/p1/probe.out.txt`).

## 1. Correctifs livrés

| ID | Lane | Livraison |
|---|---|---|
| COR-05 | infra-rust | Admin **Bearer-only** (`?token=` → 401), `app.js` sans token d'URL (localStorage uniquement), logs HTTP **path-only** (`request_path`, jamais query/Referer), audit-log mutations sans secret. |
| COR-06 | protocole-v2 + Lead | `Origin` allowlist `/ws` (env `WS_ALLOWED_ORIGINS`, défauts `API_URL` + `tauri.localhost` + `ADDITIONAL_CORS_ORIGINS` + loopback dev) → close **1008** `origin_not_allowed` ; plafonds **3 sockets/client**, **16/guilde** → close **4409** `too_many_connections` ; `TimeoutLayer` 30 s hors `/ws`, `/overlay/media/`, `/overlay/meme-board/`, `/ingest` → **408** `request_timeout`. |
| COR-07 | protocole-v2 | `play` émis et compté pour les jobs **texte-seul** (statut PLAYING, plus de FAILED) ; révocation → `stop {reason:"revoked"}` puis **4402** ; purge → `stop {reason:"purge",jobId:null}` via `AdminHub::broadcast_purge` (impl par défaut → `NoopAdminHub` inchangé). |
| COR-08 | http-core | Migration `0002_playback_single_playing` (index unique partiel `WHERE status='PLAYING'`, normalisation des doublons), retry `SQLITE_BUSY` (`with_busy_retry`), récupération `.old` du migrateur, purge non destructive du board épinglé, `mode=ro` strict. |
| COR-09 | db-core | Publication atomique des fichiers (tmp+fsync+rename), racine média **fail-closed**, pipeline partagé pour le board (plus de `MediaPipeline::new` par requête), worker de purge, TTL média plafonné à la vérification (`exp ≤ now + MEDIA_URL_TTL_MAX_SEC` → 403). |
| COR-10 | infra-rust | Rate-limit admin **429** (100 req/15 min/IP, 429 avant auth, ordre 503→403→401 sous budget), compose `127.0.0.1:3000:3000` + `ADMIN_UI_LOCAL_ONLY=true` par défaut, `.env.example` et docs alignés. |
| COR-11 | poc-tauri | Appairage/WS refusent `http://` par défaut (dérogation `LIVECHAT_INSECURE_TLS=1` limitée au debug), refus du downgrade `apiBaseUrl` https→http, carte tweet neutralisée (`DOMParser`/`textContent`, plus de script tiers, CSP `script-src 'self'`/`frame-src 'none'`), capabilities Tauri par fenêtre + `COMMAND_MATRIX`, updater explicitement désactivé pour 3.0.0, `bridge-test.html`/`README.md` sortis du bundle, board via `Authorization: Bearer`. |

## 2. Gates d'intégration

| Gate | Résultat |
|---|---|
| `cargo fmt --all -- --check` | **0** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **0** |
| `cargo test --workspace` | **668 passed / 0 failed** (39 binaires, doctests inclus) |
| `cargo deny check` | **0** (`advisories ok, bans ok, licenses ok, sources ok`) |
| Flake `lc-playback::store::claim_est_unique_pour_une_guilde` | corrigé (patch test-only : `Utc::now() + 1 s`) |

## 3. Probes runtime (17/17 PASS)

Exécution : serveur debug sur `127.0.0.1:3530`, base SQLite fraîche + seed
(`.phase0/p1/seed.py`), clients overlay A (révoqué en fin de scénario) et B (purge),
média `mov_bbb.mp4` (788 493 o), `NODE_ENV=production`.

| Probe | Attendu | Observé |
|---|---|---|
| `admin_query_token_rejected` | 401 | 401 |
| `admin_bearer_ok` | 200 | 200 |
| `media_exp_beyond_max_403` | 403 | 403 `media_url_invalid` (log plafond TTL) |
| `media_valid_200` | 200 | 200 (octets mp4, `no-store`) |
| `ws_evil_origin_refused` | close 1008 | 1008 `origin_not_allowed` |
| `ws_socket_{1..3}_welcome` | welcome | welcome ×3 |
| `ws_socket_cap_4409` | close 4409 | 4409 `too_many_connections` |
| `ws_text_only_play` | `play.media=null` | reçu, `media=None`, texte « Texte seul P1 » |
| `admin_revoke_200` | 200 | 200 `revoked:true` |
| `ws_stop_revoked_and_4402` | stop revoked + 4402 | `stop` puis close 4402 |
| `ws_purge_client_welcome` | welcome | welcome |
| `admin_purge_200` | 200 | 200 `purged:true` |
| `ws_stop_purge_reason` | `{reason:"purge",jobId:null}` | conforme |
| `admin_rate_limit_429` | 429 présent | 87×200 / 43×429 |
| `rate_limit_headers` | `limit=100`, `remaining=0`, `retry-after` | conformes |

## 4. Régression trouvée par les probes et corrigée

La première exécution a révélé qu'à chaque démarrage le worker de purge supprimait
un fichier média **référencé** (`media_valid_200` → 404 `media_not_found_on_disk`,
`orphan_files=1`). Cause : `purge_storage_files` comparait les `storage_path` bruts
de la base (`C:\...`, chemins relatifs importés du legacy) aux entrées de
`read_dir` héritant du préfixe verbatim `\\?\` de la racine canonicalisée ;
tout fichier non canonicalisé était donc considéré comme orphelin et supprimé
après la grâce d'1 h (risque de perte de données pour toute base migrée).

Correctif (`apps/lc-server/src/media/purge.rs`) : normalisation des deux côtés
avant comparaison (`normalized_path` = `canonicalize` avec repli sur le chemin
brut) + test de régression
`keeps_referenced_files_when_storage_path_is_not_canonical`. Rebuild, re-seed et
probe rejouée : **17/17 PASS**, fichier média intact (`media_present=True`).

## 5. Écarts et limites connus

- Auto-update overlay **désactivé** pour 3.0.0 (clé minisign réelle absente) ;
  activation = P2 release.
- Installation NSIS/upgrade non testés localement (VM recommandée).
- `cargo audit` bloquant configuré en CI (13 `--ignore` justifiés) mais non rejoué
  dans ce journal de gates locales (deny couvre le périmètre local).
- Plafond `peers=8` documenté dans `server-ws.md` (écart spec assumé).
- Re-audit ciblé des diffs P0/P1 (`verification-2.md`) et P2/P3 du backlog restent
  à planifier ; décision de release 3.0.0 après ces points.

## 6. Re-audit ciblé (V2) et correctifs associés

`verification-2.md` (deux vérificateurs indépendants + synthèse Lead) : 0 Bloquant,
0 Élevé résiduel. Trois Majeurs serveur corrigés dans la foulée — confinement de la
purge (symlinks S-001, chemins relatifs/cwd S-002, fichier partagé S-004), avec
tests de régression et harnais admin ajusté — et deux Faibles overlay (tray
updater O-001, scoping `clientToken` O-002). Reste `AUD-V2-S-003` (board hors
`MediaService`) en dette P2 `AUD-COR-13`. Gates finales : fmt/clippy/deny 0,
`cargo test --workspace` 0 échec.
