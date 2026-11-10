# Panneau d'administration — implémentation Rust (Phase 7b)

> Document de référence (SSOT) pour le panneau d'administration du serveur Rust
> 3.0. Contrat fonctionnel d'origine : `docs/rust/admin-panel.md` (analyse du
> legacy `server/src/components/admin/adminRoutes.ts`). Ce document décrit
> l'**implémentation** : fichiers, décisions, contrats exacts, divergences et
> procédures.
>
> Statut : implémenté en Phase 7b (lane `task-18`). Périmètre : panneau
> d'administration uniquement ; la page HTML, le CSS et le JS livrés en Phase 7a
> (`static/admin/`) sont repris **tels quels**, sans modification.

## 1. Périmètre et découpage

Le panneau est servi par le serveur Rust sous le préfixe `/admin` :

| Élément | Décision |
|---|---|
| Page et assets (`/admin/`, `/admin/app.js`, `/admin/styles.css`) | **publics** : aucun token requis pour charger la coquille HTML/JS/CSS |
| 13 endpoints JSON (`/admin/api/...`) | **protégés** par le token d'administration |
| Token | `ADMIN_UI_TOKEN`, transmis en `Authorization: Bearer` avec repli `?token=` |
| Restriction réseau | `ADMIN_UI_LOCAL_ONLY=true` (défaut) : seules les connexions loopback |

### Décision E9 — page publique, API protégées

L'analyse de sécurité (`docs/rust/admin-panel.md`, point E9) relevait un
problème d'ergonomie : le JS a besoin du token (`?token=`) puis le stocke en
`localStorage`, mais la page elle-même ne contient aucune donnée sensible. En
v1, la page était **protégée** ; en v2 elle est **publique** (assets seulement).
Le token reste obligatoire pour chaque appel `/admin/api/...`, et cette
décision est couverte par des tests (`page_admin_publique_mais_api_protegee`).

## 2. Fichiers livrés

