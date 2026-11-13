# lc-db — persistance SQLite (Phase 2)

Crate `lc-db` : pool SQLite (`sqlx` 0.8), migrations embarquées (schéma initial
`0001_init`, invariant de lecture `0002_playback_single_playing`) et repositories
typés pour les 8 tables du contrat `docs/rust/schema-v3.sql`. Aucune requête
`query!` compile-time : toutes les requêtes sont **runtime** (`sqlx::query`,
`query_as`, `query_scalar`) avec des structs `FromRow` explicites. Un binaire
n'a donc pas besoin d'une base accessible à la compilation.

Ce document est la référence d'API pour la Phase 3 (serveur) et la Phase 4
(migration des données Prisma).

## 1. Contrat de persistance

| Élément | Décision |
| --- | --- |
| DDL | `crates/lc-db/migrations/` contient `0001_init.sql` (schéma initial) et `0002_playback_single_playing.sql` (index unique partiel). Leur concaténation dans l'ordre des versions est une **copie octet pour octet** de `docs/rust/schema-v3.sql` (9 253 octets, SHA-256 `48D67DCD…2C70`). Test `crates/lc-db/tests/schema_contract.rs`. |
| Version de schéma | `app_metadata['schema_version'] = '3.0.0'` (constante `lc_db::SCHEMA_VERSION`). Vérifiée à l'ouverture. |
| Invariant de lecture | Index unique partiel `idx_playback_jobs_single_playing` sur `playback_jobs(guild_id) WHERE status='PLAYING'` (migration `0002`) : le moteur refuse tout second job `PLAYING` pour une guilde, même hors `claim_next`. |
| Dates | `INTEGER` millisecondes epoch UTC, **jamais** `DEFAULT CURRENT_TIMESTAMP`. Conversion aux frontières via `lc_domain::time` ; côté crate, `lc-db/src/convert.rs`. |
| Booléens | `INTEGER` 0/1 avec `CHECK (col IN (0,1))`. |
| Énumérations | Texte **MAJUSCULE** (`IMAGE`, `PLAYING`, `INVITE_READ_ONLY`), conversions dans `lc-domain::enums`. |
| Identifiants | UUID v4 stockés en `TEXT` canonique `8-4-4-4-12` ; `GuildId`/`PairingCode` = chaînes externes validées. |
| Migrations | Appliquées automatiquement à l'ouverture (`sqlx::migrate!("./migrations")`), idempotentes (`_sqlx_migrations`). Une cible migrée depuis le legacy reçoit `0002`..`0005` au **premier démarrage** du serveur : le registre sqlx du migrateur ne couvre que `0001` (voir `docs/rust/migrator.md` §10.1). |

Pragmas posés sur **chaque connexion** du pool (`after_connect` implicite de
`SqliteConnectOptions`) :

| Pragma | Valeur | Raison |
| --- | --- | --- |
| `journal_mode` | `WAL` | lectures concurrentes pendant une écriture ; indispensable avec plusieurs connexions. |
| `foreign_keys` | `ON` | le DDL déclare des FK (`media_board_items`, `playback_jobs`) ; SQLite les ignore sinon. |
| `busy_timeout` | `5000` ms | attente sur verrou d'écriture au lieu d'un `SQLITE_BUSY` immédiat. |
| `synchronous` | `NORMAL` | bon compromis durabilité/perf en WAL (le mode `FULL` coûte un fsync par commit). |

## 2. Ouverture

```rust
use lc_db::{Db, DbConfig};

// Chemin fichier : le dossier parent est créé, les migrations sont jouées.
let db = Db::open("data/livechat.db", &DbConfig::default()).await?;

// URL `file:` Prisma (reprise du legacy) ou URL `sqlite:` sqlx.
let db = Db::connect("file:./data/livechat.db", &DbConfig::default()).await?;
let db = Db::connect("sqlite://data/livechat.db", &DbConfig::default()).await?;

// Base mémoire (pool forcé à max_connections = 1), pour tests/démos.
let db = Db::open_in_memory(&DbConfig::default()).await?;

// Lecture seule stricte : `?mode=ro` force `read_only(true)`, ne crée ni
// fichier ni dossier, ne joue aucune migration et n'active pas WAL.
let db_ro = Db::connect("file:./data/livechat.db?mode=ro", &DbConfig::default()).await?;
```

