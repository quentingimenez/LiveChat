# AUDIT A7 — Sécurité transversale & hygiène

> Audit indépendant, lane A7, réalisé sur le workspace Rust (`feat/rust-rebuild`).
> **Aucune exploitation active**, aucun code modifié, aucun commit.

| Champ | Valeur |
|---|---|
| Auditeur | `protocole-v2` (A7) |
| Date | 2026-11-10 |
| Commit audité | `e0a381f` (`feat/rust-rebuild`) — *fix(server): accept legacy `?token=` fallback on `/overlay/meme-board` routes* |
| Périmètre | `crates/*`, `apps/*`, `xtask`, `.github/workflows`, `deny.toml`, `deploy/.env.example`, `deploy/*`, `static/admin`, `apps/lc-overlay/web` (config/updater/CSP), `docs/rust` |
| Hors périmètre | `server/` legacy Node, `overlay/` Electron legacy, `extension/`, exploitation dynamique/pentest, infra réellement déployée, chiffrement au repos SQLite, flotte d'overlays |
| Méthode | Revue statique + lecture ciblée, `cargo deny check`, `cargo audit`, `cargo tree -i`, greps, inspection config/CI. Aucun build workspace complet (contrainte de lane), aucun test d'intrusion. |
| Logs | `.phase0/audit/A7/cargo-deny.txt`, `.phase0/audit/A7/cargo-audit.txt` (UTF-16 pour le premier) |
| Références | `docs/rust/migration-plan.md` §2 (S1–S6), `docs/rust/admin-panel.md`, `docs/rust/overlay-bridge.md`, `docs/rust/lc-overlay-8b.md`, `docs/rust/packaging.md`, `docs/rust/audit-dynamique.md` (legacy npm), `docs/rust/audit/04-pipeline-media.md` (lane média, non recouvrant) |

## 0. Résumé exécutif

Le portage Rust a corrigé plusieurs risques majeurs du legacy (tokens hachés au repos, HMAC média en temps constant, CSP stricte côté serveur, SQL 100 % paramétré, zéro `unwrap`/`expect`/`panic` en production, conteneur non-root). **La sécurité transversale reste néanmoins non livrable en l'état** : les quatre constats bloquants ci-dessous sont démontrables statiquement et touchent la chaîne release, l'authentification et la surface réseau.

**Top 3 risques (P0) :**

1. **SSRF non filtrée** dans tout le pipeline média (`/ingest`, meme board, résolveurs) : un token overlay/ingest donne accès à toute URL `http(s)` — loopback, RFC1918, métadonnées cloud, avec redirections suivies. C'est le S1 de `migration-plan.md` resté ouvert. → **AUD-A7-001**
2. **CI de sécurité cassée et non bloquante** : `cargo deny check` échoue (licences, wildcards), `cargo audit` n'est pas bloquant, actions non épinglées, `--no-tests=pass`, et les jobs Linux compilent l'app Tauri sans ses dépendances système. Le gate « rust.yml » ne peut pas être vert. → **AUD-A7-002** (+ **AUD-A7-011**)
3. **Canal de release ambigu et updater non opérationnel** : `overlay-release.yml` publie encore l'overlay Electron en `--latest`, aucun workflow Tauri n'existe, deux workflows poussent la même image GHCR `livechat-server` avec des tags `server-v*` qui se recouvrent, et la clé de signature de l'updater est un placeholder. → **AUD-A7-003**

Quatrième bloquant : **tokens en query et journaux** — `?token=` accepté par l'admin et le meme board, réinjecté dans l'URL/localStorage, et l'URI complète est journalisée à INFO par `TraceLayer`. → **AUD-A7-004**

**Compteurs : 4 Élevé · 5 Moyen · 6 Faible · 5 Info (20 findings).**

## 1. Synthèse des risques legacy S1–S6

| Réf | Constat legacy (migration-plan §2) | Statut v3 | Preuve / finding |
|---|---|---|---|
| S1 | SSRF via `/ingest` (URL arbitraire, redirections, DNS) | ❌ **Ouvert** | AUD-A7-001 |
| S2 | Tokens overlay acceptés en `?token=` | ⚠️ **Partiel** : média signé HMAC ✔ ; admin `?token=` et board preview `?token=` subsistent | AUD-A7-004 |
| S3 | TLS désactivé côté overlay (`rejectUnauthorized: false`) | ⚠️ **Atténué** : rustls strict par défaut, mais escape hatch `LIVECHAT_INSECURE_TLS` | AUD-A7-010 |
| S4 | Admin : token en query, CSP `unsafe-inline`, HTML inline 96 Ko | ⚠️ **Partiel** : assets statiques + CSP strictes ✔ ; token query/localStorage et absence d'audit-log mutations restent | AUD-A7-004, AUD-A7-019 |
| S5 | CORS `origin: true` + credentials | ✅ **Corrigé** (origines explicites, `allow_credentials` absent) ; réserve `tauri.localhost` permanent | AUD-A7-013 |
| S6 | Rate limit global, exemptions admin/localhost, pas de limite `/ingest` | ⚠️ **Partiel** : 100 req/15 min/IP ; **admin totalement exempté**, pas de limite par route, map non bornée | AUD-A7-005 |

## 2. Définitions

- **Sévérité** : impact métier/technique si exploité (Élevé > Moyen > Faible > Info).
- **Confiance** : certitude de l'auditeur sur le constat (certaine / haute / moyenne / basse).
- Chaque finding indique *preuve* (`fichier:ligne` + commande), *impact*, *correctif*, *test* et *effort* (j-h).

---

## 3. Findings P0 — bloquants release

### AUD-A7-001 — SSRF non filtrée dans le pipeline média

