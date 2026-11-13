# Serveur — ingestion REST et service média persistant

> **Statut : Phase 3c + 5b livrée ; durcie par task-49 (COR-13/COR-15).**
> Owner : `http-core` (task-16 puis task-49).
> Périmètre : `apps/lc-server/src/http/ingest/**`, `apps/lc-server/src/media/**`,
> `apps/lc-server/src/http/overlay/{media,media_board}.rs` (unification board),
> `crates/lc-media/**` (sécurités pipeline) et ce document.

## 1. Objectif

Reproduire côté Rust les deux routes `/ingest` du serveur legacy
(`server/src/components/ingest/ingestRoutes.ts`) et le service de persistance
média associé :

- `POST /ingest/pair/consume` : appairage d'un client d'ingestion à partir d'un
  code `INVITE_READ_ONLY` produit par l'overlay ;
- `POST /ingest` : soumission d'une URL/média/texte → ingestion dans le cache
  média, création d'un `PlaybackJob` (ou ajout au board média) ;

avec les garanties associées :

- cycle `MediaAsset` `PROCESSING → READY` (ou `FAILED` avec code legacy),
  réutilisation par `source_hash`, TTL de cache ;
- budgets de stockage (cache LRU et board média épinglé) ;
- hook `on_ingested` pour déclencher la lecture en temps réel (Phase 4b/6b) ;
- arrêt propre : les processus `ffmpeg`/`yt-dlp` sont tués au drop
  (`kill_on_drop` dans `lc-media`), donc à l'abandon d'un handler au shutdown.

## 2. Architecture

```
apps/lc-server/src/
├── http/ingest/
│   ├── mod.rs      routeur /ingest + helpers de réponse JSON legacy
│   ├── body.rs     normalisation du corps JSON (règles stringUtils.ts)
│   ├── auth.rs     authentification Bearer → client d'ingestion actif
│   ├── pair.rs     POST /ingest/pair/consume
│   └── submit.rs   POST /ingest
└── media/
    ├── settings.rs  Config serveur → lc_media::MediaConfig
    ├── tokens.rs    génération/hash SHA-256 des tokens d'ingestion
    ├── errors.rs    MediaServiceError (média / base / indisponible)
    ├── hook.rs      trait IngestHook + registre global
    ├── assets.rs    cycle PROCESSING/READY/FAILED + réutilisation + TTL
    ├── quotas.rs    budgets cache (LRU) et board média
    ├── board.rs     ajout au board média (épinglage 100 ans)
    ├── jobs.rs      création du PlaybackJob (règles de durée legacy)
    └── service.rs   MediaService : pipeline lc-media + persistance
```

Le routeur est monté par `crate::app::router` (déjà présent avant la phase) :
`http::ingest::router()` renvoie un `Router<AppState>` et porte lui-même le
préfixe `/ingest` ; `app.rs` fait le merge puis `.with_state(state)`. Aucune
modification de `app.rs`, `lib.rs`, `http/mod.rs` ou `Cargo.toml` n'a été
nécessaire (dépendances déjà déclarées par le Lead).

### Flux d'une soumission

```
POST /ingest
  → auth::resolve          (401 unauthorized / 503 ingest_api_disabled)
  → NormalizedIngest       (validation/normalisation)
  → MediaService::for_state (pipeline partagé lc-media + limites)
      → assets::find_reusable (cache hit par source_hash, touch TTL)
      → assets::begin_processing (ligne PROCESSING)
      → MediaPipeline::ingest_url (resolve → download → probe → transcode)
      → quotas::enforce_cache_budget (éviction LRU éventuelle)
      → assets::mark_ready | assets::mark_failed
  → board::add_to_board    (saveToBoard)  OU  jobs::create_for_ingest
  → hook::notify_ingested  (déclenchement lecture, Phase 4b)
  → 201 JSON legacy
```

## 3. Contrats HTTP (parité legacy)

### `POST /ingest/pair/consume`

Corps : `{ "code": string, "deviceName"?: string }`.

