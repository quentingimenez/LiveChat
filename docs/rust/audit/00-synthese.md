# Synthèse d'audit — LiveChat 3.0.0 (serveur Rust + overlay Tauri)

| Champ | Valeur |
|---|---|
| Date | 2026-11-10 |
| Commit audité | `e0a381f` (branche `feat/rust-rebuild`) — `fix(server): accept legacy ?token= fallback on /overlay/meme-board routes` |
| Périmètre | `apps/lc-server` (REST, WS, admin, ingest, média, DB, Discord), `apps/lc-overlay` (Tauri 2 + renderer), crates, CI/GHCR, workflows de release, `deny.toml`, `deploy/` |
| Hors périmètre | `server/` et `overlay/` legacy Node/Electron, `extension/`, pentest d'infrastructure réelle, chiffrement au repos, flotte d'overlays, charge > 100 clients |
| Méthode | 7 lanes d'audit indépendantes (A1–A7) + 1 vérificateur adversarial (V) + probes du Lead, en lecture seule : **aucun fichier de production modifié, aucun commit** |
| Baseline verte | `cargo fmt`, `cargo clippy -D warnings`, `cargo test --workspace` (230 tests serveur, 45 xtask, 15 protocole, doctests) |
| Livrables | `docs/rust/audit/01-…07-*.md`, `verification.md`, ce fichier, `backlog-corrections.md` |

## 0. Verdict

**L'ensemble est architecturalement cohérent et globalement propre sur le chemin nominal, mais il n'est PAS prêt pour une exposition publique en l'état.**

- Le découpage en crates tient (protocole gelé, domain/db, playback, media, discord, serveur, overlay) ; le protocole v2 est respecté dans le cas nominal (handshake, enveloppe, closes, unicast/broadcast, isolation de guilde), le contrat de schéma v3 est verrouillé par un test octet-pour-octet, et le code de production ne contient **aucun `unwrap`/`expect`/`panic`**.
- Mais l'audit a trouvé **3 défauts de criticité P0** : un **LFI/SSRF arbitraires** via les entrées média (`file:` et `http://` non filtrés, redirections suivies), le **migrateur legacy qui produit une base que le serveur refuse de démarrer** (ledger `_sqlx_migrations` absent), et un **gate CI/release cassé** (`cargo deny check` rouge, workflows de release contradictoires).
- S'y ajoutent **~20 findings majeurs** (jetons en query/journalisés, absence de plafonds WS, écritures meme-board ignorées en mode `INVITE_READ_ONLY`, écarts de contrat d'arrêt, confinement média fail-open, updater non signé…) et **~100 findings mineurs/info** de dette, robustesse et doc.

**Réponse à la question posée (« est-ce que tout est cohérent, est-ce que tout est propre ? »)** : oui pour l'architecture et le contrat nominal ; non pour la sécurité des entrées et la chaîne de release. Trois correctifs ciblés (≈ 1 à 2 jours-homme) font passer le risque résiduel de « bloquant » à « majeur gérable ».

## 1. Findings par lane (bruts, avant dédoublonnage)

Grille unifiée : Bloquant / Majeur / Moyen / Faible / Info (les lanes ont utilisé des libellés voisins : critique→Bloquant, haute/élevée→Majeur, moyenne→Moyen, basse/nit→Faible).