À l'ouverture : pragmas → migrations → vérification de `schema_version`.
Une base d'une autre version renvoie `DbError::SchemaVersion { expected, found }`
plutôt que de continuer silencieusement.

`?mode=ro` (URL `file:` ou `sqlite:`) est un mode **strict** : la configuration
effective force `create_if_missing = false`, `create_parent_dirs = false` et
`run_migrations = false`, et les pragmas d'écriture (`journal_mode`,
`synchronous`) ne sont pas appliqués — seuls `foreign_keys` et `busy_timeout`
le sont. Ouvrir un fichier absent échoue au lieu de le créer ; toute écriture
renvoie l'erreur SQLite `readonly` (pas de bascule WAL).

`DbConfig` (defaut) :

| Champ | Défaut | Rôle |
| --- | --- | --- |
| `max_connections` | 5 | taille du pool (contrat Phase 2) |
| `min_connections` | 1 | connexion chaude |
| `acquire_timeout` | 30 s | attente d'une connexion libre |
| `busy_timeout` | 5 s | `PRAGMA busy_timeout` |
| `create_if_missing` | `true` | crée le fichier |
| `create_parent_dirs` | `true` | crée le dossier parent |
| `run_migrations` | `true` | joue les migrations |
| `check_schema_version` | `true` | vérifie `3.0.0` |

Les pragmas de contrat ne sont **pas** configurables : ce sont des invariants de
la crate, pas des options d'environnement.

### Pool et concurrence

`Db` est `Clone` : le clone partage le même pool (compteur de références
`sqlx`). Le serveur doit créer **un** `Db` au démarrage et le cloner dans ses
tasks (Axum state, workers Discord, scheduler, overlay).

SQLite reste mono-écrivain : les écritures se sérialisent au niveau du fichier.
Le `busy_timeout` de 5 s absorbe les pics ; les transactions de la Phase 3
doivent rester courtes (pas d'I/O réseau dans une transaction). Pour un débit
d'écriture élevé, `max_connections` > 1 reste utile (lectures parallèles) mais
ne parallélise pas les écritures.

## 3. API publique

```rust
pub const SCHEMA_VERSION: &str = "3.0.0";
pub struct Db;            // open / open_in_memory / connect / migrate
                          // ensure_schema_version / get_metadata / set_metadata
                          // pool / config / close
pub struct DbConfig;
pub enum DbError;         // Sqlx, Migrate, Io, Domain, SchemaVersion, Corrupt, InvalidUrl
pub type DbResult<T> = Result<T, DbError>;

// Réessaie une opération si SQLite répond SQLITE_BUSY/locked (voir §3.1).
pub async fn with_busy_retry<T, F, Fut>(operation: F) -> DbResult<T>;
```

`DbError::is_busy()` reconnaît les erreurs de verrouillage SQLite
(`database is locked`, `database is busy`, …) ; les repositories `claim_*` et
`claim_next` l'utilisent via `with_busy_retry` (4 tentatives, backoff
25/50/100 ms) au lieu de faire échouer le scheduler.

Accesseurs de repositories (chacun `Clone`) :

| Méthode `Db` | Repository | Opérations |
| --- | --- | --- |
| `metadata()` | `MetadataRepository` | `get(key)`, `set(key, value)` (upsert) |
| `media_assets()` | `MediaAssetsRepository` | `insert`, `get`, `get_by_source_hash`, `update_status`, `touch_last_accessed`, `list_expired`, `purge_expired`, `list_by_status` |
| `media_board()` | `MediaBoardRepository` | `create`, `get`, `get_by_guild_and_asset`, `list_by_guild`, `update(title, message)`, `delete` |
| `playback_jobs()` | `PlaybackJobsRepository` | `create`, `get`, `claim_next`, `set_status`, `finish`, `list_pending_by_guild`, `count_by_status`, `purge_done_before`, `set_playback_state`, `requeue` |
| `guilds()` | `GuildsRepository` | `upsert`, `get`, `set_busy_until`, `update_settings`, `list` |
| `overlay_clients()` | `OverlayClientsRepository` | `create`, `get`, `get_by_token_hash`, `list_by_guild`, `touch_last_seen`, `revoke`, `update_mode` |
| `ingest_clients()` | `IngestClientsRepository` | `create`, `get`, `get_by_token_hash`, `list`, `list_by_guild`, `touch_last_seen`, `revoke` |
| `pairing_codes()` | `PairingCodesRepository` | `create`, `get`, `consume`, `revoke`, `delete_expired`, `list`, `list_by_guild` |

