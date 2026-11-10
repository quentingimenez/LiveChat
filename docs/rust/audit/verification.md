# Audit V — vérificateur adversarial indépendant (task-34)

| Champ | Valeur |
|---|---|
| Rôle | Vérificateur indépendant / adversarial (lane V) — n'a écrit aucun code de production |
| Date | 2026-11-10 (Europe/Paris) |
| Branche | `feat/rust-rebuild` (arbre de travail non modifié par cette lane ; seuls `.phase0/audit/V/**` et ce document ont été créés) |
| Périmètre Phase 1 | `crates/lc-media/src/{ingest,download,source,http,transcode}.rs`, `apps/lc-server/src/http/overlay/{auth,media,meme_board}.rs`, `apps/lc-server/src/ws/{hub,session}.rs`, `apps/lc-server/src/app.rs`, `http/rate_limit.rs`, `http/ingest/**` |
| Périmètre Phase 2 | rapports `docs/rust/audit/01…07*.md` (findings Bloquants/Majeurs/Hauts ; Moyens vérifiés quand peu coûteux) |
| Méthode | revue statique ligne à ligne + reproductions dynamiques sur binaires `target/debug/{lc-server,xtask}.exe`, port 3444/3445, DB et stockage média isolés sous `.phase0/audit/V/` |
| Livrable | ce document uniquement ; aucun code modifié, aucun commit |

---

## 1. Synthèse exécutive

**Phase 1 — findings propres : 6 findings, dont 2 Bloquants, 3 Majeurs, 1 Moyen.**

| ID | Sévérité | Titre | Statut |
|---|---|---|---|
| AUD-V-001 | **Bloquant** | Lecture de fichier local arbitraire (`file:`) via `/ingest` et meme board, puis exfiltration par l'API média | **Confirmé (repro)** |
| AUD-V-002 | **Bloquant** | SSRF arbitraire (loopback/LAN/métadonnées, redirections suivies) via `/ingest` et meme board | **Confirmé (repro)** |
| AUD-V-003 | **Majeur** | Jeton `INVITE_READ_ONLY` autorisé à créer/modifier/supprimer des items meme board | **Confirmé (repro)** |
| AUD-V-004 | Moyen | Oracle d'existence de fichiers locaux via les erreurs 422 détaillées | Confirmé (repro) |
| AUD-V-005 | **Majeur** | Token en query string journalisé en clair à INFO (`TraceLayer`) + réinjecté dans l'URL/localStorage par `app.js` | **Confirmé (repro)** |
| AUD-V-006 | **Majeur** | Hub WS sans plafond de connexions par client/guilde, `broadcast_peers` O(N²), canaux non bornés | **Confirmé (repro)** |

**Phase 2 — 24 findings majeurs/hauts traités : 23 confirmés (dont 1 dormant et 1 « mécanisme seulement »), 1 réfuté.**

- Confirmés : A1-001…005 (5 Majeurs), A2-001/A2-002, A3-002/A3-003, A4-001/A4-002, A5-001…005, A6-002/A6-003, A7-001…005.
- **Réfuté (faux positif)** : A6-001 — l'affirmation « board fonctionnellement cassé contre le serveur v2 » est **fausse** : `?token=` **est** accepté sur `/overlay/meme-board/*` (dynamique : 200) ; seule la fuite du token en URL reste vraie (doublon A3-002/A7-004).
- Confirmé mais **dormant** : A6-002 (le serveur ne produit jamais de carte tweet : `tweet_card: None`).
- Confirmé **au mécanisme** (pas de bout-en-bout) : A5-005 (SQLITE_BUSY reproduit, arrêt du moteur vérifié statiquement).
- Les rapports 01 et 02 ont été publiés après la Phase 1 (02:27 et 02:32) et sont couverts en Phase 2.

Aucun finding Bloquant des rapports A1–A7 n'est resté sans vérification indépendante.

---

## 2. Phase 1 — findings propres AUD-V

### AUD-V-001 — Lecture de fichier local arbitraire (`file:`) et exfiltration — Bloquant

