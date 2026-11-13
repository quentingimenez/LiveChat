# Backlog de corrections — audit LiveChat 3.0.0

Baseline : commit `e0a381f` (branche `feat/rust-rebuild`), audit du 2026-11-10.
Synthèse : `docs/rust/audit/00-synthese.md`. Références : `AUD-<lane>-<n>` = finding du rapport de lane, `AUD-V-<n>` = finding du vérificateur.

Ordre de travail conseillé : P0 (bloquants) → gates CI vertes → P1 (majeurs) → P2 (dette) → P3 (doc/nits).
Convention d'effort : XS ≤ 0,5 j-h, S ≤ 1 j-h, M 1–2 j-h, L > 2 j-h.

## P0 — bloquants (avant exposition publique / release)

### AUD-COR-01 — Garde SSRF/LFI centrale dans `lc-media`
- **Réf.** `AUD-A2-001`, `AUD-A2-002`, `AUD-A4-001`, `AUD-A4-002`, `AUD-A4-003`, `AUD-A7-001`, `AUD-V-001`, `AUD-V-002`.
- **Problème** : `POST /ingest` et `POST /overlay/meme-board/items` acceptent `file:` (lecture locale, exfiltration par URL signée) et `http(s)://` vers loopback/LAN/métadonnées, redirections suivies sans revalidation.
- **Fichiers** : `crates/lc-media/src/download.rs` (`download_http`, `download_file`), `resolve/mod.rs`, `source.rs` ; routes `apps/lc-server/src/http/ingest/submit.rs`, `apps/lc-server/src/http/overlay/meme_board.rs`.
- **Correctif** :
  1. Allowlist stricte des schémas : `http`/`https` uniquement (refus `file:`, `ftp:`, `data:`, UNC `\\host\share`).
  2. Résolution DNS puis refus des IP loopback/RFC1918/link-local/ULA/multicast/`0.0.0.0` ; `connect` sur l'IP validée (anti-rebinding).
  3. Redirections : `Policy::custom` avec revalidation schéma+hôte+IP à chaque saut (max 3).
  4. Limites : mb max avant écriture, timeout global, `Content-Length` contrôlé.
  5. Tests : wiremock/loopback (200 attendu en refus), `file:` rejeté, redirect vers loopback rejeté, test `#[ignore]` promu.
- **Effort** : M.
- **Acceptation** : `file:C:\Windows\win.ini` → 400/422 sans création d'asset ; `http://127.0.0.1:<trap>` → refus avant toute connexion ; redirect 302→loopback refusé ; suites `cargo test -p lc-media -p lc-server` vertes.

### AUD-COR-02 — Ledger `_sqlx_migrations` dans le migrateur
- **Réf.** `AUD-A5-001` (confirmé V).
- **Problème** : `xtask migrate-legacy` produit une base sans ledger → `lc-server` refuse de démarrer (`table app_metadata already exists`).
- **Fichiers** : `xtask/src/migrate/{target,verify,schema}.rs`, `crates/lc-db/src/lib.rs` (`MIGRATION_SQL`).
- **Correctif** : créer `_sqlx_migrations` dans la cible avec la ligne `version=1, description='init', success=1, checksum SHA-384 de MIGRATION_SQL, execution_time` ; VERIFY vérifie le ledger ; `--dry-run` compare aussi.
- **Effort** : XS.
- **Acceptation** : migration → `lc-server` démarre → `/health` `db:"ok"` ; test d'intégration migrator→boot.

### AUD-COR-03 — `session_mode` sur les écritures meme-board
- **Réf.** `AUD-V-003`.
- **Problème** : en `INVITE_READ_ONLY`, POST/PATCH/DELETE d'items board acceptés.
- **Fichiers** : `apps/lc-server/src/http/overlay/meme_board.rs`, `auth.rs` (exposer `session_mode` sur `AuthedOverlayClient`).
- **Correctif** : garde 403 `read_only_session` sur les mutations ; `GET` autorisé.
- **Effort** : XS.
- **Acceptation** : jeton `INVITE_READ_ONLY` → GET 200 / POST 403 ; jeton normal inchangé ; test d'intégration.

### AUD-COR-04 — Gate CI/release verte
- **Réf.** `AUD-A7-002`, `AUD-A7-003`, baseline Lead (`cargo deny check` exit 7).
- **Problème** : `cargo deny` rouge (licences LLVM-exception/CDLA, wildcards path-deps, bug unresolved-workspace-dependency), `cargo audit` non bloquant, actions non SHA, workflows legacy contradictoires, updater Tauri placeholder.
- **Fichiers** : `deny.toml`, `.github/workflows/rust.yml`, `server-image.yml`, `server.yml`, `overlay-release.yml`, `tauri.conf.json`/`tauri.beta.conf.json`, `apps/lc-overlay/scripts/release-build.ps1`.
- **Correctif** :
  1. `deny.toml` : autoriser `Apache-2.0 WITH LLVM-exception` et `CDLA-Permissive-2.0` ; passer `wildcards` à `allow` pour les deps path workspace (liste `exclude`/`skip`) ; corriger `unresolved-workspace-dependency` ; trier/ignorer les doublons `windows_*`/`winnow`.
  2. `cargo audit` bloquant ; actions GitHub épinglées SHA ; retirer `--no-tests=pass` ou scoper Linux ; installer les deps système Tauri sur Linux.
  3. Release : workflow Tauri (NSIS + minisign + `latest.json`) ou désactiver explicitement l'auto-update (pubkey réelle sinon) ; retirer `overlay-release.yml` legacy Electron ; dédupliquer les tags GHCR server.
