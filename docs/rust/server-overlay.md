# Serveur Rust — routes `/overlay/*` (Phase 3b)

Statut : **implémenté** (routes, auth, média signé, media board, tests in-process).
Propriétaire : lane `overlay-bridge` (task-15). Sources de vérité : code ci-dessous,
legacy Node (`server/src/components/overlay/overlayRoutes.ts`), protocole v2
(`docs/rust/protocol-v2.md` §8/§9) et plan de migration
(`docs/rust/migration-plan.md`).

## 1. Objet et périmètre

Ce module remplace le sous-système HTTP de l'overlay Electron :

| Legacy Node | Remplacé par |
|-------------|--------------|
| `server/src/components/overlay/overlayRoutes.ts` (568 l.) | `apps/lc-server/src/http/overlay/**` |
| `server/src/services/overlayAuth.ts` (143 l.) | `apps/lc-server/src/http/overlay/auth.rs` + `crates/lc-domain` + `crates/lc-db` |
| `GET /overlay/config` | **supprimé** en v2 (la route répond 404) |
| `?token=` sur les routes board | **Bearer uniquement** en v2 ; repli transitoire `?token=` **toléré sur `/overlay/media-board/*`** (renderer legacy, Bearer prioritaire, WARN unique) — voir §4.1 |

Hors périmètre (autres lanes) : hub WebSocket Phase 4b (`ws/**`), service média
Phase 5b (`media/**` : ingestion en tâche de fond, cache/éviction), admin Phase 7b
(`http/admin/**`), panneau renderer Phase 8. Les dépendances de ces lanes sont
documentées en §12.

## 2. Montage et chaîne de middlewares

`http::overlay::router()` est monté tel quel par `crate::app::router`
(`apps/lc-server/src/app.rs:150`) : aucune modification de `app.rs`, `lib.rs`,
`http/mod.rs` ou `Cargo.toml` n'a été nécessaire. Le sous-routeur porte
lui-même le préfixe `/overlay` (même convention que `http::ingest::router`) :
`app.rs` le fusionne sans `nest`.

```
crate::app::router(state)                       # apps/lc-server/src/app.rs:136-168
 ├─ DefaultBodyLimit::max(1 MiB)
 ├─ rate limiting global (100 req / 15 min)
 ├─ normalize_errors          # conserve un corps JSON, réécrit le reste
 ├─ CompressionLayer (global) # cf. §6.4 et §12.1
 ├─ CORS / security headers / trace / x-request-id
 └─ http::overlay::router()   # apps/lc-server/src/http/overlay/mod.rs:77
      ├─ POST /pair/consume
      ├─ GET  /media/{asset_id}
      └─ media board (auth par extracteur)
```

Conventions appliquées dans tout le module :

- instant courant **toujours** lu via `state.clock` (`mod.rs:99`) — jamais
  `Utc::now()` (parité `docs/rust/server-core.md` §9) ;
- base accédée via `state.db: Option<Db>` ; base absente →
  `503 {"error":"database_unavailable"}` (`mod.rs:129`) ;
- erreurs applicatives déjà en JSON minimal legacy ; `normalize_errors` laisse
  intact un corps JSON (`http/error.rs`) ;
- aucun `unwrap`/`expect`/`panic` hors tests (lints workspace) ;
- `clippy::result_large_err` est neutralisé au niveau du module
  (`#![allow(...)]` justifié dans `mod.rs`) : les helpers renvoient
  `Result<_, Response>` et `Response` est le type d'erreur natif d'axum ; le
  boxing systématique n'apporterait rien sur ces chemins.

### Fichiers

| Fichier | Rôle | Lignes clés |
|---------|------|-------------|
| `http/overlay/mod.rs` | routeur, helpers partagés (`now`, `db`, `json_response`, `error`, `db_failure`), ré-export de `sign_media_url` | `router` :66, `pub use media::sign_media_url` :56 |
| `http/overlay/auth.rs` | extracteur `AuthedOverlayClient`, token/hash, Bearer | extracteur :92, `generate_token` :62, `hash_token` :69, `bearer_token` :78 |
| `http/overlay/pair.rs` | `POST /pair/consume` | handler :82 |
| `http/overlay/media.rs` | `GET /media/{assetId}` signé HMAC, `ServeFile` | handler :81, `sign_media_url` :189, `resolve_media_file` :214, `serve_file` :263, `touch_asset` :300 |
| `http/overlay/media_board.rs` | CRUD board, ingestion asynchrone, liste des ingestions | handlers `create_item`, `list_ingestions`, `complete_board_item` |
| `http/overlay/media_upload.rs` | upload brut → dossier temporaire → tâche asynchrone | `upload_item`, `UploadTempDir` |
| `http/overlay/media_ingestion.rs` | tâche `tokio::spawn` : pipeline, item, registre, WS | `spawn_ingestion`, `run_ingestion` |
| `http/overlay/tests.rs` | 15 tests in-process + 1 test unitaire | liste §11 |

## 3. Table des routes

| Méthode | Chemin | Auth | Handler | Succès |
|---------|--------|------|---------|--------|
| `POST` | `/overlay/pair/consume` | code d'appairage (corps) | `pair.rs:84` | `200` JSON (§5) |
| `POST` | `/overlay/ingest-code` | `Authorization: Bearer` (membre, droit `TriggerPlayback`) | `ingest_code.rs` | `200 {code,expiresAt,apiBaseUrl}` |
| `GET` | `/overlay/media/{assetId}` | URL signée HMAC (`clientId`, `exp`, `sig`) | `media.rs:86` | `200`/`206`, binaire |
| `GET` | `/overlay/media-board/items` | `Authorization: Bearer` (repli `?token=`) | `media_board.rs:list_items` | `200 {total,limit,offset,items}` |
| `POST` | `/overlay/media-board/items` | `Authorization: Bearer` (repli `?token=`) | `media_board.rs:create_item` | `202 {ingestionId}` |
| `POST` | `/overlay/media-board/upload?filename=&title=&message=` | `Authorization: Bearer` (repli `?token=`) | `media_upload.rs:upload_item` | `202 {ingestionId}` |
| `GET` | `/overlay/media-board/ingestions` | `Authorization: Bearer` (repli `?token=`) | `media_board.rs:list_ingestions` | `200 {items:[…]}` |
| `GET` | `/overlay/media-board/media/{assetId}` | `Authorization: Bearer` (repli `?token=`) | `media_board.rs:get_media` | `200`/`206`, binaire |
| `PATCH` | `/overlay/media-board/items/{itemId}` | `Authorization: Bearer` (repli `?token=`) | `media_board.rs:update_item` | `200 {updated:true,item}` |
| `DELETE` | `/overlay/media-board/items/{itemId}` | `Authorization: Bearer` (repli `?token=`) | `media_board.rs:delete_item` | `200 {deleted}` |
| — | `/overlay/config` | — | **absent** | `404 {"error":"request_error"}` (fallback global) |