Conventions de signatures :

- identifiants UUID : passés **par valeur** (`Copy`) ; `GuildId`/`PairingCode`
  (chaînes) : passés **par référence** ;
- `DateTime<Utc>` : par valeur (`Copy`) ;
- mutations « ciblées » : retournent `bool` (`false` = ligne absente ou
  condition d'état non remplie) ;
- suppressions en masse : retournent `u64` (nombre de lignes), sauf
  `purge_expired` qui retourne les **lignes supprimées** (`Vec<MediaAsset>`)
  pour que l'appelant supprime les fichiers correspondants ;
- `playback_jobs().set_status(job, Playing)` sur une guilde qui a déjà un
  `PLAYING` renvoie `DbError::Domain(DomainError::InvalidState)` (index unique
  partiel) au lieu d'une erreur SQLite brute ;
- violations de contraintes : `UNIQUE` conjugué métier (`media_board_items`,
  `overlay_clients`, `ingest_clients`, `pairing_codes`) → `DomainError::Conflict`
  enveloppé dans `DbError::Domain` ; `FOREIGN KEY`/`CHECK` brutes accessibles via
  `DbError::is_foreign_key_violation()` / `is_check_violation()` ;
- lignes illisibles (date hors plage, enum inconnue) → `DbError::Corrupt`,
  jamais de panic ni de valeur silencieusement fausse.

### Exemple : démarrage serveur

```rust
use lc_db::{Db, DbConfig};

let db = Db::open(&std::env::var("LIVECHAT_DB_PATH")?, &DbConfig::default()).await?;
let state = AppState { db: db.clone(), /* … */ };
```

### Exemple : boucle du scheduler (Phase 3)

```rust
let now = chrono::Utc::now();
if let Some(job) = db.playback_jobs().claim_next(now).await? {
    // Le job est atomiquement passé PLAYING et n'appartient qu'à ce worker.
    dispatch_to_overlay(&job).await?;
    db.playback_jobs().finish(job.id, JobStatus::Done, chrono::Utc::now()).await?;
}
```

`claim_next` est **la** primitive de concurrence : sélection et
`PENDING → PLAYING` dans une seule instruction `UPDATE … RETURNING`, avec les
critères du scheduler legacy :

1. `status = 'PENDING'`, `scheduled_at <= now`, `finished_at IS NULL` ;
2. un job de reprise (`resumes_after_job_id`) n'est éligible que si son parent
   n'est plus `PENDING`/`PLAYING` (ou n'existe plus) ;
3. aucun autre job `PLAYING` dans la même guilde (au plus un job en lecture par
   guilde) ;
4. `guilds.busy_until` absents ou `<= now` (verrou anti-collision) ;
5. ordre `priority DESC`, `submission_date ASC`, `id ASC`.

Deux workers concurrents ne peuvent pas obtenir le même job ; le perdant reçoit
`None` (ou un autre job éligible). Si SQLite répond `SQLITE_BUSY`/`locked`
(écrivain externe, checkpoint, autre instance), `claim_next` **réessaie**
automatiquement (4 tentatives, 25/50/100 ms) puis renvoie une erreur bornée
`DbError` plutôt que de tuer la boucle du scheduler.

L'invariant « au plus un `PLAYING` par guilde » est en outre garanti au niveau
du schéma par l'index unique partiel `idx_playback_jobs_single_playing`
(migration `0002`) : un `UPDATE` direct vers `PLAYING` sur une guilde déjà en
lecture échoue, et `set_status` le traduit en `DomainError::InvalidState`.
Un job `PLAYING` orphelin (crash du serveur) reste en base au redémarrage ; le
watchdog du moteur le libère (`DONE`) après la grâce de 10 s, il n'y a **pas**
de recovery de masse au boot.

### Exemple : appairage atomique

```rust
let code = PairingCode::parse(user_input)?; // trim + MAJUSCULES
if let Some(pairing) = db.pairing_codes().consume(&code, Utc::now()).await? {
    // code valide, non consommé, non expiré : un seul gagnant possible.
    create_overlay_client(pairing).await?;
}
```

`consume` pose `used_at` **et** vérifie `used_at IS NULL AND (expires_at IS
NULL OR expires_at > now)` dans la même instruction : pas de course possible,
même avec plusieurs instances du serveur.

### Tokens d'overlay/ingestion