**Preuve statique**
- `crates/lc-media/src/source.rs:136-157` — `canonicalize_source_url` laisse passer tel quel tout chemin commençant par `file:` (ou `gtts:`) ; `classify_source` → `SourceKind::LocalFile`.
- `crates/lc-media/src/download.rs:62-82` — `SourceKind::LocalFile → download_local`.
- `crates/lc-media/src/download.rs:104-143` + `local_path_from_url:150-165` — `download_local` ouvre le chemin (`file:`, `file://`, `file:///`) via `fs::metadata`/`fs::copy` : **aucune racine autorisée, aucune canonicalisation, aucun confinement**.
- `crates/lc-media/src/transcode.rs:81-127` — `probe().await.ok()` (échec ffprobe ignoré) puis `MediaKind::Image → normalize_image` = `fs::copy(input, output)` : la reproduction fonctionne **sans ffmpeg**.
- Points d'entrée HTTP : `POST /ingest` (`http/ingest/submit.rs:89-130` → `MediaService::ingest` → `pipeline.ingest_url`) et `POST /overlay/meme-board/items` (`http/overlay/meme_board.rs:177-369` → `pipeline.ingest_url`).

**Reproduction** (`.phase0/audit/V/evidence/ingest-evidence.json`, `evidence/evidence.json`)
```
POST /ingest  Authorization: Bearer <ingest>   {"guildId":"999999999999999999",
  "url":"file:C:\\Windows\\Web\\Wallpaper\\Windows\\img0.jpg","saveToBoard":true}
→ 201 {"accepted":true,"created":true,"mediaAssetId":"5f29802a-…","savedToBoard":true}
GET /overlay/meme-board/media/5f29802a-…?token=<overlay>
→ 200, 1 602 752 octets, sha256 = a37491a5e479e191cfe547b484426037455ce8aacff7ab15f91925330c24864f
  (= sha256 du fichier local ; variante `file:///C:/…` : même résultat)
