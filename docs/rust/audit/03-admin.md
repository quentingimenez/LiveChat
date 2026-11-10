# Audit A3 — Admin UI & authentification

- **Auditeur** : `overlay-bridge` (audit indépendant, n'a pas écrit cette lane)
- **Date** : 2026-11-10
- **Dépôt** : `C:\Users\SuperKnacky\code\LiveChat`, branche `feat/rust-rebuild`
- **Périmètre** : `apps/lc-server/src/http/admin/**`, `static/admin/**`, `docs/rust/server-admin.md`, `docs/rust/admin-panel.md`
- **Mode** : revue statique exhaustive + preuves dynamiques sur le binaire existant `target/debug/lc-server.exe` (version 3.0.0, protocole 2, schéma SQLite 3.0.0, build du 2026-11-10 01:43) — **aucun build**
- **Hors périmètre** : WebSocket/hub (audité par ailleurs), pipeline média, protocole overlay, extension Chromium
- **Aucun code modifié, aucun commit.** Artefacts de reproduction : `.phase0/audit/A3/`
- **Statut task board** : `task-29`, audit read-only ; livrable unique `docs/rust/audit/03-admin.md`

---

## 1. Résumé exécutif

L'admin Rust est globalement **bien au-dessus du legacy** sur les points sensibles : contrôle d'accès centralisé par extracteur, ordre `503 → 403 → 401` volontaire et fiable, comparaison à temps constant, aucune confiance aux en-têtes `X-Forwarded-For`, page statique sans script inline, CSP dédiée, `no-store` partout, échappement HTML systématique du front (aucun XSS trouvé, y compris avec payloads stockés testés en navigateur headless), SQL entièrement paramétré, 27 tests d'intégration couvrant les 13 routes JSON.

En revanche, l'audit relève **une faiblesse Élevée** : le token admin est accepté en query string sur **toutes** les méthodes et l'URI complète — token compris — est journalisée en `INFO` par défaut (`RUST_LOG=info` recommandé par le modèle de déploiement). Un opérateur qui consulte les journaux, l'historique navigateur ou une capture d'écran peut récupérer le token et prendre la main sur l'admin (purge irréversible incluse). S'y ajoutent deux points Moyens : l'exemption totale de rate-limit sur `/admin*` et l'exposition réseau de l'API admin dans le déploiement Compose fourni (`ADMIN_UI_LOCAL_ONLY=false` + port publié, TLS délégué au seul reverse proxy).

| ID | Sévérité | Confiance | Titre |
|----|----------|-----------|-------|
| AUD-A3-001 | **Moyenne** (Élevée si le port 3000 est joignable) | Élevée (PoC) | Compose de production publie l'API admin hors loopback, sans TLS applicatif ni rate-limit |
| AUD-A3-002 | **Élevée** | Élevée (PoC) | Token accepté en `?token=` sur toutes les méthodes + URI complète journalisée en INFO + URL réécrite avec le token par `app.js` |
| AUD-A3-003 | Moyenne | Élevée (PoC) | `/admin*` exempté du rate-limit : bruteforce illimité et agrégats non throttlés ; aucune longueur minimale de token validée |
| AUD-A3-004 | Faible | Élevée (PoC) | Cookie TikTok renvoyé en clair par l'API et stocké en clair dans `admin_runtime.json`, sans permissions explicites |
| AUD-A3-005 | Faible | Élevée (PoC) | Aucun contrôle `Origin`/`Referer` sur les mutations (PATCH/POST/DELETE) ; mutation possible avec un token passé en URL |
| AUD-A3-006 | Faible | Élevée | Purge destructive uniquement protégée par l'écho client de `confirmGuildId` (pas de ré-auth, pas d'audit) |
| AUD-A3-007 | Faible | Élevée | Hub admin global `OnceLock` hors `AppState` ; second montage silencieux ; doc de câblage obsolète |
| AUD-A3-008 | Faible | Élevée | Création de client ingest : label non borné, pas de dédoublonnage, prolifération de tokens |
| AUD-A3-009 | Info | Élevée | Fermeture WebSocket à la révocation/purge non couverte de bout en bout (NoopAdminHub dans les tests) |
| AUD-A3-010 | Info | Élevée | Page `/admin/` publique (décision E9) et copies d'interface trompeuses (« accès local uniquement », « persisté dans .env ») |
| AUD-A3-011 | Faible | Moyenne (raisonnement statique) | `ADMIN_UI_LOCAL_ONLY` inopérant derrière un reverse proxy local ; `TRUST_PROXY_HEADERS` sans effet sur l'auth admin malgré une doc ambiguë |
| AUD-A3-012 | Info | Élevée | Traçabilité des mutations admin limitée à l'URI/statut HTTP (pas d'identité, pas de corps, purge non auditée) |