- **Sévérité** : Élevé · **Confiance** : certaine
- **Preuve** :
  - `crates/lc-media/src/source.rs:101` (`MediaSource::from_raw`) et `:253-277` (`classify_source`) : seuls `http`/`https` sont acceptés, **aucun filtrage d'hôte/IP** (loopback, RFC1918, link-local, ULA, multicast, `169.254.169.254`, DNS rebinding).
  - `crates/lc-media/src/http.rs:14-19` : `reqwest` avec `redirect::Policy::limited(10)` — les redirections sont suivies **sans revalidation de destination**.
  - Points d'entrée authentifiés : `apps/lc-server/src/http/ingest/submit.rs:42-207` (`POST /ingest`, champ `url`/`media` arbitraire) ; `apps/lc-server/src/http/overlay/meme_board.rs:176-260` (`create_item`, `url` arbitraire d'un client overlay).
  - `crates/lc-media/src/download.rs:389-470` (`download_http`, plafond de taille + rejet `text/html`) et `:340-386` (branche yt-dlp via `process::run_command`, args tableau — pas d'injection shell, bon point).
- **Commande** : `Get-ChildItem crates,apps -Recurse -Filter *.rs | Select-String -Pattern 'from_raw|classify_source|redirect::Policy'`
- **Impact** : un client overlay (token potentiellement fuité, cf. AUD-A7-004) ou un client ingest peut faire requêter au serveur le réseau interne (services d'admin, SQLite via API locale, métadonnées cloud, panels non exposés), scanner/DoS en aveugle, et exfiltrer des réponses vers un stockage média (puis lecture via `/overlay/media/...` signé). Les tailles/type MIME limitent mais n'empêchent pas l'exfiltration.
- **Correctif** : guard SSRF central dans `lc-media` avant toute requête : résolution DNS, rejet des IP privées/loopback/link-local/multicast/unspecified ; revalidation **à chaque redirection** (interdire ou re-résoudre) ; plafond de redirections ; timeout court ; option d'allowlist d'hôtes en production ; tests unitaires avec résolveur injecté + `wiremock`. Alternative court terme : désactiver les schémas non-`https` publics et l'accès aux plages privées par configuration.
- **Test** : fixtures `wiremock` simulant 127.0.0.1 / 10.0.0.1 / 169.254.169.254 / redirection vers loopback → doivent être refusées avec code d'erreur média dédié ; test DNS rebinding avec résolveur mocké.
- **Effort** : 2–3 j-h.

### AUD-A7-002 — Gate CI de sécurité cassé / non bloquant

- **Sévérité** : Élevé · **Confiance** : certaine
- **Preuve** (sorties complètes : `.phase0/audit/A7/cargo-deny.txt`, `.phase0/audit/A7/cargo-audit.txt`) :
  - `cargo deny check` → **`advisories FAILED, bans FAILED, licenses FAILED, sources ok`** (exit ≠ 0).
    - Licences refusées (4) : `target-lexicon 0.12.16` (`Cargo.toml:33`, `Apache-2.0 WITH LLVM-exception` — variante absente de `deny.toml:17-28`) ; `webpki-root-certs 1.0.11`, `webpki-roots 0.26.11`, `webpki-roots 1.0.11` (`CDLA-Permissive-2.0`) — idem.
    - Bans : **7 erreurs `wildcard`** — deps workspace path sans `version` (`Cargo.toml:85-91`) : lc-db ×2, lc-discord ×2, lc-media ×2, lc-overlay ×2, lc-playback ×3, lc-server ×6, xtask ×1, incompatibles avec `[bans] wildcards = "deny"` (`deny.toml:38`). S'y ajoutent **8 `bug[unresolved-workspace-dependency]`** (probable interaction cargo-deny 0.20.2 ↔ deps workspace `{ workspace = true, features = [...] }`).
    - **45 warnings `duplicate`** (base64 ×3, getrandom ×3, hashbrown ×3, syn ×3, toml_edit ×3, windows-sys ×4, rustls ×2, rustls-webpki ×2, tungstenite ×2, tokio-tungstenite ×2, reqwest ×2, thiserror ×2, etc.) — `multiple-versions = "warn"` (`deny.toml:36`).
  - `.github/workflows/rust.yml` : job `deny` (`:92-99`, action `EmbarkStudios/cargo-deny-action@v2`) donc **toujours rouge** ; job `audit` `continue-on-error: true` (`:102-108`) → jamais bloquant ; `--no-tests=pass` (`:87`) masque l'absence de tests ; actions épinglées par tag (`actions/checkout@v4`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, `taiki-e/install-action@nextest`, `EmbarkStudios/cargo-deny-action@v2`) et **non par SHA**.
  - Jobs Linux (`:80-90`) : `cargo nextest/clippy --workspace --all-features` compile `apps/lc-overlay` (Tauri 2, GTK/WebKit) **sans installer les dépendances système Linux** (aucun `apt-get install` dans `rust.yml`) → jobs ubuntu probablement rouges (à confirmer par un run CI réel ; plusieurs workflows legacy existent encore).
- **Impact** : le gate supply-chain n'est pas fiable ; une vulnérabilité ou une licence non conforme peut entrer en release ; l'équipe s'habitue à un CI rouge.
- **Correctif** : (1) compléter `deny.toml` (ajouter variantes SPDX justifiées ou `exceptions`) ; (2) ajouter `version` aux deps workspace path (ou passer `wildcards = "allow"` pour les path-deps, décision Lead) ; (3) traiter/clarifier les `unresolved-workspace-dependency` ; (4) rendre `audit` bloquant ou ouvrir un ticket d'ignorés documentés ; (5) épingler toutes les actions par SHA ; (6) installer les deps Tauri Linux dans les jobs ou exclure `apps/lc-overlay` des jobs Linux (plateforme Windows pour cet app) ; (7) supprimer `--no-tests=pass` une fois la suite fournie.
- **Test** : CI doit être verte sur `feat/rust-rebuild` puis rouge sur un lockfile contenant une advisory (test de non-régression du gate).
- **Effort** : 1–2 j-h.

### AUD-A7-003 — Canal de release ambigu : updater placeholder + workflows legacy concurrents

- **Sévérité** : Élevé · **Confiance** : certaine
- **Preuve** :
  - `apps/lc-overlay/tauri.conf.json:77` : `"pubkey": "RWQPLACEHOLDER_MINISIGN_PUBLIC_KEY_A_REMPLACER_EN_PHASE_9="` → **updater non vérifiable/non fonctionnel** ; `:60` `createUpdaterArtifacts: false` ; `:74-75` endpoint `https://github.com/quentingimenez/LiveChat/releases/latest/download/latest.json`, alors que `docs/rust/lc-overlay-shell.md:213` mentionne un autre owner (`Babz-Discord`) et qu'aucun workflow Tauri ne produit d'artefacts.
  - `.github/workflows/overlay-release.yml` (legacy Electron) : tag `overlay-v*` (`:6`), `contents: write` (`:10`), build electron-builder (`:106-113`), crée/édite la release `--latest` (`:123-135`) et publie `latest.yml`/blockmap (`:162-163`) → **canal de mise à jour de l'overlay v3 mélangé avec l'Electron legacy**.
  - `.github/workflows/server.yml` (legacy Node) publie **la même image** `ghcr.io/<owner>/livechat-server` (`:83-97`, contexte `./server`) que `.github/workflows/server-image.yml` (Rust, `:32`, `:75-99`, `deploy/server.Dockerfile`), avec des tags `server-v*` qui se recouvrent (`server.yml:15` et `server-image.yml:8-10`) → **écrasement/ambiguïté des artefacts de release**.
- **Impact** : un utilisateur (ou l'updater) peut recevoir un binaire non signé/legacy ou une image Docker d'un autre pipeline ; impossibilité de prouver l'intégrité de mise à jour ; risque de rollback impossible.
- **Correctif** : générer/committer la vraie clé minisign (Phase 9), `createUpdaterArtifacts: true`, workflow Tauri release dédié (`tauri-action`), désactiver ou renommer `overlay-release.yml`/`server.yml` (legacy) avant cutover ; renommer l'image legacy ou supprimer son push GhCR ; un seul propriétaire de tag par artefact.
- **Test** : dry-run release sur tag de préprod ; vérifier signature `latest.json`, hash de l'installeur et que l'updater refuse une signature invalide.
- **Effort** : 1–2 j-h (hors génération de clé/Phase 9).

### AUD-A7-004 — Tokens en query + journalisation des URI complètes

- **Sévérité** : Élevé · **Confiance** : certaine
- **Preuve** :
  - Serveur : `apps/lc-server/src/app.rs:186-187` (`TraceLayer`) et `:229-235` (`make_span` — `uri = %request.uri()`) → **toute URI, query incluse, est émise en span INFO** (niveau par défaut info via `init_tracing`) ; `apps/lc-server/src/http/error.rs:53-105` journalise aussi `method/uri/status` sur les erreurs non-JSON.
  - Admin : `apps/lc-server/src/http/admin/auth.rs:122-156` accepte `Authorization: Bearer` **puis** `?token=` (décodé form-urlencoded, trimé) ; `:167` comparaison temps constant. `static/admin/app.js:127-134` réinjecte le token dans l'URL (`history.replaceState`), `:141-144,149-156` le lit depuis `?token=` puis le persiste dans `localStorage['livechat_admin_token']`, `:170-178` l'envoie en Bearer.
  - Overlay : `apps/lc-server/src/http/overlay/auth.rs:109-130` accepte `?token=` **uniquement pour `/overlay/meme-board/*`** ; commit `e0a381f` l'a introduit pour compatibilité legacy. Le board construit encore `?token=` pour les previews : `apps/lc-overlay/web/dist/renderer/board/legacy-preview-media-utils.js:24-27` ; `docs/rust/overlay-bridge.md:398,448` exige des URLs média **verbatim** ; `docs/rust/legacy-ts-removal.md:121` affirme « Bearer only (`?token=` supprimé) » → **contradiction documentaire**.
- **Mitigations déjà en place** : `Referrer-Policy: no-referrer` global (`http/security.rs`), tokens stockés hachés SHA-256 (`docs/rust/lc-db.md:173`, `overlay/auth.rs`), comparaisons temps constant, `web/bridge.js:156` masque les clés sensibles dans ses logs.
- **Impact** : fuite de `ADMIN_UI_TOKEN`/tokens overlay via logs serveur, proxy, historique navigateur, `localStorage` (XSS admin) ; si l'admin token fuit → contrôle total (stop/purge/revoke) ; si un token overlay fuit → SSRF (AUD-A7-001), meme board, WS.
- **Correctif** : supprimer le fallback query côté admin (bootstrap par fragment `#token=` + POST one-shot ou cookie HttpOnly) ; supprimer le fallback board (renderer : URL verbatim + Bearer) ; journaliser `path()` seul (jamais `uri()` avec query) et redacter les en-têtes sensibles ; ne jamais persister le token en `localStorage` si cookie HttpOnly possible ; aligner la doc.
- **Test** : test de non-régression « aucun `?token=` journalisé » (capture de logs), test `query_token` refuse/absent, test du bundle board sans token en URL.
- **Effort** : 1–2 j-h.

---

## 4. Findings P1 — à traiter avant/avec la bêta

### AUD-A7-005 — DoS : timeouts absents, admin non limité, WS non plafonné

- **Sévérité** : Moyen · **Confiance** : certaine
- **Preuve** : `apps/lc-server/src/app.rs:186-215` (couches trace/compression/CORS/sécurité, **aucun `TimeoutLayer`** ni limite de connexions ; seul `DefaultBodyLimit::max(1 Mio)` `:201`, constante `:63`) ; `apps/lc-server/src/http/rate_limit.rs:46` (`ADMIN_PATH_PREFIX = "/admin"`), `:225-228` (`is_exempt` : tout `/admin` exempté, plus loopback en dev), `:230-240`/`:258-267` (IP = `ConnectInfo`, ou premier `X-Forwarded-For` si `TRUST_PROXY_HEADERS=true` — spoofing si proxy mal configuré), `:109-120` purge opportuniste d'un `HashMap` **non borné** en entrées ; `apps/lc-server/src/ws/mod.rs:68-75` + `session.rs:91-100,113` : WS protégé par token, 64 Kio/trame et 20 msg/s **par connexion**, hello 5 s — mais **aucun plafond global/par guilde de connexions** ni validation `Origin`/`Host` ; `apps/lc-server/src/http/health.rs:110-146` expose version/uptime/activeJobs/discord sans auth.
- **Impact** : slowloris/header lent non couvert ; bruteforce `ADMIN_UI_TOKEN` illimité (le seul facteur limitant est le token lui-même) ; saturation mémoire par IP variées/spoofées ; flood de connexions WS depuis un réseau autorisé ; SSRF (AUD-A7-001) peut occuper le pipeline d'ingestion.
- **Correctif** : `TimeoutLayer`/`RequestBodyLimitLayer` + timeouts de headers (via proxy) ; rate-limit administré sur `/admin` (au moins auth) et par route (`/ingest` plus strict) ; compteur borné (LRU) avec purge périodique ; plafond global + par guilde de sockets WS ; valider `Origin` (allowlist API_URL/tauri.localhost/overlay) et `Host` si proxy ; health public minimal ou protégé en option.
- **Test** : tests de charge legers (10k requêtes admin → 429), ouverture >N sockets, requête sans `Origin` refusée sur `/ws` en prod, test headers lents via tower::Service.
- **Effort** : 1–2 j-h.

### AUD-A7-006 — Secret tiers (cookie TikTok) renvoyé en clair au navigateur admin

- **Sévérité** : Moyen · **Confiance** : certaine
- **Preuve** : `apps/lc-server/src/http/admin/runtime_settings.rs:63-67` (`GET /admin/api/runtime-settings` → `"tiktokCookie": settings.tiktok_cookie` en clair, plus `hasTikTokCookie`) ; `:88-94` (PATCH renvoie la valeur) ; `:147` lit `Config::tiktok_cookie` ; persistance fichier runtime settings.
- **Impact** : un cookie de session TikTok (secret tiers) est exposé à toute compromission du navigateur admin (XSS/extensions), aux logs proxy, aux captures ; permet l'usurpation TikTok de l'opérateur.
- **Correctif** : GET masqué (`hasTikTokCookie` + longueur/préfixe seulement), PATCH set-only (ne jamais réémettre), stockage chiffré/secret manager, rotation documentée ; envisager de traiter ce cookie comme `Secret` (Debug redacted déjà existant).
- **Test** : test API GET ne contenant pas la valeur ; PATCH → réponse 204 sans écho ; grep de non-régression sur la sérialisation.
- **Effort** : 0,5 j-h.

### AUD-A7-007 — Surface Tauri : token en clair remis au renderer, plugin fs large, harness embarqué, CSP large

- **Sévérité** : Moyen · **Confiance** : haute
- **Preuve** :
  - `apps/lc-overlay/src/config.rs:104-116` : `OverlayConfigView` contient `client_token: Option<String>` ; `apps/lc-overlay/src/commands.rs:134-142` : `overlay_get_config` renvoie la vue au renderer (les webviews overlay/board/pairing reçoivent donc le token en clair).
  - `apps/lc-overlay/capabilities/default.json:5-18` : `core:default`, `fs:default`, `fs:allow-read-text-file`, `fs:allow-write-text-file`, scope `$APPDATA/LiveChat/**`, `$APPCONFIG/**/*`, `$APPLOCALDATA/**/*` (lecture/écriture texte au-delà du seul fichier de config) ; `apps/lc-overlay/src/lib.rs:189` `tauri_plugin_fs::init()` ; commentaire `default.json:3` : commandes applicatives **non ACL-gated**.
  - `apps/lc-overlay/tauri.conf.json:10` `withGlobalTauri: true` ; `:54` CSP large : `img-src … http: https:`, `media-src … http: https:`, `connect-src … http: https: ws: wss:`, `script-src 'self' https://platform.twitter.com` ; `:7` `frontendDist: "web"` → **tout `apps/lc-overlay/web/` est embarqué, y compris le harnais de dev `web/bridge-test.html` (25 470 o, suivi par git)**.
  - `apps/lc-overlay/web/dist/renderer/overlay/legacy-tweet-card-renderer.js:55,78` rend du HTML tweet via `innerHTML` (cf. AUD-A7-008).
- **Impact** : combinaison (XSS même limitée + token lisible par JS) → vol de token overlay, appels API arbitraires, écriture de fichiers dans le profil, exfiltration réseau large ; le harnais de test exposé augmente la surface.
- **Correctif** : ne pas exposer `client_token` au renderer (le bridge Rust fait les appels authentifiés ; le renderer board utilise déjà des URLs signées v2) ; réduire le scope fs au strict nécessaire (ou supprimer le plugin fs si les commandes Rust couvrent les besoins) ; retirer `bridge-test.html` du bundle (déplacer hors `web/` ou l'exclure) ; restreindre CSP (`connect-src` à API_URL/WS réels, `img-src`/`media-src` à l'origine API + twitter, `script-src 'self'`) ; `withGlobalTauri: false` si le bridge le permet.
- **Test** : test d'intégration Tauri (ou revue) : `overlay_get_config` ne contient aucun token ; scan du bundle `beforeBuildGeneratedAssets` sans `bridge-test.html` ; test CSP via `tauri build` + devtools.
- **Effort** : 1–2 j-h.

### AUD-A7-008 — Sanitizer tweet maison contournable (défense en profondeur CSP)

- **Sévérité** : Moyen · **Confiance** : moyenne
- **Preuve** : `crates/lc-media/src/html.rs:322-325` : `sanitize_embed_html` = suppression naïve des blocs `<script>` et `<iframe>` (`remove_tag_blocks`, recherche insensible à la casse) — **aucune allowlist de balises/attributs** (attributs `on*`, `<svg onload>`, `<object>`, `<style>`, URLs `javascript:` non traités) ; utilisé par `crates/lc-media/src/resolve/twitter.rs:71-74,126` sur le HTML oEmbed, rendu par `innerHTML` côté overlay (`web/dist/renderer/overlay/legacy-tweet-card-renderer.js:55,78`).
- **Mitigation existante** : CSP overlay sans `unsafe-inline` pour `script-src` → handlers inline et `javascript:` bloqués ; `frame-src` limité à Twitter ; risque résiduel = injection de HTML/CSS/inline styles, contenu trompeur, et exfiltration **si** un contournement CSP apparaît (le `connect-src` très large (http(s)/ws(s)) aggrave l'impact).
- **Correctif** : sanitizer robuste côté serveur (`ammonia`/`scraper` allowlist : balises `blockquote/p/a/img/br/strong/em/span`, attributs `class/href/src/alt`, schémas `http(s)`), ou rendu en `iframe sandbox srcdoc` avec CSP dédiée ; supprimer le sanitizer maison (tests existants conservés comme cas de base).
- **Test** : corpus de payloads XSS (`<img src=x onerror=alert(1)>`, `<svg onload=…>`, `<a href="javascript:…">`, `<style>@import…`, casse mixée) → sortie neutre ; test de rendu (fixture HTML) sans nœud script.
- **Effort** : 1 j-h (+ dépendance `ammonia` à valider licence Apache-2.0/MIT ✔).

### AUD-A7-009 — Expositions de tokens/config au repos dans le profil overlay

- **Sévérité** : Faible · **Confiance** : certaine
- **Preuve** : `apps/lc-overlay/src/config.rs:190-196` (chemin forcé `%APPDATA%\LiveChat\config.json`), `:389-400` (écriture `fs::write` non atomique + copie `.bak`, token en clair) ; pas de restriction ACL explicite.
- **Impact** : tout processus utilisateur (ou malware) lisant le profil récupère le token overlay ; corruption possible en cas de crash pendant l'écriture.
- **Correctif** : DPAPI/Credential Manager (Windows) ou ACL explicite ; écriture atomique (fichier temporaire + rename) ; ne pas conserver `.bak` contenant le token.
- **Test** : test d'écriture concurrente/crash simulé ; revue ACL.
- **Effort** : 1 j-h.

### AUD-A7-010 — Path confinement fail-open pour les médias

- **Sévérité** : Faible · **Confiance** : certaine
- **Preuve** : `apps/lc-server/src/http/overlay/media.rs:242-260` : si `canonicalize(MEDIA_STORAGE_DIR)` échoue, un `warn` est émis puis la fonction retourne quand même `Ok(canonical)` **sans vérifier le confinement** ; le chemin vient de `asset.storage_path` (DB, non contrôlable directement par un client, d'où la sévérité).
- **Impact** : défense en profondeur : si la base est corrompue/injectée ou le dossier absent (déploiement neuf), un chemin arbitraire serait servi (traversée).
- **Correctif** : fail-closed (404/500) lorsque la base n'est pas canonicalisable ; créer le dossier au boot ; test dédié.
- **Test** : test unitaire `resolve_media_file` avec `MEDIA_STORAGE_DIR` inexistant → erreur, pas de fichier servi.
- **Effort** : 0,2 j-h.

### AUD-A7-011 — Dépendances vulnérables atteignables dans le binaire serveur

- **Sévérité** : Moyen · **Confiance** : certaine (advisory DB au 2026-11-10)
- **Preuve** (`.phase0/audit/A7/cargo-audit.txt`, `cargo tree -i`) :
  - `libsqlite3-sys 0.30.1` ← `sqlx-sqlite 0.8.6` ← `sqlx 0.8.6` ← `lc-db`/`lc-playback`/`lc-server`/`xtask` : **RUSTSEC-2026-0459** (via CVE SQLite CVE-2026-119375), fix `>=0.39.0` (à date, aucune version sqlx publiée ne l'embarque — à suivre).
  - `rustls 0.22.4` + `rustls-webpki 0.102.8` ← `tokio-rustls 0.25.0` ← `tokio-tungstenite 0.21.0` ← `serenity 0.12.5` ← `lc-discord`/`lc-server` : RUSTSEC-2026-0346 (Debug TLS), 2026-0349 (downgrade), 2026-0049/0098/0099/0104/0382 (webpki). Chaîne du client Discord du serveur.
  - `rsa 0.9.10` (RUSTSEC-2023-0071, Marvin) : présent via `sqlx-mysql` **optionnel**, mais `cargo tree -i rsa` n'imprime rien → **non atteignable** dans le graphe compilé (feature `mysql` désactivée). À documenter dans `deny.toml` (ignore justifié) plutôt qu'à corriger.
  - `rustls 0.23.46` + `rustls-webpki 0.103.16` (reqwest/tauri) non concernés.
- **Impact** : parseur SQLite embarqué vulnérable (surface locale surtout) ; chaîne TLS du bot Discord exposée (client non serveur) ; `rsa` non atteignable.
- **Correctif** : suivre/piloter montée `sqlx` (ou `sqlite-unbundled` avec SQLite système patché) ; passer `serenity` en backend `native_tls` ou monter `serenity`/`rustls 0.23` ; documenter l'ignore `rsa` (feature mysql interdite) ; réévaluer à chaque CI.
- **Test** : CI `cargo audit` bloquant (cf. AUD-A7-002) ; test de non-régression du graphe (script `cargo tree -i rustls@0.22` vide attendu après fix).
- **Effort** : 1–2 j-h (dépend des releases amont).

---

## 5. Findings P2 — hygiène & durcissement

### AUD-A7-012 — MSRV déclarée non vérifiée en CI

- **Faible · certaine.** `Cargo.toml:21` `rust-version = "1.85"`, `clippy.toml:4` `msrv = "1.85"`, or `rust-toolchain.toml:4` `channel = "stable"` et CI `dtolnay/rust-toolchain@stable` — aucun job 1.85 ; `resolver = "2"` n'applique pas la MSRV aux deps. **Fix** : job CI `1.85` (cargo check) ou retirer la revendication ; **test** : CI verte en 1.85 ; **effort** 0,5 j-h.

### AUD-A7-013 — CORS : `tauri.localhost` permanent + en-têtes cookies inutiles

- **Faible · certaine.** `apps/lc-server/src/http/cors.rs:44-47` ajoute toujours `http(s)://tauri.localhost` (même hors dev) ; `:63` allow_headers contient `set-cookie,cookie` alors que `allow_credentials` est absent (test `:237-242`). L'origine reste autorisée côté CORS seulement (pas un contrôle d'accès, token requis). **Fix** : restreindre `tauri.localhost` en prod ou documenter ; retirer cookie/set-cookie. **Effort** 0,2 j-h.

### AUD-A7-014 — `/health` non authentifié : divulgation d'état

- **Faible · certaine.** `apps/lc-server/src/http/health.rs:110-146` expose `version`, `protocolVersion`, `db`, `discord`, `uptimeSec`, `activeJobs` à tout client. Utile pour probes/K8s ; réduire la surface publique (`/health/live` minimal + `/health` détaillé protégé) ou documenter comme accepté. **Effort** 0,2 j-h.

### AUD-A7-015 — Escape hatch TLS côté overlay

- **Faible · certaine.** `apps/lc-overlay/src/net/ws.rs:44-45,479-498` : `LIVECHAT_INSECURE_TLS` active `danger_accept_invalid_certs(true)` + `danger_accept_invalid_hostnames(true)` (native-tls). **Fix** : n'activer qu'en build debug/dev, log WARN explicite, bannir en release via `cfg!(debug_assertions)`, documenter le risque MITM. **Effort** 0,5 j-h.

### AUD-A7-016 — cargo-deny : deps workspace non résolues / politique wildcards incompatible

- **Info · certaine.** 8 × `bug[unresolved-workspace-dependency]` + 7 × `error[wildcard]` sur `Cargo.toml:85-91` (deps path sans `version`). Deux options : ajouter `version = "3.0.0"` aux deps workspace (cohérent semver interne) ou assouplir la politique. **Ne pas** désactiver bêtement `wildcards = "deny"`. **Effort** 0,3 j-h.

### AUD-A7-017 — Durcissement supply-chain conteneurs/CI

- **Info · certaine.** `deploy/server.Dockerfile` : images de base **par tag** (`:22` `rust:${RUST_VERSION}-${DEBIAN_RELEASE}`, `:83` `debian:${DEBIAN_RELEASE}-slim` sans digest) ; `apt-get install` sans versions épinglées (`:100-104` : ca-certificates/curl/ffmpeg/tini) ; yt-dlp correctement épinglé version + SHA-256 (`:88-89,109-112`) ✔ ; user non-root 10001 (`:142`) ✔ ; `provenance: true` (`server-image.yml:99`) ✔ ; image GHCR tag mutable (`docker-compose.yml:20`, `${LIVECHAT_IMAGE}` surchargeable) et `:35` `ports: "3000:3000"` exposé sans TLS/reverse proxy dans le compose ; limites mémoire commentées (`:60-66`). **Fix** : digests d'images de base + apt snapshot/versions, compose derrière proxy TLS (Caddy/Traefik) + limites actives + `read_only` sur /app hors data ; **effort** 0,5 j-h.

### AUD-A7-018 — Artefacts générés suivis / harnais de dev embarqué

- **Info · certaine.** `apps/lc-overlay/web/dist/**` = 80 fichiers (~246,6 Ko) **suivis** (justifié `docs/rust/migration-plan.md:242` pour build propre, toléré mais à revoir) ; `apps/lc-overlay/web/bridge-test.html` (25 470 o) suivi et **embarqué** (`tauri.conf.json:7` `frontendDist: "web"`) ; `apps/lc-overlay/gen/schemas/` ignoré localement ✔ ; `check.log` à la racine ignoré (`*.log`) ✔ ; `.phase0/` ignoré ✔. **Fix** : exclure `bridge-test.html` du bundle, générer `web/dist` en CI (check de parité), supprimer `check.log` ; **effort** 0,2 j-h.

### AUD-A7-019 — Admin : pas de rate-limit dédié ni d'audit-log des mutations

- **Info · certaine.** `docs/rust/admin-panel.md:285-286` recommande lui-même rate-limit admin + audit-log + ne pas logguer secrets ; `rust.yml`/`rate_limit.rs:46` exemptent `/admin` ; aucune table/écriture d'audit des actions sensibles (`stop`, `purge`, `revoke`, création de tokens) vérifiée dans `apps/lc-server/src/http/admin/*`. **Fix** : rate-limit admin par IP + token, audit-log structuré des mutations (sans secrets), alerting. **Effort** 1 j-h.

### AUD-A7-020 — TLS/HSTS attendus côté reverse proxy, non matérialisés

- **Info · certaine.** `apps/lc-server/src/http/security.rs` : CSP/nosniff/XFO/Referrer-Policy/CORP/Permissions-Policy présents ; **pas de HSTS, COOP/COEP** ; `deploy/docker-compose.yml:35-36` expose directement `3000:3000` sans terminaison TLS ; `docs/rust/packaging.md:25-26,287,329,342` délègue TLS au reverse proxy mais aucun HSTS/redirection documenté de bout en bout. **Fix** : imposer HTTPS dans le runbook/compose de prod (proxy TLS), documenter HSTS/redirection, configurer `TRUST_PROXY_HEADERS` en conséquence ; **effort** 0,2 j-h.

---

## 6. Plan de durcissement priorisé

### P0 — bloquants release (total ≈ 6–9 j-h)

| Ordre | Finding(s) | Action | Effort |
|---|---|---|---|
| 1 | AUD-A7-001 | Guard SSRF central (DNS/IP privées + redirects), tests wiremock | 2–3 j-h |
| 2 | AUD-A7-002, 011, 016 | CI : `cargo deny` vert (licences/wildcards), `cargo audit` bloquant, actions SHA, deps Linux Tauri, MSRV, bumps sqlx/serenity | 1–2 j-h |
| 3 | AUD-A7-003 | Updater réel + workflow Tauri release + nettoyage workflows legacy/images | 1–2 j-h |
| 4 | AUD-A7-004 | Suppression `?token=` (admin/board) + logs sans query | 1–2 j-h |

### P1 — avant bêta (total ≈ 4–7 j-h)

| Finding(s) | Action | Effort |
|---|---|---|
| AUD-A7-005 | Timeouts HTTP, rate-limit admin/par route, cap WS + Origin allowlist | 1–2 j-h |
| AUD-A7-006 | TikTok cookie masqué (GET) + PATCH set-only | 0,5 j-h |
| AUD-A7-007 | Renderer sans token, fs scope réduit, CSP restreinte, bundle nettoyé | 1–2 j-h |
| AUD-A7-008 | Sanitizer robuste HTML tweet (allowlist) | 1 j-h |
| AUD-A7-011 | Bump/atténuation sqlite/rustls, ignore rsa documenté | 1–2 j-h |

### P2 — durcissement continu (total ≈ 3–4 j-h)

| Finding(s) | Action | Effort |
|---|---|---|
| AUD-A7-009 | Fail-closed confinement média | 0,2 j-h |
| AUD-A7-010 | Escape hatch TLS dev-only | 0,5 j-h |
| AUD-A7-012 | MSRV vérifiée CI ou retirée | 0,5 j-h |
| AUD-A7-013, 014, 020 | CORS/health/TLS-proxy resserrés + doc | 0,5 j-h |
| AUD-A7-015 | DPAPI/ACL config overlay + écriture atomique | 1 j-h |
| AUD-A7-017, 018, 019 | Digests/limites compose, artefacts, audit-log admin | 1,5 j-h |

---

## 7. Hygiène & CI — état détaillé

### 7.1 CI (`.github/workflows/rust.yml`)

- Déclencheurs : push `feat/rust-rebuild`/`main` + PR, filtrés sur `crates/**`, `apps/**`, `xtask/**`, manifests, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml`, `.cargo/**`, workflow (l.9-34) — raisonnable.
- Jobs : `fmt` (l.51-56), `clippy -D warnings` workspace (l.63-69), `nextest` matrix ubuntu+windows + doctests (l.80-90), `deny` (l.92-99), `audit` non bloquant (l.102-114).
- Faiblesses : deny rouge (cf. AUD-A7-002), audit non bloquant, `--no-tests=pass` (l.87), actions non épinglées SHA, deps Tauri Linux absentes, pas de job MSRV, pas de `cargo deny check sources` séparé (couvert), pas de scan secrets/dépendances Docker.
- Workflows legacy encore actifs : `server.yml` (Node, paths `server/**`), `overlay-release.yml` (Electron), `overlay-nightly.yml`, `extension.yml` — **hors périmètre de migration mais à désactiver au cutover** ; collision d'images GHCR cf. AUD-A7-003.
- Aucun workflow de release Tauri (attendu Phase 9).

### 7.2 Supply chain

- `Cargo.lock` versionné ✔ ; aucune dépendance Git (`deny.toml:44` `allow-git = []`, sources crates.io only) ✔.
- `cargo audit` : 647 crates scannés, **9 vulnérabilités + 5 warnings** (détail AUD-A7-011 ; warnings unmaintained/unsound de la chaîne Tauri : `proc-macro-error`, `zbus_names`, `zvariant`, `zvariant_derive`, `glib 0.18.5` RUSTSEC-2024-0429 — app desktop, pas le binaire serveur).
- `cargo deny` : advisories/bans/licenses FAILED (détail AUD-A7-002).
- Images Docker : tags mutables, bases non digest-pinnées, apt non épinglé, yt-dlp épinglé ✔, provenance ✔, non-root ✔.

### 7.3 Code & configuration

- **Zéro `unwrap`/`expect`/`panic` en production** : seule occurrence `crates/lc-db/src/lib.rs` (documentation) ; `panic!` restants exclusivement en tests ; `#![forbid(unsafe_code)]` partout sauf `apps/lc-overlay/src/win32.rs` (isolé, justifié) ✔.
- `#[allow]` : quasi exclusivement `clippy::unwrap_used,expect_used,panic` sur modules de test inline ; `too_many_arguments` (5 emplacements : `crates/lc-discord/src/handlers.rs:250,388`, `apps/lc-server/src/media/board.rs:42`, `apps/lc-server/src/ws/hub.rs:122`, `ws/session.rs:494`) ; `dead_code` (5 en production : `ws/session.rs:820`, `apps/lc-overlay/src/net/ws.rs:815`, `http/admin/static_files.rs:120`, `http/overlay/auth.rs:58`, `http/overlay/pair.rs:76`).
- TODO/FIXME : 1 seul (`crates/lc-media/src/download.rs`, unrelated sécurité).
- Secrets : scan de 562 fichiers suivis (patterns clés privées, `sk-`, `ghp_`, JWT) → **aucun hit** ; `.env` non suivi, `.env.example` sans secret ✔.
- SQL : 100 % paramétré (`?1`) dans `apps/lc-server/src/http/admin/queries.rs` ✔.
- Processus externes : args tableau, pas de shell (`crates/lc-media/src/process.rs`) ✔.
- Comparaisons de secrets à temps constant : admin (`admin/auth.rs:167`), HMAC média (`crates/lc-protocol`/`apps/lc-server/src/media_url.rs`) ✔.
- Tokens au repos : SHA-256 (`OverlayClient.token_hash`) ✔.
- Documentation cohérente sur l'existant mais **contradictoire sur le token query** (`docs/rust/legacy-ts-removal.md:121` vs commit `e0a381f`/`overlay/auth.rs:109-130`, et `docs/rust/lc-overlay-8b.md:83-85,174` qui acte la dette board) — à trancher et aligner dans la doc v3.

---

## 8. Points positifs à préserver

1. `libsqlite3-sys`/SQLite et `rustls` mis à part, la surface serveur n'embarque pas de parseur non rust.
2. Zéro panic/unwrap en production, lints clippy workspace `deny` (aucun `as any` équivalent).
3. HMAC média constant-time, secret vide refusé, expiration inclusive, `Cache-Control: no-store`, `Accept-Ranges`/HEAD.
4. Tokens overlay/ingest **hachés SHA-256** en base, jambe comparaison constant-time côté admin.
5. CSP serveur stricte (admin : `default-src 'none'`, `frame-ancestors 'none'`) + `nosniff`/`XFO`/`Referrer-Policy`/`CORP`.
6. CORS explicite sans credentials (S5 corrigé).
7. Conteneur non-root (10001), `no-new-privileges`, provenance GHCR, yt-dlp épinglé SHA256.
8. SQL paramétré, aucun shell, ServeDir anti-traversal, écritures `no-store`.
9. `.env.example` documenté sans secrets et validations de config avec erreurs explicites.
10. Tests CI (227 unitaires + 20 WS + 3 intégration DB en local) et harness de parité en place.

## 9. Limites de l'audit

- Revue **statique** : aucune exploitation réelle, pas de pentest réseau/Tauri, pas de run CI Linux (les échecs de build Tauri Linux sont une hypothèse forte, non mesurée).
- Base d'advisories au **2026-11-10** ; les compteurs évolueront.
- Hors périmètre : legacy Node/Electron/extension, infrastructure cloud réellement déployée, chiffrement au repos SQLite, sécurité physique, menaces internes, revue ligne à ligne de `lc-media` pipeline (voir lane 04).
- L'auditeur est aussi auteur de `apps/lc-server/src/ws/**` (revue de son propre code : les points durs WS sont signalés en AUD-A7-005).

## 10. Annexe — commandes exécutées

```powershell
# Contexte
git rev-parse --short HEAD; git branch --show-current; git status --short
cargo --version; rustc --version; cargo deny --version; cargo audit --version

# Supply chain (logs .phase0/audit/A7/)
cargo deny check                       # advisories FAILED, bans FAILED, licenses FAILED, sources ok
cargo audit                            # 9 vulnérabilités, 5 warnings, 647 crates
cargo tree -i libsqlite3-sys; cargo tree -i rustls; cargo tree -i tokio-tungstenite; cargo tree -i rsa

# Secrets / hygiène
git ls-files | Where-Object { $_ -match '\.(rs|toml|yml|yaml|json|js|ts|html|md)$' } |
  ForEach-Object { Select-String -Path $_ -Pattern 'BEGIN (RSA|OPENSSH|EC) PRIVATE KEY|sk-[A-Za-z0-9]{20,}|ghp_[A-Za-z0-9]{20,}|MTA[A-Za-z0-9+/=]{20,}\.' }
Get-ChildItem apps,crates,xtask -Recurse -Filter *.rs | Select-String -Pattern 'TODO|FIXME|HACK|XXX'
Get-ChildItem crates,apps -Recurse -Filter *.rs | Select-String -Pattern '#\[allow\('

# Lectures ciblées (revue)
get-content .github/workflows/{rust,server,server-image,overlay-release}.yml
get-content deploy/{.env.example,docker-compose.yml,server.Dockerfile}
get-content deny.toml; get-content rust-toolchain.toml; get-content clippy.toml
get-content apps/lc-server/src/{app.rs,config.rs,media_url.rs}
get-content apps/lc-server/src/http/{rate_limit,cors,security,error}.rs
get-content apps/lc-server/src/http/admin/{auth,runtime_settings,mod}.rs
get-content apps/lc-server/src/http/overlay/{auth,media}.rs
get-content apps/lc-overlay/{tauri.conf.json,capabilities/default.json}
get-content apps/lc-overlay/src/{lib.rs,config.rs,commands.rs,net/ws.rs}
get-content crates/lc-media/src/{source,http,download,html}.rs
```

> Les logs `cargo-deny.txt` sont encodés UTF-16 par `Tee-Object` sous Windows : relire avec `Get-Content -Encoding Unicode`.

---

*Rapport A7 — 20 findings (4 Élevé, 5 Moyen, 6 Faible, 5 Info). Aucun code modifié, aucun commit.*