- **Effort** : M.
- **Acceptation** : `cargo deny check` exit 0 en local et CI ; `cargo audit` exit 0 bloquant ; un workflow de release Tauri signe et publie un artefact de test.

## P1 — majeurs (avant exposition réseau large)

> ✅ **Vague 2 exécutée le 2026-11-10** : AUD-COR-05 à AUD-COR-11 corrigés
> (5 lanes, gates vertes, probes 17/17). Détails, preuves et régression purge
> corrigée : `p1-correctifs.md`. Reste : re-audit ciblé (`verification-2.md`)
> avant décision de release.

### AUD-COR-05 — Supprimer les jetons des URLs et des logs
- **Réf.** `AUD-A3-002`, `AUD-A7-004`, `AUD-V-005`, `AUD-A6-001` (volet fuite).
- **Correctif** : retirer `?token=` admin (Bearer uniquement, + cookie `HttpOnly` optionnel) ; côté board, injecter le jeton via `bridge.js` (header `Authorization`) au lieu de `buildAuthedUrl` ; redacter `uri`/`Referer` dans `TraceLayer` (ne logguer que le path) ; retirer la réécriture d'URL de `app.js`.
- **Fichiers** : `apps/lc-server/src/http/{admin,overlay}/auth.rs`, `app.rs`, `static/admin/app.js`, `apps/lc-overlay/web/{bridge.js,dist/renderer/board/*}`.
- **Effort** : M. **Acceptation** : grep logs = 0 token ; board fonctionne sans `?token=` ; tests adaptés.

### AUD-COR-06 — Plafonds WS / anti-DoS
- **Réf.** `AUD-V-006`, `AUD-A7-005`, `AUD-A2-002`, `AUD-A4-009`.
- **Correctif** : plafond sockets par client (ex. 3) et par guilde (ex. 16), refus close 4409/1013 documenté ; validation `Origin` sur `/ws` (allowlist configurable) ; `TimeoutLayer` global hors média ; rate-limit admin + ingest par route ; limite d'ingestion par client (concurrence + débit).
- **Effort** : M. **Acceptation** : 5 sockets même jeton → refus au-delà du plafond ; 26 heartbeats/s → 4409 (déjà) ; Origin evil → refus ; tests.

### AUD-COR-07 — Contrat d'arrêt et jobs texte-seul
- **Réf.** `AUD-A1-001`, `AUD-A1-002`, `AUD-A1-003`.
- **Correctif** : `play` émis sans média si `media == null` et compté dans `recipients` ; `stop {reason:"revoked"}` puis close 4402 ; purge avec raison dédiée (`AdminHub::broadcast_purge` ou `StopReason::Purge`) ; mettre à jour spec §6/§10/§13 si le plafond peers est conservé.
- **Effort** : S. **Acceptation** : job texte seul → `play` reçu et statut PLAYING ; révocation → `stop revoked` + 4402 ; tests ws.