Tout est sous `apps/lc-server/src/http/admin/` (aucun autre fichier du crate
n'a été modifié) :

| Fichier | Rôle |
|---|---|
| `mod.rs` | routeur `/admin`, couche d'en-têtes `secure`, helpers partagés |
| `auth.rs` | extracteur `AdminAccess` (503/403/401), lecture Bearer/`?token=` |
| `hub.rs` | trait `AdminHub`, `NoopAdminHub`, installation `OnceLock` |
| `static_files.rs` | `ServeDir` sur `static/admin`, redirection `/admin` → `/admin/` |
| `overview.rs` | `GET /admin/api/overview` |
| `runtime_settings.rs` | `GET`/`PATCH /admin/api/runtime-settings`, fichier JSON |
| `ingest_authors.rs` | `GET /admin/api/ingest-authors`, `POST /admin/api/ingest-clients` |
| `guilds.rs` | settings partiels, stop, purge d'une guilde |
| `clients.rs` | révocation des clients overlay et ingest |
| `pairings.rs` | liste / purge des expirés / révocation des codes de pairing |
| `queries.rs` | requêtes SQL brutes que les repositories de `lc-db` ne couvrent pas |
| `error.rs` | `AdminError` → réponses `{"error": code}` |
| `tests.rs` | 27 tests d'intégration via `crate::app::router()` |

Les routes sont montées par `admin::router()` appelé sans argument depuis
`http/mod.rs` (fichier hors périmètre de cette lane).

## 3. Montage et ordre des couches

```text
app::router()
  └─ merge …
      ├─ admin::router()
      │    ├─ Router::new()
      │    │    ├─ 13 routes /admin/api/...        (handlers)
      │    │    └─ layer(admin_headers)            ─┐
      │    ├─ merge static_files::router()           │ secure() : no-store,
      │    │    ├─ GET /admin → 308 /admin/          │ nosniff, DENY, CSP admin,
      │    │    └─ nest_service("/admin/", ServeDir) │ Permissions-Policy
      │    └────────────────────────────────────────┘
      └─ autres routeurs (root, health, ws, overlay, ingest…)
  └─ couches globales (ordre extérieur → intérieur) :
       SetRequestId → PropagateRequestId → trace → security_headers
       → cors → CompressionLayer → normalize_errors → rate_limit
       → DefaultBodyLimit(1 Mio)
```

Deux conséquences importantes :

1. **`/admin*` est exempté du rate limit** (comme dans le legacy ; voir
   `http/rate_limit.rs`). Les rafraîchissements automatiques du panneau
   (overview toutes les 2 s) ne doivent pas déclencher de 429.
2. La couche `security.rs` globale posait ses en-têtes en **écrasant** ceux des
   handlers ; le Lead l'a corrigée en `entry(...).or_insert(...)`. La couche
   admin peut donc poser sa CSP stricte avec `insert` sans être écrasée.

### Middleware d'en-têtes (et non `tower-http::set-header`)

`tower_http::set_header` aurait été l'outil naturel, mais la feature
`set-header` n'est pas activée pour `lc-server` et `Cargo.toml` est hors
périmètre de la lane. La couche `secure()` est donc un middleware maison :

```rust
pub(crate) fn secure(router: Router<AppState>) -> Router<AppState> {
    router.layer(middleware::from_fn(admin_headers))
}
```

`admin_headers` pose les en-têtes sur **toutes** les réponses `/admin`
(statique compris) :

| En-tête | Valeur |
|---|---|
| `cache-control` | `no-store` |
| `x-content-type-options` | `nosniff` |
| `x-frame-options` | `DENY` |
| `referrer-policy` | `no-referrer` |
| `permissions-policy` | héritée de la couche globale `http::security` : `camera=(), microphone=(), geolocation=(), payment=(), usb=()` (le panneau ne la redéfinit pas ; la couche globale étant extérieure, elle s'applique après les handlers) |
| `content-security-policy` | CSP admin (voir ci-dessous) |

### CSP admin

```text
default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline';
img-src 'self' https://cdn.discordapp.com https://media.discordapp.net;
connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none';
frame-ancestors 'none'
```

- `style-src 'unsafe-inline'` est requis par les deux barres de progression
  d'occupation (le JS pose `style="width:…%"` via `innerHTML`, point E6 connu).
- `img-src` autorise les avatars Discord et les médias servis par le serveur.
- Le test `csp_admin_et_csp_globale_ne_se_recouvrent_pas` vérifie, via
  `app::router()`, que `/admin/` renvoie la CSP admin et que `/health` continue
  de renvoyer la CSP globale (`http::security::CONTENT_SECURITY_POLICY`) sans
  hériter du `no-store`.

## 4. Authentification

L'authentification est un **extracteur axum**, pas un middleware. Un
`middleware::from_fn` posé sur un `Router<AppState>` avant l'hydratation de
l'état ne peut pas extraire `State<AppState>` (et `admin::router()` est appelé
sans argument), d'où le choix suivant :

```rust
pub struct AdminAccess; // Debug + Clone + Copy, sans donnée

impl FromRequestParts<AppState> for AdminAccess {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response>;
}
```

Chaque handler JSON déclare `_access: AdminAccess` en paramètre : la
vérification a lieu **avant** l'exécution du handler, et la signature garantit
qu'un futur endpoint ne peut pas oublier l'authentification.

### Ordre de vérification (identique au legacy)

1. `ADMIN_UI_TOKEN` vide/espaces → **503**
   `{"error":"admin_ui_disabled","message":"ADMIN_UI_TOKEN is not configured"}`
2. `ADMIN_UI_LOCAL_ONLY=true` et IP non loopback → **403**
   `{"error":"admin_local_only"}`
   - connect info absent (ex. test unitaire sans `ConnectInfo`) ⇒ considéré non
     loopback, donc **refus**.
3. Token absent ou différent → **401** `{"error":"unauthorized"}`.
4. Sinon le handler s'exécute.

### Lecture du token

- `Authorization: Bearer <token>` **prioritaire** (scheme insensible à la casse,
  token trimé non vide).
- Repli `?token=<token>` (décodé via `url::form_urlencoded::parse`), utilisé par
  `app.js` qui le lit une fois puis envoie le Bearer.
- Comparaison **à temps constant** : égalité de longueur puis
  `subtle::ConstantTimeEq` (identique à `safeTokenEquals` du legacy).

### IP loopback

Sont acceptés : `127.0.0.0/8` (`Ipv4Addr::is_loopback`), `::1`, et les adresses
IPv4-mappées (`::ffff:127.x.x.x` via `to_ipv4_mapped`). La confiance dans
`ConnectInfo` repose sur `axum::serve(... into_make_service_with_connect_info)`
utilisé par `lib.rs` (`X-Forwarded-For` n'est **pas** consulté ; le legacy suivait
`trustProxyHeaders` mais l'admin reste réservé au loopback).

## 5. Assets statiques

### Résolution du dossier

`static_files::router()` cherche, dans l'ordre :

1. variable d'environnement `ADMIN_STATIC_DIR` ;
2. `static/admin` relatif au répertoire courant ;
3. `static/admin` à côté de `current_exe()` ;
4. `current_exe()/../../static/admin` (dev : `target/debug/…`) ;
5. `CARGO_MANIFEST_DIR/../../static/admin` (tests et `cargo run`).

En dernier recours, le chemin `static/admin` est utilisé (un `warn!` est
journalisé si aucun candidat n'existe). Le dossier est résolu **au démarrage**,
pas à chaque requête.

### Routes statiques

- `GET /admin` → **308** `Location: /admin/` (le JS construit des URLs
  absolues ; le slash final est obligatoire, point E15).
- `GET /admin/` → `ServeDir` avec `append_index_html_on_directories(true)`
  (sert `index.html`).
- `GET /admin/app.js`, `/admin/styles.css` → fichiers tels quels, en-têtes
  `secure()` appliqués (dont `no-store`).

## 6. Trait `AdminHub`

Le panneau ne dépend **pas** directement du hub WebSocket réel : il dialogue via
un trait installé une seule fois par processus. `protocole-v2` (`task-17`)
implémente ce trait sur le hub réel ; le câblage final (appel de
`install_admin_hub`) appartient au Lead.

```rust
#[async_trait::async_trait]
pub trait AdminHub: Send + Sync {
    // Synchrone : interrogation d'état, utilisable dans l'overview sans await.
    fn connected_count(&self) -> usize;
    fn connected_for_guild(&self, guild_id: &GuildId) -> usize;
    fn is_connected(&self, client_id: &Uuid) -> bool;
    fn close_client(&self, client_id: &Uuid, code: u16) -> bool;

    // Asynchrone : diffusion d'ordres aux sockets.
    async fn broadcast_stop(&self, guild_id: &GuildId) -> usize;
    async fn broadcast_settings(&self, guild_id: &GuildId) -> usize;
}
```

- `NoopAdminHub` (0 / `false`) est l'implémentation par défaut : le panneau
  fonctionne sans hub (utilisé par tous les tests).
- `install_admin_hub(Arc<dyn AdminHub>) -> bool` ne réussit qu'une fois
  (`OnceLock`) ; un second appel renvoie `false` (utilisé comme sonde par les
  tests).
- `admin_hub(&AppState)` renvoie le hub installé ou `NoopAdminHub`.
- Les handlers font `await` sur les broadcasts ; la sémantique des compteurs
  renvoyés (nombre de sockets touchés) est celle de l'implémentation réelle.

## 7. Contrats des 13 endpoints

Tous les corps sont en JSON, en `camelCase`, dates ISO 8601 UTC
(`to_rfc3339_opts(SecondsFormat::Millis, true)`, suffixe `Z`). Tous exigent
l'authentification (§4). Toute erreur de validation renvoie
`{"error":"<code>"}` avec le statut indiqué.

### 7.1 `GET /admin/api/overview`

Agrégat du panneau (une requête par famille de données, en parallèle) :

```json
{
  "generatedAt": "2026-11-10T00:00:00.000Z",
  "cache": {"usedBytes": 0, "maxBytes": 5368709120, "remainingBytes": 5368709120, "usedPercent": 0.0},
  "totals": {"guildCount": 0, "overlayClientCount": 0, "overlayConnectedCount": 0, "ingestClientCount": 0, "pairingCodeCount": 0},
  "pairingCodes": {"active": 0, "expired": 0, "total": 0},
  "guilds": [{
    "id": "42", "name": "Guilde 42", "editableName": "Guilde 42", "busyUntil": null,
    "settings": {
      "defaultMediaTime": 5, "maxMediaTime": null, "displayMediaFull": false,
      "overrides": {"defaultMediaTime": null, "maxMediaTime": null, "displayMediaFull": null}
    },
    "overlays": {"total": 0, "connectedCount": 0, "clients": [
      {"id": "…", "label": "…", "authorName": null, "authorImage": null,
       "lastSeenAt": null, "createdAt": "…", "connected": false, "sessionMode": "normal"}
    ]},
    "ingest": {"total": 0, "clients": [{"id": "…", "label": "…", "lastSeenAt": null, "createdAt": "…"}]},
    "memeBoard": {"itemsCount": 0, "usedBytes": 0, "maxBytes": 16106127360, "remainingBytes": 16106127360, "usedPercent": 0.0},
    "playback": {"pending": 0, "playing": 0, "failed": 0, "done": 0}
  }]
}
```

Règles d'agrégation :

- `guilds` = union triée (`BTreeSet<String>` puis ordre `id`) des guildes
  présentes dans `guilds`, `overlay_clients` (actifs), `ingest_clients` (actifs),
  `meme_board_items`, `playback_jobs` et `pairing_codes`, plus les guildes
  présentes dans le hub.
- `name` = nom persisté trimé, sinon `Guild {id}` ; `editableName` = nom persisté
  sinon `""` (champ conservé pour le contrat, inutilisé par le JS — point E7).
- `settings.defaultMediaTime` = valeur persistée sinon `DEFAULT_DURATION`
  (défaut 5) ; `overrides` distingue explicitement « non défini » (`null`) de la
  valeur par défaut.
- `overlays.clients` ne contient que les clients **actifs** (`revoked_at IS
  NULL`) ; `connected` vient du hub ; `sessionMode` est la valeur **wire**
  (`normal` / `invite_read_only`), priorité à la session du socket connecté.
- `ingest.clients` ne contient que les clients actifs, triés
  `(guild_id, label)` puis regroupés par guilde.
- `memeBoard.usedBytes` = somme des `size_bytes` des médias du board ;
  `cache.usedBytes` = somme des `size_bytes` des médias `READY` **non** présents
  dans `meme_board_items` (les deux ensembles sont disjoints, comme le legacy).
- Bornes : `cache.maxBytes = max(1, MEDIA_CACHE_MAX_TOTAL_MB) * 1024 * 1024`
  (défaut 5120 Mio = 5 Gio) ; `memeBoard.maxBytes = max(1,
  MEDIA_BOARD_MAX_TOTAL_MB) * 1024 * 1024` (défaut 15360 Mio = 15 Gio).
- `usedPercent` : `toPercent(used, max)` = 0 si `max <= 0`, sinon
  `clamp(0..100, round(used/max*100, 2))` ; renvoyé en nombre.
- `busyUntil`/dates absentes → `null` (jamais de date « epoch »).
- La résolution du nom Discord (`resolveDiscordGuildName` dans le legacy) n'est
  **pas** faite ici : le hub n'expose pas cette information en v2. Le nom
  affiché est le nom persisté ou `Guild {id}` (divergence documentée §10).

### 7.2 `GET` / `PATCH /admin/api/runtime-settings`

```json
GET  → {"tiktokCookie": "", "hasTikTokCookie": false}
PATCH {"tiktokCookie": "sessionid=…"} → {"updated": true, "tiktokCookie": "sessionid=…", "hasTikTokCookie": true}
```

- `PATCH` : `tiktokCookie` doit être une chaîne (`invalid_payload` sinon, 400).
- Normalisation : préfixe `cookie:`/`Cookie:`/`COOKIE:` retiré, espaces
  extérieurs trimés (`normalizeTikTokCookie` du legacy).
- Le cookie est **stocké en clair** (parité v1) et renvoyé en entier par le GET.
- Persistance : fichier JSON à côté de la base (voir §8), relecture à chaque
  GET/PATCH, rechargement au démarrage suivant.
- Écart assumé avec le legacy, qui réécrivait `.env` : le fichier de réglages
  runtime est isolé du `.env` pour éviter de modifier l'environnement d'un
  serveur en service (§10).

### 7.3 `GET /admin/api/ingest-authors`

Fusion des auteurs connus : codes de pairing + clients d'ingestion, par
`createdByDiscordUserId` :

```json
{"items": [{"discordUserId": "1001", "displayName": "Alice", "image": null,
            "lastSeenAt": "2026-11-10T00:00:00.000Z", "source": "pairing"}],
 "count": 1}
```

- `source` ∈ `pairing`, `ingest`, `mixed` (fusion des deux origines).
- `displayName` = dernier nom non vide connu, sinon l'identifiant.
- `image` = dernière image non vide connue.
- `lastSeenAt` = `created_at` le plus récent entre les deux sources.
- Tri : `displayName` (insensible à la casse/accents de base), puis
  `discordUserId` — équivalent de
  `localeCompare(..., {sensitivity:"base"})` du legacy.

### 7.4 `POST /admin/api/ingest-clients`

```json
body: {"guildId": "42", "authorDiscordUserId": "1001", "label": "Chrome"}
→ 200 {"created": true,
       "ingestApiToken": "<64 hex, affiché UNE SEULE FOIS>",
       "client": {"id": "…", "guildId": "42", "label": "Chrome-Alice",
                  "defaultAuthorName": "Alice", "defaultAuthorImage": null,
                  "createdByDiscordUserId": "1001", "createdAt": "…"}}
```

- Erreurs 400 dans cet ordre : `invalid_guild_id`, `invalid_author_discord_user_id`,
  `invalid_label`, `author_not_found`.
- L'auteur doit exister dans la fusion du §7.3 (`author_not_found` sinon).
- Label persisté = `"{label}-{displayName}"` (suffixe identifiant si le nom est
  vide), comme `deviceLabel` du legacy.
- Token : 32 octets aléatoires (`rand::rng().fill_bytes`) en hexadécimal (64
  caractères). **Seul le SHA-256 hexadécimal est stocké** (`token_hash`) ; le
  token en clair n'apparaît que dans cette réponse — ne jamais le journaliser
  (point E12).

### 7.5 `PATCH /admin/api/guilds/{guildId}/settings`

```json
body: {"defaultMediaTime": 9, "maxMediaTime": null, "displayMediaFull": true}
→ 200 {"updated": true,
       "guild": {"id": "42", "defaultMediaTime": 9, "maxMediaTime": null,
                 "displayMediaFull": true, "busyUntil": null}}
```

- Champs tous optionnels ; au moins un doit être présent
  (`invalid_payload` sinon). Sémantique de conversion identique au legacy
  (`toOptionalGuildSettingInt`) :
  - `undefined` (absent) → on ne touche pas ;
  - `null` ou `""` → `null` ;
  - nombre fini → `Math.floor`, puis `<= 0` → `null` ;
  - autre type → `invalid_payload`.
- Upsert de la ligne `guilds` : les champs absents conservent leur valeur
  existante (fusion explicite relue en base) ; `name` et `busy_until` ne sont
  jamais modifiés.
- Diffusion `hub.broadcast_settings(guild_id)` après écriture (le hub réel
  notifiera les overlays connectés).
- `invalid_guild_id` si l'identifiant n'est pas un `GuildId` valide (trim,
  1..=20 chiffres).

### 7.6 `POST /admin/api/guilds/{guildId}/stop`

```json
→ 200 {"stopped": true, "guildId": "42", "releasedCount": 1}
```

Ordre : `hub.broadcast_stop(guild_id)` (équivalent du `io.to(room).emit(STOP,
{jobId:"manual-stop"})` legacy) → libération SQL des jobs `PLAYING`
(`status=DONE`, `finished_at=now`) → `guilds.busy_until = NULL`. Le compteur
`releasedCount` est le nombre de jobs libérés en base. Le legacy appelait en
plus `playbackScheduler.onManualStop` : le scheduler v2 n'est pas exposé au
panneau (divergence §10).

### 7.7 `POST /admin/api/guilds/{guildId}/purge`

```json
body: {"confirmGuildId": "42", "removeOrphanMedia": true}   // 2e champ optionnel, défaut true
→ 200 {"purged": true, "guildId": "42",
       "stop": {"releasedPlayingJobs": 1, "disconnectedOverlaySockets": 2},
       "deleted": {"guildSettings": 1, "overlayClients": 2, "ingestClients": 1,
                   "pairingCodes": 3, "playbackJobs": 5, "memeBoardItems": 2,
                   "orphanMediaAssets": 1, "orphanMediaFileDeleteErrors": 0}}
```

- Validation : `invalid_guild_id` ; `confirmGuildId` absent ou différent de
  l'identifiant d'URL → `confirm_guild_id_mismatch` ; `removeOrphanMedia`
  présent et non booléen → `invalid_payload`.
- Ordre (identique au legacy) : candidates médias **avant** suppression →
  stop (§7.6) → fermeture des sockets de la guilde → suppression
  ingest/overlay/pairing/jobs/board/guilde → purge des médias orphelins.
- `disconnectedOverlaySockets` = nombre de clients overlay connectés dont la
  socket a été fermée via `hub.close_client(uuid, 1000)` (divergence mineure
  §10 : le legacy comptait les sockets déconnectés, ici les clients distincts).
- Média orphelin : candidat s'il est **référencé par la guilde** (board ou
  playback) et plus référencé par aucun board/job après suppression ; son
  fichier `storage_path` est supprimé (`NotFound` toléré, autre erreur →
  `orphanMediaFileDeleteErrors += 1`), puis la ligne `media_assets`.
- Les échecs de suppression des clients d'ingestion ne bloquent pas la purge
  (compteur 0 + `warn!`), comme le legacy.

### 7.8 `POST /admin/api/overlay-clients/{clientId}/revoke`

```json
→ 200 {"found": true, "revoked": true, "clientId": "…", "guildId": "42", "label": "Salon principal"}
```

- `invalid_client_id` (400) si le paramètre est vide ; UUID invalide ou inconnu
  → **404** `overlay_client_not_found` (parité legacy : un identifiant non
  parsable est traité comme introuvable).
- Révoque (`revoked_at = now`) si encore actif ; `revoked` reflète le nombre de
  lignes modifiées (donc `false` au second appel, contrôlé par test).
- Si révoqué, ferme la socket avec le code **4402** via
  `hub.close_client(uuid, 4402)`.

### 7.9 `POST /admin/api/ingest-clients/{clientId}/revoke`

Identique au §7.8, avec 404 `ingest_client_not_found`. Pas de socket à fermer
(les clients d'ingestion sont authentifiés par requête HTTP).

### 7.10 `GET /admin/api/pairing-codes?guildId&status=all|active|expired`

```json
{"items": [{"code": "ACTIF001", "guildId": "42",
            "createdByDiscordUserId": "1001", "authorName": "Alice",
            "expiresAt": "…", "usedAt": null, "createdAt": "…", "isActive": true}],
 "counts": {"total": 1, "active": 1, "expired": 0}}
```

- `isActive` = `usedAt` absent **et** (`expiresAt` absent ou `> now`).
- Le filtre `status` est appliqué **avant** le calcul de `counts` : les
  compteurs portent sur les éléments renvoyés (comportement legacy conservé).
- `guildId` invalide → filtre appliqué mais aucun résultat (pas d'erreur).
- Tri : `created_at` décroissant puis `code`.
- `status` inconnu/vide → `all`.

### 7.11 `DELETE /admin/api/pairing-codes/expired?guildId`

```json
→ 200 {"deletedCount": 1, "guildId": null}
```

Supprime les codes `used_at IS NOT NULL` **ou** `expires_at <= now`,
éventuellement limités à la guilde fournie. `guildId` renvoyé = valeur de la
requête normalisée, `null` si absent.

### 7.12 `POST /admin/api/pairing-codes/{code}/revoke`

```json
→ 200 {"found": true, "revoked": true, "guildId": "42"}
```

- `invalid_code` (400) si le code est vide ; code normalisé en majuscules.
- Introuvable → 404 `pairing_code_not_found`.
- Révoque **seulement** si le code est encore actif (`used_at IS NULL` et non
  expiré) : `used_at = now` **et** `expires_at = now` (comme le legacy : il
  n'existait pas de colonne « révoqué »). Sinon `revoked: false`.

## 8. Réglages runtime persistés

Fichier : `admin_runtime.json`, **à côté du fichier SQLite** utilisé par le
serveur.

- Chemin déduit de `DATABASE_URL` (préfixes `sqlite://`, `sqlite:`, `file://`,
  `file:` ; `:memory:`/`mode=memory` → répertoire courant) ;
- Contenu : `{"tiktokCookie": "…"}` (camelCase, JSON indenté) ;
- Écriture **atomique** : écriture de `<fichier>.tmp` puis `rename` — sous
  Windows, `rename` échoue si la destination existe, donc le fichier cible est
  supprimé juste avant ; le reste du système n'observe jamais d'état partiel ;
- Lecture : à chaque GET/PATCH (pas de cache mémoire) ; fichier absent ou
  illisible → valeur par défaut issue de `TIKTOK_COOKIE` (variable
  d'environnement), avec `warn!` si le fichier existait mais était illisible ;
- « Rechargé au boot » = au **prochain démarrage** du serveur, pas en cours
  d'exécution.

## 9. Divergences avec le legacy (assumées et documentées)

| Sujet | Legacy TS | Implémentation v2 | Raison |
|---|---|---|---|
| Page `/admin/` | protégée par token | **publique** (assets), API protégées | décision E9 validée par le Lead |
| `.env` | `PATCH runtime-settings` réécrit `.env` | `admin_runtime.json` à côté de la base | ne pas muter l'environnement d'un process en service ; Lead a validé |
| `disconnectedOverlaySockets` | sockets distincts déconnectés | clients overlay distincts fermés via le hub | le hub v2 (trait) expose les clients, pas les sockets |
| Scheduler | `playbackScheduler.onManualStop` appelé | seul `broadcast_stop` + libération SQL | le scheduler v2 n'est pas exposé au panneau (hub = surface d'intégration) |
| Noms de guilde | `resolveDiscordGuildName` (cache + fetch) | nom persisté sinon `Guild {id}` | pas d'accès Discord dans le panneau ; peut être ajouté plus tard via un service dédié |
| Persistance des codes | `updateMany` Prisma | SQL brut (`queries.rs`) | les repositories de `lc-db` ne couvrent pas ces filtres (guilde, `expires_at` posé à la révocation, suppressions en masse) |
| `editableName` | renvoyé mais inutilisé | renvoyé pour parité | point E7 connu, retrait possible plus tard |

## 10. Tests

Tous les tests passent par `crate::app::router()` — donc à travers les couches
globales (en-têtes, rate limit, limite de corps, normalisation d'erreurs) — et
non seulement par `admin::router()`. Chaque test crée sa propre base SQLite en
mémoire et son dossier temporaire (`DATABASE_URL` pointe dedans), donc
`admin_runtime.json` n'est jamais écrit dans le dépôt.

Fichier : `apps/lc-server/src/http/admin/tests.rs` (27 tests d'intégration, plus
les tests unitaires de `auth.rs`).

Couverture par thème :

- **Auth** : 503 token vide ; 403 IP externe ; 401 sans token ; Bearer puis
  `?token=` ; `ADMIN_UI_LOCAL_ONLY=false` accepte un client externe ; page
  `/admin/` publique alors que l'API est protégée.
- **Statique/en-têtes** : 308 `/admin` → `/admin/` ; index + `no-store` + CSP
  admin + Permissions-Policy ; CSP admin (`/admin/`) vs CSP globale (`/health`)
  et absence de `no-store` sur `/health`.
- **Runtime settings** : défauts, PATCH normalisé et persisté
  (`admin_runtime.json` écrit, pas de `.tmp` résiduel), payloads invalides.
- **Overview** : bornes de configuration à vide ; agrégats (board 1 Mio, cache
  2 Mio, 4 statuts de playback, pairing actif/expiré) ; clients actifs
  uniquement et `sessionMode` wire.
- **Guildes** : settings partiels (création puis fusion), erreurs ;
  `stop` libère les jobs `PLAYING` et remet `busy_until` à `NULL` ; purge
  complète (compteurs, fichier orphelin supprimé, média référencé conservé,
  confirmation erronée refusée).
- **Révocations** : overlay (révocation puis `revoked:false`), ingest inconnu
  → 404, UUID invalide → 404.
- **Pairing** : liste/filtre/compteurs, purge des expirés, révocation (dates
  posées) et 404.
- **Ingestion** : fusion des auteurs, création de client (token 64 hexadécimal
  affiché une fois, hash SHA-256 en base, label `Chrome-Alice`), erreurs de
  validation.
- **Hub** : Noop par défaut, installation unique via `OnceLock`.

Commandes de vérification (depuis `apps/lc-server`) :

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_BUILD_JOBS = "4"
cargo fmt -p lc-server -- --check
cargo clippy -p lc-server --all-targets -- -D warnings
cargo test -p lc-server
```

## 11. Runbook et dépannage

### Démarrer le panneau

1. Définir `ADMIN_UI_TOKEN` (non vide) — sinon *toute* l'API répond 503.
2. Vérifier que le serveur écoute bien sur la machine administrée
   (`ADMIN_UI_LOCAL_ONLY=true` par défaut, donc accès local uniquement).
3. Ouvrir `http://127.0.0.1:<port>/admin/` (slash final obligatoire, la page
   redirige `/admin` → `/admin/`), puis saisir le token ou utiliser
   `.../admin/?token=...`.

### Problèmes fréquents

| Symptôme | Cause probable | Action |
|---|---|---|
| 503 `admin_ui_disabled` | `ADMIN_UI_TOKEN` vide | définir la variable puis redémarrer |
| 403 `admin_local_only` | accès depuis une IP non loopback | accéder en local, ou (conscient du risque) `ADMIN_UI_LOCAL_ONLY=false` |
| 401 `unauthorized` | token absent/faux | vérifier le Bearer ou `?token=` |
| 404 `overlay_client_not_found` | identifiant non UUID | le legacy répondait 404 pour un id invalide ; conservé |
| `admin_runtime.json` absent | jamais de PATCH | fichier créé au premier PATCH (défaut = `TIKTOK_COOKIE`) |
| Pas de nom de guilde Discord | pas de résolution Discord en v2 | divergence §9 (nom persisté ou `Guild {id}`) |
| Barres de progression absentes | CSP trop stricte | ne pas retirer `style-src 'unsafe-inline'` (point E6) |

### Sécurité — points d'attention

- Le token admin est comparé à temps constant ; ne jamais le journaliser.
- Le token d'ingestion n'est affiché qu'une seule fois (réponse de création),
  seul son hash est en base.
- La purge est **destructive par défaut** (point E14) et exige
  `confirmGuildId` identique à l'URL.
- La CSP admin et le `no-store` couvrent aussi les assets statiques.

## 12. Hors périmètre / suites possibles

- Résolution du nom de guilde Discord dans l'overview (via un service dédié,
  pas d'appel Discord depuis le panneau).
- Retrait du champ `editableName` si le JS n'en dépend plus (E7).
- Réduction des rafraîchissements automatiques (E11 : overview 2 s, pairing
  12 s) si la charge devient sensible ; `/admin*` est déjà exempté du rate
  limit.
- Stockage chiffré du cookie TikTok (actuellement en clair, parité v1).
- Câblage réel du hub (`install_admin_hub`) : Lead après implémentation du trait
  par `protocole-v2` ; la couche admin n'a pas à changer.