Les routes protégées portent l'authentification via l'extracteur
`AuthedOverlayClient` (premier paramètre des handlers), exécuté avant le handler
par axum (§4). Aucun middleware `.layer(...)` spécifique n'est nécessaire sur le
sous-routeur.

Les anciens chemins `/overlay/meme-board/...` et la compatibilité overlay 3.0.0
sont supprimés : les routes et les clés `addMeme`/`editAnyMeme` n'existent plus.

**Sessions en lecture seule** : une session `INVITE_READ_ONLY` peut lire
(`GET items`, `GET ingestions`, `GET media`) mais **pas** modifier la board. Les
routes d'écriture (`POST items`, `POST upload`, `PATCH`, `DELETE`) répondent
`403 {"error":"read_only_session"}` après authentification et avant toute
validation de payload (garde `media_board::ensure_writable`) — voir §7.0 et
AUD-V-003.

## 4. Authentification Bearer (`auth.rs`)

### 4.1 Contrat

- La voie nominale est l'en-tête `Authorization: Bearer <token>`, qui prime
  toujours. **Exception board** (test local 2026-11-10) : les routes
  `/overlay/media-board/*` acceptent en repli `?token=<token>`
  (`auth::query_token`) car le renderer board HTML legacy construit ses URLs et
  ses balises `src` (`<video>`, `<img>` d'aperçu) sans en-tête possible
  (`board/legacy-preview-media-utils.js`). Ce repli est **strictement limité**
  au préfixe `/overlay/media-board/` ;
  `/overlay/media/{assetId}` reste exclusivement HMAC signé et aucune autre route
  overlay n'accepte de jeton en query.
- **Statut temporaire (AUD-COR-05)** : ce repli `?token=` ne survit que le temps
  de la migration du renderer legacy (P1-11). Chaque utilisation émet un `WARN`
  unique par processus (target `lc_server::overlay`,
  `warn_query_token_deprecation_once` dans `auth.rs`) pour mesurer l'usage avant
  suppression ; il disparaîtra avec le renderer Rust. Aucun autre jeton ne doit
  figurer dans une URL (l'admin, lui, n'accepte plus que le Bearer).
- **Mécanique exacte du repli** (état HEAD P2) : `query_token` n'extrait
  `?token=` que si le chemin commence par `/overlay/media-board/`
  (`auth.rs`) ; la valeur est décodée (`application/x-www-form-urlencoded`),
  `trim`ée, rejetée si vide ou > 200 caractères (`MAX_TOKEN_LEN`). Le repli
  n'est consulté que si `bearer_token` renvoie `None` : en-tête absent, schéma
  différent de `Bearer` (casse ignorée), token vide ou segments supplémentaires
  (en-tête malformé) — un Bearer valide prime donc toujours, et un Bearer
  malformé retombe sur le query token. Le token reste ensuite haché SHA-256 et
  résolu en base comme la voie nominale ; le warn de dépréciation est émis une
  seule fois par processus.
- Le token brut est généré côté serveur : 32 octets aléatoires encodés en hex
  (64 caractères), `auth.rs:65` — équivalent de
  `crypto.randomBytes(32).toString('hex')` (`overlayAuth.ts:40-42`).
- Seul le SHA-256 hexadécimal du token est persisté dans
  `overlay_clients.token_hash` (`auth.rs:72`, parité `hashOverlayToken`
  `overlayAuth.ts:36-38`). Le token brut n'est renvoyé qu'une fois, dans la
  réponse d'appairage.
- La recherche se fait par `OverlayClientsRepository::get_by_token_hash`, qui
  filtre déjà `revoked_at IS NULL` ; un client révoqué est donc rejeté.
- `last_seen_at` est rafraîchi best-effort (`touch_last_seen`) : un échec est
  journalisé et n'échoue pas la requête (`auth.rs:95-150`).

### 4.2 Extracteur `AuthedOverlayClient`

```rust
impl FromRequestParts<AppState> for AuthedOverlayClient {
    type Rejection = Response; // 401 unauthorized / 500 internal_error / 503 database_unavailable
}
```

Champs exposés : `client: lc_domain::OverlayClient`, `authenticated_at:
DateTime<Utc>` et les raccourcis `guild_id() -> &GuildId`,
`session_mode() -> OverlaySessionMode` et `is_read_only() -> bool`
(`auth.rs:62-89`). Le mode de session est chargé depuis
`overlay_clients.session_mode` **au moment de l'authentification** : il n'est
jamais dérivé d'un en-tête, d'un paramètre de requête ou du corps (`sessionMode`
du corps d'appairage ignoré, §5.2) et ne peut donc pas être contourné côté
client. Les handlers le reçoivent directement (`auth: AuthedOverlayClient`) et
comparent la guilde des ressources à `auth.guild_id()` (défense en profondeur :
`403 forbidden` pour un `guildId` de requête incohérent, `404 item_not_found`
pour un item d'une autre guilde). Les écritures media board utilisent
`auth.is_read_only()` via `media_board::ensure_writable` (§7.0).

> Note d'implémentation : le middleware `from_fn` initialement prévu a été
> remplacé par un extracteur strictement équivalent. `from_fn` avec
> `State<AppState>` exige `from_fn_with_state` (état concret), or le sous-routeur
> `/overlay` est construit sans état et complété par `app::router` ; l'extracteur
> évite cette contrainte sans modifier `app.rs`.

## 5. `POST /overlay/pair/consume`

### 5.1 Requête

```json
{ "code": "abc123", "deviceName": "Poste salon", "appVersion": "3.0.0" }
```

- `code` obligatoire, trim + MAJUSCULES (`PairingCode::parse` : alphanumérique,
  max 32) ; vide → `400 {"error":"invalid_payload"}` ;
- `deviceName` facultatif ; `label` et `clientName` acceptés en alias
  (anciens clients) ;
- `appVersion`/`sessionMode` sont **ignorés** pour la décision : le mode vient
  toujours du code persisté (`pairing_codes.mode`), un client ne peut pas
  s'auto-attribuer une session normale (`pair.rs:75-78`).

### 5.2 Traitement (`pair.rs:84-199`)

1. normalisation du code puis `delete_expired(now)` best-effort (ne bloque pas) ;
2. `PairingCodesRepository::consume(code, now)` — UPDATE atomique
   (`used_at IS NULL AND (expires_at IS NULL OR expires_at > now)`), donc un
   code ne peut être consommé qu'une fois, y compris en concurrence ;
3. mode : `PairingMode::InviteReadOnly → OverlaySessionMode::InviteReadOnly`,
   `Normal → Normal` ;
4. libellé : `deviceName` trim non vide tronqué à 200 caractères, sinon repli
   legacy `Overlay-<authorName|User>` ;
5. révocation des clients actifs de la même guilde portant le même libellé
   (parité legacy : un poste = une session active) ;
6. création de l'`OverlayClient` (token + hash, `default_author_name`,
   `default_author_image`, `created_by_discord_user_id`) ; collision de token
   unique → `409 overlay_client_conflict` (théoriquement impossible) ;
7. log `tracing::info!` (client, guilde, mode, appVersion) puis réponse.

### 5.3 Réponse `200`

```json
{
  "clientToken": "<64 hex>",
  "clientId": "<uuid>",
  "guildId": "123456789012345678",
  "apiBaseUrl": "https://api.exemple.tld",
  "deviceName": "Poste salon",
  "authorName": "Alice" | null,
  "sessionMode": "normal" | "invite_read_only",
  "protocolVersion": 2
}
```

`protocolVersion` est un ajout **additif** v2 (le legacy ne le renvoyait pas) :
il permet au client Tauri de refuser un serveur incompatible avant le handshake
WebSocket. Tous les autres champs sont ceux lus par le client Electron
(`overlay/src/main/services/pairing-consume-service.ts:106-115`).

### 5.4 Erreurs

| Statut | Corps | Cas |
|--------|-------|-----|
| `400` | `{"error":"invalid_payload"}` | `code` absent/vide |
| `404` | `{"error":"pairing_code_invalid_or_expired"}` | code inconnu, déjà utilisé ou expiré |
| `409` | `{"error":"overlay_client_conflict"}` | collision improbable de token |
| `500` | `{"error":"internal_error"}` | erreur interne (hors panique) |
| `503` | `{"error":"database_unavailable"}` | base absente |

## 6. `GET /overlay/media/{assetId}` — média signé HMAC

### 6.1 Format d'URL et signature

```text
{API_URL}/overlay/media/{assetId}?clientId={clientId}&exp={unixSecs}&sig={hex}
sig = HMAC-SHA256(MEDIA_URL_SECRET, "assetId|clientId|exp")
```

- `sig` : hex minuscule, 64 caractères ;
- `clientId` est **signé** : une URL volée ne sert que si elle est rejouée avec
  le bon `clientId`, lui-même vérifié actif en base ;
- `exp` en secondes UNIX ; TTL calculé par le hub (Phase 4b) via
  `crate::media_url::expiry_for` / `lc_protocol::media_url_ttl_sec` (image 300 s,
  audio/vidéo `clamp(duration+120, 300, 3600)`), jamais prolongé par ce handler ;
- `startOffsetSec` et `#t=` ne sont pas signés et sont ignorés côté serveur
  (seul le lecteur les utilise) — `media.rs:69-70` ;
- **liaison à la lecture en cours (AUD-A2-004)** : une signature valide ne suffit
  plus ; le handler exige qu'un `playback_jobs` de la guilde du client référence
  l'asset (n'importe quel statut), sinon `403 {"error":"forbidden"}`. Parité
  legacy `403 forbidden` quand aucun job de la guilde ne référence le média ;
- vérification via `crate::media_url::verify` (HMAC + `subtle::ConstantTimeEq`),
  **aucune duplication** de la logique HMAC dans ce module.

### 6.2 Ordre exact des contrôles (`media.rs:86-159`)

| # | Contrôle | Échec |
|---|----------|-------|
| 1 | `clientId`, `exp`, `sig` présents ; UUID `assetId`/`clientId` valides ; `exp` numérique ; `sig` = 64 hex ; `MEDIA_URL_SECRET` non vide | sinon `403 {"error":"media_url_invalid"}` (sauf UUID asset invalide → `404 media_not_found`) |
| 2 | signature HMAC valide (constant-time) | `403 media_url_invalid` |
| 3 | `exp` plafonné : `exp <= now + MEDIA_URL_TTL_MAX_SEC` (`media_url::is_expiry_within_max`, `now` calculé une seule fois) | `403 {"error":"media_url_invalid"}` |
| 4 | `now >= exp` (`media_url::is_expired`) | `403 {"error":"media_url_expired"}` |
| 5 | client existant et non révoqué | `403 media_url_invalid` |
| 6 | asset existant, `status == READY`, non expiré | `404 {"error":"media_not_found"}` |
| 7 | **liaison lecture** : `EXISTS(SELECT 1 FROM playback_jobs WHERE media_asset_id = ? AND guild_id = ?)` pour la guilde du client (AUD-A2-004) | `403 {"error":"forbidden"}` |
| 8 | fichier présent et confiné dans `MEDIA_STORAGE_DIR` | `404 {"error":"media_not_found_on_disk"}` |
| 9 | stream `HEAD`/`Range` | `200`/`206`/`416` |

Le plafond `exp` (AUD-A2-002, task-42) borne les signatures forgées ou les TTL
abusifs : une URL dont `exp` dépasse `now + MEDIA_URL_TTL_MAX_SEC` (3600 s par
défaut) est refusée même avec une signature HMAC valide, avant le test
d'expiration. `MEDIA_URL_TTL_SEC` reste validé au démarrage
(`config.rs:433-442`) et le hub n'émet jamais de TTL supérieur ; ce contrôle est
la seconde barrière côté vérification.

### 6.3 Résolution du fichier et en-têtes

- `resolve_media_file` (`media.rs:219-265`) : chemin absolu conservé, chemin
  relatif résolu depuis le cwd du serveur, `tokio::fs::canonicalize`, puis
  confinement strict `canonical.starts_with(canonical(MEDIA_STORAGE_DIR))` —
  empêche toute traversée. **Fail-closed (AUD-A2-003/A4-014, task-42)** : si la
  racine média n'existe plus ou n'est pas canonicalisable à chaud, le handler
  journalise un avertissement et refuse en `404 media_not_found_on_disk` ; il ne
  sert plus jamais un chemin arbitraire (l'ancien comportement fail-open pouvait
  répondre `200` avec un fichier hors racine quand `MEDIA_STORAGE_DIR` était
  absent). Le démarrage vérifie déjà la racine
  (`media::ensure_storage_root`), donc cette branche ne couvre que la disparition
  en cours de vie.
- `serve_file` (`media.rs:268-300`) : `tower_http::services::ServeFile` sert
  `GET`/`HEAD`, gère `Range` (`206` + `Content-Range`), `416` + `Content-Range:
  bytes */total`, `Content-Length`, `Content-Type` par extension. Le handler
  force ensuite `Content-Type: <asset.mime>`, `Cache-Control: no-store` et
  `Accept-Ranges: bytes` (en-têtes legacy conservés).
- `touch_asset` (`media.rs`) : `last_accessed_at = now` uniquement, best-effort,
  jamais bloquant (AUD-A2-005). Le stream **ne prolonge plus** `expires_at` :
  une lecture ne repousse plus le TTL de 12 h à chaque requête (comportement
  legacy corrigé) ; seule la libération d'un pin board ou une ingestion réussie
  modifient l'expiration.

### 6.4 Compression

`tower-http` ne compresse **jamais** une réponse portant `Content-Range` : les
`206` sont sûres. En revanche le prédicat par défaut de `CompressionLayer` global
n'exclut pas `video/*` / `audio/*` : une réponse `200` complète peut être
compressée (et perdre `Accept-Ranges`/`Content-Length`). Correction attendue
côté `app.rs` (prédicat excluant `/overlay/media` ou les types vidéo/audio) —
suivi en §12.1.

## 7. Board média (`/overlay/media-board/*`)

### 7.0 Sessions en lecture seule (AUD-V-003)

| Session du jeton | `GET items` / `GET ingestions` | `GET media` | `POST items` / `POST upload` | `PATCH item` | `DELETE item` |
|------------------|-------------|-------------|--------------|--------------|---------------|
| `normal` | `200` | `200`/`206` | comportement §7.2 | comportement §7.4 | comportement §7.5 |
| `invite_read_only` | `200` | `200`/`206` | **`403 read_only_session`** | **`403 read_only_session`** | **`403 read_only_session`** |

La garde `ensure_writable` est évaluée en tête des quatre handlers d'écriture
(`create_item`, `upload_item`, `update_item`, `delete_item`), **avant** la
validation de `url`/`title`/`message` et avant tout accès base : une session
invitée ne reçoit ni `400` de validation, ni `404` d'identifiant, seulement le
`403 {"error":"read_only_session"}`. Le mode appliqué est celui du jeton
serveur (`overlay_clients.session_mode`), que l'authentification passe par
`Authorization: Bearer` ou par le repli `?token=` du board ; si les deux sont
présents, le Bearer prime (§4.2). Le mode n'étant jamais relu depuis la requête,
aucun en-tête/corps ne peut élever une session invitée en session normale.

### 7.1 `GET /overlay/media-board/items` (`media_board.rs:119`)

Query : `q`, `limit`, `offset`, `guildId`.

- `limit` : défaut 50, clamp `1..=200` (`media_board.rs:565-570`) ;
- `offset` : défaut 0, négatif ramené à 0 (`:567`) ;
- `q` : trim, max 240 caractères, insensible à la casse, filtre `contains` sur
  `title`, `message`, `createdByName`, `media.sourceUrl` (`row_matches`) ;
- `guildId` : s'il est fourni et diffère de la guilde du Bearer →
  `403 {"error":"forbidden"}` (défense en profondeur) ;
- **chargement borné (AUD-A2-013)** : une requête SQL join unique
  (`load_board_rows`, `media_board_items` × `media_assets`, ordre
  `created_at DESC, id ASC`) fournit les lignes ; le filtre `q`, le `total` et
  la pagination sont calculés en mémoire, puis seuls les assets de la page
  (`≤ 200`) sont rechargés via `media_assets().get` — plus de N+1 non borné sur
  toute la guilde ni de double parcours repo+Rust ;
- réponse : `{"total": n, "limit": l, "offset": o, "items": [...]}`.

Payload d'un item (`item_payload`, `media_board.rs:531-552`) :

```json
{
  "id": "<uuid>", "guildId": "…", "mediaAssetId": "<uuid>",
  "title": "…" | null, "message": "…" | null, "createdByName": "…" | null,
  "createdAt": "2026-11-10T12:00:00.000Z",
  "media": { "id": "<uuid>", "kind": "video", "mime": "video/mp4",
             "durationSec": 30, "width": 1280, "height": 720,
             "isVertical": false, "sizeBytes": 12345,
             "sourceUrl": "https://…" }
}
```

`createdAt` est en ISO-8601 UTC avec millisecondes (`to_rfc3339_opts(Millis,
true)`) : le legacy renvoyait la sérialisation JSON de `Date` (ISO), pas un
epoch — le renderer board tolère les deux (`toSafeDateLabel`).

### 7.2 `POST /overlay/media-board/items` (`media_board.rs:189`)

Corps legacy : `{"url": "…", "title"?: "…", "message"?: "…", "forceRefresh"?: bool}`.

0. session `INVITE_READ_ONLY` → `403 {"error":"read_only_session"}` **avant
   toute validation** (aucun appel réseau, aucune lecture du corps) ; un corps
   JSON illisible (`Result<Json<CreateItemRequest>, JsonRejection>`) donne
   ensuite `400 {"error":"invalid_payload","message":"corps JSON invalide"}`
   (AUD-A2-009), jamais le `request_error` générique de la normalisation ;
1. `url` trim vide → `400 {"error":"invalid_payload","message":"url is required"}`
   (aucun appel réseau dans ce cas) ;
2. **réservation d'ingestion** : `IngestionRegistry::start(guilde, clientUuid,
   now)` crée une entrée UUID v4 `processing`. Au-delà de **20** ingestions
   `processing` pour le même client overlay →
   `429 {"error":"too_many_ingestions"}` (aucune tâche lancée) ;
3. **réponse immédiate** : `202 {"ingestionId":"<uuid>"}` ; la connexion HTTP
   n'est plus retenue par le téléchargement/transcodage. Une coupure client
   (Cloudflare 100 s, Nginx 90 s) n'annule rien ;
4. **tâche de fond** (`media_ingestion::spawn_ingestion`, `tokio::spawn`) :
   `MediaService::for_state` puis `service.ingest(Some(&url), None,
   force_refresh)` — même cache `READY`/`source_hash`, même sémaphore
   `MEDIA_MAX_CONCURRENT_INGESTIONS`, même budget cache et même
   `record_failure` (`FAILED` + message `[CODE] …`) que `/ingest`, puis
   `complete_board_item` (dédoublonnage, quota, épinglage, insertion, auteur) ;
5. **fin de tâche** : le registre passe `ready` (`created`, `itemId`) ou `failed`
   (`error {code, message?}`), puis `media.ingestion` est envoyé à **toutes** les
   sockets du client demandeur (`AdminHub::send_to_client`). Si un item a été
   créé, `board.changed {change:"added", itemId}` est diffusé à la guilde
   (`AdminHub::broadcast_to_guild`) ;
6. codes d'échec exposés dans `media.ingestion` : `media_ingestion_failed`
   (+ message lisible, ex. quota `BOARD_STORAGE_LIMIT_REACHED`),
   `unsupported_media` (upload), `internal_error` (base/pipeline indisponible).

Le résultat reste consultable une heure via §7.2.1 même si aucun socket n'était
connecté au moment de la fin.

`forceRefresh` est interprété par `coerce_flag` (booléen JSON, `"true"`,
`"1"`, `"yes"`, `"on"`, casse ignorée) — parité `toBooleanFlag` legacy
(AUD-A2-014).

### 7.2.1 `GET /overlay/media-board/ingestions`

Réponse `200` :

```json
{ "items": [
  { "ingestionId": "<uuid>", "status": "processing" },
  { "ingestionId": "<uuid>", "status": "ready", "created": true,
    "itemId": "<uuid>" },
  { "ingestionId": "<uuid>", "status": "failed",
    "error": { "code": "unsupported_media" } }
] }
```

- renvoie uniquement les ingestions du **client overlay authentifié** (même
  UUID), jamais celles d'un autre appareil de la guilde ;
- inclut les entrées `processing` et les entrées terminées depuis **moins d'une
  heure** ; les plus récentes d'abord ;
- `created`/`itemId` sont omis hors succès, `error.message` est omis quand le
  serveur n'a pas de détail ; `status` ∈ `processing` | `ready` | `failed` ;
- session invitée autorisée (lecture) ; le repli `?token=` du board s'applique ;
- but : après une reconnexion WebSocket, le board récupère un `media.ingestion`
  manqué. La purge est paresseuse (insertion/lecture), aucun timer.

### 7.2.2 `POST /overlay/media-board/upload?filename=&title=&message=`

- corps brut (pas de multipart), `filename` obligatoire : trim non vide,
  ≤ 255 caractères, sans `/`, `\`, `..` ni caractère de contrôle → sinon
  `400 {"error":"invalid_payload","message":…}` ;
- session invitée → `403 read_only_session` **avant** toute lecture du corps ;
- corps vide → `400 invalid_payload` ; dépassement
  `MEDIA_MAX_SIZE_MB` pendant la réception → `413 {"error":"payload_too_large"}` ;
- le fichier est écrit **entièrement** dans un dossier temporaire
  (`UploadTempDir`, `lc-media-upload-<uuid>`) avec un SHA-256 incrémental avant
  la réponse ; ce dossier est ensuite possédé par la tâche de fond et supprimé à
  sa fin (succès, échec, `created:false`) ;
- mêmes étapes que §7.2 (registre, `202`, quota 20, dédoublonnage, quota board,
  épinglage, `media.ingestion`, `board.changed`) ; le cache upload utilise
  `upload:sha256:<hex>` et `media_assets.source_url = upload:<filename>` ;
- un type non supporté est signalé asynchronement par
  `media.ingestion {ok:false, error:{code:"unsupported_media"}}` (plus de `415`
  synchrone, puisque le type n'est connu qu'après le sniffing) ;
- `title` par défaut = nom de fichier sans extension ; `title`/`message` tronqués
  aux limites du board.

### 7.3 `GET /overlay/media-board/media/{assetId}` (`media_board.rs:388`)

- Bearer obligatoire ; l'item `(guild du client, asset)` doit exister
  (`get_by_guild_and_asset`) sinon `404 media_not_found` (pas de fuite
  inter-guilde) ;
- asset `READY` et non expiré sinon `404 media_not_found` ;
- fichier résolu/confiné sinon `404 media_not_found_on_disk` ;
- partage le streaming `serve_file` (`Range`/`HEAD`), sans `touch_asset`.

### 7.4 `PATCH /overlay/media-board/items/{itemId}` (`media_board.rs:428`)

- session `INVITE_READ_ONLY` → `403 {"error":"read_only_session"}` (avant la
  validation du corps et la lecture de l'item) ;
- corps JSON illisible → `400 {"error":"invalid_payload","message":"corps JSON
  invalide"}` (AUD-A2-009, `Result<Json<Value>, JsonRejection>`) ;
- corps JSON objet ; ni `title` ni `message` → `400 {"error":"invalid_payload",
  "message":"title or message is required"}` ;
- champ absent → conservé ; `null`/chaîne vide → effacé
  (`normalize_optional_text` :651 ; parité `hasOwnProperty` +
  `toNonEmptyString`) ;
- item inconnu ou d'une autre guilde → `404 {"error":"item_not_found"}` ;
- après une mise à jour effective, `board.changed {change:"updated", itemId}`
  est diffusé à toutes les sockets de la guilde ;
- réponse `{"updated":true,"item":{"id","title","message"}}`.

### 7.5 `DELETE /overlay/media-board/items/{itemId}` (`media_board.rs:500`)

- session `INVITE_READ_ONLY` → `403 {"error":"read_only_session"}` (avant la
  validation de l'identifiant) ;
- id invalide ou item d'une autre guilde → `200 {"deleted":false}` (parité
  legacy : pas de 404 sur delete) ;
- suppression **transactionnelle** (`delete_item_and_release`) : `DELETE` de
  l'item `(id, guilde)`, `COUNT(*)` des références board restantes, puis si 0
  `UPDATE media_assets SET last_accessed_at, expires_at = now +
  MEDIA_CACHE_TTL_HOURS` — le tout dans une transaction SQLite, fin des
  comptages non transactionnels (AUD-A2-013) ;
- si la suppression a eu lieu, `board.changed {change:"deleted", itemId}` est
  diffusé à la guilde ;
- réponse `{"deleted": bool}`.

### 7.6 Erreurs media board (récapitulatif)

| Statut | Corps | Cas |
|--------|-------|-----|
| `202` | `{"ingestionId":"<uuid>"}` | ingestion acceptée (`POST items`, `POST upload`) |
| `400` | `{"error":"invalid_payload",…}` | `url`/`filename` manquant, `title`+`message` absents, JSON illisible, corps vide |
| `401` | `{"error":"unauthorized"}` | Bearer absent/invalide/révoqué |
| `403` | `{"error":"forbidden"}` | `guildId` query ≠ guilde du Bearer |
| `403` | `{"error":"read_only_session"}` | **écriture (`POST`/`PATCH`/`DELETE`) avec une session `invite_read_only`** |
| `404` | `{"error":"item_not_found"}` / `media_not_found` / `media_not_found_on_disk` | item/asset/fichier absent |
| `413` | `{"error":"payload_too_large"}` | upload > `MEDIA_MAX_SIZE_MB` |
| `429` | `{"error":"too_many_ingestions"}` | 20 ingestions `processing` déjà actives pour ce client |
| `500`/`503` | `internal_error` / `database_unavailable` | erreur interne/base |

Les échecs d'ingestion (`media_ingestion_failed`, `unsupported_media`,
`internal_error`) ne sont plus des réponses HTTP : ils arrivent dans
`media.ingestion`, avec un code REST/WS commun produit par
`media_service_error_payload` / `upload_service_failure`.