| Lane | Rapport | Findings | Bloquant | Majeur | Moyen | Faible | Info |
|---|---|---:|---:|---:|---:|---:|---:|
| A1 — protocole v2 & hub WS | `01-protocole-ws.md` | 14 | 0 | 5 | 7 | 2 | 0 |
| A2 — REST overlay/ingest | `02-rest-overlay-ingest.md` | 17 | 0 | 1 | 7 | 8 | 1 |
| A3 — admin | `03-admin.md` | 12 | 0 | 1 | 2 | 6 | 3 |
| A4 — pipeline média | `04-pipeline-media.md` | 18 | 0 | 2 | 7 | 8 | 1 |
| A5 — DB / concurrence / migrateur | `05-db-concurrence.md` | 21 | 1 | 4 | 7 | 8 | 1 |
| A6 — overlay Tauri | `06-overlay-tauri.md` | 18 | 0 | 3 | 6 | 7 | 2 |
| A7 — sécurité transversale | `07-securite-transversale.md` | 20 | 0 | 4 | 5 | 6 | 5 |
| V — vérificateur adversarial | `verification.md` | 6 | 2 | 3 | 1 | 0 | 0 |
| **Total** | | **126** | **3** | **23** | **42** | **45** | **13** |

Après consolidation des doublons inter-lanes (SSRF, LFI, jetons en query, confinement média fail-open), on obtient **≈ 118 problèmes distincts**. Le vérificateur V a confirmé indépendamment **23 des 24 findings majeurs/hauts** des lanes, réfuté 1 faux positif (A6-001 « board cassé »), et découvert **2 bloquants supplémentaires** (LFI, SSRF) ainsi que 3 majeurs (écritures `INVITE_READ_ONLY`, jetons loggés, plafonds WS).

## 2. P0 — bloquants (à corriger avant toute exposition)

### P0-1. LFI + SSRF arbitraires via les sources média (convergence A2/A4/A7/V)
**Réf.** `AUD-A2-001`, `AUD-A2-002`, `AUD-A4-001`, `AUD-A4-002`, `AUD-A7-001`, `AUD-V-001`, `AUD-V-002`.

- `POST /ingest` et `POST /overlay/meme-board/items` acceptent une URL arbitraire. `lc-media` ne filtre ni les schémas (`file:`, UNC/SMB) ni les IP (loopback, RFC1918, link-local `169.254.169.254`, multicast), et **suit les redirections sans revalidation** (`redirect::Policy::limited(10)`).
- PoC V : `file:C:\Windows\Web\Wallpaper\Windows\img0.jpg` → `201`, puis `GET /overlay/meme-board/media/{asset}` → `200`, 1 602 752 o, **SHA-256 identique** au fichier local. `http://127.0.0.1:3455/secret.jpg` → contenu d'un service interne exfiltré de la même manière. Lead : `file:///…/test.jpg` → `201 accepted`, `file:///C:/Windows/win.ini` → `422 INVALID_MEDIA` (oracle de format, pas de refus du schéma).
- Correctif : garde central dans `lc-media` (allowlist `http`/`https`, résolution DNS puis refus des IP internes, revalidation à chaque redirection, refus `file:`/UNC), + tests wiremock/loopback, + revue des deux routes. **Effort : 2–3 j-h.**

### P0-2. Le migrateur legacy produit une base qui ne démarre pas (A5-001)
**Réf.** `AUD-A5-001`, confirmé par V.

- `cargo run -p xtask -- migrate-legacy …` sort `0` mais la cible n'a **pas de table `_sqlx_migrations`** ; au boot, `Db::open` rejoue `0001_init` sur un schéma existant → `exit 1`, `table app_metadata already exists` (contredit le runbook `migrator.md:277`).
- Correctif : le migrateur écrit le ledger `_sqlx_migrations` (`version=1, description='init', checksum SHA-384`) avant le rename atomique ; acceptation : migrer → démarrer → `/health` `db:"ok"`. **Effort : 0,5 j.**

### P0-3. Écritures meme-board en mode `INVITE_READ_ONLY` (V-003)
- Création/PATCH/DELETE d'items acceptés avec un jeton `invite_read_only` (aucun contrôle `session_mode` sur `/overlay/meme-board/*`), alors que le mode est censé être restrictif.
- Correctif : garde `session_mode == NORMAL` sur toutes les écritures board ; test d'intégration. **Effort : 0,5 j.**

