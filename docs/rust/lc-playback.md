# `lc-playback` — ordonnanceur de lecture

> Résultat de la tâche **task-11 « Phase 4a — lc-playback »**.
> Extraction du service legacy `server/src/services/playbackScheduler.ts` (979 lignes)
> dans le crate Rust `crates/lc-playback`, avec horloge et transport injectés.

## 1. Objectif et périmètre

`lc-playback` possède la **machine à états** de lecture d'une guilde : file de jobs
FIFO, dispatch vers les overlays, suivi de progression, arrêts ciblés/globaux,
watchdog des durées et purge de rétention. Un média demandé pendant qu'un autre joue
attend son tour : aucun job n'interrompt ni ne suspend le job en cours.

Il ne possède **pas** :

- le transport WebSocket ni l'authentification (Phase 4b, `apps/lc-server`) ;
- la signature des URLs média (le hub par-client signe l'URL canonique produite ici) ;
- la création des jobs depuis Discord/mèmes (services appelants) ;
- la base de données (lc-db fournit les repositories).

Le crate est testable sans réseau ni temporisation réelle : `Clock` et `PlaybackSink`
sont des paramètres génériques, et les tests utilisent `FixedClock` + `FakeSink`.

## 2. Écarts assumés par rapport à `lc-db`

`lc-db` ne fournit pas encore certaines primitives dont le moteur a besoin :

| Primitive manquante | Impact |
|---|---|
| lister les jobs `PLAYING` d'une guilde dans l'ordre de démarrage | watchdog / anomalie multi-`PLAYING` |
| libération groupée des jobs `PLAYING` d'une guilde (arrêt global) | `stop.request` sans `jobId` |
| écriture de `duration_sec` / `execution_date` au dispatch | durée effective calculée au dispatch |

Décision : ces opérations sont implémentées dans le **module privé** `src/store.rs`
(`PlaybackStore`), en SQL `sqlx` direct sur la connexion exposée par
`Db::pool()`. Aucune autre crate n'est modifiée. Ces requêtes devront être
**promues dans `lc-db`** (repositories) à la première occasion, avec tests dédiés ;
le module est isolé pour que la promotion soit mécanique.

Le détail des requêtes réutilise les invariants de `claim_next`
(`crates/lc-db/src/repos/playback_jobs.rs`) : les critères d'éligibilité
(statut, `scheduled_at`, absence d'autre `PLAYING`, `busy_until`) et l'ordre FIFO
(`submission_date ASC, id ASC`) sont identiques.

## 3. Câblage type

```rust
use std::sync::Arc;
use lc_db::DbConfig;
use lc_playback::{PlaybackConfig, PlaybackEngine, PlaybackSink};

let db = lc_db::Db::open("livechat.sqlite", &DbConfig::default()).await?;
let sink = Arc::new(HubSink::new(hub.clone()));  // implémente PlaybackSink
let engine = PlaybackEngine::new(
    db,
    PlaybackConfig::default().with_api_base_url(env.api_url.clone()),
    sink,
    lc_domain::SystemClock,
);
```

Le serveur garde `Arc<PlaybackEngine<..>>`, lance `engine.clone().run_until_shutdown()`
dans une tâche et appelle les méthodes de la section 4 depuis les handlers WS,
les workers Discord et l'admin.

`PlaybackEngine` est `Send + Sync` (mutex interne) : les méthodes asynchrones
prennent `&self` et peuvent être appelées depuis plusieurs tâches.

## 4. API publique

### 4.1 `PlaybackConfig`

| champ | défaut | rôle |
|---|---|---|
| `tick_interval` | 500 ms | période de la boucle de fond (watchdogs, wake, file) |
| `default_duration_sec` | 30 | durée d'un job texte sans `duration_sec` |
| `retention` | 24 h | rétention des jobs terminés pour `purge` |
| `api_base_url` | `http://127.0.0.1:3000` | base des URLs média canoniques |
| `state_persist_min_interval` | 1 s | throttle d'écriture de `playback.state` |
| `max_guild_iterations` | 25 | borne anti-boucle d'une passe de guilde |
| `lock_padding_ms` | 250 | marge du verrou `busy_until` |
| `stale_release_grace_ms` | 10 000 | grâce avant libération d'un job stale |
| `min_busy_lock_ms` | 5 000 | plancher du verrou après un `playback.state` |
| `state_snapshot_max_age_ms` | 15 000 | fraîcheur maximale du keep-alive `playback.state` |

Builders : `with_api_base_url`, `with_tick_interval`, `with_retention`,
`with_default_duration_sec`.

### 4.2 `PlaybackSink`

```rust
pub trait PlaybackSink: Send + Sync {
    fn on_play(&self, guild_id: &GuildId, payload: PlayPayload)
        -> impl Future<Output = usize> + Send; // nb de clients ciblés
    fn on_stop(&self, guild_id: &GuildId, payload: StopPayload)
        -> impl Future<Output = ()> + Send;
}
```

- `on_play` renvoie le nombre de sockets réellement ciblés. `0` reproduit le
  comportement legacy « room vide » : le job passe `FAILED` et n'est pas rejoué.
- `on_stop` est un broadcast room (tous les overlays de la guilde).
- Le hub WS de la Phase 4b implémente `on_play` en **unicast par socket** (URL
  média signée avec le `clientId` du socket) et `on_stop` en broadcast.

### 4.3 Événements et résultats

- `PlaybackStateEvent { guild_id, job_id: Option<PlaybackJobId>, state, remaining_ms }`
  (constructeur `new`).
- `PlaybackEndedEvent { guild_id, job_id, reason }` (`PlaybackEndedReason`).
- `PlaybackStateOutcome::{Persisted, Throttled, Ignored}`.
- `ReleaseOutcome { released, next_started }` (retour de `handle_playback_ended`).
- `StopOutcome { stopped: Vec<String>, payload: StopPayload }`.
- `TickReport { processed_guilds, active_guilds }`.
- `PurgeReport { playback_jobs, pairing_codes }`.

### 4.4 Méthodes du moteur

| méthode | sémantique |
|---|---|
| `bootstrap()` | enregistre les guildes ayant des jobs `PENDING`/`PLAYING` ; retourne le nombre |
| `tick()` | une passe : bootstrap + traitement de chaque guilde active |
| `enqueue(&PlaybackJob)` | crée le job en base puis traite sa guilde immédiatement |
| `notify(&GuildId)` | demande le traitement d'une guilde au prochain tick |
| `handle_playback_state(event)` | clamp `remaining_ms` à `MAX_PLAYBACK_REMAINING_MS`, met à jour `busy_until` (`now + max(remaining, 5 s) + 250 ms`), persiste le keep-alive (`last_playback_state_at`), throttle 1 s par job |
| `handle_playback_ended(event)` | termine le job (`DONE`), libère le verrou, démarre le suivant ; idempotent |
| `handle_error(&GuildId, PlaybackJobId)` | marque `FAILED`, libère le verrou, passe au suivant |
| `stop(&GuildId, Option<PlaybackJobId>, StopRequestSource)` | arrêt `Manual` ciblé ou global ; renvoie le payload `stop` émis |
| `stop_with_reason(&GuildId, Option<PlaybackJobId>, StopReason)` | idem avec raison explicite (`Revoked`, `Purge`, …) |
| `current_job(&GuildId)` | `Option<CurrentJobPayload>` pour `welcome` (reprise après reconnexion) |
| `purge(now)` | supprime les jobs terminés au-delà de `retention` + codes de pairing expirés |
| `run_until_shutdown()` | boucle `tick` toutes les `tick_interval` jusqu'à `request_shutdown` |
| `run(shutdown_future)` | variante pilotée par un future de shutdown |
| `request_shutdown()` / `is_shutting_down()` | arrêt coopératif de la boucle |

## 5. Invariants

1. **Un seul job `PLAYING` par guilde.** Garanti par les claims SQL atomiques
   (même critère `NOT EXISTS … status = 'PLAYING'` que `claim_next`) et vérifié
   par le test `dispatch_texte_puis_watchdog_passe_au_suivant`.
2. **Ordre de sélection** : FIFO strict, `submission_date ASC, id ASC`. Aucune
   priorité n'existe plus dans le schéma.
3. **Aucune interruption.** Un job en attente (média du board, `/ingest`, commande
   Discord, `say`) ne stoppe jamais le job en cours : il attend la fin du job actif
   (`playback.ended`, watchdog ou arrêt manuel). Aucun `stop { reason: "preempt" }`
   n'est émis.
4. **Durée effective** : `job.duration_sec` (ou durée du média, ou défaut 30 s),
   bornée par `guild.max_media_time` si définie, moins l'offset de départ, minimum 1 s.
   Le payload `play.durationSec` est toujours la durée **effective**.
5. **Verrou de guilde** : `busy_until = started_at + durée_effective*1000 + 250 ms`
   au dispatch ; `playback.state` le repousse à `now + max(remaining, 5 s) + 250 ms`.
   Les claims refusent une guilde dont `busy_until > now`.
6. **Idempotence** : un `playback.ended` pour un job qui n'est plus `PLAYING` ne
   fait rien (`ReleaseOutcome { released: false, .. }`) ; un second arrêt ne
   réémet pas de `stop` vide.
7. **Watchdog** : un job `PLAYING` dont `started_at + durée*1000 + 250 ms + 10 s`
   est dépassé est libéré (`DONE`) et le suivant démarre ; couvre un crash d'overlay
   ou un `ended` perdu.
8. **`current_job`** : au plus un `PLAYING` (anomalie → le plus récent par
   `started_at` est retenu et un `warn` est émis) ; `remaining_ms` =
   `started_at + duration_sec*1000 - now`.

**Keep-alive de file locale.** Un overlay multi-guilde qui reçoit un `play` pendant
qu'une autre guilde occupe l'écran garde ce job en file locale et envoie toutes les
3 s un `playback.state` `paused` pour lui. Tant que le dernier checkpoint persisté a
moins de `PLAYBACK_STATE_MAX_AGE_MS` (15 s), le watchdog ne libère pas le job ; sans
keep-alive, la libération survient au plus 15 s après le dernier.

## 6. Chronologie

1. **Dispatch** (`dispatch_pending_job`) : média requis (`READY` sinon `FAILED`),
   timings, claim → `PLAYING`, `busy_until`, `sink.on_play`, puis watchdog.
2. **Progression** (`playback.state`) : clamp, `busy_until`, keep-alive throttlé 1 s/job.
3. **Fin** (`playback.ended`) : `DONE` + verrou libéré + job suivant.
4. **Arrêt** (`stop.request`) : libération ciblée ou totale, `stop { reason:
   "manual", jobId: … }` broadcast (`jobId` = cible, ou unique job libéré, sinon `null`).
5. **Watchdog / stale** : libération des jobs expirés (pause prolongée non couverte),
   démarrage du suivant.
6. **Purge** : à appeler périodiquement (tick serveur) : jobs terminés > rétention,
   codes de pairing expirés.

Équivalences legacy les plus sensibles (fichier de référence
`server/src/services/playbackScheduler.ts`) :

| legacy | Rust |
|---|---|
| `PLAYBACK_LOCK_PADDING_MS` (l.7) | `PLAYBACK_LOCK_PADDING_MS` / `config.lock_padding_ms` |
| `PLAYBACK_STALE_RELEASE_GRACE_MS` (l.9) | `PLAYBACK_STALE_RELEASE_GRACE_MS` / `stale_release_grace_ms` |
| `MIN_ACTIVE_PLAYBACK_BUSY_LOCK_MS` (l.11) | `MIN_ACTIVE_PLAYBACK_BUSY_LOCK_MS` / `min_busy_lock_ms` |
| `PLAYBACK_STATE_MAX_AGE_MS` (l.13) | `PLAYBACK_STATE_MAX_AGE_MS` / `state_snapshot_max_age_ms` |
| `GUILD_RUN_MAX_ITERATIONS` (l.15) | `GUILD_RUN_MAX_ITERATIONS` / `max_guild_iterations` |
| `estimateRemainingMs` (l.999) | `estimated_remaining_ms` |
| `releaseStalePlayingJob` (l.601) | branche stale de `run_guild` |
| `recomputeRootPendingExecutionDates` (l.1055) | **non porté** (voir §8) |

## 7. Intégration hub WebSocket (Phase 4b)

```text
trame JSON -> Envelope<ClientMessage>
  playback.state   -> engine.handle_playback_state(PlaybackStateEvent::new(...))
  playback.ended   -> engine.handle_playback_ended(PlaybackEndedEvent::new(...))
  error            -> log + engine.handle_error(guild, job_id)
  stop.request     -> engine.stop(guild, payload.job_id, payload.source)
  media.trigger    -> createPlaybackJob(source = JobSource::Board)
                      puis engine.enqueue(&job) : le média attend la fin du job
                      en cours, sans interruption
  peers.request    -> rooms peers (hors crate)
hello (avant)      -> auth + welcome ; currentJob = engine.current_job(&guild)
```

Le serveur journalise chaque création de job (`job de lecture créé`, avec `source`,
`source_label` pour les ingestions et l'identifiant du média), chaque dispatch
(`job de lecture dispatché`, avec `duration_sec` et `start_offset_sec`) et chaque fin
(`job de lecture terminé`, avec `source` et `reason`) en niveau INFO : la source d'un
média qui attend ou qui joue est donc identifiable dans les logs du serveur.

Implémentation du sink côté hub :

- `on_play` : pour chaque socket de la room guilde, construire l'URL signée à
  partir de `payload.media.url` (canonique) + `clientId` + `exp` + `sig`, puis
  envoyer le payload sur ce socket ; retourner le nombre de sockets ciblés.
- `on_stop` : `room.emit(stop, payload)`.

Boucle de fond : `tokio::spawn(engine.clone().run_until_shutdown())` ; purge sur un
interval séparé (1 h par exemple) : `engine.purge(SystemClock.now()).await`.

Limitation transactionnelle : `createPlaybackJob` + `enqueue` ne sont pas atomiques
(le job est créé PENDING puis dispatché). Un crash entre les deux est récupéré au
`bootstrap`/tick suivant.

## 8. Limites et écarts connus

- **URL média non signée** : le payload produit l'URL canonique
  `{api_base_url}/overlay/media/{assetId}` (+ `?startOffsetSec=N#t=N` si le job
  démarre à un offset, p. ex. URL YouTube).
  Le hub signe par client lors de l'envoi (TTL géré en Phase 4b).
- **`tweetCard` absent** du payload émis : le crate ne décode pas le rich payload
  legacy (`__LC_OVERLAY_RICH__::`). Les durées/offsets effectifs sont déjà stockés
  en base par les services appelants ; les cartes tweet devront être injectées par
  le service de création de job (ou une évolution de `lc-core`).
- **Pas de `recomputeRootPendingExecutionDates`** : le legacy réordonnait les
  `execution_date` des jobs en attente après chaque dispatch. Ici les jobs sont
  réclamés à la demande (`scheduled_at <= now`, ordre FIFO `submission_date`) ; l'ordre
  d'exécution est donc équivalent, sans réécriture des dates estimées. Si l'admin
  doit afficher des dates d'exécution prévisionnelles, cette logique est à ajouter
  (hors périmètre Phase 4a).
- **Granularité de la boucle** : watchdogs et wake sont pilotés par `tick`
  (500 ms) ; pas de timers par job. Une précision de ±500 ms doit être acceptable.
- **`playback.ended` tardif** après une libération stale : traité en idempotence
  (aucun effet).
- **Multi-`PLAYING` (anomalie/crash)** : le moteur libère les plus anciens `DONE`
  et ne garde que le plus récent, en émettant un `warn`.
- **Pas d'`unwrap`/`expect`/`panic`** hors tests ; `#![forbid(unsafe_code)]`.

## 9. Tests

`cargo test -p lc-playback` : scénarios moteur sur SQLite en mémoire
+ tests d'intégration des primitives de claim.

| test | couverture |
|---|---|
| `tick_sans_job_ne_fait_rien` | passe vide |
| `dispatch_texte_puis_watchdog_passe_au_suivant` | payload texte, `busy_until`, un seul `PLAYING`, watchdog |
| `ordre_de_soumission` | FIFO : le job soumis en premier joue en premier |
| `un_media_du_board_attend_la_fin_du_job_en_cours` | média du board sans `stop`, dispatch après `playback.ended` |
| `stop_cible_puis_global` | `stop.request` ciblé puis global, `jobId` du payload |
| `stop_avec_raison_revoked` | révocation admin |
| `busy_until_futur_bloque_puis_expire` | verrou de guilde |
| `aucun_overlay_connecte_fait_echouer_le_job` | `on_play → 0` ⇒ `FAILED` |
| `purge_respecte_la_retention` | purge 24 h |
| `current_job_reprend_la_position_et_est_throttle` | `welcome.currentJob`, throttle 1 s, estime `remaining_ms` |
| `job_texte_sans_duree_utilise_la_duree_par_defaut` | défaut 30 s |
| `watchdog_recupere_un_job_stale_apres_crash` | récupération après crash |
| `watchdog_ne_libere_pas_un_job_confirme_recemment` | keep-alive `playback.state` |
| `watchdog_libere_un_job_sans_keepalive_depuis_quinze_secondes` | expiration du keep-alive |
| `max_media_time_borne_la_duree_et_le_verrou` | borne `max_media_time` |
| `claim_next_respecte_l_ordre_de_soumission` | `lc-db::claim_next` FIFO |
| `busy_until_bloque_le_claim_puis_le_libere` | `lc-db` + `guilds.busy_until` |
| `deux_jobs_meme_guilde_jouent_l_un_apres_l_autre` | un seul `PLAYING`, enchaînement |

Vérifications : `cargo fmt -p lc-playback -- --check`,
`cargo clippy -p lc-playback --all-targets -- -D warnings`,
`cargo test -p lc-playback`.

## 10. Points à confirmer / suite

1. Promotion des requêtes de `src/store.rs` dans `lc-db` (même sémantique, avec
   tests de repository) — réduit le SQL embarqué et centralise les invariants.
2. Injection de `tweetCard` : décider si le décodage du rich payload revient à
   `lc-core` (création de job) ou au hub Phase 4b.
3. La purge de rétention côté serveur doit être planifiée (interval) et exposée
   dans `/health`/métriques.
4. Recette de parité avec le legacy sur les scénarios de la table §9, à brancher
   dans `xtask parity`.
