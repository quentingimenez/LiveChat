# Serveur Rust — routes `/overlay/*` (Phase 3b)

Statut : **implémenté** (routes, auth, média signé, meme board, tests in-process).
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
| `?token=` sur les routes board | **supprimé** en v2 (Bearer uniquement) |

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
      └─ meme board (auth par extracteur)
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
| `http/overlay/meme_board.rs` | CRUD board + média board, ingestion best-effort | handlers :107, :177, :368, :408, :476 |
| `http/overlay/tests.rs` | 15 tests in-process + 1 test unitaire | liste §11 |

## 3. Table des routes

| Méthode | Chemin | Auth | Handler | Succès |
|---------|--------|------|---------|--------|
| `POST` | `/overlay/pair/consume` | code d'appairage (corps) | `pair.rs:84` | `200` JSON (§5) |
| `GET` | `/overlay/media/{assetId}` | URL signée HMAC (`clientId`, `exp`, `sig`) | `media.rs:86` | `200`/`206`, binaire |
| `GET` | `/overlay/meme-board/items` | `Authorization: Bearer` | `meme_board.rs:107` | `200 {total,limit,offset,items}` |
| `POST` | `/overlay/meme-board/items` | `Authorization: Bearer` | `meme_board.rs:177` | `200 {created,item}` |
| `GET` | `/overlay/meme-board/media/{assetId}` | `Authorization: Bearer` | `meme_board.rs:372` | `200`/`206`, binaire |
| `PATCH` | `/overlay/meme-board/items/{itemId}` | `Authorization: Bearer` | `meme_board.rs:412` | `200 {updated:true,item}` |
| `DELETE` | `/overlay/meme-board/items/{itemId}` | `Authorization: Bearer` | `meme_board.rs:480` | `200 {deleted}` |
| — | `/overlay/config` | — | **absent** | `404 {"error":"request_error"}` (fallback global) |

Les routes protégées portent l'authentification via l'extracteur
`AuthedOverlayClient` (premier paramètre des handlers), exécuté avant le handler
par axum (§4). Aucun middleware `.layer(...)` spécifique n'est nécessaire sur le
sous-routeur.

## 4. Authentification Bearer (`auth.rs`)

### 4.1 Contrat

