# Serveur — ingestion REST et service média persistant

> **Statut : Phase 3c + 5b livrée.** Owner : `http-core` (task-16).
> Périmètre : `apps/lc-server/src/http/ingest/**`, `apps/lc-server/src/media/**`,
> ce document. Le crate `lc-media` (pipeline) et les repositories `lc-db` ne sont
> pas modifiés par cette phase.

## 1. Objectif

Reproduire côté Rust les deux routes `/ingest` du serveur legacy
(`server/src/components/ingest/ingestRoutes.ts`) et le service de persistance
média associé :

- `POST /ingest/pair/consume` : appairage d'un client d'ingestion à partir d'un
  code `INVITE_READ_ONLY` produit par l'overlay ;
- `POST /ingest` : soumission d'une URL/média/texte → ingestion dans le cache
  média, création d'un `PlaybackJob` (ou ajout au meme board) ;

avec les garanties associées :

- cycle `MediaAsset` `PROCESSING → READY` (ou `FAILED` avec code legacy),
  réutilisation par `source_hash`, TTL de cache ;
- budgets de stockage (cache LRU et meme board épinglé) ;
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
    ├── quotas.rs    budgets cache (LRU) et meme board
    ├── board.rs     ajout au meme board (épinglage 100 ans)
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
| code absent/vide | 400 | `{"error":"invalid_payload"}` |
| base absente (DATABASE_URL vide) | 503 | `{"error":"ingest_api_disabled"}` |
| code inconnu/expiré/consommé/mal formé | 404 | `{"error":"pairing_code_invalid_or_expired"}` |
| code réservé à l'overlay (`INVITE_READ_ONLY`) | 403 | `{"error":"pairing_code_scope_mismatch"}` |
| succès | 200 | `{"apiBaseUrl","ingestApiToken","ingestClientId","guildId","deviceName","authorName","authorImage"}` |

Ordre : purge des codes expirés → lookup du code utilisable → refus si mode
`INVITE_READ_ONLY` → consommation atomique (usage unique) → révocation des
anciens clients de la même guilde portant le même libellé → création du client.
Le token en clair n'est renvoyé qu'une fois ; seul son SHA-256 est persisté.

### `POST /ingest`

Authentification **avant** toute validation de corps, comme le legacy.

| Cas | Statut | Corps |
| --- | --- | --- |
| aucun client actif / base absente | 503 | `{"error":"ingest_api_disabled"}` |
| token absent, invalide ou révoqué | 401 | `{"error":"unauthorized"}` |
| `guildId` ≠ guilde du client | 403 | `{"error":"guild_mismatch"}` |
| `authorName`/`authorImage` fournis | 400 | `{"error":"invalid_payload","message":"authorName/authorImage are token-bound for ingest clients"}` |
| ni `url`/`media` ni `text` | 400 | `{"error":"invalid_payload"}` |
| `saveToBoard` sans `url`/`media` | 400 | `{"error":"invalid_payload","message":"saveToBoard requires a resolvable media source"}` |
| ingestion média en échec | 422 | `{"error":"media_ingestion_failed","code":<CODE>,"message":…}` |
| `saveToBoard` réussi | 201 | `{"accepted":true,"savedToBoard":true,"created":bool,"guildId","memeBoardItemId","mediaAssetId"}` |
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
`MediaService::for_state(&AppState)` est le chemin des routes : le
`MediaPipeline` (client HTTP + sémaphore de concurrence) est construit une seule
fois par processus (`OnceLock`), les appels suivants réutilisent l'instance. La
limite `MEDIA_MAX_CONCURRENT_INGESTIONS` est donc globale au serveur, comme le
legacy. Une base absente renvoie `Unavailable` → `503 ingest_unavailable`.

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

## 6. Quotas et meme board

Deux budgets distincts, plans purs fournis par `lc_media::cache` :

- **cache non persistant** : total = assets `READY` non épinglés (les assets du
  meme board ne comptent pas) ; les assets épinglés ou référencés par un job
  `PENDING`/`PLAYING` ne sont jamais évincés ; éviction LRU
  (`last_accessed_at`, puis `created_at`) si le total + la nouvelle taille dépasse
  `MEDIA_CACHE_MAX_TOTAL_MB`. Éviction = suppression des fichiers normalisés puis
  de la ligne. Budget insuffisant → `CACHE_STORAGE_LIMIT_REACHED` avec le
  message legacy.