`token_hash` est le SHA-256 du token (jamais le token en clair). `get_by_token_hash`
ne retourne que les clients **non révoqués** ; `revoke` est idempotent
(`false` la seconde fois) et le hash reste réservé (`UNIQUE`).

### Guildes

Les lignes `guilds` sont créées à la volée (`upsert`, `set_busy_until`,
`update_settings` font un upsert partiel), comme le `prisma.guild.upsert` du
legacy. `upsert` remplace la ligne complète ; `set_busy_until` et
`update_settings` ne touchent qu'à leurs colonnes et **créent** la ligne si
besoin.

## 4. Purges (parité legacy)

| Méthode | Requête | Legacy |
| --- | --- | --- |
| `media_assets().purge_expired(now)` | `DELETE … WHERE expires_at <= now AND NOT EXISTS (item de board) AND NOT EXISTS (job PENDING/PLAYING) RETURNING *` | purge du cache média |
| `playback_jobs().purge_done_before(cutoff)` | `DELETE … WHERE status IN ('DONE','FAILED') AND finished_at IS NOT NULL AND finished_at <= cutoff` | `purgeOldPlaybackJobs` |
| `pairing_codes().delete_expired(now)` | `DELETE … WHERE used_at IS NOT NULL OR (expires_at IS NOT NULL AND expires_at <= now)` | `purgeStalePairingCodes` |

La purge des médias est **protectrice** : un asset expiré encore épinglé au
board média ou référencé par un job `PENDING`/`PLAYING` n'est jamais supprimé
(aucune cascade silencieuse d'items de board). Elle retourne les lignes
effectivement supprimées (`Vec<MediaAsset>`) : l'appelant (worker de purge) y
lit `storage_path` pour effacer les fichiers, puis les FK du contrat
s'appliquent aux références restantes (`media_board_items` en cascade,
`playback_jobs.media_asset_id` à `NULL` pour les jobs terminés).

## 5. Tests

`crates/lc-db/tests/` :

- `migration.rs` — migration fraîche + idempotence (0001..0005), 11 tables,
  index unique partiel `idx_playback_jobs_single_playing`, pragmas réels
  (`journal_mode=wal`, `foreign_keys=1`, `busy_timeout=5000`, `synchronous=1`),
  version inattendue/absente, FK refusée, CHECK refusés, création du dossier
  parent, URL `file:`, base mémoire ;
- `invariants.rs` — rejouer `0002` sur une base contenant deux `PLAYING` de la
  même guilde normalise les doublons (le plus récent est conservé, les autres
  repassent `PENDING`), recrée l'index, puis `set_status(Playing)` échoue ;
- `media_assets.rs` — CRUD, `source_hash` unique, listes/limites, touche, purge
  protectrice (board épinglé et jobs actifs conservés, lignes supprimées
  retournées), aller-retour ms (0, 2023, 2100), date hors plage → `Corrupt` ;
- `media_board.rs` — CRUD, unicité `(guild_id, media_asset_id)` → `Conflict`, FK,
  migration `0005` (base arrêtée à `0004`, renommage, relecture via
  `Db::media_board()`) ;
- `playback_jobs.rs` — ordre de la file, `scheduled_at`, un seul `PLAYING` par
  guilde, priorité/soumission, `busy_until`, reprise après parent, claims
  concurrents (2 tasks tokio), retry `SQLITE_BUSY` de `claim_next` (verrou
  externe relâché), `set_status(Playing)` refusé par l'index unique partiel,
  `finish`/purge/`requeue`/état de lecture ;
- `guilds.rs` — upsert complet, mises à jour partielles qui créent la ligne ;
- `clients.rs` — overlay + ingest : CRUD, auth par hash, révocation idempotente,
  unicité du hash ;
- `pairing_codes.rs` — normalisation MAJUSCULES, `consume` unique, consommation
  concurrente (un seul gagnant), expiration, codes permanents, révocation,
  purge ;
- `read_only.rs` — `Db::connect("file:…?mode=ro")` : lecture OK, config
  effective `create_if_missing`/`create_parent_dirs`/`run_migrations = false`,
  `journal_mode` inchangé (pas de bascule WAL), écriture refusée (`readonly`),
  fichier absent non créé.

Chaque test ouvre **sa** base dans un `tempfile::TempDir` (fichier réel, WAL et
migrations comprises) et n'utilise pas de base partagée.

## 6. Écarts et points ouverts