### AUD-COR-08 — Invariants DB et récupération du migrateur
- **Réf.** `AUD-A5-003`, `AUD-A5-004`, `AUD-A5-005`, `AUD-A5-007`, `AUD-A5-012`.
- **Correctif** : index unique partiel `WHERE status='PLAYING'` (+ migration) ; retry SQLITE_BUSY dans `claim_next`/moteur ; récupération des `.old` orphelins au démarrage du migrateur ; purge non destructive des items épinglés ; `Db::connect` respecte `mode=ro` (refus d'écriture).
- **Effort** : M. **Acceptation** : tests de concurrence existants + nouveaux cas.

### AUD-COR-09 — Confinement média et pipeline partagé
- **Réf.** `AUD-A2-003`, `AUD-A2-004`, `AUD-A2-006`, `AUD-A4-007`, `AUD-A4-008`, `AUD-A4-014`.
- **Correctif** : fail-closed si la racine média ne se canonicalise pas ; cache/quotas/limites appliqués au board (pipeline partagé, pas de `MediaPipeline::new` par requête) ; stockage atomique (tmp+rename) et worker de purge ; TTL média plafonné à la vérification (`exp ≤ now + TTL_MAX`).
- **Effort** : M. **Acceptation** : probes A2/A4 rejouées.

### AUD-COR-10 — Durcissement admin et compose
- **Réf.** `AUD-A3-001`, `AUD-A3-003`, `AUD-A3-006`, `AUD-A7-005`.
- **Correctif** : `deploy/docker-compose.yml` bind `127.0.0.1:3000:3000` par défaut + `ADMIN_UI_LOCAL_ONLY=true` ; doc TLS/reverse-proxy ; rate-limit admin ; confirmation purge renforcée (ré-auth ou double saisie) ; audit-log minimal des mutations.
- **Effort** : S. **Acceptation** : compose par défaut non joignable LAN ; bruteforce admin → 429.

### AUD-COR-11 — Overlay : TLS, carte tweet, updater
- **Réf.** `AUD-A6-002`, `AUD-A6-003`, `AUD-A6-004`, `AUD-A6-005`.
- **Correctif** : appairage/WS refusent `http://` sauf `LIVECHAT_INSECURE_TLS=1` explicite (documenté dev) ; ne pas accepter `apiBaseUrl` http en downgrade ; capabilities Tauri minimales (une capability par fenêtre, exclure les commandes sensibles de l'overlay) et carte tweet désactivée/derrière flag tant que `tweet_card` non exposé ; updater signé (clé réelle) ou désactivé dans les artefacts 3.0.0.
- **Effort** : M. **Acceptation** : appairage http refusé par défaut ; exe release sans `bridge-test.html` ; test capabilities.

## P2 — dette et robustesse (post-release)

> Re-audit ciblé exécuté le 2026-11-10 (`verification-2.md`) : les Majeurs/findings
> résiduels P0/P1 ont été corrigés (confinement purge S-001/S-002/S-004, tray
> updater et scoping `clientToken` côté overlay). Reste notamment AUD-V2-S-003
> (board hors `MediaService`) ci-dessous.

| ID | Réf. | Action | Effort |
|---|---|---|---|
| AUD-COR-12 | A1-006…014 | Closes 1006→4408 pour trames > 256 Kio, validation `id`, liveness Pong 45 s, `error` S→C, champs inconnus conservés | S |
| AUD-COR-13 | A2-001…017 + V2-S-003 | JSON malformé → erreur dédiée, branche 409, O(N) board, doc `?token=`, `deviceName`, binding job du média signé ; **unifier l'ingestion board sur `MediaService`** (cache/quota/`record_failure`, nettoyage `forceRefresh`) | M |
| AUD-COR-14 | A3-004…012 | `Origin`/`Referer` sur mutations, `AdminHub` dans `AppState`, labels bornés, messages UI, `TRUST_PROXY_HEADERS`/local-only derrière proxy | S |
| AUD-COR-15 | A4-004…018 | `svg+xml` ré-encodé, réponses bornées, options ffprobe, cookie TikTok échappé, tests réseau réels, invalidation cache | M |
| AUD-COR-16 | A5-002, 006, 008…021 | Snapshot source de migration, WAL/`busy_timeout` doc, checksums, testsâ€¦ | M |
| AUD-COR-17 | A6-006…018 | Overlay `enabled=false`, config atomique/per-window, IPC par fenêtre, nettoyage bundle release | S |
| AUD-COR-18 | A7-005…020 | HSTS/COOP, MSRV testée, sanitizer tweet allowlist, doublons deps, audit-log | M |

## P3 — documentation / nits

- ~~Corriger `docs/rust/server-overlay.md` (le `?token=` board est accepté depuis `e0a381f`) et la décision `overlay-bridge.md` §8.4/§8.6~~ — fait (repli board documenté §4.1 + en-tête du tableau).
- ~~Corriger le runbook `migrator.md` (ledger, ordre stop/migrate/start)~~ — fait (P0/P2-D).
- ~~Documenter le plafond `peers` (8) ou changer la spec~~ — fait (protocol-v2 rév. 3, §6.4).
- ~~Supprimer ou implémenter `crates/lc-admin` (crate vide)~~ — fait (crate supprimé le 2026-11-10, workspace mis à jour).
- ~~Mettre à jour `docs/rust/migration-plan.md` §12.11 avec les résultats d'audit et les décisions de release~~ — fait (§12.11 smoke, §12.12 état P0→P2 et reste avant release).
- ~~Nettoyer `bridge-test.html`/`README.md` du bundle release~~ — fait (COR-11, `apps/lc-overlay/dev/`).

## Séquencement proposé (multi-agents)

1. **Vague 1 (2 agents, ≈ 1 j)** : AUD-COR-01 (SSRF/LFI, le plus gros) + AUD-COR-02/03 (migrateur, session_mode) en parallèle ; Lead : AUD-COR-04 (CI/release).
2. **Gates** : fmt/clippy/test/deny/audit verts + probes Lead rejouées (`/ingest` file/http loopback, WS, admin, rate-limit) + build release.
3. **Vague 2 (3–4 agents, ≈ 2 j)** : AUD-COR-05 à 11.
4. **Re-audit ciblé** des diffs P0/P1 (V ou subagent frais) → `verification-2.md`.
   *Exécuté le 2026-11-10 : 3 Majeurs serveur (confinement purge) + 2 Faibles
   overlay corrigés, gates rejouées ; AUD-V2-S-003 renvoyé en P2.*
5. Décision release 3.0.0 ; P2/P3 en dette documentée.