- **meme board** : jamais d'éviction automatique. Un ajout qui dépasserait
  `MEDIA_BOARD_MAX_TOTAL_MB` échoue en `BOARD_STORAGE_LIMIT_REACHED` (message
  legacy). Un ajout existant est idempotent (`created:false`) et réépingle
  l'asset (`expires_at = now + 100 ans`, `last_accessed_at = now`).

Le comptage « actif » (jobs `PENDING`/`PLAYING`) et « épinglé » est calculé par
requêtes SQL sur `media_asset_id` : les repositories n'offrent pas ces filtres.

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
gracieux d'axum abandonne les handlers en cours : les futures d'ingestion sont
droppées, donc les processus `ffmpeg`/`yt-dlp` sont tués et l'asset reste en
`PROCESSING` (nettoyé par le purge worker de la Phase 6, comme le
`STALE_PROCESSING_TTL` legacy de 2 h). Aucun endpoint de debug/shutdown n'est
exposé.

## 9. Tests

50 tests ajoutés (unitaires et in-process, base SQLite temporaire
`tempfile::tempdir()`, aucun réseau réel) :

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
| `media::service` | 3 | cache hit sans réseau, échec pipeline → FAILED, source absente |
| `http::ingest::body` | 4 | normalisation, types, drapeaux, durées |
| `http::ingest::auth` | 1 | parsing Bearer |
| `http::ingest::pair` | 4 | succès + hash + code consommé, expiré/inconnu, scope invité, code absent |
| `http::ingest::submit` | 8 | 503/401, job texte, guild mismatch, champs auteur, corps vide, saveToBoard sans média, révocation |
| `http::ingest::mod` | 1 | montage des deux variantes de slash |

Vérifications de la lane : `cargo fmt -p lc-server -- --check`,
`cargo clippy -p lc-server --all-targets -- -D warnings`,
`cargo test -p lc-server`.

## 10. Écarts et décisions vs legacy

| Sujet | Legacy | Rust |
| --- | --- | --- |
| Auth `/ingest` | token de client uniquement | idem ; `INGEST_API_TOKEN` ignoré pour ces routes (écart assumé vs task) |
| Erreur inattendue d'ingestion | `DOWNLOAD_FAILED` (classification) | 500 `internal_error` (erreur serveur, pas média) |
| Rich tweet card | résolution oEmbed + payload riche | **hors scope 3c** (Phase 6b) ; les URLs de tweet passent par le pipeline média standard |
| `saveToBoard` | 201 board, pas de job | idem |
| Réponse `/pair/consume` `apiBaseUrl` | `env.API_URL` | `api_url` validée, slash final retiré |
| Purge worker cache | boucle toutes les 60 s | **non porté** : fonctions `purge_candidates`/`evict_source_hashes` prêtes, worker en Phase 6 |
| Corps JSON invalide | 400 Fastify (parsing avant handler) | `Option<Json>` → corps vide ; auth d'abord, donc un token invalide peut donner 401 avant 400 |
| SQL direct | Prisma | `db.pool()` pour `mark_ready` et l'éviction (repositories lc-db incomplets) |

## 11. Risques et points Phase 4b/6b

- **Hook** : contrat `notify` (pas `enqueue`) ; à câbler en Phase 4b au hub WS
  (`install_ingest_hook`) sans importer `src/ws` depuis `src/media`.
- **Pipeline global** : la config du pipeline est figée au premier usage
  (`OnceLock`) ; cohérent avec une configuration d'environnement immuable, mais
  rend les tests de routes dépendants de la première configuration vue. Les
  tests média contournent le global via `MediaService::new`.
- **Dette — SQL direct** : `assets.rs`/`quotas.rs` utilisent `db.pool()` pour le
  passage complet en `READY`, l'éviction et le total du board, faute de méthodes
  dans les repositories `lc-db`. À promouvoir en méthodes de repository
  (`MediaAssetsRepository`) une fois l'API stabilisée, puis supprimer les requêtes
  ad hoc de `src/media` (décision Lead : accepté temporairement, dette tracée).
- **Lignes `PROCESSING` orphelines** après crash/arrêt : purge worker Phase 6.
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

## 12. Références

- `server/src/components/ingest/ingestRoutes.ts` (contrat HTTP legacy)
- `server/src/services/ingestAuth.ts`, `overlayAuth.ts`
- `server/src/services/media/{mediaIngestion,mediaCache,mediaErrors,mediaSourceResolver}.ts`
- `server/src/services/memeBoard.ts`, `server/src/services/playbackJobs.ts`
- `docs/rust/lc-media.md`, `docs/rust/lc-playback.md`, `docs/rust/lc-db.md`
- `docs/rust/migration-plan.md` (Phases 3c/5b), `docs/rust/protocol-v2.md`