### P0-4. Gate CI/release cassé (convergence A7-002/A7-003 + baseline Lead)
- `cargo deny check` **échoue** : 4 licences hors allowlist (`target-lexicon` LLVM-exception, 3× `webpki-*` CDLA-Permissive-2.0), 7 « wildcards » (deps path workspace), 1 bug unresolved-workspace-dependency, 45 warnings de doublons.
- `cargo audit` non bloquant dans la CI ; actions GitHub non épinglées en SHA ; `--no-tests=pass` ; jobs Linux compilent Tauri sans deps système.
- Release : pubkey Tauri placeholder + `createUpdaterArtifacts:false`, aucun workflow Tauri, `overlay-release.yml` publie encore l'Electron `--latest`, `server.yml` et `server-image.yml` poussent la même image GHCR avec des tags qui se recouvrent.
- Correctif : `deny.toml` (licences + exception path-deps), `cargo audit` bloquant, SHA des actions, workflow Tauri réel (ou retrait de l'auto-update), nettoyage des workflows legacy, gates CI vertes. **Effort : 2–4 j-h.**

**Total P0 ≈ 6–9 j-h** (1–1,5 jour avec 2 agents parallèles + revue).

## 3. P1 — majeurs (à corriger avant exposition réseau large / release)

| Thème | Findings | Correctif attendu |
|---|---|---|
| Jetons dans les URLs et les logs | A3-002, A7-004, V-005, A6-001 | Supprimer `?token=` (admin + board), redacter la query dans `TraceLayer`, passer le jeton au board via le bridge/Bearer, retirer la réécriture d'URL de `app.js`. |
| Plafonds WS / DoS | A7-005, V-006, A2-002, A4-009 | Plafond sockets par client/guilde, validation d'`Origin`, `TimeoutLayer`, rate-limit admin/ingest par route, limite par client d'ingestion. |
| Contrat d'arrêt et jobs | A1-001, A1-002, A1-003 | `play` texte-seul émis, `stop {reason:"revoked"}` avant close 4402, purge avec raison dédiée (`AdminHub` étendu). |
| Concurrence DB | A5-003, A5-004, A5-005, A5-007 | Récupération des `.old` orphelins, index unique partiel `WHERE status='PLAYING'`, retry SQLITE_BUSY, purge non destructive des items épinglés. |
| Confinement / média | A2-003, A2-006, A4-007, A4-008, A4-014, A2-004 | Fail-closed du confinement, pipeline partagé (pas d'instanciation par requête board), stockage atomique, worker de purge, cap du TTL à la vérification, contrôle job/guilde sur le média signé. |
| Admin | A3-001, A3-003, A3-006 | Compose : `127.0.0.1:3000`, `ADMIN_UI_LOCAL_ONLY=true` par défaut, TLS/reverse-proxy documenté ; rate-limit admin ; confirmation renforcée des purges. |
| Overlay / chaîne de livraison | A6-002, A6-003, A6-005, A6-004 | Carte tweet : capabilities/IPC restreintes ou désactivation ; refus de `http://` (sauf flag explicite) ; updater signé ou désactivé ; `LIVECHAT_INSECURE_TLS` limité au dev. |

**Effort P1 ≈ 15–22 j-h** (3–4 jours à 4–5 agents).

## 4. P2/P3 — dette, robustesse, doc

- **A1** : trame > 256 Kio → close 1006 au lieu de 4408 ; `id` non validé ; liveness Pong ; course `ensure_runtime` ; `error` S→C jamais émis ; champs inconnus non conservés.
- **A2** : JSON malformé → `request_error` générique ; branche 409 morte ; O(N) board ; `?token=` contradictoire doc/code ; `deviceName` tronqué ; media signé sans binding job.
- **A3** : `Origin`/`Referer` absents sur mutations ; purge sans audit ; `AdminHub` global hors `AppState` ; labels de clients ingest non bornés ; copie d'UI trompeuse ; `ADMIN_UI_LOCAL_ONLY` inopérant derrière reverse proxy.
- **A4** : `svg+xml` non ré-encodé ; réponses non bornées ; options ffprobe via chemin local ; cookie TikTok sans échappement ; tests réseau tous `#[ignore]` ; cache par URL (contenu obsolète).
- **A5** : source mouvante (pas de snapshot) ; `Db::connect` ignore `?mode=ro` ; documentation du runbook à corriger.
- **A6** : overlay affiché même `enabled=false` ; config non atomique/partagée ; IPC communes aux 3 fenêtres ; `bridge-test.html` embarqué en release.
- **A7** : HSTS/COOP absents ; MSRV non testée ; sanitizer tweet contournable (dormant) ; doublons `windows_*`/`winnow`.
- **Divers** : `crates/lc-admin` est un crate vide (1 fichier, 5 lignes) — supprimer ou implémenter ; `docs/rust/server-overlay.md` contredit le correctif `?token=`.

## 5. Points forts confirmés

- Protocole v2 conforme au nominal : 23 tests `ws` + 37 probes (32 PASS), closes 4401/4402/4403/4408/4409 correctes, isolation de guilde, unicast/broadcast conformes.
- Auth : jetons hachés SHA-256, comparaison constant-time, ordre fail-closed 503→403→401 sur l'admin, aucune confiance X-Forwarded-For.
- Média signé : HMAC constant-time, binding `assetId|clientId|exp`, TTL clampé, `Range`/`206`/`416`/`HEAD`, confinement anti-traversée vérifié (404 + log), `Cache-Control: no-store`.
- DB : `claim_next` atomique (50 jobs / 8 threads → 0 double), consume pairing atomique, DDL identique octet-pour-octet au contrat v3, migrateur réellement read-only, verify comptes/sommes/échantillon/FK.
- Qualité : zéro `unwrap`/`expect`/`panic` en production, clippy `-D warnings` vert, erreurs normalisées JSON, CSP stricte, conteneur non-root.
- Overlay : WS v2 conforme (welcome/currentJob signé, heartbeat, closes), bridge 13 méthodes, 51 tests, transparence/click-through/topmost validés par POC.

## 6. Verdict go / no-go 3.0.0

| Contexte | Verdict |
|---|---|
| Test local / interne (loopback, jetons forts, ingest de confiance, admin local) | **GO** — validé E2E le 2026-11-10 (appairage, lecture, board, stop, reconnexion) |
| Réseau privé (LAN, /ingest exposé à l'extension) | **NO-GO** avant P0 (LFI/SSRF) + P1 (jetons, plafonds WS) |
| Release publique 3.0.0 (serveur + overlay) | **NO-GO en l'état** (P0 + gate CI/release) ; **GO** après P0 (≈ 1–2 j) puis P1 (≈ 3–4 j) et un re-audit ciblé des diffs + gates vertes |

## 7. Limites de cet audit

Lecture seule (aucun correctif appliqué) ; probes dynamiques sur Windows local uniquement, sans pentest d'infra ni charge > 100 clients ; `ffmpeg`/`yt-dlp` absents → transcodage réel et l'ingestion YouTube/TikTok/Twitter non rejoués ; Discord live non exercé (`discord:disabled`) ; installateurs Tauri non installés/désinstallés ; pas de test multi-écrans/DPI/HDR. Ces limites sont détaillées dans chaque rapport de lane et dans `verification.md`.

## 8. Suite proposée

1. Revue de cette synthèse et arbitrage des priorités.
2. Phase correctifs séparée (branchée sur `feat/rust-rebuild`) : P0 d'abord, tests de non-régression, gates vertes.
3. Re-audit ciblé des diffs P0/P1 par V (ou un subagent frais), puis réexécution des probes Lead (`/ingest`, média signé, WS, admin, rate-limit).
4. Décision release 3.0.0 ; en parallèle, actions opérateur déjà listées (goldens de parité v1/v2, smoke Docker, tag `legacy-ts-2.0.2`).