| Cas | Statut | Corps |
| --- | --- | --- |
| corps JSON illisible | 400 | `{"error":"invalid_payload"}` (AUD-A2-009) |
| code absent/vide | 400 | `{"error":"invalid_payload"}` |
| base absente (DATABASE_URL vide) | 503 | `{"error":"ingest_api_disabled"}` |
| code inconnu/expiré/consommé/mal formé | 404 | `{"error":"pairing_code_invalid_or_expired"}` |
| code réservé à l'overlay (`INVITE_READ_ONLY`) | 403 | `{"error":"pairing_code_scope_mismatch"}` |
| succès | 200 | `{"apiBaseUrl","ingestApiToken","ingestClientId","guildId","deviceName","authorName","authorImage"}` |

Ordre : purge des codes expirés → lookup du code utilisable → refus si mode
`INVITE_READ_ONLY` → consommation atomique (usage unique) → révocation des
anciens clients de la même guilde portant le même libellé → création du client.
Le token en clair n'est renvoyé qu'une fois ; seul son SHA-256 est persisté.

`deviceName` est trimé puis tronqué à **200 caractères** (`MAX_DEVICE_NAME_CHARS`,
comme l'overlay depuis AUD-A2-012) ; repli `Extension-<authorName>` si absent.
La collision `token_hash` (improbable) est réessayée une fois avec un token
régénéré, puis remontée en erreur serveur — AUD-A2-010.

### Code d'appairage émis depuis l'overlay

Une guilde sans Discord peut appairer une extension depuis l'overlay :
`POST /overlay/ingest-code` (Bearer overlay, droit `TriggerPlayback` ; un invité
reçoit `403 {"error":"forbidden"}` et un overlay sans membre
`409 {"error":"member_required"}`) crée un code `NORMAL` à usage unique dans
`pairing_codes`. Le nom du membre sert d'`author_name` ; son avatar n'est
recopié que s'il s'agit d'une URL `http(s)` (une image `data:` est trop
volumineuse pour l'extension).

Réponse `200` : `{"code":"ABCD2345","expiresAt":"2026-11-10T12:10:00.000Z",
"apiBaseUrl":"https://api.exemple.tld"}`. Le code expire au bout de
**10 minutes** et se consomme inchangé avec `POST /ingest/pair/consume`
ci-dessus.

L'auteur persisté est préfixé `member:` dans `created_by_discord_user_id`
(colonne NOT NULL), ce qui distingue un code émis par un membre overlay d'un
code émis depuis Discord. Côté overlay, `POST /overlay/pair/consume` refuse un
tel code avec `403 {"error":"pairing_code_scope_mismatch"}` **sans le
consommer** : le code reste utilisable par l'extension.

### `POST /ingest`

Authentification **avant** toute validation de corps, comme le legacy.

| Cas | Statut | Corps |
| --- | --- | --- |
| corps JSON illisible | 400 | `{"error":"invalid_payload"}` (après l'auth, AUD-A2-009) |
| aucun client actif / base absente | 503 | `{"error":"ingest_api_disabled"}` |
| token absent, invalide ou révoqué | 401 | `{"error":"unauthorized"}` |
| `guildId` ≠ guilde du client | 403 | `{"error":"guild_mismatch"}` |
| `authorName`/`authorImage` fournis | 400 | `{"error":"invalid_payload","message":"authorName/authorImage are token-bound for ingest clients"}` |
| ni `url`/`media` ni `text` | 400 | `{"error":"invalid_payload"}` |
| `saveToBoard` sans `url`/`media` | 400 | `{"error":"invalid_payload","message":"saveToBoard requires a resolvable media source"}` |
| ingestion média en échec | 422 | `{"error":"media_ingestion_failed","code":<CODE>,"message":…}` |
| `saveToBoard` réussi | 201 | `{"accepted":true,"savedToBoard":true,"created":bool,"guildId","mediaBoardItemId","mediaAssetId"}` |
| job créé | 201 | `{"accepted":true,"jobId","guildId","hasMedia":bool,"hasText":bool}` |
| erreur interne (base) | 500 | `{"error":"internal_error"}` |

Champs acceptés : `guildId`, `url`, `media`, `text`, `showText`, `forceRefresh`,
`authorName`, `authorImage`, `durationSec`, `saveToBoard`. Conversions legacy
(`server/src/services/stringUtils.ts`) : chaînes trim non vides uniquement ;
`showText` n'accepte qu'un vrai booléen ; `forceRefresh`/`saveToBoard` tolèrent
`true`, `1`, `"1"`, `"true"`, `"yes"`, `"on"` ; `durationSec` doit être un
nombre fini > 0 (arrondi supérieur).

`media` est prioritaire sur `url` (comme `pickSource` legacy). L'offset de départ
YouTube est extrait via `lc_discord::text::youtube_start_offset_sec` puis encodé
dans le texte du job par `lc_discord::rich_payload::build_media_text_payload`.
`saveToBoard` ne crée **pas** de `PlaybackJob` (retour anticipé du legacy).

## 4. Authentification

- `Authorization: Bearer <token>` uniquement ; schéma insensible à la casse.
- Lookup par `SHA-256(token)` hexadécimal dans `ingest_clients.token_hash` ;
  les clients révoqués (`revoked_at`) sont exclus par le repository.
- `last_seen_at` mis à jour en best-effort : une erreur d'écriture ne refuse pas
  une requête authentifiée.
- `isIngestApiEnabled` legacy = existence d'au moins un client non révoqué :
  c'est ce qui distingue `503 ingest_api_disabled` de `401 unauthorized`.

**Écart assumé avec la formulation de la task** : `INGEST_API_TOKEN` n'est
**pas** accepté comme token d'ingestion. Le legacy ne l'utilise jamais pour
authentifier `/ingest` (grep exhaustif : uniquement `env.ts` + affichage UI
admin) ; la variable reste configurée pour parité, sans effet sur ces routes.

Pas de comparaison constant-time nécessaire ici : l'authentification se fait par
lookup de hash (le hash présenté est comparé par la base, pas en mémoire).

## 5. Service média (`src/media/`)

### Configuration

`settings::media_config(&Config)` traduit les champs `media_*` du serveur vers
`lc_media::MediaConfig` (mêmes variables d'environnement et défauts que legacy,
cf. `docs/rust/lc-media.md`). Aucune duplication de configuration : `lc-media`
reste la source de vérité pour les defaults, le bridge n'écrase que ce que le
serveur connaît. `lc-media` utilise des `std::time::Duration` (les conversions
sont saturantes).

### Pipeline partagé

`MediaService::new(config, db, clock)` est testable (pipeline propre).
`MediaService::for_state(&AppState)` est le chemin de **toutes** les routes média
depuis task-49 : le `MediaPipeline` (client HTTP + sémaphore de concurrence) est
construit une seule fois par processus (`OnceLock`), les appels suivants
réutilisent l'instance. La limite `MEDIA_MAX_CONCURRENT_INGESTIONS` est donc
globale au serveur, comme le legacy. Une base absente renvoie `Unavailable` →
`503 ingest_unavailable`.

Le board média (`POST /overlay/media-board/items` et
`POST /overlay/media-board/upload`) utilise la **même** `MediaService`
(AUD-V2-S-003) : cache `READY`/`source_hash`, sémaphore, budget cache,
`record_failure` et nettoyage `discard_partial_files` sont partagés. Ces deux
routes sont désormais **asynchrones** : elles valident la requête, enregistrent
l'ingestion, répondent `202 {ingestionId}` puis laissent une tâche `tokio::spawn`
détachée de la connexion HTTP exécuter le pipeline et l'insertion board. La fin
de tâche met à jour le registre et notifie le client (`media.ingestion`) ainsi
que la guilde (`board.changed`). Voir `docs/rust/server-overlay.md` §7.2.

### Confinement du stockage (fail-closed)

Au démarrage (`lib.rs`), `media::ensure_storage_root(&config.media_storage_dir)`
crée puis canonicalise `MEDIA_STORAGE_DIR` ; un chemin inutilisable (vide, fichier,
permissions) fait échouer le démarrage avec un contexte explicite. Le pipeline
refuse aussi de publier dans une racine non canonicalisable (`lc_media::storage::ensure_canonical`,
`DOWNLOAD_FAILED`). À la lecture, `resolve_media_file` applique le confinement
strict : si la racine ne peut plus être canonicalisée à chaud, l'accès est refusé
en `404 media_not_found_on_disk` (plus de service fail-open hors racine — AUD-A2-003/A4-014).

### Worker de purge (fichiers orphelins et assets périmés)

`media::purge::spawn_purge_worker(&AppState)` démarre un worker `tokio` (première
passe immédiate puis toutes les 10 min, `PURGE_INTERVAL`) si une base est
configurée. Chaque passe `purge_once(db, storage, now)` :

1. supprime les assets `FAILED` inactifs depuis plus de 1 h et `PROCESSING` depuis
   plus de 2 h (`STALE_PROCESSING_TTL`/`FAILED_TTL` legacy, AUD-A4-008) — jamais
   ceux épinglés par le board média ou référencés par un job `PENDING`/`PLAYING`,
   avec suppression du fichier associé ;
2. évince les assets `READY` expirés via `quotas::purge_candidates` puis
   `evict_source_hashes` (fichiers + lignes) ;
3. scanne `MediaStorage::list_variants` : supprime les fichiers de transit
   `.tmp-*` et les orphelins absents de la table `media_assets`, uniquement après
   une grâce de 1 h (`ORPHAN_GRACE`) et si le fichier n'est pas plus récent.

Ces opérations sont idempotentes et best-effort (un fichier déjà absent n'échoue
pas la passe).

### Cycle des assets

1. Cache hit : asset `READY`, non expiré, fichier présent sur disque
   (`tokio::fs::try_exists`) → expiration repoussée à `max(expiration, now+TTL)`
   puis retour immédiat (pas de réseau, pas de pipeline) sauf `forceRefresh=true`.
2. Cache miss : `begin_processing` crée ou réarme la ligne par `source_hash`
   (`PROCESSING`, expiration TTL cache) avant le téléchargement.
3. Succès : budget cache vérifié, puis `mark_ready` persiste kind/MIME/durée/
   dimensions/orientation/chemin/taille et pousse `expires_at` à `now+TTL`.
4. Échec : `mark_failed` écrit `"[CODE] message"` (tronqué à 500 caractères,
   format legacy `[DOWNLOAD_FAILED] …`) et fixe `expires_at = now + 1 h`.

**SQL direct** : `lc-db` n'expose que `update_status(id, status, error)`, alors
que le passage en `READY` doit écrire l'ensemble des métadonnées ; de même
l'éviction doit supprimer une ligne d'asset. Ces deux opérations utilisent
`db.pool()` avec les colonnes du schéma `crates/lc-db/migrations/0001_init.sql`
(documenté ici pour que la Phase 7 puisse décider d'enrichir les repositories ou
de conserver ces requêtes). Aucun accès SQL direct en dehors de
`assets.rs`/`quotas.rs`.

### Mapping d'erreurs

`lc_media::MediaError` porte les 11 codes legacy et leurs libellés
(`media_ingestion_failed` + `code`) ; `MediaServiceError` ajoute les erreurs de
base et l'indisponibilité :

- `Media` → 422 `media_ingestion_failed` (code legacy dans `code`) ;
- `Unavailable` → 503 `ingest_unavailable` ;
- `Database` / inattendu → 500 `internal_error`.

Une erreur inattendue du service (`MediaServiceError` non média) donne 500, et
non `DOWNLOAD_FAILED` comme le legacy : décision assumée, documentée ici.

### Sécurités pipeline (task-49 / COR-15)

- **SVG refusé** (AUD-A4-004) : `normalize_image` rejette `image/svg+xml` avant
  toute copie (Content-Type, extension `.svg`, ou signature `<?xml`/`<svg` sur
  les premiers octets) → `422 media_ingestion_failed` code `INVALID_MEDIA`.
  `kind_from_mime` exclut aussi SVG. Les images sont donc toujours des raster
  ré-encodés, jamais servies telles quelles.
- **MIME dérivé du probe** (AUD-A4-010) : pour les images, le codec ffprobe
  prime (whitelist png/jpeg/webp/gif/bmp/avif), puis le Content-Type distant
  whitelisté, puis l'extension. Le Content-Type distant seul n'est jamais
  stocké/servi tel quel.
- **Options ffprobe** (AUD-A4-012) : `probe_args` place `-i` avant le chemin
  (un chemin local commençant par `-` n'est plus interprété comme option).
- **Sorties de commandes bornées** (AUD-A4-005) : `lc_media::process::run_command`
  draine stdout/stderr avec un plafond de 8 Mio ; dépassement →
  `INVALID_MEDIA` (« sortie de … trop volumineuse »), plus de buffer illimité.
- **Cookie TikTok** (AUD-A4-013) : `sanitize_tiktok_cookie` trim, retire le
  préfixe `cookie:`, refuse vide/caractères de contrôle, plafonne à 4096
  caractères ; la valeur n'est jamais journalisée.
- **Sanitizers HTML allowlist** (AUD-A4-015) : `lc_media::html::sanitize_embed_html`
  (oEmbed/embed) et `crate::media::html::sanitize_tweet_embed` (carte tweet,
  Phase 6b) n'autorisent qu'une liste fermée de tags/attributs, suppriment
  `script/style/iframe/object/embed/form`, forcent `rel`/`target` sur `<a>` et
  n'acceptent que `http(s)` pour `href`/`src` (attributs ré-échappés).

Tests réseau réels (SSRF, redirection privée, taille, `file:`) restent
`#[ignore]` : la CI ne dépend d'aucun réseau.

## 6. Quotas et board média

Deux budgets distincts, plans purs fournis par `lc_media::cache` :

- **cache non persistant** : total = assets `READY` non épinglés (les assets du
  board média ne comptent pas) ; les assets épinglés ou référencés par un job
  `PENDING`/`PLAYING` ne sont jamais évincés ; éviction LRU
  (`last_accessed_at`, puis `created_at`) si le total + la nouvelle taille dépasse
  `MEDIA_CACHE_MAX_TOTAL_MB`. Éviction = suppression des fichiers normalisés puis
  de la ligne. Budget insuffisant → `CACHE_STORAGE_LIMIT_REACHED` avec le
  message legacy.
- **board média** : jamais d'éviction automatique. Un ajout qui dépasserait
  `MEDIA_BOARD_MAX_TOTAL_MB` échoue en `BOARD_STORAGE_LIMIT_REACHED` (message
  legacy). Un ajout existant est idempotent (`created:false`) et réépingle
  l'asset (`expires_at = now + 100 ans`, `last_accessed_at = now`).

Le comptage « actif » (jobs `PENDING`/`PLAYING`) et « épinglé » est calculé par
requêtes SQL sur `media_asset_id` : les repositories n'offrent pas ces filtres.

Le board média exposé par `http::overlay::media_board` partage ces budgets :
`POST /overlay/media-board/items` (sans alias legacy) enregistre une ingestion
asynchrone, répond `202 {ingestionId}` et passe dans la tâche de fond par la même
`MediaService` puis `assets::pin` (expiry à +100 ans) ; la liste charge en une
jointure bornée et la suppression est transactionnelle
(`delete_item_and_release`). Un échec de la tâche produit `media.ingestion
{ok:false}` avec `media_ingestion_failed`/`internal_error` et un message lisible.
Voir `docs/rust/server-overlay.md` §7.2.

### `POST /overlay/media-board/upload`

Ajout d'un média au board depuis un fichier local, sans URL ni multipart.

Paramètres de query :

| Paramètre | Obligatoire | Contrainte |
| --- | --- | --- |
| `filename` | oui | 1 à 255 caractères, sans `/`, `\`, `..` ni caractère de contrôle |
| `title` | non | tronqué à 240 caractères ; défaut = nom du fichier sans extension |
| `message` | non | tronqué à 500 caractères |

Le corps est le fichier brut (`application/octet-stream` ou tout autre
`Content-Type`, utilisé seulement comme indice). L'authentification et la garde
`invite_read_only` sont évaluées **avant** toute lecture du corps
(`403 read_only_session`). Un `filename` absent/invalide répond
`400 {"error":"invalid_payload","message":…}`.

Le handler lit le corps en streaming avec `axum::body::Body::into_data_stream`
(jamais tout le fichier en mémoire), écrit dans un répertoire temporaire du même
espace que les téléchargements `lc-media`, calcule le SHA-256 au fil de l'eau et
répond **après réception complète** du corps. Le répertoire temporaire est ensuite
déplacé dans la tâche asynchrone et supprimé à sa fin, succès ou échec (garde
RAII). Une coupure client après la réponse n'annule plus la conversion. La limite
globale `DefaultBodyLimit` de 1 Mio ne s'applique pas à cette route (`Body`, pas
`Bytes`/`Json`) ; les autres routes la conservent. La route est exemptée du
timeout global (`app::is_timeout_exempt`) car la réception peut être longue.

Limites et erreurs synchrones (avant la réponse) :

| Cas | Statut | Corps |
| --- | --- | --- |
| `filename` absent, vide, trop long ou interdit | 400 | `{"error":"invalid_payload","message":…}` |
| corps vide | 400 | `{"error":"invalid_payload",…}` |
| corps > `MEDIA_MAX_SIZE_MB` (2048 Mo par défaut) | 413 | `{"error":"payload_too_large"}` |
| session `invite_read_only` | 403 | `{"error":"read_only_session"}` |
| 20 ingestions `processing` déjà actives | 429 | `{"error":"too_many_ingestions"}` |
| base/service indisponible | 500/503 | `{"error":"internal_error"}` / `database_unavailable` |
| fichier accepté | 202 | `{"ingestionId":"<uuid>"}` |

Après le `202`, l'ingestion passe par
`MediaPipeline::ingest_local_file_with_hash` : détection du type par les octets,
puis les mêmes contrôles de taille, transcodage, publication content-addressed et
limiteur de concurrence que `ingest_url`. Les SVG sont refusés (`INVALID_MEDIA`).
Une image raster locale valide peut être plus petite que le seuil anti-fichier-vide
de 1 Kio des sources distantes (icône 1×1) ; ce seuil reste inchangé pour
`ingest_url`. Les échecs sont signalés par `media.ingestion` :
`unsupported_media` pour un type non reconnu, puis `media_ingestion_failed` /
`internal_error` avec message lisible.

La clé de cache/déduplication est `upload:sha256:<sha256 du contenu>` ; si un
asset `READY` existe déjà pour cette clé il est réutilisé sans nouvelle
ingestion. La colonne `media_assets.source_url` reçoit `upload:<filename>`. Les
étapes board (quota `MEDIA_BOARD_MAX_TOTAL_MB`, dédoublonnage, épinglage 100 ans,
insertion, auteur) sont partagées avec `create_item` : `media.ingestion` porte
`created:true` pour un nouvel item, `created:false` si l'asset était déjà sur le
board (aucun `board.changed` dans ce second cas).

## 7. Jobs et hook temps réel

`jobs::create_for_ingest` reproduit `createPlaybackJob` :

1. durée candidate = durée explicite → durée de l'asset (médias non-image) →
   `DEFAULT_DURATION` du serveur si média → `default_media_time` de la guilde →
   défaut serveur ;
2. clamp au `max_media_time` de la guilde s'il est défini ;
3. si média non-image et offset YouTube intégré au texte : durée réduite de
   l'offset, `resume_offset_sec` = offset ;
4. `status = PENDING`, `submission_date`/`execution_date`/`scheduled_at` = maintenant,
   priorité 0 ;
5. persistance via `playback_jobs().create`.

Puis `hook::notify_ingested(&job)` est appelé. **Contrat Phase 4b** : le hook
doit appeler `PlaybackEngine::notify(&job.guild_id)` (re-traitement de la
guilde), **pas** `enqueue` : `PlaybackEngine::enqueue` insère lui-même le job en
base (contrainte d'unicité sinon). Le job est déjà persisté par ce service.

Le registre de hook est un `RwLock<Option<Arc<dyn IngestHook>>>` global
(`install_ingest_hook`), remplaçable ; absent → no-op. Il est isolé de
`crate::ws` : `src/media` ne dépend pas du hub WebSocket.

## 8. Arrêt et annulation

`lc-media` lance les processus externes avec `kill_on_drop`. Le shutdown
gracieux d'axum abandonne les handlers en cours : les futures d'ingestion
synchrones (`/ingest`) sont droppées, donc les processus `ffmpeg`/`yt-dlp` sont
tués et l'asset reste en `PROCESSING` (nettoyé ensuite par le worker
`media::purge` — task-42 — via `STALE_PROCESSING_TTL` de 2 h, comme le legacy).
Les tâches asynchrones du board (`tokio::spawn`) ne dépendent pas de la connexion
HTTP : une déconnexion client ne les annule pas ; un arrêt du runtime les
interrompt comme toute tâche Tokio, et le même worker de purge nettoie les
`PROCESSING` orphelins. La publication du fichier étant atomique (transit
`.tmp-*` + `rename`), aucune annulation ne laisse de média partiellement publié.
Aucun endpoint de debug/shutdown n'est exposé.

## 9. Tests

Tests de la lane (état task-49 : **47 tests `media::` + 21 tests `http::ingest`**,
suite complète `cargo test -p lc-server --lib` = 284 passed, 1 ignoré), unitaires
et in-process, base SQLite temporaire `tempfile::tempdir()`, aucun réseau réel :

| Module | Tests | Couverture |
| --- | --- | --- |
| `media::settings` | 6 | bridge config, encodeurs, conversions saturantes |
| `media::tokens` | 2 | longueur/unicité, hash SHA-256 connu |
| `media::errors` | 2 | classification média vs interne |
| `media::hook` | 1 | installation/notification/no-op |
| `media::assets` | 6 | PROCESSING→READY, réutilisation, FAILED + troncature, fraîcheur/fichier |
| `media::quotas` | 4 | éviction LRU, assets actifs protégés, budget board |
| `media::board` | 3 | ajout+épinglage, idempotence, budget dépassé |
| `media::jobs` | 5 | durées par défaut/guilde/explicite, image, offset riche |
| `media::service` | 4 | cache hit sans réseau, échec pipeline → FAILED, source absente, nettoyage fichier après échec `forceRefresh` |
| `media::html` | 4 | sanitizer tweet allowlist : tags/attrs autorisés, blocs actifs supprimés, `href`/`src` http(s), texte conservé |
| `media::confinement` | 2 | racine créée/canonicalisée idempotente, chemins vides/fichiers refusés |
| `media::purge` | 3 | FAILED/PROCESSING périmés supprimés (fichier inclus), assets épinglés board conservés et READY expiré évincé, transit/orphelins supprimés seulement après la grâce |
| `http::ingest::body` | 4 | normalisation, types, drapeaux, durées |
| `http::ingest::auth` | 1 | parsing Bearer |
| `http::ingest::pair` | 6 | succès + hash + code consommé, expiré/inconnu, scope invité, code absent, JSON illisible, deviceName long tronqué à 200 |
| `http::ingest::submit` | 9 | 503/401, job texte, guild mismatch, champs auteur, corps vide, saveToBoard sans média, révocation, JSON illisible |
| `http::ingest::mod` | 1 | montage des deux variantes de slash |

Vérifications de la lane (task-49) : `cargo fmt -p lc-media -p lc-server -- --check`,
`cargo clippy -p lc-media -p lc-server --all-targets -- -D warnings`,
`cargo test -p lc-media` (130 unit + 12 intégration + 1 doctest, 4 réseau `#[ignore]`),
`cargo test -p lc-server --lib`.

## 10. Écarts et décisions vs legacy

| Sujet | Legacy | Rust |
| --- | --- | --- |
| Auth `/ingest` | token de client uniquement | idem ; `INGEST_API_TOKEN` ignoré pour ces routes (écart assumé vs task) |
| Erreur inattendue d'ingestion | `DOWNLOAD_FAILED` (classification) | 500 `internal_error` (erreur serveur, pas média) |
| Rich tweet card | résolution oEmbed + payload riche | **hors scope 3c** (Phase 6b) ; les URLs de tweet passent par le pipeline média standard |
| `saveToBoard` | 201 board, pas de job | idem ; le `board.changed {change:"added"}` est diffusé si l'item est nouveau |
| Réponse `/pair/consume` `apiBaseUrl` | `env.API_URL` | `api_url` validée, slash final retiré |
| `deviceName` | champ libre | trimé puis tronqué à 200 caractères, comme l'overlay (AUD-A2-012) |
| Collision `token_hash` à l'appairage | erreur 500 Prisma | réessai unique avec token régénéré, puis 500 (AUD-A2-010) |
| Purge worker | boucle toutes les 60 s | worker `media::purge` toutes les 10 min (task-42) : FAILED > 1 h, PROCESSING > 2 h, READY expirés, transit `.tmp-*` et orphelins du storage après 1 h de grâce |
| Corps JSON invalide | 400 Fastify (parsing avant handler) | 400 `invalid_payload` dédié ; l'auth/token reste évaluée avant le refus de corps (AUD-A2-009) |
| SVG / MIME distant | SVG accepté, Content-Type distant de confiance | SVG refusé (`INVALID_MEDIA`), MIME dérivé du probe (AUD-A4-004/A4-010) |
| Commandes externes | sortie non bornée | stdout/stderr plafonnés à 8 Mio (`run_command`), ffprobe `-i` (AUD-A4-005/A4-012) |
| Cookie TikTok | préfixe retiré, valeur brute | validation contrôle/cap 4096, jamais journalisée (AUD-A4-013) |
| HTML embeds | suppression script/iframe seulement | allowlist tags/attrs + `rel`/`target` forcés (AUD-A4-015) |
| SQL direct | Prisma | `db.pool()` pour `mark_ready` et l'éviction (repositories lc-db incomplets) |

## 11. Risques et points Phase 4b/6b

- **Hook** : contrat `notify` (pas `enqueue`) ; à câbler en Phase 4b au hub WS
  (`install_ingest_hook`) sans importer `src/ws` depuis `src/media`.
- **Pipeline global** : la config du pipeline est figée au premier usage
  (`OnceLock`), y compris via `MediaService::for_state` désormais utilisé par le
  board. Cohérent avec une configuration d'environnement immuable, mais rend les
  tests de routes dépendants de la première configuration vue ; les tests média
  contournent le global via `MediaService::new`. Réinitialisation = redémarrage.
- **`forceRefresh`** : le téléchargement est refait et les variantes partielles
  sont supprimées en cas d'échec (`discard_partial_files`) ; les anciens fichiers
  d'un asset étaient déjà nettoyés par le pipeline avant publication atomique.
  L'invalidation de cache est donc effective, mais aucune purge opportuniste des
  autres assets n'est déclenchée dans la foulée (worker de purge séparé).
- **Dette — SQL direct** : `assets.rs`/`quotas.rs` utilisent `db.pool()` pour le
  passage complet en `READY`, l'éviction et le total du board, faute de méthodes
  dans les repositories `lc-db`. À promouvoir en méthodes de repository
  (`MediaAssetsRepository`) une fois l'API stabilisée, puis supprimer les requêtes
  ad hoc de `src/media` (décision Lead : accepté temporairement, dette tracée).
- **Lignes `PROCESSING` orphelines** après crash/arrêt : purge worker Phase 6 —
  implémenté en task-42 (`media::purge`, TTL 2 h), avec purge des `FAILED` > 1 h et
  des fichiers orphelins/transit.
- **Comptage « actif »** par SQL direct sur `playback_jobs` : à consolider si
  `lc-db` expose un jour la requête.
- **Rich tweet card** et **offset média** : la Phase 6b devra étendre
  `submit.rs` (payload riche, carte tweet) ; l'encodage texte est déjà délégué à
  `lc-discord`.
- **Auth** : le token global `INGEST_API_TOKEN` reste inutilisé pour `/ingest`
  (parité legacy) ; si la Phase 7 décide de l'accepter, l'ajouter dans
  `auth::resolve` uniquement (pas de changement de contrat de réponse).
- **Concurrence** : limite globale `MEDIA_MAX_CONCURRENT_INGESTIONS` portée par
  le pipeline partagé ; pas de limite par client d'ingestion (legacy idem).
- **Tests réseau** : les scénarios SSRF/redirection/taille/sortie de commande
  sont couverts par des tests locaux et des `#[ignore]` ; une vraie CI réseau
  reste à décider (aucun test ne dépend du réseau par défaut).

## 12. Références

- `server/src/components/ingest/ingestRoutes.ts` (contrat HTTP legacy)
- `server/src/services/ingestAuth.ts`, `overlayAuth.ts`
- `server/src/services/media/{mediaIngestion,mediaCache,mediaErrors,mediaSourceResolver}.ts`
- `server/src/services/memeBoard.ts`, `server/src/services/playbackJobs.ts`
- `docs/rust/lc-media.md`, `docs/rust/lc-playback.md`, `docs/rust/lc-db.md`
- `docs/rust/migration-plan.md` (Phases 3c/5b), `docs/rust/protocol-v2.md`