```
Repro meme board seul : `POST /overlay/meme-board/items {"url":"file:C:\\Windows\\Web\\Wallpaper\\Windows\\img0.jpg"}` → 200, asset `e2d0f321-…`, média 200 / 1 602 752 o, sha identique.

**Impact** : lecture arbitraire de fichiers locaux du serveur (secrets `.env`, DB SQLite, configuration, clés) et exfiltration via l'API média/board ; un jeton overlay **ou** ingest suffit, y compris `INVITE_READ_ONLY` (cf. AUD-V-003).

**Correctif** : allowlist de schémas (`http`, `https` uniquement) à l'entrée du pipeline ; si `file:` reste nécessaire (usage local/outillage), le restreindre à une racine explicite (`MEDIA_LOCAL_ROOTS`) avec canonicalisation + vérification du préfixe, refus des UNC/SMB ; tests unitaires `source.rs`/`download.rs` (schéma refusé, chemin hors racine refusé) + test d'intégration `/ingest` 422.

**Test de non-régression** : `POST /ingest {"url":"file:C:\\Windows\\win.ini"}` → 400/422 `unsupported_source` ; `POST /ingest {"url":"file:///etc/passwd"}` → idem ; chemin autorisé sous `MEDIA_LOCAL_ROOTS` → 201.

### AUD-V-002 — SSRF arbitraire (loopback, LAN, redirections) — Bloquant

**Preuve statique**
- `crates/lc-media/src/http.rs:15-27` — client reqwest sans filtre d'IP/hôte/DNS, `redirect::Policy::limited(10)`.
- `crates/lc-media/src/download.rs:390-505` — `download_http` fait `client.get(url)` sur n'importe quelle URL `http(s)`.
- `crates/lc-media/src/http.rs:80-91` — `resolve_short_url` (HEAD puis GET) suit les redirections.
- Aucune validation d'hôte dans `ingest.rs` ni dans `meme_board.rs::create_item`.

**Reproduction** (`evidence/evidence.json`, `logs/internal-requests.log`)
```
POST /overlay/meme-board/items {"url":"http://127.0.0.1:3455/secret.jpg"} → 200 created:true
GET  /overlay/meme-board/media/<asset> → 200, sha256 identique au fichier du service interne
POST /overlay/meme-board/items {"url":"http://127.0.0.1:3455/redirect"} → 200, redirection suivie → /secret.jpg
POST /ingest {"url":"http://127.0.0.1:3455/secret.jpg"} → 201 accepted
```
Journal du service interne factice : `GET /secret.jpg peer=127.0.0.1 ua=Mozilla/5.0 … Chrome/133.0.0.0 Safari/537.36` (UA reqwest du serveur) → la requête part bien du serveur.

**Impact** : accès à tout service interne non exposé (admin, bases, panels, métadonnées cloud), scan/DoS en aveugle, exfiltration de réponses vers le stockage média puis lecture via l'API board/média ; combiné à AUD-V-001, exfiltration locale.

**Correctif** : garde SSRF central appliqué à chaque URL et à **chaque saut** de redirection : résolution DNS puis refus des IP privées/loopback/link-local/multicast/`169.254.169.254`/IPv6 équivalentes ; désactiver les redirections par défaut ou les revalider (scheme + host + IP) ; allowlist d'hôtes optionnelle ; tests `wiremock` (redirection locale refusée, loopback refusé, IP publique autorisée).

### AUD-V-003 — `INVITE_READ_ONLY` écrit sur le meme board — Majeur

**Preuve statique** : `apps/lc-server/src/http/overlay/auth.rs:122-161` authentifie uniquement le token ; `meme_board.rs:177-369` (`create_item`), `:412-477` (`update_item`), `:480-528` (`delete_item`) ne contrôlent **jamais** `session_mode` (aucune référence à `InviteReadOnly` dans le fichier).

**Reproduction** (`evidence/evidence.json`) : avec le jeton guest `INVITE_READ_ONLY` :
```
POST   /overlay/meme-board/items           → 200 {"created":true,"item":{…}}   (item 51beb5fe-…)
PATCH  /overlay/meme-board/items/<itemId>  → 200 {"updated":true}
DELETE /overlay/meme-board/items/<itemId>  → 200 {"deleted":true}
```
**Impact** : escalade de privilèges : un overlay invité (lecture seule) peut injecter du contenu dans le board de la guilde, modifier ou supprimer les items existants (y compris ceux des autres overlays).

**Correctif** : refuser les méthodes non-GET (`POST`/`PATCH`/`DELETE`) du meme board si `session_mode == InviteReadOnly` (403 `read_only`), comme c'est déjà fait pour `stop.request`/`meme.trigger` côté WS ; test d'intégration guest → 403, normal → 200.

### AUD-V-004 — Oracle d'existence de fichiers locaux via les erreurs 422 — Moyen

**Reproduction** : `file:C:/Windows/definitely-not-there-auditv.png` → 422 `MEDIA_NOT_FOUND` « fichier local introuvable » ; `file:…icon.png` (621 o) → 422 `INVALID_MEDIA` « média normalisé vide ou trop petit » ; fichier existant et assez gros → 200. Les messages distinguent donc l'existence et la nature du fichier.

**Impact** : énumération de fichiers locaux (noms de secrets, chemins de configuration) et d'URLs internes (codes/erreurs différents), en complément d'AUD-V-001/002.

**Correctif** : erreur uniforme `media_ingestion_failed` / `unsupported_source` sans détail de chemin ni distinction existant/inexistant ; loguer le détail côté serveur uniquement.

### AUD-V-005 — Token en query journalisé en clair (INFO) et réinjecté dans l'URL — Majeur

**Preuve statique**
- `http/overlay/auth.rs:109-120` — `?token=` accepté pour `/overlay/meme-board/*` (Bearer sans limite de longueur, `MAX_TOKEN_LEN` 200 pour la query).
- `http/admin/auth.rs:10-12` — l'admin accepte aussi `?token=` (Bearer prioritaire).
- `static/admin/app.js:127-153` — lit `?token=` depuis l'URL, le stocke en `localStorage` (`livechat_admin_token`) et le réinjecte dans l'URL via `history.replaceState` (`syncTokenInUrl` : `searchParams.set('token', …)`).
- `apps/lc-server/src/app.rs` — `TraceLayer` journalise `uri=` complet à INFO.

**Reproduction** : serveur isolé, `GET /overlay/meme-board/items?token=<64 hex>&limit=1` → 200 ; le log serveur contient :
`INFO http_request{method=GET uri=/overlay/meme-board/items?token=<TOKEN COMPLET>&limit=1 …}: tower_http::trace::on_request`.
**Impact** : fuite de `ADMIN_UI_TOKEN`/tokens overlay via logs serveur, proxies, historique navigateur, `localStorage` (XSS admin) ; un token overlay fuité permet SSRF (AUD-V-002), board, WS ; un token admin fuité permet stop/purge/révocation.

**Correctif** : supprimer l'acceptation `?token=` (Bearer uniquement) pour l'admin **et** le board ; redacter/`uri` sans query dans `TraceLayer` ; `app.js` ne doit pas réinjecter le token dans l'URL ; test : requête avec `?token=` → 401, log sans token.

### AUD-V-006 — Hub WS sans plafond de connexions par client/guilde, broadcast O(N²) — Majeur

**Preuve statique** : `ws/hub.rs:123-155` (`register` : aucune limite par client/guilde), `:214-259` (`broadcast_peers` → un `peers` par connexion à chaque register/unregister), `mpsc::UnboundedSender<OutFrame>` par connexion (`:244-259`).

**Reproduction** (`evidence/ws.json`, `evidence/ws-rss.json`) : 100 connexions avec le **même** jeton : 99 acceptées (100e → HTTP 429 du rate-limit HTTP global, pas d'un plafond WS) ; la première connexion reçoit 101 trames (welcome + settings + 99 `peers`) ; RSS serveur +5,9 Mo pour 99 sockets ; `elapsed=10 s`.
**Impact** : DoS mémoire/CPU par un client authentifié (ou un jeton fuité) : flood de connexions, amplification O(N²) des trames `peers`, saturation des canaux non bornés ; le rate-limit HTTP 100/15 min/IP ne protège que par effet de bord et est exempté en développement/loopback.

**Correctif** : plafond de connexions par `client_id` (ex. 4) et par guilde (ex. 16), refus 4429/close 4409 au-delà ; borner les canaux (`mpsc::channel`) et le nombre de pairs diffusés ; rate-limit WS dédié (par client/IP) ; test de charge (N connexions → refus propre, RSS borné).

---

## 3. Phase 2 — vérification indépendante des rapports A1–A7

### 3.1 Tableau de synthèse

| AUD-id | Sévérité rapport | Statut V | Preuve indépendante |
|---|---|---|---|
| A1-001 | Majeur | **Confirmé** | dynamique : job texte-seul `media=NULL` → aucune trame `play`, statut `FAILED` (`.phase0/audit/V/evidence/ws-behavior.json`) ; statique `hub.rs:732`, `engine.rs:662-667` |
| A1-002 | Majeur | **Confirmé** | dynamique : revoke 200 → close `4402 revoked`, `stopFrameCount=0` (`ws-behavior.json`) ; statique `clients.rs:56`, `hub.rs:522-536` |
| A1-003 | Majeur | **Confirmé (statique)** | `admin/guilds.rs` purge → `AdminHub::broadcast_stop` → `hub.rs:545 stop_with_reason(Manual)` ; la spec §6.3 attend `purge` |
| A1-004 | Majeur | **Confirmé** | dynamique : 10 clients **distincts** connectés → dernier `peers` tronqué à **8** (`evidence/ws-peers.json`) ; statique `hub.rs:185-210` (MAX_PEERS=8) ; non documenté §6.4 |
| A1-005 | Majeur | **Confirmé (statique)** | `hub.rs:437-447` : `tokio::spawn(run_engine.run(...))` + `tracing::error!` seulement ; `engine.rs:509-527` `tick().await?` ; aucun redémarrage/supervision |
| A2-001 | Haute | **Confirmé** | idem AUD-V-001 (repro `/ingest` + meme board) |
| A2-002 | Moyenne | **Confirmé** | idem AUD-V-002 (repro loopback + redirection) |
| A3-002 | Élevée | **Confirmé** | dynamique : token complet dans `uri=` du log INFO ; statique `auth.rs:109-120`, `static/admin/app.js:127-153` |
| A3-003 | Moyenne | **Confirmé (statique)** | `http/rate_limit.rs` : `is_exempt` = `/admin` ou loopback en dev ; aucune limite par route |
| A4-001 | Élevée | **Confirmé** | idem AUD-V-002 (repro `/ingest` 201, service interne sollicité) |
| A4-002 | Élevée | **Confirmé** | idem AUD-V-001 (repro `/ingest` + `saveToBoard` → 201, média 1 602 752 o, sha identique) |
| A5-001 | **Critique** | **Confirmé** | `xtask migrate-legacy` exit 0 ; cible = 8 tables sans `_sqlx_migrations` ; `lc-server` **exit 1** (stderr « table app_metadata already exists ») ; cible mutée (WAL, ledger vide) avant crash (`evidence/a5-repro.json`) |
| A5-002 | Haute | **Confirmé (best effort)** | writer concurrent pendant `xtask migrate-legacy` → exit 0, **aucun warning** (source mouvante non détectée) |
| A5-003 | Haute | **Confirmé** | crash simulé (rename de la cible en `sim.sqlite.old`) → nouvelle cible 151 552 o, orphelin `sim.sqlite.old` 159 744 o laissé |
| A5-004 | Haute | **Confirmé** | insertion directe de 2 jobs `PLAYING` même guilde acceptée (`jobs_playing=2`) → invariant non garanti par la DB |
| A5-005 | Haute | **Confirmé (mécanisme)** | `BEGIN IMMEDIATE` concurrent → `SQLITE_BUSY:database is locked` après 5,42 s ; arrêt moteur vérifié statiquement (`engine.rs` `tick().await?`, pas de redémarrage) |
| A6-001 | Haute | **Réfuté (partie « board cassé v2 »)** ; fuite token confirmée (doublon A3-002/A7-004) | dynamique : `POST /overlay/meme-board/items?token=…` → 200, `GET …media?token=…` → 200 ; Bearer → 200 aussi ; `auth.rs:109-120` accepte la query |
| A6-002 | Haute | **Confirmé (dormant)** | code latent : `legacy-tweet-card-renderer.js:55,78 innerHTML = model.html`, `legacy-twitter-widgets-utils.js:32-35` charge `platform.twitter.com/widgets.js` ; serveur ne produit jamais de carte tweet : `apps/lc-server/src/media/assets.rs:349 tweet_card: None` (seul autre : test `ws/tests.rs:882`) |
| A6-003 | Haute | **Confirmé (statique + partiel dynamique)** | `apps/lc-overlay/src/net/pairing.rs:112-113` accepte `http`/`https` ; `:148` lit `api_base_url` (downgrade) ; `pair.rs` renvoie `apiBaseUrl` ; appairage HTTP réel réussi en repro |
| A7-001 | Élevé | **Confirmé** | idem AUD-V-002 |
| A7-002 | Élevé | **Confirmé** | `cargo deny check` → **exit 7** : `advisories FAILED, bans FAILED, licenses FAILED, sources ok` (RUSTSEC-2026-0372 `zvariant_derive` via zbus/tauri-plugin-single-instance ; wildcards `*` dans les crates workspace ; licence `Apache-2.0 WITH LLVM-exception` de `target-lexicon` non autorisée) ; `rust.yml` contient un job `deny` bloquant (cargo-deny-action@v2) → gate ne peut pas être vert |
| A7-003 | Élevé | **Confirmé (statique)** | `tauri.conf.json:60 createUpdaterArtifacts:false`, `:74-75` endpoint GitHub `latest.json`, `:77` pubkey placeholder ; `overlay-release.yml` = Electron/electron-builder sur tags `overlay-v*` ; **aucun workflow Tauri** (workflows : extension, overlay-ci, overlay-nightly, overlay-release, rust, server-image, server) |
| A7-004 | Élevé | **Confirmé** | idem AUD-V-005 (dynamique + statique) |
| A7-005 | Moyen | **Confirmé** | `TimeoutLayer` absent (count 0) ; admin exempté du rate-limit ; WS sans plafond (repro AUD-V-006) |

### 3.2 Faux positifs écartés

1. **A6-001 (a) « board fonctionnellement cassé contre le serveur v2 » — RÉFUTÉ.** Le rapport affirme que le serveur v2 est « Bearer-only, `?token=` ignoré » en citant `docs/rust/server-overlay.md:18,95,399`. Le code réel (`apps/lc-server/src/http/overlay/auth.rs:109-120`) accepte explicitement `?token=` pour tout chemin commençant par `/overlay/meme-board/`, avec une limite de 200 caractères. Vérifié dynamiquement : `POST /overlay/meme-board/items?token=<64 hex>` → 200 ; `GET /overlay/meme-board/media/<asset>?token=<64 hex>` → 200 ; contrôle Bearer → 200 également. La partie (b) du finding — fuite du token dans les URLs — reste vraie et est confirmée (doublon A3-002/A7-004). **La doc `server-overlay.md` citée est donc en contradiction avec le code, pas l'inverse.**
2. **A2-002 (SSRF) classé « Moyenne » — désaccord de sévérité, pas faux positif.** La reproduction démontre une exfiltration complète de service interne via l'API média ; en contexte de production (réseau interne/cloud), nous la classons Bloquante (AUD-V-002). A4-001 la classe Élevée ; A7-001 Élevé. La sévérité « Moyenne » du rapport A2 est sous-évaluée.
3. **A1-004 : plafond `peers` à 8 — confirmé, non faux positif.** La spec `protocol-v2.md` §6.4 ne documente effectivement aucun plafond (recherche `peers`/`MAX_PEERS` dans la spec) ; la troncature est dans `hub.rs:185-210`.

### 3.3 Précisions sur des findings « confirmés au mécanisme »

- **A5-002** : la repro n'a pas provoqué d'échec de migration (le writer insère pendant la migration, `xtask` exit 0 sans warning). Le finding est confirmé dans son scénario « source mouvante non détectée » ; l'absence d'échec ne le réfute pas (le rapport note « seulement alertée » ; nous n'avons observé aucune alerte).
- **A5-005** : le `SQLITE_BUSY` est reproduit (5,42 s). La mort effective du moteur (arrêt définitif après `?`) n'a pas été forcée bout-en-bout ; elle est vérifiée statiquement (`engine.rs:509-527` : `tick().await?` ; `hub.rs:437-447` : spawn + `tracing::error!`, pas de redémarrage).
- **A6-002** : code latent confirmé, mais non déclenché : aucune source serveur ne remplit `tweet_card` (uniquement `None` dans `media/assets.rs:349` et les tests).

---

## 4. Limites de cette vérification

- Les reproductions tournent en **loopback**, en **mode développement** (exemption rate-limit loopback) et sans TLS : les résultats démontrent les failles logiques, pas leur exploitabilité via Internet (nécessite un token overlay/ingest valide, ce qui est le modèle de menace des findings).
- Pas de test cloud metadata (`169.254.169.254`) : le service interne factice `127.0.0.1:3455` suffit à prouver l'absence de filtre ; le comportement serait identique sur toute IP privée.
- Les findings Moyens/Bas non listés au §3.1 n'ont pas été rejoués (périmètre Phase 2 = Bloquants/Majeurs/Hauts ; Moyens vérifiés quand peu coûteux : A2-002, A3-003, A7-005).
- A5-002/A5-005 confirmés au mécanisme (pas d'E2E complet), A6-002 dormant (non déclenchable avec le serveur actuel), A1-003 confirmé statiquement seulement (pas de purge dynamique rejouée).
- Les sondes de charge restent locales et modestes (99–100 sockets) ; elles démontrent l'absence de plafond, pas une saturation du serveur en production.
- Aucune installation/désinstallation, aucun test d'upgrade, aucun test multi-écrans ou GPU.
- Les rapports A1–A7 ont été écrits par d'autres lanes ; cette lane n'a modifié ni leurs fichiers ni le code audité.

---

## 5. Artefacts et commandes de reproduction

Répertoire isolé : `.phase0/audit/V/` (ne touche pas `.phase0/local-test/`). Serveurs isolés : ports **3444** (DB `.phase0/audit/V/server/`) et **3445** (DB `.phase0/audit/V/server2/`), service interne factice **3455**.

| Artefact | Rôle |
|---|---|
| `seed_v.py` | crée la guilde `999999999999999999` + clients NORMAL et INVITE_READ_ONLY (token_hash SHA-256) |
| `internal_server.py` | service interne factice 127.0.0.1:3455 (`/secret.jpg`, `/redirect`, journal des requêtes) |
| `probe_v.py`, `run_repro.ps1` | reproduction LFI/SSRF meme board → `evidence/evidence.json`, `db.json`, `internal-requests.log` |
| `seed_ingest.py`, `probe_ingest.py`, `run_ingest_probe.ps1` | reproduction LFI/SSRF via `POST /ingest` → `evidence/ingest-evidence.json` |
| `ws_probe.py`, `run_ws_probe.ps1` | DoS WS (100 sockets même jeton, O(N²), RSS) → `evidence/ws.json`, `ws-rss.json` |
| `ws_peers_probe2.py`, `run_behavior_probe2.ps1` | plafond peers 8 avec 10 clients distincts → `evidence/ws-peers.json` |
| `ws_behavior_probe.py`, `run_behavior_probe2.ps1` | A1-001 (texte-seul) + A1-002 (révocation sans stop) → `evidence/ws-behavior.json` |
| `run_a5_repro.ps1`, `run_a5_repro2.ps1`, `a5_invariant.py`, `a5_busy.py`, `writer.py`, `inspect_target.py` | A5-001 (ledger absent → exit 1), A5-002/003/004/005 → `evidence/a5-repro.json`, `a5/` |
| `logs/` | logs serveurs isolés, service interne, `deny-check.log` |
| `query_jobs.py`, `schema_q.py`, `q2.py` | inspections DB ponctuelles |

Commandes (PowerShell 5.1, `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"`) :
```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .phase0\audit\V\run_repro.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .phase0\audit\V\run_ingest_probe.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .phase0\audit\V\run_ws_probe.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .phase0\audit\V\run_behavior_probe2.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .phase0\audit\V\run_a5_repro2.ps1
cargo deny check    # A7-002, exit 7 attendu
```
Chaque script nettoie ses processus et vérifie `0` process `lc-server` résiduel ; les serveurs sont démarrés avec `DATABASE_URL`/`MEDIA_STORAGE_DIR`/`ADMIN_UI_TOKEN`/`MEDIA_URL_SECRET` pointant sous `.phase0/audit/V/`.

---

## 6. Recommandations priorisées (pour le Lead / Phase 4)

| Priorité | Finding(s) | Action |
|---|---|---|
| **P0** | AUD-V-001, AUD-V-002 (=A2-001/A2-002, A4-001/A4-002, A7-001) | garde SSRF/LFI central dans `lc-media` : allowlist schémas, résolution + refus IP privées/loopback/métadonnées, revalidation à chaque redirection, confinement `file:` ; tests wiremock + intégration `/ingest` et meme board |
| **P0** | A5-001 (Critique) | le migrateur `xtask` doit écrire le ledger `_sqlx_migrations` (ou marquer la cible comme migrée) ; test E2E migrate → start |
| **P0** | AUD-V-003 | contrôle `session_mode` sur les écritures meme board (403 `read_only`) |
| **P1** | AUD-V-005 (=A3-002/A7-004) | suppression de `?token=` (admin + board), redaction de la query dans les logs, `app.js` sans token en URL |
| **P1** | AUD-V-006 (=A7-005 partiel) | plafonds de connexions WS par client/guilde, canaux bornés, rate-limit WS dédié |
| **P1** | A1-001, A1-002, A1-003 | payload `play` texte-seul émis + comptage recipients ; `stop {reason:revoked}` avant close 4402 ; raison `purge` sur purge |
| **P2** | A5-003, A5-004, A5-005 | snapshot/rename atomique + récupération `.old` ; contrainte « un seul PLAYING par guilde » ; redémarrage moteur sur erreur DB |
| **P2** | A6-001 (fuite), A6-003 | board en Bearer (plus de token en query) ; exiger `https`/`wss` (ou opt-in explicite documenté) pour l'appairage et le WS |
| **P2** | A7-002, A7-003 | `cargo deny` vert (licences/wildcards), `cargo audit` bloquant, workflow de release Tauri + clé updater réelle |
| **P3** | A6-002, A1-004, A1-005 | durcissement webview (pas de `innerHTML` sur HTML de carte tweet, ou sanitisation) ; documenter/plafonner `peers` ; supervision du moteur |