- Seul l'en-tête `Authorization: Bearer <token>` est accepté ; **le `?token=`
  legacy est supprimé** (`overlayAuth.ts:116-143` le tolérait). Le query string
  est totalement ignoré : une requête `?token=<valide>` sans Bearer répond
  `401 {"error":"unauthorized"}`.
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
DateTime<Utc>` et le raccourci `guild_id() -> &GuildId` (`auth.rs:47-60`).
Les handlers le reçoivent directement (`auth: AuthedOverlayClient`) et
comparent la guilde des ressources à `auth.guild_id()` (défense en profondeur :
`403 forbidden` pour un `guildId` de requête incohérent, `404 item_not_found`
pour un item d'une autre guilde).

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
- vérification via `crate::media_url::verify` (HMAC + `subtle::ConstantTimeEq`),
  **aucune duplication** de la logique HMAC dans ce module.

### 6.2 Ordre exact des contrôles (`media.rs:86-159`)

| # | Contrôle | Échec |
|---|----------|-------|
| 1 | `clientId`, `exp`, `sig` présents ; UUID `assetId`/`clientId` valides ; `exp` numérique ; `sig` = 64 hex ; `MEDIA_URL_SECRET` non vide | sinon `403 {"error":"media_url_invalid"}` (sauf UUID asset invalide → `404 media_not_found`) |
| 2 | signature HMAC valide (constant-time) | `403 media_url_invalid` |
| 3 | `now >= exp` (`media_url::is_expired`) | `403 {"error":"media_url_expired"}` |
| 4 | client existant et non révoqué | `403 media_url_invalid` |
| 5 | asset existant, `status == READY`, non expiré | `404 {"error":"media_not_found"}` |
| 6 | fichier présent et confiné dans `MEDIA_STORAGE_DIR` | `404 {"error":"media_not_found_on_disk"}` |
| 7 | stream `HEAD`/`Range` | `200`/`206`/`416` |

### 6.3 Résolution du fichier et en-têtes

- `resolve_media_file` (`media.rs:219-265`) : chemin absolu conservé, chemin
  relatif résolu depuis le cwd du serveur, `tokio::fs::canonicalize`, puis
  confinement strict `canonical.starts_with(canonical(MEDIA_STORAGE_DIR))` —
  empêche toute traversée. Répertoire média absent : confinement non
  vérifiable → avertissement + service (compatibilité tests/tempdir).
- `serve_file` (`media.rs:268-300`) : `tower_http::services::ServeFile` sert
  `GET`/`HEAD`, gère `Range` (`206` + `Content-Range`), `416` + `Content-Range:
  bytes */total`, `Content-Length`, `Content-Type` par extension. Le handler
  force ensuite `Content-Type: <asset.mime>`, `Cache-Control: no-store` et
  `Accept-Ranges: bytes` (en-têtes legacy conservés).
- `touch_asset` (`media.rs:302-319`) : `last_accessed_at = now`,
  `expires_at = max(expires_at, now + MEDIA_CACHE_TTL_HOURS)` — best-effort,
  jamais bloquant ; ne prolonge pas la signature.

### 6.4 Compression

`tower-http` ne compresse **jamais** une réponse portant `Content-Range` : les
`206` sont sûres. En revanche le prédicat par défaut de `CompressionLayer` global
n'exclut pas `video/*` / `audio/*` : une réponse `200` complète peut être
compressée (et perdre `Accept-Ranges`/`Content-Length`). Correction attendue
côté `app.rs` (prédicat excluant `/overlay/media` ou les types vidéo/audio) —
suivi en §12.1.

## 7. Mème board (`/overlay/meme-board/*`)

### 7.1 `GET /overlay/meme-board/items` (`meme_board.rs:107`)

Query : `q`, `limit`, `offset`, `guildId`.

- `limit` : défaut 50, clamp `1..=200` (`meme_board.rs:565-570`) ;
- `offset` : défaut 0, négatif ramené à 0 (`:567`) ;
- `q` : trim, max 240 caractères, insensible à la casse, filtre `contains` sur
  `title`, `message`, `createdByName`, `media.sourceUrl` (`item_matches` :574) ;
- `guildId` : s'il est fourni et diffère de la guilde du Bearer →
  `403 {"error":"forbidden"}` (défense en profondeur) ;
- tri legacy : `created_at DESC` (repo `list_by_guild`), `total` calculé **avant**
  pagination ;
- réponse : `{"total": n, "limit": l, "offset": o, "items": [...]}`.

Payload d'un item (`item_payload`, `meme_board.rs:531-552`) :

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

### 7.2 `POST /overlay/meme-board/items` (`meme_board.rs:177`)

Corps legacy : `{"url": "…", "title"?: "…", "message"?: "…", "forceRefresh"?: bool}`.

1. `url` trim vide → `400 {"error":"invalid_payload","message":"url is required"}`
   (aucun appel réseau dans ce cas) ;
2. **ingestion best-effort inline** : `MediaPipeline::new(media_config(&state))`
   puis `ingest_url(url).await` — échec → `422` legacy
   `{"error":"media_ingestion_failed","code":"<CODE>","message":"<msg>"}` ;
   la configuration est dérivée de `Config` (répertoire, TTL cache, quota board) ;
3. asset par `source_hash` :
   - existant `READY` et `!forceRefresh` → réutilisé, expiry épinglée si besoin ;
   - existant non prêt → `update_status(Ready)` puis réutilisation ;
   - absent → insertion `MediaAsset` (`status: READY`, expiry épinglée) ; collision
     unique → relecture par `source_hash` ;
4. dédoublonnage board `(guild, asset)` → `200 {"created":false,"item":…}` ;
5. quota board (somme `sizeBytes` de la board + nouvel asset vs
   `MEDIA_BOARD_MAX_TOTAL_MB`) → `422 {"error":"media_ingestion_failed",
   "code":"BOARD_STORAGE_LIMIT_REACHED",…}` ;
6. insertion item (`title` ≤240, `message` ≤500, `createdByName` =
   `default_author_name` sinon `label`, ≤120), réponse
   `200 {"created":true,"item":…}` ; collision unique concurrente → relecture et
   `created:false`.

Expiry épinglée = `now + 100 ans` (`PINNED_YEARS`, `meme_board.rs:60`,
`pinned_expiry` :678) — parité `ensurePinnedExpiry` legacy.

### 7.3 `GET /overlay/meme-board/media/{assetId}` (`meme_board.rs:372`)

- Bearer obligatoire ; l'item `(guild du client, asset)` doit exister
  (`get_by_guild_and_asset`) sinon `404 media_not_found` (pas de fuite
  inter-guilde) ;
- asset `READY` et non expiré sinon `404 media_not_found` ;
- fichier résolu/confiné sinon `404 media_not_found_on_disk` ;
- partage le streaming `serve_file` (`Range`/`HEAD`), sans `touch_asset`.

### 7.4 `PATCH /overlay/meme-board/items/{itemId}` (`meme_board.rs:412`)

- corps JSON objet ; ni `title` ni `message` → `400 {"error":"invalid_payload",
  "message":"title or message is required"}` ;
- champ absent → conservé ; `null`/chaîne vide → effacé
  (`normalize_optional_text` :651 ; parité `hasOwnProperty` +
  `toNonEmptyString`) ;
- item inconnu ou d'une autre guilde → `404 {"error":"item_not_found"}` ;
- réponse `{"updated":true,"item":{"id","title","message"}}`.

### 7.5 `DELETE /overlay/meme-board/items/{itemId}` (`meme_board.rs:480`)

- id invalide ou item d'une autre guilde → `200 {"deleted":false}` (parité
  legacy : pas de 404 sur delete) ;
- suppression puis, si plus aucun board ne référence le média
  (`board_reference_count`), libération de l'expiry épinglée :
  `expires_at = now + MEDIA_CACHE_TTL_HOURS` best-effort ;
- réponse `{"deleted": bool}`.

### 7.6 Erreurs meme board (récapitulatif)

| Statut | Corps | Cas |
|--------|-------|-----|
| `400` | `{"error":"invalid_payload",…}` | `url` manquante, `title`+`message` absents |
| `401` | `{"error":"unauthorized"}` | Bearer absent/invalide/révoqué |
| `403` | `{"error":"forbidden"}` | `guildId` query ≠ guilde du Bearer |
| `404` | `{"error":"item_not_found"}` / `media_not_found` / `media_not_found_on_disk` | item/asset/fichier absent |
| `422` | `{"error":"media_ingestion_failed","code","message"}` | ingestion, quota board |
| `500`/`503` | `internal_error` / `database_unavailable` | erreur interne/base |

## 8. Configuration utilisée

Les variables sont validées par `crate::config::Config` (aucune copie locale,
`Config::from_pairs`) :

| Variable | Utilisation dans `/overlay` |
|----------|------------------------------|
| `API_URL` | `apiBaseUrl` renvoyé par l'appairage (`api_origin()`) |
| `MEDIA_URL_SECRET` | vérification HMAC du média signé ; absent → `403 media_url_invalid` + warn |
| `MEDIA_STORAGE_DIR` | confinement des fichiers ; répertoire du `MediaPipeline` |
| `MEDIA_CACHE_TTL_HOURS` | prolongation `expires_at` au stream et à la suppression board |
| `MEDIA_BOARD_MAX_TOTAL_MB` | quota de la board (422 `BOARD_STORAGE_LIMIT_REACHED`) |
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
| 1 | Plus de contrôle « un playback job existe pour `(guilde, média)` » avant de servir `/overlay/media` | `lc-db` n'expose pas cette requête (`PlaybackJobsRepository`) et le hub est la seule source d'URL signées ; l'accès est déjà borné par HMAC + `clientId` actif + TTL court. Documenté handoff Phase 4b/5b |
| 2 | `?token=` supprimé ; Bearer obligatoire | protocole v2 §9 ; le renderer Phase 8 devra envoyer l'en-tête (écart de handoff) |
| 3 | `GET /overlay/config` supprimé | protocole v2 ; config locale overlay (Tauri) |
| 4 | Ingestion média **inline** dans `POST items` (pipeline) | pas de file d'ingestion côté serveur en Phase 3b ; `MediaPipeline::ingest_url` ne consulte pas le cache et retélécharge — la Phase 5b doit fournir le worker, l'éviction et les transitions de statut |
| 5 | Un asset non-`READY` rencontré dans `POST items` est repassé `READY` sans re-validation | best-effort Phase 3b ; la Phase 5b devient propriétaire des statuts |
| 6 | Troncature titre/message/nom par **caractères Unicode** (`chars`) | le legacy JS tronquait par unités UTF-16 ; écart uniquement sur paires de substitution (emoji) — bénin, plus correct |
| 7 | `createdAt` ISO-8601 ms via `to_rfc3339_opts` | même format observable que `Date.toJSON()` |
| 8 | `protocolVersion` additif dans la réponse d'appairage | permet au client de refuser un serveur incompatible avant WS |
| 9 | Révocation des clients même `(guilde, label)` conservée ; modes depuis le code persisté | parité legacy |
| 10 | `sessionMode` du corps d'appairage ignoré | sécurité : un client ne s'auto-attribue pas une session normale |
| 11 | `delete` renvoie toujours `200 {deleted}` (jamais 404) | parité legacy |
| 12 | Compression possible des réponses média `200` | `app.rs` (Lead) doit exclure `/overlay/media` du `CompressionLayer` — cf. §12.1 |

## 11. Tests

`apps/lc-server/src/http/overlay/tests.rs` — base SQLite en mémoire
(`Db::open_in_memory`), horloge figée `FixedClock` (2026-11-10T12:00:00Z),
routeur **complet** via `crate::app::router` (middlewares réels) et
`tower::ServiceExt::oneshot`. État : **15/15 verts** (filtre
`cargo test -p lc-server --lib http::overlay::tests`).

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
| `meme_board_requires_bearer_and_rejects_query_token` | 401 sans/mauvais Bearer, `?token=` ignoré, `guildId` étranger → 403 |
| `meme_board_lists_filters_and_paginates` | ordre, payload media, filtres `q`, pagination, clamps 1..=200 |
| `meme_board_patch_and_delete_semantics` | validation corps, conservation/effacement, isolation guilde, delete idempotent |
| `meme_board_create_rejects_missing_url_without_network` | 400 `url is required`, aucun réseau |
| `meme_board_media_requires_auth_and_binding` | 401, 404 hors guilde, 200 + `no-store` |
| `overlay_config_route_is_absent_in_v2` | 404 `request_error` |
| `sign_media_url_matches_protocol_v2_format_and_verifies` (unitaire) | format exact, vérification HMAC, mauvais secret rejeté |

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
- le hub est seul à décider quels assets sont accessibles ; le contrôle legacy
  « playback job » n'existe plus côté REST (§10 #1).

### 12.3 Service média (Phase 5b)

- worker d'ingestion, transitions `Processing/Failed`, purge/éviction du cache ;
- `POST items` repasse aujourd'hui un asset non-`READY` en `READY` sans
  re-validation (§10 #5) : à supprimer quand le service média est en place ;
- `ingest_url` ne consulte pas le cache disque : dédoublonnage uniquement par
  `source_hash` en base.

### 12.4 Admin (Phase 7b)

- création/révocation des codes d'appairage (`PairingCodesRepository`) ;
- révocation des clients overlay (`OverlayClientsRepository::revoke`) ;
- quota board global/guilde, purge des assets.

### 12.5 Renderer (Phase 8)

- appels `/overlay/meme-board/*` : envoyer `Authorization: Bearer` au lieu de
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