## 8. Configuration utilisée

Les variables sont validées par `crate::config::Config` (aucune copie locale,
`Config::from_pairs`) :

| Variable | Utilisation dans `/overlay` |
|----------|------------------------------|
| `API_URL` | `apiBaseUrl` renvoyé par l'appairage (`api_origin()`) |
| `MEDIA_URL_SECRET` | vérification HMAC du média signé ; absent → `403 media_url_invalid` + warn |
| `MEDIA_STORAGE_DIR` | confinement des fichiers ; répertoire du `MediaPipeline` |
| `MEDIA_CACHE_TTL_HOURS` | libération de l'expiry à la suppression board (`now + TTL`) ; une lecture média ne prolonge plus l'expiration |
| `MEDIA_BOARD_MAX_TOTAL_MB` | quota de la board (`media.ingestion` `media_ingestion_failed`, détail `BOARD_STORAGE_LIMIT_REACHED`) |
| `PAIRING_CODE_TTL_MINUTES` | (création admin des codes, Phase 7b ; pas ce module) |
| `MEDIA_URL_TTL_SEC` | (validation config ; le TTL effectif de l'URL est calculé par le hub) |

## 9. Réutilisation par le hub WebSocket (Phase 4b)

Le hub doit générer les URLs média signées sans dupliquer la logique :

```rust
// apps/lc-server/src/http/overlay/media.rs:194
pub fn sign_media_url(
    secret: &[u8],
    api_url: &str,
    asset_id: &MediaAssetId,
    client_id: &OverlayClientId,
    exp_sec: u64,
) -> Result<String, crate::media_url::MediaUrlError>;
```

- ré-exporté comme `crate::http::overlay::sign_media_url` (`mod.rs:63`) ;
- produit exactement le format du §6.1 et délègue la signature à
  `crate::media_url::sign` (module partagé du Lead — pas de HMAC dupliqué) ;
- `exp_sec` se calcule via `crate::media_url::expiry_for(kind, duration_sec,
  now_sec)` ;
- testé en aller-retour signé/vérifié (`tests.rs`,
  `sign_media_url_matches_protocol_v2_format_and_verifies`) : c'est le contrat
  que le hub peut réutiliser tel quel.

## 10. Écarts assumés avec le legacy

| # | Écart | Raison / mitigation |
|---|-------|---------------------|
| 1 | `?token=` supprimé pour le protocole WS et les URLs média signées ; Bearer obligatoire, **sauf** le repli `?token=` des routes board `/overlay/media-board/*` | protocole v2 §9 ; le repli board est l'unique exception REST (renderer HTML legacy `overlay/renderer/board.js` charge `<video>`/`<img>` par URL), limité au chemin board, Bearer prioritaire, token trimé/≤200, et les mutations restent soumises à la garde `invite_read_only` (§7.0). **Temporaire** depuis AUD-COR-05 : WARN unique par processus (`warn_query_token_deprecation_once`), suppression prévue avec le renderer Rust (P1-11) |
| 2 | `GET /overlay/config` supprimé | protocole v2 ; config locale overlay (Tauri) |
| 3 | Ingestion média du board **asynchrone** : `202 {ingestionId}`, travail dans une tâche `tokio::spawn` détachée de la connexion HTTP | plus de coupure proxy (Cloudflare 100 s / Nginx 90 s) qui annule la conversion ; registre partagé, plafond de 20 `processing` par client, rattrapage via `GET /ingestions`, notification `media.ingestion` (AUD-V2-S-003, AUD-A2-006/007). L'ingestion reste déléguée à `MediaService` (cache, sémaphore, budget, `record_failure`, nettoyage) |
| 4 | Une lecture `/overlay/media` **ne prolonge plus** `expires_at` (seul `last_accessed_at` est touché) | AUD-A2-005 : le legacy repoussait le TTL de 12 h à chaque requête, gonflant la rétention. Seules l'ingestion, le pin board et la libération de pin modifient l'expiration |
| 5 | Troncature titre/message/nom par **caractères Unicode** (`chars`) | le legacy JS tronquait par unités UTF-16 ; écart uniquement sur paires de substitution (emoji) — bénin, plus correct |
| 6 | `createdAt` ISO-8601 ms via `to_rfc3339_opts` | même format observable que `Date.toJSON()` |
| 7 | `protocolVersion` additif dans la réponse d'appairage | permet au client de refuser un serveur incompatible avant WS |
| 8 | Révocation des clients même `(guilde, label)` conservée ; modes depuis le code persisté | parité legacy |
| 9 | `sessionMode` du corps d'appairage ignoré | sécurité : un client ne s'auto-attribue pas une session normale |
| 10 | `delete` renvoie toujours `200 {deleted}` (jamais 404) | parité legacy |
| 11 | Compression possible des réponses média `200` | `app.rs` (Lead) doit exclure `/overlay/media` du `CompressionLayer` — cf. §12.1 |
| 12 | Mutations media-board **refusées** en session `invite_read_only` (`403 read_only_session`) | AUD-V-003 : le legacy ne vérifiait pas le mode sur les écritures ; le protocole v2 §9 réserve l'invite à la lecture. Garde serveur ajoutée (`media_board::ensure_writable`, §7.0), identique pour Bearer et `?token=` ; les `GET` restent inchangés |
| 13 | JSON illisible → `400 {"error":"invalid_payload"}` dédié (et non `request_error` générique) | AUD-A2-009 : `Result<Json<…>, JsonRejection>` sur pair/board ; l'authentification et la garde read-only restent évaluées avant le refus de corps |

## 11. Tests

`apps/lc-server/src/http/overlay/tests.rs` — base SQLite en mémoire
(`Db::open_in_memory`), horloge figée `FixedClock` (2026-11-10T12:00:00Z),
routeur **complet** via `crate::app::router` (middlewares réels) et
`tower::ServiceExt::oneshot`. État (task-49) : **23/23 verts** (filtre
`cargo test -p lc-server --lib http::overlay`) ; la suite complète
`cargo test -p lc-server --lib` passe à **284 passed, 1 ignored**.

| Test | Couverture |
|------|------------|
| `pair_consume_creates_client_and_returns_legacy_payload` | 200, token 64 hex, hash stocké, code à usage unique |
| `pair_consume_rejects_missing_and_expired_codes` | 400 `invalid_payload`, 404 code expiré |
| `pair_consume_honors_invite_read_only_and_default_label` | mode invité, label `Overlay-User`, autor null |
| `pair_consume_revokes_previous_client_with_same_label` | révocation même libellé |
| `signed_media_serves_file_with_legacy_headers_and_range` | 200 + en-têtes legacy, HEAD, 206 `bytes=2-5`, 416 `bytes */16`, absence de compression sur 206 |
| `signed_media_rejects_bad_signature_expiry_and_missing_params` | params absents, sig falsifiée, exp malformé/expiré, asset inconnu |
| `signed_media_rejects_revoked_client_and_expired_asset` | client révoqué → 403, asset expiré → 404 |
| `signed_media_missing_or_escaped_file_is_rejected` | fichier absent / hors `MEDIA_STORAGE_DIR` → 404 |
| `media_board_requires_token_and_accepts_legacy_query_token` | 401 sans token, `?token=` accepté sur `/overlay/media-board/*`, Bearer prioritaire, `guildId` étranger → 403 |
| `media_board_lists_filters_and_paginates` | ordre, payload media, filtres `q`, pagination, clamps 1..=200 |
| `media_board_patch_and_delete_semantics` | validation corps, conservation/effacement, isolation guilde, delete idempotent |
| `media_board_create_rejects_missing_url_without_network` | 400 `url is required`, aucun réseau |
| `media_board_media_requires_auth_and_binding` | 401, 404 hors guilde, 200 + `no-store` |
| `overlay_config_route_is_absent_in_v2` | 404 `request_error` |
| `sign_media_url_matches_protocol_v2_format_and_verifies` (unitaire) | format exact, vérification HMAC, mauvais secret rejeté |
| `read_only_allows_reads_and_blocks_mutations` (`mod.rs`) | session invitée : GET items/media 200, POST/PATCH/DELETE 403 `read_only_session`, item intact |
| `normal_session_mutations_stay_allowed` (`mod.rs`) | session normale : POST sans `url` → 400, PATCH/DELETE 200 |
| `query_token_cannot_bypass_read_only_gate` (`mod.rs`) | `?token=` invité → 403 sur les 3 mutations ; `?token=` normal → PATCH 200 |
| `bearer_token_takes_precedence_over_query_token` (`mod.rs`) | un Bearer normal prime sur un `?token=` invité (et inversement) |
| `signed_media_requires_a_playback_job_in_the_client_guild` | 403 `forbidden` sans playback job de la guilde, 403 si job d'une autre guilde, 200 + bytes si job de la guilde (AUD-A2-004) |
| `media_board_create_rejects_malformed_json` | 400 `invalid_payload` sur corps illisible, après la garde session (AUD-A2-009) |
| `media_board_patch_rejects_malformed_json` | idem sur `PATCH /items/{id}` |
| `get_ingestions_only_lists_the_authenticated_client_entries` (`ingestion_tests.rs`) | isolation par client, session invitée autorisée |
| `processing_limit_answers_429` (`ingestion_tests.rs`) | 20 `processing` → `429 too_many_ingestions` |
| `finished_ingestions_are_purged_after_one_hour` (`ingestion_tests.rs`) | rétention 1 h via horloge injectée |
| `update_and_delete_broadcast_board_changed` (`ingestion_tests.rs`) | `updated` puis `deleted` diffusés à la guilde |
| `upload_png_is_ingested_asynchronously_and_reuses_asset` (`media_upload_tests.rs`) | 202, registre `ready`, `media.ingestion`, `board.changed added`, doublon `created:false` sans seconde diffusion |
| `failed_upload_reports_a_code_and_a_readable_message` (`media_upload_tests.rs`) | `unsupported_media` puis `media_ingestion_failed` + message |
| `upload_over_size_limit_returns_413` / `read_only_session_is_rejected_before_body_read` | validations synchrones conservées |

Commandes :

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_BUILD_JOBS = "4"
cargo fmt -p lc-server -- --check
cargo clippy -p lc-server --all-targets -- -D warnings
cargo test -p lc-server
```

## 12. Handoffs et points ouverts

### 12.1 Lead (`app.rs`) — compression média

Exclure `/overlay/media` (ou les `Content-Type` `video/*`/`audio/*`) du
`CompressionLayer` global : aujourd'hui une réponse `200` complète pourrait être
compressée, cassant `Accept-Ranges` et le streaming. Les `206` sont protégées
par `tower-http` (présence de `Content-Range`). Alternative : prédicat custom
`CompressionLayer::new().compress_when(...)`.

### 12.2 Hub WebSocket (Phase 4b, task-17)

- générer les URLs via `crate::http::overlay::sign_media_url` + `expiry_for` ;
- émettre `overlay://play` avec `media.url` **verbatim** (pas de réécriture,
  pas de `?token=`) — décisions Lead `docs/rust/overlay-bridge.md` §8 ;
- le hub reste la seule source d'URLs signées, mais **le REST vérifie désormais
  la liaison lecture** : un `playback_jobs` de la guilde du client doit
  référencer l'asset, sinon `403 forbidden` (§6.1/§6.2, AUD-A2-004). Le hub doit
  donc signer en priorité l'asset du job courant/en cours de lecture ; une URL
  signée n'est plus un laissez-passer autonome.

### 12.3 Service média (Phase 5b)

- le service média persistant est livré : `POST items` passe désormais par
  `MediaService` (cache/quota/`FAILED`/nettoyage), et le worker de purge
  (`apps/lc-server/src/media/purge.rs`) existe — voir `docs/rust/server-ingest.md`
  §5/§6 pour les budgets et le cycle de vie ;
- restent pour les phases suivantes : branche « rich tweet card » (Phase 6b,
  hors scope REST), invalidation de cache documentée après `forceRefresh`
  (`discard_partial_files` couvre l'échec), et ré-encodage/rasterisation médias
  avancée (COR-15 traité pour le SVG, sécurités associées).

### 12.4 Admin (Phase 7b)

- création/révocation des codes d'appairage (`PairingCodesRepository`) ;
- révocation des clients overlay (`OverlayClientsRepository::revoke`) ;
- quota board global/guilde, purge des assets.

### 12.5 Renderer (Phase 8)

- appels `/overlay/media-board/*` : envoyer `Authorization: Bearer` au lieu de
  `?token=` (`overlay/src/renderer/board/legacy-preview-media-utils.ts:44-53`
  legacy) ;
- média : utiliser `media.url` verbatim (URL signée du hub), pas de token
  ajouté ;
- `GET /overlay/config` n'existe plus : configuration via le bridge Tauri
  (`overlay_get_config`).

## 13. Références

- Legacy : `server/src/components/overlay/overlayRoutes.ts` (568 l.),
  `server/src/services/overlayAuth.ts` (143 l.),
  `server/src/services/memeBoard.ts` (376 l.),
  `server/src/services/media/mediaCache.ts`.
- Protocole : `docs/rust/protocol-v2.md` §8 (URLs média signées, l.631-686),
  §9 (modes), §12 (parité JSON).
- Serveur : `docs/rust/server-core.md` §5 (surface HTTP), §9 (intégration
  Phase 3b), `docs/rust/migration-plan.md` §4/§6.
- Code partagé : `apps/lc-server/src/media_url.rs` (HMAC), `crates/lc-protocol`
  (TTL, `media_signature_input`), `crates/lc-domain` (entités/enums/ids),
  `crates/lc-db` (repositories), `crates/lc-media` (pipeline/erreurs).
- Bridge overlay : `docs/rust/overlay-bridge.md` §6 (mapping commandes/événements
  Tauri) et §8 (décisions Lead).