### Écarts assumés (à valider avec le Lead)

1. **DDL** : aucun écart — la concaténation de `migrations/0001_init.sql` et
   `migrations/0002_playback_single_playing.sql` est la copie exacte de
   `schema-v3.sql` (mêmes tables, colonnes, CHECK, index, `STRICT`) ; le test
   `schema_contract.rs` compare la concaténation complète.
2. **Nommage côté domaine** : l'identifiant typé `PairingCode` (newtype de
   chaîne) entre en collision de nom avec l'entité de la table
   `pairing_codes` ; l'entité est exposée sous `PairingCodeEntry` (champ
   `code: PairingCode`). Pas d'impact SQL.
3. **Dates du domaine** : la consigne de board mentionnait `i64` ms dans les
   entités ; la consigne directe exige `chrono::DateTime<Utc>` — c'est la
   seconde qui est appliquée, la conversion ms se fait dans `lc-db`.
4. **`set_busy_until`/`update_settings`** font un upsert (et non un `UPDATE`)
   pour respecter le legacy où `guilds` est créée par `prisma.guild.upsert`.
5. **`Cargo.lock`** : le premier build a ajouté sqlx/chrono/uuid/tokio/thiserror
   (absents du lock initial). Fichier racine hors scope `lc-db`, modification
   inévitable et attendue.

### Corrections issues de l'audit A5 (P1-08, task-41)

1. **Invariant « un seul PLAYING par guilde »** : désormais garanti par la
   migration `0002` (index unique partiel + normalisation des doublons
   historiques) et par la garde applicative `set_status` ; `claim_next` reste la
   primitive de concurrence. Un `PLAYING` orphelin au crash reste en base et est
   libéré par le watchdog, sans recovery de masse.
2. **`SQLITE_BUSY`** : `claim_next` et les claims de `lc-playback` réessaient via
   `with_busy_retry` (borné) au lieu de remonter une erreur qui tuait le
   scheduler.
3. **Purge média protectrice** : `purge_expired` ne supprime plus les assets
   épinglés au board ou référencés par un job actif, et retourne les lignes
   supprimées pour que l'appelant efface les fichiers.
4. **`mode=ro`** : `?mode=ro` est un mode lecture seule strict (pas de création,
   pas de migration, pas de WAL).
5. **Migrateur** : récupération explicite d'un `<to>.old` orphelin au démarrage
   (voir `docs/rust/migrator.md`).

### Points ouverts Phase 3/4

1. **`busy_until`** : la sémantique (durée = `max(remaining_ms, 5000) + 250`,
   protocole §5.3) relève du service `playback.state` ; `lc-db` ne fait que
   stocker. À implanter en Phase 3, avec la même valeur que le legacy.
2. **Checkpoint WAL / sauvegarde** : prévoir une tâche d'administration
   (`PRAGMA wal_checkpoint(TRUNCATE)`, `VACUUM INTO` pour les backups) et la
   rotation éventuelle des `playback_jobs` terminés (`purge_done_before`).
3. **Import Prisma (Phase 4)** : mapper les dates `DateTime` Prisma en ms epoch
   (voir `docs/rust/prisma-dates.txt`), les `Int` restent `i64`, les colonnes
   `Boolean` en 0/1 ; importer dans l'ordre `media_assets` → `media_board_items`
   → `playback_jobs`/`guilds` → clients/pairing pour satisfaire les FK.
4. **Génération des codes d'appairage** (alphabet `ABCDEFGHJKLMNPQRSTUVWXYZ23456789`,
   6 caractères, TTL) : service (`lc-discord`), pas `lc-db` ; `pairing_codes` ne
   valide que le format (MAJUSCULES alphanumériques, ≤ 32).
5. **Compteurs/statistiques admin** : `count_by_status` couvre le besoin
   immédiat ; si l'admin Phase 3 demande des agrégats par guilde/période,
   ajouter des index ou des requêtes dédiées (le contrat de schéma fige les
   index existants).
6. **Guildes sans FK** : le contrat ne déclare pas de FK `guild_id → guilds.id`
   (comme le legacy) ; ne pas s'appuyer sur une erreur FK pour détecter une
   guilde inconnue, utiliser `GuildsRepository::get`.
7. **`max_connections`** : 5 est le contrat Phase 2 ; mesurer en Phase 3 le
   débit réel (scheduler + overlays + Discord) avant d'augmenter, et garder les
   transactions courtes.