**Total : 1 Élevée, 2 Moyennes, 6 Faibles, 3 Info.** Effort de remédiation estimé : **2,5 à 4 j-h** (l'Élevée ≈ 0,5–1 j-h, les deux Moyennes ≈ 1–1,5 j-h).

Les trois actions à plus fort rendement, dans l'ordre : (1) ne plus journaliser la query string dans le span de trace et retirer le token de l'URL côté front ; (2) réintroduire un rate-limit spécifique admin (ou limiter `/admin/api/*` et laisser seulement les assets statiques exemptés) ; (3) durcir le Compose (publication `127.0.0.1:3000:3000` ou profil proxy, TLS obligatoire, réseau d'admin isolé documenté comme prérequis).

---

## 2. Méthode

1. **Revue statique intégrale** : 13 fichiers de `apps/lc-server/src/http/admin/` (2 815 lignes), `static/admin/{index.html,app.js,styles.css}` (1 415 lignes), `docs/rust/server-admin.md` (484 l.) et `docs/rust/admin-panel.md` (229 l.), plus les dépendances qui conditionnent l'admin : `http/security.rs`, `http/rate_limit.rs`, `http/cors.rs`, `app.rs`, `lib.rs`, `config.rs`, `wiring.rs`, `shutdown.rs`, `media_url.rs`.
2. **Vérification croisée** : `grep` des sinks front (`innerHTML`, `insertAdjacentHTML`, `outerHTML`, `eval`, `localStorage`, `history.replaceState`), des lectures d'en-têtes de proxy, de l'usage du hub, des exemptions de rate-limit, des codes d'erreur.
3. **Preuves dynamiques** (port et DB isolés, jamais `.phase0/local-test`) :
   - binaire `target/debug/lc-server.exe`, bind `0.0.0.0:18181`, `NODE_ENV=production`, `RUST_LOG=info`, `DATABASE_URL=sqlite://.phase0/audit/A3/data/audit.sqlite?mode=rwc`, `MEDIA_STORAGE_DIR=.phase0/audit/A3/media`, `MEDIA_URL_SECRET=<64 hex>`, `ADMIN_UI_TOKEN=probe-admin-token-A3`, `ADMIN_UI_LOCAL_ONLY=true` ;
   - secondes instances isolées : port 18182 avec `ADMIN_UI_TOKEN` vide (preuve du 503) puis avec `ADMIN_UI_LOCAL_ONLY=false` (preuve de l'exposition LAN) ;
   - insertion de payloads stockés via `python 3.14` (module `sqlite3`) dans la DB isolée, puis dump DOM de la page avec Chrome headless `157.0.8103.40` (`--headless=new --virtual-time-budget=12000 --dump-dom`) ;
   - toutes les requêtes HTTP via `curl.exe` ; sorties conservées dans `.phase0/audit/A3/probes.txt`.
4. Ce qui n'a pas pu être exécuté sans risque (proxy TLS réel, droits POSIX Linux, socket WS réelle, navigateur d'un attaquant) est marqué **statique** et reclassé en confiance Moyenne.

---

## 3. Surface auditée

### 3.1 Chaîne de contrôle d'accès (statique)

`apps/lc-server/src/http/admin/auth.rs:48-78` — extracteur `AdminAccess`, exécuté avant chaque handler :

1. `ADMIN_UI_TOKEN` vide après `trim()` ⇒ **503** `{"error":"admin_ui_disabled","message":"ADMIN_UI_TOKEN is not configured"}` (`auth.rs:55-59`, `auth.rs:80-88`) ;
2. `ADMIN_UI_LOCAL_ONLY` vrai et IP de connexion non loopback ⇒ **403** `{"error":"admin_local_only"}` (`auth.rs:61-66`) ;
3. token absent ⇒ **401** `{"error":"unauthorized"}` (`auth.rs:68-70`) ;
4. token différent ⇒ **401**, comparaison longueur puis `subtle::ConstantTimeEq` (`auth.rs:72-74`, `auth.rs:159-168`).

- Lecture du token : `Authorization: Bearer` prioritaire, puis `?token=` via `form_urlencoded` (`auth.rs:123-156`).
- Local-only : uniquement `ConnectInfo<SocketAddr>` ; **refus si l'extension est absente** (fail-closed), **aucun `X-Forwarded-For`** (`auth.rs:111-120`). Loopback accepté : tout `127.0.0.0/8`, `::1`, `::ffff:127.0.0.1` (`auth.rs:99-109`) — légèrement plus large que le legacy `127.0.0.1`.
- En-têtes de réponse admin : `no-store`, `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, CSP dédiée (`http/admin/mod.rs:60-66`, `:118-142`). `Permissions-Policy` est fournie par la couche globale (`http/security.rs:44`, `:73`) ; le CSP global utilise `entry().or_insert()` pour laisser gagner le CSP admin plus strict (`http/security.rs:51-57`).
- CORS : allowlist explicite (origine API + origines additionnelles + `http(s)://tauri.localhost`, loopback réservé au développement), `Authorization` autorisé en pré-vol (`http/cors.rs:35-105`).

### 3.2 Inventaire des routes (13 endpoints JSON + assets statiques)

| # | Méthode & route | Handler | Auth | Contrat vérifié | Tests / preuve |
|---|-----------------|---------|------|-----------------|----------------|
| 1 | `GET /admin/api/overview` | `overview.rs` | oui | camelCase, dates RFC3339 ms `Z`, agrégats + compteurs, clients overlay actifs uniquement | `overview_*` (3), probe 200 |
| 2 | `GET /admin/api/runtime-settings` | `runtime_settings.rs:58` | oui | `{tiktokCookie, hasTikTokCookie}` | `runtime_settings_*` (2), probe 200 |
| 3 | `PATCH /admin/api/runtime-settings` | `runtime_settings.rs:76` | oui | corps `{tiktokCookie: string}`, 400 sinon | idem, probe 200 + Origin hostile |
| 4 | `GET /admin/api/ingest-authors` | `ingest_authors.rs` | oui | fusion pairing+ingest, camelCase | `ingest_authors_fusionne_*` |
| 5 | `POST /admin/api/ingest-clients` | `ingest_authors.rs:92` | oui | 400 `invalid_guild_id` / `invalid_author_discord_user_id` / `invalid_label` / `author_not_found` ; token clair une fois | `ingest_client_create_*` (2) |
| 6 | `PATCH /admin/api/guilds/{guild_id}/settings` | `guilds.rs` | oui | partiel, `floor` des flottants, `null`/`<=0` efface, 400 sinon | `settings_patch_*` (2) |
| 7 | `POST /admin/api/guilds/{guild_id}/stop` | `guilds.rs:133` | oui | libère les jobs PLAYING, busyUntil=null | `stop_libere_les_jobs_playing` |
| 8 | `POST /admin/api/guilds/{guild_id}/purge` | `guilds.rs:172` | oui | exige `confirmGuildId` identique au path (400 sinon) | `purge_*` (2), AUD-A3-006 |
| 9 | `POST /admin/api/overlay-clients/{id}/revoke` | `clients.rs:31` | oui | idempotent, 400 `invalid_client_id`, 404 `overlay_client_not_found`, close WS 4402 si actif | `revoke_overlay_*`, AUD-A3-009 |
| 10 | `POST /admin/api/ingest-clients/{id}/revoke` | `clients.rs:76` | oui | 400/404 `ingest_client_not_found` | `revoke_ingest_inconnu_renvoie_404` |
| 11 | `GET /admin/api/pairing-codes` | `pairings.rs` | oui | filtre `guildId`/`status=all|active|expired`, compteurs après filtre | `pairing_liste_filtre_et_compte` |
| 12 | `DELETE /admin/api/pairing-codes/expired` | `pairings.rs` | oui | `{deletedCount,guildId}` | `pairing_supprime_les_expires_*`, probe 200 |
| 13 | `POST /admin/api/pairing-codes/{code}/revoke` | `pairings.rs` | oui | 400 `invalid_code`, 404 `pairing_code_not_found` | `pairing_supprime_les_expires_et_revoque_un_code` |
| 14 | `GET /admin`, `GET /admin/`, `/admin/*` | `static_files.rs:49-60` | **non** | 308 vers `/admin/`, `index.html`, assets ; `no-store` + CSP admin | `statique_*` (3), `page_admin_publique_*` |

Constats transverses vérifiés : contrats camelCase respectés sur tous les handlers testés ; dates ISO-8601 UTC millisecondes `Z` ; erreurs JSON `{"error":"<code_snake_case>"}` ; `no-store` et CSP admin présents **même sur les 401/403/503** (probe) ; aucune route `/admin/api/*` sans `AdminAccess` ; `/overlay/config` n'existe pas dans ce routeur (v2).

Observation : une méthode non déclarée (ex. `POST /admin/api/overview`) répond **405 avant** l'extracteur d'auth (routage axum avant handler). Aucune donnée n'est exposée, mais cela confirme l'existence de la route sans token — négligeable.

---

## 4. Findings

### AUD-A3-001 — Le Compose de production publie l'API admin hors loopback, sans TLS applicatif ni rate-limit

- **Sévérité** : Moyenne (Élevée si le port publié est joignable depuis un réseau non maîtrisé). **Confiance** : Élevée (PoC).
- **Composants** : `deploy/docker-compose.yml`, `deploy/.env.example`, `docs/rust/packaging.md`.
- **Preuve statique** :
  - `deploy/docker-compose.yml:30` : `ADMIN_UI_LOCAL_ONLY: "false"` ; `:35-36` : `ports: - "3000:3000"` (publication sur toutes les interfaces) ; `:11-13` seulement `restart`/`env_file` ;
  - `deploy/.env.example:22` : `ADMIN_UI_LOCAL_ONLY=false` ; `:20` : `ADMIN_UI_TOKEN=` (vide ⇒ admin désactivée tant que l'opérateur ne la configure pas — garde-fou partiel) ;
  - `docs/rust/packaging.md:287-288` : TLS délégué au reverse proxy et `ADMIN_UI_LOCAL_ONLY=false` assumé, avec recommandation d'un réseau d'admin isolé/VPN ; `packaging.md:329` : « l'exposition publique/TLS reste au reverse proxy ». Le risque est donc **documenté**, mais le fichier livré, lui, publie le port et désactive le filtre loopback.
- **Preuve dynamique** : instance isolée port 18182 avec `ADMIN_UI_LOCAL_ONLY=false`, `ADMIN_UI_TOKEN=probe-admin-token-A3`, depuis l'IP LAN `172.25.224.1` :
  - `GET http://172.25.224.1:18182/admin/` → **200** ;
  - `GET http://172.25.224.1:18182/admin/api/overview` + Bearer correct → **200** ;
  - Bearer erroné → 401.
  - (Avec `ADMIN_UI_LOCAL_ONLY=true`, les mêmes requêtes LAN donnent **403** — le filtre fonctionne quand il est activé, y compris avec `X-Forwarded-For: 127.0.0.1` forgé.)
- **Impact** : si le port 3000 du conteneur est exposé au-delà de l'hôte (règles Docker/iptables par défaut : oui) et que le reverse proxy TLS n'est pas devant, un attaquant réseau peut (a) brute-forcer le token sans limite (AUD-A3-003), (b) lire le token en clair sur le réseau (HTTP) lorsqu'un opérateur utilise l'interface, puis (c) exécuter toutes les actions admin (purge comprise). Même derrière un proxy, l'accès direct au port publié contourne la terminaison TLS si le pare-feu l'autorise.
- **Correctif** :
  1. dans `docker-compose.yml`, publier `127.0.0.1:3000:3000` (accès via proxy local/TLS ou tunnel SSH) et laisser `ADMIN_UI_LOCAL_ONLY=false` avec une note explicite, **ou** séparer l'admin sur un second port/bind loopback dédié ;
  2. exiger un reverse proxy TLS dans la doc Compose et vérifier `API_URL` https (déjà recommandé) ;
  3. refuser au démarrage un `ADMIN_UI_TOKEN` non vide avec `ADMIN_UI_LOCAL_ONLY=false` si `API_URL` n'est pas `https://` (garde-fou de configuration, sauf opt-out documenté) ;
  4. documenter le réseau d'admin isolé comme **prérequis**, pas comme option.
- **Test de régression** : test de configuration (`Config::from_pairs`) qui rejette/warn `ADMIN_UI_LOCAL_ONLY=false` + `API_URL` http en production ; test d'intégration existant `auth_local_only_desactive_accepte_un_client_externe` à conserver comme documentation du comportement.
- **Effort** : 0,25–0,5 j-h (config + doc), 0,5 j-h si garde-fou au démarrage.

### AUD-A3-002 — Token en query string sur toutes les méthodes, URI complète journalisée en INFO, URL réécrite avec le token par le front

- **Sévérité** : **Élevée**. **Confiance** : Élevée (PoC).
- **Composants** : `apps/lc-server/src/http/admin/auth.rs:123-156`, `apps/lc-server/src/app.rs:223-243`, `static/admin/app.js:127-167`, `deploy/.env.example:14`.
- **Preuve statique** :
  - `auth.rs:139` : repli systématique `query_token(parts.uri.query())` après l'échec du Bearer ; `auth.rs:143-156` accepte la première clé `token` non vide, **pour toutes les méthodes** (l'extracteur ne distingue pas GET des mutations) ;
  - `app.rs:223-243` (`make_span`) : `uri = %request.uri()` — **query string incluse** ; le span est émis en `INFO` par `TraceLayer`, avec `include_headers=false` (donc le Bearer n'est pas journalisé, seulement l'URL) ;
  - `deploy/.env.example:14` : `RUST_LOG=info` recommandé par défaut ;
  - `static/admin/app.js:127-167` : `readToken()` lit `?token=`, le stocke dans `localStorage` et **réécrit l'URL avec le token** via `history.replaceState` (`syncTokenInUrl`) ; `writeToken()` fait de même à chaque saisie.
- **Preuve dynamique** (traces réelles conservées dans `.phase0/audit/A3/probes.txt` et `server.out.log`) :
  - `GET /admin/api/overview?token=probe-admin-token-A3` → **200** ; même requête avec token erroné → 401 ;
  - `PATCH /admin/api/runtime-settings?token=probe-admin-token-A3` (sans Bearer) → **200** ;
  - `DELETE /admin/api/pairing-codes/expired?guildId=…&token=probe-admin-token-A3` (sans Bearer) → **200** `{"deletedCount":0,…}` ;
  - journal serveur : `INFO http_request{method=DELETE uri=/admin/api/pairing-codes/expired?guildId=424242424242424242&token=probe-admin-token-A3 …}: tower_http::trace::on_request: started processing request` — **token en clair, statut INFO, chemin de journalisation par défaut** (docker `json-file`, 10 Mo × 3, voir `deploy/docker-compose.yml:52-56`).
- **Impact** : le token admin est un secret statique **partagé, sans expiration ni révocation** ; toute copie des journaux (support, capture docker logs, agrégateur), de l'historique du navigateur, d'une capture d'écran ou d'un champ `Referer` (atténué par `Referrer-Policy: no-referrer`, mais pas par les journaux) permet une prise de contrôle complète : lecture des données, révocation de clients, stop, purge irréversible. Un proxy/access-log en amont rejoue le même problème. La combinaison (query token + log INFO) transforme une bonne pratique (Bearer) en fuite de credential.
- **Correctif** :
  1. **journalisation** : dans `make_span`, ne journaliser que `request.uri().path()` (et `method`), jamais la query ; ou redacter explicitement `token`/`sig`/`exp` (les URL média signées sont aussi concernées, cf. audit A4) ;
  2. **transport** : réserver `?token=` aux navigations initiales de la page (jamais aux API mutantes) ou le supprimer complètement ; le front doit migrer le token vers `localStorage` **puis retirer le paramètre de l'URL** au lieu de le réinjecter ;
  3. ajouter un en-tête `Cache-Control: no-store` déjà présent, mais aussi éviter que le token apparaisse dans les éventuels logs d'accès (nginx `log_format` à documenter) ;
  4. à terme, remplacer le token statique par une session courte (cookie `HttpOnly`/`SameSite=Strict` + CSRF token) ou un jeton à durée de vie, mais ce n'est pas nécessaire pour corriger cette fuite.
- **Test de régression** : test d'intégration qui envoie `GET /admin/api/overview?token=…` et vérifie que la réponse est 401 (si le query token est retiré) ; test unitaire de `make_span` vérifiant que le champ `uri` ne contient pas `token=` ; test front (jsdom/Playwright) vérifiant qu'après lecture du token l'URL ne contient plus `?token=`.
- **Effort** : 0,5–1 j-h (dont 0,25 j-h de doc pour le retrait du query token, qui est un écart de parité legacy assumé).

### AUD-A3-003 — `/admin*` exempté du rate-limit : bruteforce illimité et agrégats non throttlés

- **Sévérité** : Moyenne. **Confiance** : Élevée (PoC).
- **Composants** : `apps/lc-server/src/http/rate_limit.rs:40-46`, `:175-177`, `:226`, `apps/lc-server/src/config.rs:227-230`, `:345`.
- **Preuve statique** :
  - `http/rate_limit.rs:46` : `const ADMIN_PATH_PREFIX: &str = "/admin";` ; `:226` : `path.starts_with(ADMIN_PATH_PREFIX)` ⇒ **exemption** ; `:40` `RATE_LIMIT_MAX=100`, fenêtre 15 min ; `:175-177` initialisation du limiteur ;
  - `http/admin/guilds.rs:172` (purge) et `overview.rs` (agrégats multi-tables) sont des routes coûteuses ;
  - `config.rs:227-230` : `ADMIN_UI_TOKEN` = `Secret` sans contrainte de longueur/entropie ; `:345` : simple lecture d'environnement (`env.secret`), aucune validation ⇒ un token court choisi par l'opérateur est possible (et `.env.example:19-20` ne fixe aucun minimum).
- **Preuve dynamique** : 110 requêtes `GET /admin/api/overview?token=wrong` → **401 ×110, 429 ×0** ; sur le même serveur, 101 requêtes `GET /health` → **429 à partir de la 100ᵉ** (429 ×2 dans la fenêtre, 200 ×99). L'exemption est donc bien effective, alors que le limiteur fonctionne hors admin.
- **Impact** : (a) le token étant le seul facteur d'authentification, le bruteforce n'est borné que par la longueur/entropie du token choisi — l'absence de minimum rend un token court/deviner craquable sans coût réseau ; (b) `overview` agrège toutes les guildes (N requêtes DB par guilde) et peut être appelée en boucle pour saturer le serveur (DoS applicatif) sans jamais déclencher 429 ; (c) l'exemption couvre tout le préfixe (`/admin...`), y compris de futures routes sensibles.
- **Correctif** :
  1. limiter `/admin/api/*` avec un compteur dédié (par IP, ou global pour les échecs d'auth) ; ne garder l'exemption que pour les assets statiques `/admin/` (page + JS/CSS) ou supprimer l'exemption et augmenter la limite admin ;
  2. ajouter un compteur d'échecs d'auth (par IP + global) avec backoff temporaire sur 401 répétés ;
  3. valider à la configuration `ADMIN_UI_TOKEN.len() >= 32` (sinon erreur au démarrage) et refuser les tokens triviaux ;
  4. rate-limiter `overview` séparément (cache court ou intervalle minimal).
- **Test de régression** : test existant `rate_limit_blocks_after_100_requests_and_exempts_admin` (`app.rs:471-496`) à inverser pour `/admin/api/*` ; test qui envoie 101 échecs d'auth admin et attend un 429 ; test unitaire de validation du token minimal.
- **Effort** : 0,5–0,75 j-h.

### AUD-A3-004 — Cookie TikTok exposé en clair (API + fichier) sans permissions explicites

- **Sévérité** : Faible. **Confiance** : Élevée (PoC).
- **Composants** : `apps/lc-server/src/http/admin/runtime_settings.rs`, `static/admin/index.html:29`.
- **Preuve statique** :
  - `runtime_settings.rs:58-68` : `GET /admin/api/runtime-settings` renvoie `{"tiktokCookie": settings.tiktok_cookie, "hasTikTokCookie": …}` — **valeur complète**, sans masquage ;
  - `runtime_settings.rs:40` : fichier `admin_runtime.json` ; `:219-260` : chemin déduit de `DATABASE_URL` (à côté de la DB ; en conteneur `/app/data/admin_runtime.json`) ; `:151-203` : écriture `<fichier>.tmp` puis `remove_file` + `rename` (fenêtre de perte documentée en commentaire `:153-154`) ;
  - **aucune** API de permissions/chmod : le fichier hérite de l'umask du processus (souvent `0644` sur Linux) et des ACL du dossier de données ;
  - `static/admin/index.html:29` : « Appliqué immédiatement et persisté dans le fichier `.env`. » — **faux** : le fichier réel est `admin_runtime.json` ; `:15` : « Accès local uniquement. Token requis. » — faux aussi quand `ADMIN_UI_LOCAL_ONLY=false`.
- **Preuve dynamique** : `GET /admin/api/runtime-settings` Bearer → `{"hasTikTokCookie":true,"tiktokCookie":"sessionid=SUPERSECRET-A3"}` ; `PATCH` avec `Origin: https://evil.example` puis `?token=` → 200 ; fichier observé `{"tiktokCookie":"sessionid=QUERY-TOKEN-MUTATION"}` en clair ; `icacls` Windows : ACL **héritées** (SYSTEM/Administrateurs/utilisateur en contrôle total), aucune ACL dédiée.
- **Impact** : le cookie de session TikTok est un credential de compte (permettant l'ingestion média authentifiée). Il est lisible par toute personne ayant le token admin (déjà privilégiée, donc impact modéré) **et** par toute personne ayant accès en lecture au volume `data/` (sauvegardes, exports, utilisateurs du groupe Docker, hôte compromis) ; en cas d'API exposée (AUD-A3-001) il transite en clair. La fenêtre `remove_file`/`rename` peut faire perdre le réglage en cas de crash (fallback `TIKTOK_COOKIE` d'environnement, perte silencieuse).
- **Correctif** : masquer la valeur dans `GET` (renvoyer uniquement `hasTikTokCookie` + préfixe/suffixe, la valeur complète n'étant nécessaire qu'à l'écriture) ; à la sauvegarde, `chmod 0600` (nix) / ACL restreinte ou chiffrement au repos ; documenter que ce fichier contient un secret ; corriger le texte UI (`.env` → `admin_runtime.json`, et « accès local uniquement » conditionnel) ; journaliser un avertissement au chargement si les permissions sont trop ouvertes (statique).
- **Test de régression** : test qui vérifie que `GET` ne renvoie pas le cookie complet ; test des permissions du fichier après écriture (gated Unix) ; test de la persistance après un `PATCH` (existe déjà).
- **Effort** : 0,25–0,5 j-h.

### AUD-A3-005 — Aucun contrôle `Origin`/`Referer` sur les mutations ; mutation possible via token d'URL

- **Sévérité** : Faible. **Confiance** : Élevée (PoC).
- **Composants** : `apps/lc-server/src/http/admin/auth.rs:123-156`, `static/admin/app.js:169-205`, `apps/lc-server/src/http/cors.rs:35-105`.
- **Preuve statique** : aucune occurrence de `Origin`/`Referer` dans `http/admin/**` (grep) ; les mutations acceptent un token en query (`auth.rs:139`) ; le front n'envoie pas de jeton CSRF (`app.js:169-205`, toujours Bearer + JSON).
- **Preuve dynamique** : `PATCH /admin/api/runtime-settings` avec `Origin: https://evil.example` + Bearer → **200 updated** ; même requête avec **uniquement** `?token=` (sans Bearer, sans contrôle d'origine) → **200** ; `DELETE …?token=` → 200 (cf. AUD-A3-002).
- **Impact** : **pas de CSRF exploitable aujourd'hui** — l'auth ne repose pas sur un cookie ambiant : un site tiers ne peut pas ajouter le header `Authorization`, et un formulaire HTML ne peut pas produire un `PATCH`/`DELETE` JSON (415), ni connaître le token. Le CORS est une allowlist stricte (`cors.rs:35-105`), donc le JS cross-origin est bloqué en pré-vol. Le risque est résiduel : il devient réel si le token fuit (AUD-A3-002) ou si une session cookie est introduite sans CSRF, et l'absence de vérification d'origine prive d'une défense en profondeur gratuite.
- **Correctif** : exiger `Origin` (ou `Sec-Fetch-Site: same-origin`) sur les méthodes non sûres, ou un en-tête anti-CSRF (`X-Requested-With`) ; plus simple : ne plus accepter le token en query pour les méthodes non-GET (recoupe AUD-A3-002).
- **Test de régression** : tests `PATCH`/`POST`/`DELETE` avec `Origin: https://evil.example` et sans `Origin` ⇒ 403 (si la politique est adoptée), et tests navigateur (Playwright) du parcours normal.
- **Effort** : 0,25 j-h.

### AUD-A3-006 — Purge destructive protégée par un simple écho client

- **Sévérité** : Faible. **Confiance** : Élevée (statique ; la purge n'a pas été rejouée pour ne pas détruire la DB isolée, mais les tests d'intégration la couvrent).
- **Composants** : `apps/lc-server/src/http/admin/guilds.rs:169-280`.
- **Preuve statique** : `guilds.rs:181-183` : `confirmGuildId` doit être **strictement égal** au path, sinon 400 `confirm_guild_id_mismatch` — c'est une confirmation **fournie par le client**, sans ré-authentification, sans double validation serveur, sans diff ni délai ; la purge supprime réglages, clients, codes, jobs, board et **fichiers média orphelins** (`:260-280`, `remove_file`) ; aucune journalisation applicative de l'action (seul le couple URI/statut passe dans la trace HTTP, AUD-A3-012).
- **Impact** : un opérateur (ou un porteur de token, cf. AUD-A3-002) peut détruire irréversiblement les données d'une guilde avec une seule requête ; l'écho de `confirmGuildId` protège d'une mauvaise URL mais pas d'une erreur humaine ni d'un token compromis. Les fichiers média supprimés ne sont pas récupérables.
- **Correctif** : exiger une confirmation à deux étapes (endpoint de prévisualisation `GET /admin/api/guilds/{id}/purge-preview` + token de confirmation signé/à durée courte), ou la ré-authentification du token (rejeu du Bearer) ; journaliser l'action (guilde, compte, compteurs) en `WARN`/audit ; envisager une corbeille (déplacement des fichiers au lieu de `remove_file`) pour une fenêtre de rétention.
- **Test de régression** : test que la purge sans `confirmGuildId`/avec valeur différente renvoie 400 (existe) ; test de l'audit log ; test de la corbeille si implémentée.
- **Effort** : 0,5–1 j-h.

### AUD-A3-007 — Hub admin global hors `AppState`, second montage silencieux, doc de câblage obsolète

- **Sévérité** : Faible. **Confiance** : Élevée.
- **Composants** : `apps/lc-server/src/http/admin/hub.rs:109-127`, `apps/lc-server/src/wiring.rs:34-56`, `:106-110`, `apps/lc-server/src/http/admin/tests.rs:1244-1252`.
- **Preuve statique** : `hub.rs:109` `static ADMIN_HUB: OnceLock<Arc<dyn AdminHub>>` ; `:114-115` `install_admin_hub` retourne `bool` et **ignore le second montage** ; `:123-127` `admin_hub(_state)` ignore `AppState` et crée un `NoopAdminHub` si absent ; `wiring.rs:34-56` délègue correctement au hub WS réel (méthodes async incluses) ; `:106-110` `install` avertit si déjà installé.
- **Impact** : couplage global invisible dans les signatures (un handler peut croire utiliser l'état alors qu'il utilise un hub process-global) ; tests **ordre-dépendants** (le premier `install_admin_hub` gagne, `tests.rs:1244-1252` le documente) ; en cas de double appel (ex. deux `run()` en tests, hot reload), le hub réel peut être ignoré → révocation sans fermeture WS. La doc `server-admin.md` (§12) décrit encore un câblage « à venir » alors que `wiring.rs` l'implémente — risque de confusion pour les prochains intervenants.
- **Correctif** : déplacer le hub dans `AppState` (`Option<Arc<dyn AdminHub>>` ou `Arc<dyn AdminHub>` injecté dans `router_with_hub`) et passer `Extension`/state aux handlers ; conserver `NoopAdminHub` comme défaut de test ; mettre à jour la doc §12 ; test d'intégration qui monte deux fois et vérifie que le hub effectif est bien celui du `run`.
- **Effort** : 0,5 j-h.

### AUD-A3-008 — Création de client ingest : label non borné, pas de dédoublonnage

- **Sévérité** : Faible. **Confiance** : Élevée.
- **Composants** : `apps/lc-server/src/http/admin/ingest_authors.rs:92-150`, `:275-285` (`generate_token`).
- **Preuve statique** : `ingest_authors.rs:105-116` : `label` seulement trim/non vide, puis `device_label = "{label}-{author_display}"` **sans borne de longueur** (le corps HTTP est borné à 1 MiB par `app.rs` `MAX_REQUEST_BODY_BYTES`, donc jusqu'à ~1 Mio de label) ; aucune recherche d'un client existant (même guildId/auteur) ⇒ chaque clic crée un nouveau client et un nouveau token ; le token est affiché une seule fois (`:118-146`).
- **Impact** : pollution de la table `ingest_clients` et de l'UI (labels très longs, N tokens actifs pour le même auteur), surface de révocation plus large ; un token créé et perdu doit être révoqué manuellement. Pas d'escalade de privilèges (l'auteur doit déjà exister dans le registre fusionné).
- **Correctif** : borner `label` (ex. 120 chars, cohérent avec les auteurs) ; dédoublonner (guildId, auteur) ou exiger un `confirmReplace` explicite ; éventuellement limiter le nombre de clients ingest par guilde ; documenter la rotation.
- **Test de régression** : tests de validation (label > N ⇒ 400 `invalid_label`), test de dédoublonnage, test de quota.
- **Effort** : 0,25 j-h.

### AUD-A3-009 — Fermeture WebSocket à la révocation/purge non couverte de bout en bout

- **Sévérité** : Info. **Confiance** : Élevée (statique).
- **Composants** : `apps/lc-server/src/http/admin/clients.rs:54-58`, `apps/lc-server/src/wiring.rs:34-56`, `:141-150`, `apps/lc-server/src/http/admin/tests.rs:956`, `:1244`.
- **Preuve statique** : `clients.rs:54-58` ferme la socket (`hub.close_client(id, 4402)`) **seulement si** `revoked` est vrai (idempotence) ; la délégation `wiring.rs:34-56` est correcte mais testée uniquement avec un hub vide (`wiring.rs:141-150`) ; les tests admin installent un `NoopAdminHub` (`tests.rs:1244-1252`) ou utilisent l'extracteur sans hub réel, donc **aucun test n'exerce une vraie connexion WS** puis révocation → close 4402.
- **Impact** : régression silencieuse possible si le hub WS change de contrat ou si l'installation globale (AUD-A3-007) rate ; en production, un client révoqué pourrait rester connecté jusqu'à la prochaine déconnexion. Risque fonctionnel/ sécurité faible (le token est déjà révoqué, les actions WS suivantes doivent vérifier le client — à confirmer côté lane WS).
- **Correctif** : test d'intégration avec `tokio-tungstenite` : handshake hello avec un client réel, `POST revoke`, attente d'une frame close 4402 ; idem purge. À défaut, test unitaire de `wiring::install` avec un mock comptant les appels.
- **Effort** : 0,5 j-h (test d'intégration WS).

### AUD-A3-010 — Page `/admin/` publique (décision E9) et copies d'interface trompeuses

- **Sévérité** : Info. **Confiance** : Élevée (PoC).
- **Composants** : `apps/lc-server/src/http/admin/static_files.rs:49-65`, `static/admin/index.html:15`, `:29`.
- **Preuve statique/dynamique** : la page est servie sans auth (décision documentée E9, `server-admin.md:19-20`) et ne contient **aucune donnée métier** avant appel API (le DOM initial est statique ; vérifié par dump Chrome headless : aucun token, aucun agrégat) ; `index.html:15` « Accès local uniquement. Token requis. » est faux si `ADMIN_UI_LOCAL_ONLY=false` ; `:29` « persisté dans le fichier `.env` » est faux (c'est `admin_runtime.json`).
- **Impact** : découvrabilité du panneau (info), et surtout **documentation opérateur erronée** qui peut conduire à croire que l'accès est restreint ou que le réglage va dans un fichier partagé (perte de réglage, confusion de sauvegarde).
- **Correctif** : corriger les deux libellés ; rendre le texte conditionnel si possible (ou neutre : « Accès protégé par token ») ; conserver la page publique (décision) mais documenter le caractère volontaire.
- **Effort** : 0,1 j-h.

### AUD-A3-011 — `ADMIN_UI_LOCAL_ONLY` inopérant derrière un reverse proxy local ; `TRUST_PROXY_HEADERS` sans effet sur l'auth admin

- **Sévérité** : Faible. **Confiance** : Moyenne (statique, non testé avec un vrai proxy).
- **Composants** : `apps/lc-server/src/http/admin/auth.rs:111-120`, `apps/lc-server/src/http/rate_limit.rs:230-258`, `docs/rust/admin-panel.md:221`, `docs/rust/server-admin.md:172-173`.
- **Preuve statique** : l'auth admin lit **uniquement** `ConnectInfo<SocketAddr>` ; si un reverse proxy (nginx/Caddy) tourne sur le même hôte et proxifie `/admin`, toutes les requêtes externes arrivent de `127.0.0.1` ⇒ `ADMIN_UI_LOCAL_ONLY=true` ne filtre **rien** (faux sentiment de sécurité). `TRUST_PROXY_HEADERS` n'est consulté que par le rate-limit (`rate_limit.rs:230-258`), jamais par l'admin. `admin-panel.md:221` écrit pourtant « s'appuyer sur `ConnectInfo<SocketAddr>` … **sauf `TRUST_PROXY_HEADERS=true` explicite** », ce qui laisse croire à un effet sur l'auth admin ; `server-admin.md:172-173` est plus précis (XFF non consulté) mais sans avertir du cas proxy local.
- **Impact** : un opérateur qui place l'admin derrière un proxy tout en laissant `ADMIN_UI_LOCAL_ONLY=true` croit restreindre l'accès au loopback alors que **tous** les clients externes passent. Combiné à un token faible (AUD-A3-003), risque direct.
- **Correctif** : documenter explicitement que `ADMIN_UI_LOCAL_ONLY` compare l'IP TCP et n'est donc pas une protection derrière un proxy ; soit ne pas proxifier `/admin` (recommandé, accès tunnel/SSH), soit passer `ADMIN_UI_LOCAL_ONLY=false` et compter sur token+TLS+réseau isolé ; corriger la phrase ambiguë d'`admin-panel.md:221`.
- **Test** : test d'intégration simulant `ConnectInfo` = 127.0.0.1 avec `X-Forwarded-For` externe ⇒ démontre que le filtre accepte (déjà vrai via `tests.rs:104`), à compléter par une documentation et éventuellement un warn au démarrage si `TRUST_PROXY_HEADERS=true` + `LOCAL_ONLY=true`.
- **Effort** : 0,1–0,25 j-h (doc + warning).

### AUD-A3-012 — Traçabilité des mutations admin limitée

- **Sévérité** : Info. **Confiance** : Élevée.
- **Composants** : `apps/lc-server/src/app.rs:223-243`, `apps/lc-server/src/http/admin/auth.rs`, `docs/rust/server-admin.md`.
- **Preuve statique** : la seule trace est le span HTTP (`method`, `uri` — donc query, cf. AUD-A3-002 —, `status`, latence) ; `include_headers=false` ; il n'y a **qu'un seul token partagé**, donc aucune identité d'opérateur ; aucune ligne de log applicative pour purge/stop/révocation/édition des réglages (seuls `AdminError::db` journalise les erreurs DB).
- **Impact** : après un incident (purge accidentelle, réglage modifié), il est impossible d'attribuer l'action à un opérateur ni de reconstituer le corps de la requête ; les journaux JSON de Docker sont la seule source et contiennent le token (AUD-A3-002).
- **Correctif** : journaliser en `INFO` structuré chaque mutation (route, cible, compteurs, résultat, identifiant de requête) sans secret ; si plusieurs opérateurs sont prévus, introduire des tokens nominatifs ou une session avec identité.
- **Effort** : 0,25–0,5 j-h.

---

## 5. Points forts (à préserver)

1. **Contrôle d'accès centralisé** par extracteur `FromRequestParts` : aucune route `/admin/api/*` n'oublie l'auth (vérifié par grep + tests + 401 sonde) ; ordre `503 → 403 → 401` correct et **fail-closed** si `ConnectInfo` manque (`auth.rs:111-120`).
2. **Aucune confiance aux en-têtes de proxy** pour l'auth admin : `X-Forwarded-For: 127.0.0.1` forgé depuis le LAN reste 403 (probe) ; `TRUST_PROXY_HEADERS` est confiné au rate-limit.
3. **Comparaison à temps constant** (`subtle::ConstantTimeEq`) après contrôle de longueur, comportement aligné sur le legacy documenté (`auth.rs:159-168`).
4. **CSP admin stricte et dédiée** : `default-src 'none'; script-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`, `base-uri 'none'`, `form-action 'none'` ; la page n'utilise **aucun script inline** (`index.html` charge `app.js` en `defer`) et le CSP global ne l'écrase pas (`http/security.rs:51-57`, probe des en-têtes).
5. **`no-store` systématique** : API, erreurs 401/403/503, page et assets statiques (probe) ; `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, `Permissions-Policy` héritée.
6. **Aucun XSS trouvé** : `escapeHtml` appliqué à **toutes** les interpolations `innerHTML` (`app.js:78-85`, usages `:219-550`) ; les messages d'erreur passent par `textContent` (`:121-125`) ; payloads stockés `"<img src=x onerror=document.title=\"XSS_A3\">"` insérés par SQL dans les tables guildes/pairing/auteurs/clients puis rendus en navigateur headless : le DOM contient `&lt;img …&gt;` échappé, **aucun élément `<img onerror>` créé**, titre de page inchangé (`dom-admin.html`, `overview-xss.json`).
7. **Injections SQL** : toutes les requêtes admin sont paramétrées (`queries.rs`, `pairings.rs`, `guilds.rs`) ; les identifiants sont typés (`GuildId`, UUID) et validés avant requête.
8. **Secrets** : token ingest haché SHA-256, affiché en clair une seule fois ; token admin jamais renvoyé par l'API ; `Secret` debug redacted (`config.rs:143-164`).
9. **Contrats et erreurs** : camelCase, dates ISO-8601 ms `Z`, codes d'erreur legacy conservés (`invalid_payload`, `item_not_found`, `media_not_found`, `media_ingestion_failed`…), erreurs internes sans fuite de détail (`http/admin/error.rs:39-45`).
10. **Couverture** : 27 tests d'intégration couvrant les 13 endpoints + auth + en-têtes + statique (`tests.rs`, `cargo test` du fichier : 27 passés lors du run d'audit initial du dépôt).

---

## 6. Zones non couvertes / limites de l'audit

- **Pas de build** : les binaires utilisés sont ceux du dépôt (`target/debug`, 01:43) ; le code compilé correspond à l'état observé des sources, mais un `cargo build` postérieur pourrait différer.
- **Pas de proxy TLS réel** : AUD-A3-001/011 s'appuient sur la configuration Compose et la lecture du code ; un essai nginx/Caddy est recommandé en Phase 9 (runbook de bascule).
- **Pas de permissions POSIX** : l'audit tourne sous Windows ; le `chmod` par défaut du fichier `admin_runtime.json` en conteneur Linux n'a pas été mesuré (statique : aucune API de permissions n'existe, umask standard ⇒ risque AUD-A3-004 confirmé par le code, pas par une mesure Linux).
- **Pas de vraie socket WebSocket** : AUD-A3-009 est statique (NoopAdminHub dans les tests) ; la fermeture 4402 réelle reste à tester en intégration (suggestion fournie).
- **Pas de test des navigateurs autres que Chrome headless** : le front a été exécuté sous Chrome 157 ; Firefox/Safari non testés (pas d'API exotiques utilisées).
- **Pas d'audit du pipeline média** (SSRF, contenu,types MIME) : hors périmètre A3, audité en A4 ; noter que la même origine sert les médias (`Content-Type = asset.mime`) et que `localStorage` du token admin partage cette origine ⇒ un futur défaut XSS hors admin pourrait exfiltrer le token (risque transversal documenté dans `admin-panel.md:215-221`).
- **Timing** : la comparaison à temps constant n'a pas été mesurée statistiquement (revue de code + primitive `subtle`).

---

## 7. Recommandations priorisées

| Priorité | Action | Réf. | Effort |
|----------|--------|------|--------|
| P0 | Ne jamais journaliser la query string (`make_span` → path seul) et retirer la réécriture d'URL avec token côté front ; supprimer/restrindre le repli `?token=` | AUD-A3-002 | 0,5–1 j-h |
| P0 | Durcir le déploiement Compose : publication loopback du port, TLS/reverse proxy obligatoire, réseau d'admin isolé documenté, garde-fou au démarrage | AUD-A3-001 | 0,25–0,5 j-h |
| P1 | Rate-limit dédié aux échecs d'auth admin + validation d'entropie/longueur du token | AUD-A3-003 | 0,5–0,75 j-h |
| P1 | Crash-test de la purge (confirmation en deux étapes, audit log) | AUD-A3-006, AUD-A3-012 | 0,5–1 j-h |
| P2 | Masquer le cookie TikTok en lecture, permissions de fichier, libellés UI exacts | AUD-A3-004, AUD-A3-010 | 0,25–0,5 j-h |
| P2 | Vérifier `Origin`/`Sec-Fetch-Site` sur les mutations | AUD-A3-005 | 0,25 j-h |
| P2 | Hub dans `AppState`, doc de câblage mise à jour, test WS réel | AUD-A3-007, AUD-A3-009 | 1 j-h |
| P3 | Bornes/dédoublonnage des clients ingest ; doc local-only/proxy | AUD-A3-008, AUD-A3-011 | 0,3–0,5 j-h |

---

## 8. Annexes

### 8.1 Environnement de reproduction

```
Binaire      : target/debug/lc-server.exe (3.0.0, protocole 2, schéma 3.0.0)
Port         : 18181 (bind 0.0.0.0), instances annexes 18182
Env          : NODE_ENV=production, RUST_LOG=info, API_URL=http://127.0.0.1:18181,
               DATABASE_URL=sqlite://.phase0/audit/A3/data/audit.sqlite?mode=rwc,
               MEDIA_STORAGE_DIR=.phase0/audit/A3/media, MEDIA_URL_SECRET=<64 hex>,
               ADMIN_UI_TOKEN=probe-admin-token-A3, ADMIN_UI_LOCAL_ONLY=true
Navigateur   : Chrome 157.0.8103.40, --headless=new, --virtual-time-budget=12000
Artefacts    : .phase0/audit/A3/{probes.txt,server.out.log,overview-xss.json,dom-admin.html,data/}
```

### 8.2 Reproductibilité condensée

```bash
# Auth : ordre / local-only / fail-closed / pas de confiance XFF
curl -i http://127.0.0.1:18181/admin/api/overview                        # 401 unauthorized
curl -i -H 'Authorization: Bearer probe-admin-token-A3' \
     http://127.0.0.1:18181/admin/api/overview                            # 200 JSON
curl -i http://<IP-LAN>:18181/admin/api/overview                          # 403 admin_local_only
curl -i -H 'Authorization: Bearer probe-admin-token-A3' \
     -H 'X-Forwarded-For: 127.0.0.1' http://<IP-LAN>:18181/admin/api/overview  # 403 (XFF ignoré)

# Token en URL + journalisation
curl -i 'http://127.0.0.1:18181/admin/api/overview?token=probe-admin-token-A3'   # 200
curl -i -X PATCH -H 'Content-Type: application/json' -H 'Origin: https://evil.example' \
     --data '{"tiktokCookie":"sessionid=XSS"}' \
     'http://127.0.0.1:18181/admin/api/runtime-settings?token=probe-admin-token-A3'  # 200
grep 'token=probe-admin-token-A3' .phase0/audit/A3/server.out.log               # token en clair, INFO

# Rate-limit : exempt admin / actif ailleurs
for i in $(seq 1 110); do curl -s -o /dev/null -w '%{http_code}\n' \
  'http://127.0.0.1:18181/admin/api/overview?token=wrong'; done | sort | uniq -c  # 401 x110
for i in $(seq 1 101); do curl -s -o /dev/null -w '%{http_code}\n' \
  'http://127.0.0.1:18181/health'; done | sort | uniq -c                        # 429 après 100

# Runtime settings : cookie TikTok en clair
curl -s -H 'Authorization: Bearer probe-admin-token-A3' \
     http://127.0.0.1:18181/admin/api/runtime-settings   # {"hasTikTokCookie":true,"tiktokCookie":"…"}

# Désactivation (503) : instance avec ADMIN_UI_TOKEN vide
curl -i http://127.0.0.1:18182/admin/api/overview        # 503 admin_ui_disabled
curl -i http://127.0.0.1:18182/admin/                    # 200 page publique

# XSS stocké : insertion SQL puis rendu navigateur
python .phase0/audit/A3/probe_insert2.py
chrome --headless=new --virtual-time-budget=12000 --dump-dom \
  'http://127.0.0.1:18181/admin/?token=probe-admin-token-A3' > .phase0/audit/A3/dom-admin.html
grep -o '&lt;img[^&]*&gt;' .phase0/audit/A3/dom-admin.html   # payload échappé, pas d'élément actif
```

### 8.3 Couverture des contrôles demandés

| Contrôle demandé | Verdict | Référence |
|------------------|---------|-----------|
| Extraction token Bearer puis `?token=` | conforme, **query token à restreindre** | `auth.rs:123-156`, AUD-A3-002 |
| Ordre 503 → 403 → 401 | conforme (PoC 503, 403, 401, 200) | `auth.rs:55-74`, probes |
| Comparaison timing-safe | conforme (longueur puis `ct_eq`) | `auth.rs:159-168` |
| Local-only via `ConnectInfo` | conforme, fail-closed, XFF ignoré ; **limite proxy** | `auth.rs:99-120`, AUD-A3-011 |
| Spoofing `X-Forwarded-For` | refusé (403 LAN malgré XFF 127.0.0.1) | probe |
| `TRUST_PROXY_HEADERS` | n'affecte que le rate-limit, pas l'auth admin | `rate_limit.rs:230-258` |
| Page `/admin/` publique sans donnée sensible | conforme (DOM statique vide de données) | `static_files.rs:49-65`, probe |
| 13 handlers : contrats camelCase/ISO, autorisations, erreurs | conformes | table §3.2, `tests.rs` |
| CSRF sur PATCH/POST/DELETE | pas de CSRF exploitable (pas de cookie) ; contrôle Origin absent | AUD-A3-005 |
| XSS stockée/réfléchie dans `app.js` | **aucun XSS trouvé** (échappement systématique + PoC navigateur) | §5.6, `dom-admin.html` |
| CSP admin (`style-src 'unsafe-inline'`, `script-src 'self'`) | conforme `script-src 'self'`, `object-src 'none'` ; `style-src 'unsafe-inline'` justifié par l'attribut `style` inline | `mod.rs:60-66` |
| `no-store` | conforme sur API, erreurs, page, assets | probe |
| `admin_runtime.json` : atomicité, permissions, validation au reload | partiel : écriture tmp+rename (fenêtre Windows), **aucune permission explicite**, JSON non validé (fallback warn) | `runtime_settings.rs:115-215`, AUD-A3-004 |
| Polling 2 s/12 s | conforme, pause pendant saisie | `app.js:25-26`, doc `admin-panel.md:62` |
| Exemption rate-limit `/admin*` | effective et **trop large** | `rate_limit.rs:226`, AUD-A3-003 |
| Révocation clients → close WS via AdminHub | code conforme, **non testé E2E** | `clients.rs:54-58`, AUD-A3-009 |
| Purge / stop | fonctionnels ; purge = écho client + irréversible | `guilds.rs:133-280`, AUD-A3-006 |

### 8.4 Références principales

- `apps/lc-server/src/http/admin/auth.rs` (209 l.), `mod.rs` (190 l.), `hub.rs` (149 l.), `error.rs` (103 l.), `static_files.rs` (121 l.) ;
- `apps/lc-server/src/http/admin/{overview,runtime_settings,clients,pairings,guilds,ingest_authors,queries}.rs` (309/317/150/180/428/374/271 l.) ;
- `apps/lc-server/src/http/admin/tests.rs` (1 111 l., 27 tests) ;
- `apps/lc-server/src/http/{security,rate_limit,cors}.rs`, `apps/lc-server/src/{app,lib,wiring,config}.rs` ;
- `static/admin/{index.html,app.js,styles.css}` (86/1 172/329 l.) ;
- `docs/rust/server-admin.md` (484 l.), `docs/rust/admin-panel.md` (229 l.), `docs/rust/packaging.md` (§10 Compose) ;
- `deploy/docker-compose.yml`, `deploy/.env.example` ;
- Artefacts : `.phase0/audit/A3/probes.txt`, `server.out.log`, `overview-xss.json`, `dom-admin.html`, `data/audit.sqlite`.
