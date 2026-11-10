# Hub WebSocket v2 — `apps/lc-server/src/ws`

> Phase 4b — implémentation du hub `GET /ws` du serveur Rust.
> Spécification de référence : [`docs/rust/protocol-v2.md`](protocol-v2.md) (SSOT gelée rev. 2).
> Owner : `protocole-v2` (task-17). Périmètre d'écriture : `apps/lc-server/src/ws/**` et ce document.
> Dernière mise à jour : 2026-11-09.

## 1. Périmètre

Ce module remplace le serveur socket.io 1.2.0 par un WebSocket JSON natif :

- handshake `hello` (jeton d'appairage, 5 s) puis `welcome` ;
- enveloppe `{v, type, id, ts, payload}` du crate `lc-protocol` ;
- authentification SHA-256 du jeton contre `overlay_clients.token_hash` (client non révoqué) ;
- versions : `MIN_OVERLAY_VERSION` / `serverVersion`, close `4403` avec `{"minVersion":"x.y.z"}` ;
- limites : trames 64 Kio, 20 messages/s, heartbeat 15 s, timeout 45 s, close 4408/4409 ;
- `play` **unicast** par socket avec URL média signée HMAC liée au `clientId` ;
- `stop`, `peers`, `settings` en broadcast de room `overlay-guild-{guildId}` ;
- intégration `lc-playback` via l'adaptateur `PlaybackSink` (moteur créé au premier `hello`) ;
- API admin (`connected_count`, `close_client`, `broadcast_stop`, `broadcast_settings`) consommée
  par la lane admin (task-18) via le trait `AdminHub`.

Fichiers :

| Fichier | Rôle |
| --- | --- |
| `ws/mod.rs` | montage du routeur `/ws` (`router`, `router_with_hub`) et ré-exports |
| `ws/hub.rs` | registre des connexions par guilde, files d'envoi, moteur `lc-playback`, signature des URLs média, API admin |
| `ws/session.rs` | upgrade, handshake `hello`/`welcome`, boucle de lecture/écriture, keepalive, fermetures |
| `ws/tests.rs` | 14 tests d'intégration bout-en-bout + 5 tests unitaires |

Dépendances utilisées (déjà déclarées dans `apps/lc-server/Cargo.toml`) : `axum` (feature `ws`),
`futures-util`, `tokio`, `serde`/`serde_json`, `lc-protocol`, `lc-playback`, `lc-db`, `lc-domain`,
`sha2`/`hex` (hash des jetons), `sqlx` (une requête ciblée, cf. §11), et le module partagé
`crate::media_url` (signature HMAC — aucune duplication).

## 2. Architecture

```
overlay (Tauri) ──WS /ws──> session.rs (1 tâche par socket)
                               │  hello/auth/welcome
                               │  messages C→S
                               ▼
                    Arc<Hub> (Extension<Arc<Hub>>)
                    ├─ HubInner { connections: HashMap<Uuid, Connection>,
                    │             guilds: HashMap<GuildId, BTreeSet<Uuid>> }
                    ├─ OnceCell<Arc<HubEngine>>  (moteur lazy, 1er hello)
                    ├─ OnceCell<Db>, OnceCell<Arc<Config>>
                    └─ HubSink (Weak<Hub>) ── PlaybackSink de lc-playback
                               │
                               ▼
                    lc-playback::PlaybackEngine<HubSink, SystemClock>
                      (tick 500 ms, purge horaire, arrêt sur Shutdown)
```

- **Un `Hub` par processus** : il est construit dans `ws::router()` (ou fourni par
  `ws::router_with_hub(Arc<Hub>)`, utilisé par les tests et à utiliser par le câblage final admin).
- **Registre** : `Connection` contient `guild_id`, `client_id` (domaine), `client_uuid`
  (= `client_id.as_uuid()`), `client_id_str` (canonique, pour les URLs signées), `peer_label`,
  `session_mode` (protocole), et un `mpsc::UnboundedSender<OutFrame>`.
  `OutFrame` = `Text(String)` | `Close(u16, String)` : toutes les écritures d'un socket passent
  par la même file, ce qui garantit l'ordre (par ex. `ack` avant le `stop` déclenché par le moteur).
- **Moteur** : `ensure_runtime(&AppState)` crée `PlaybackEngine::new(db, PlaybackConfig, HubSink, SystemClock)`,
  spawne `engine.run(state.shutdown.subscribe().wait())` et une purge horaire ; db absente ⇒
  `HubError::DatabaseUnavailable` ⇒ close 1011. Aucun changement d'`AppState` requis.
- **Rooms** : le hub n'utilise pas les rooms axum ; il indexe les sockets par `GuildId` et diffuse
  lui-même (déduplication et comptage faciles, pas de fuite de connexion).

## 3. Cycle de vie d'une connexion

1. **Upgrade** `GET /ws` : si la requête n'est pas un upgrade WebSocket valide, axum répond par
   notre convertisseur : HTTP `426` + `{"error":"websocket_required"}`.
   Limites d'upgrade : `max_message_size = max_frame_size = 256 Kio` (garde-fou axum ; le contrat
   64 Kio est vérifié applicativement, cf. §6).
2. **Hello** : la première trame doit être un `hello` sous **5 s**. Toute autre trame, ou un timeout,
   entraîne une fermeture (`4401 auth_timeout` pour le timeout ; `1002 protocol_error` pour une
   autre trame). Aucun jeton n'est accepté en query string ni en header.
3. **Authentification** : `token` trimé (vide ⇒ `4401 missing_token`), SHA-256 hex comparé à
   `overlay_clients.token_hash` d'un client non révoqué (inconnu ⇒ `4401 invalid_token`,
   révoqué ⇒ `4402 revoked`, erreur DB ⇒ `1011 internal_error`).
4. **Contrôles** : `clientId` fourni (facultatif) doit égaler le `clientId` canonique du client
   (sinon `4401 invalid_token`) ; `appVersion` < `MIN_OVERLAY_VERSION` (semver simple, version
   illisible = obsolète) ⇒ `4403 outdated_protocol` avec raison JSON `{"minVersion":"x.y.z"}`.
5. **Welcome** : envoi de `welcome` (avec `currentJob` re-signé pour ce socket si un job `PLAYING`
   existe), puis `settings` (modes `normal`/`invite_read_only`), puis broadcast `peers` à la room.
6. **Boucle** : trames client (parsing `ClientEnvelope`, `v == 2`), file sortante, ping toutes les 15 s
   (timeout 45 s sans activité), arrêt gracieux sur `state.shutdown`.
7. **Fermeture** : close frame si possible, désenregistrement du registre, broadcast `peers`.

Codes de fermeture émis par le serveur (cf. SSOT §3) :

| Code | Raisons | Déclencheurs |
| --- | --- | --- |
| `1000` | `client_shutdown` | close client, socket fermé par le client, fin normale |
| `1001` | `server_shutdown`, `heartbeat_timeout` | arrêt du serveur, 45 s sans activité |
| `1002` | `protocol_error`, `bad_playback_state` | JSON/type/v inconnu, payload invalide, `playback.state` incohérent |
| `1003` | `unsupported_frame` | trame binaire |
| `1011` | `internal_error` | erreur DB au handshake/runtime, base indisponible |
| `4401` | `auth_timeout`, `missing_token`, `invalid_token` | hello absent/tardif, jeton vide/inconnu, `clientId` incohérent |
| `4402` | `revoked` | client révoqué (y compris fermeture admin) |
| `4403` | `outdated_protocol` | `v != 2` ou `appVersion` < minimum (raison JSON) |
| `4408` | `frame_too_large` | trame texte > 64 Kio |
| `4409` | `rate_limited` | > 20 trames C→S/s |

## 4. Messages C→S

Toutes les trames sont des enveloppes JSON `lc_protocol::ClientEnvelope`
(`{v:2, type, id, ts, payload}`). Les champs inconnus sont tolérés ; le serveur ne renvoie jamais
d'erreur pour un `id` inconnu.

| `type` | Payload | Traitement |
| --- | --- | --- |
| `hello` | `{token, appVersion, clientId?, sessionMode?}` | handshake uniquement (§3) |
| `heartbeat` | `{clientId, guildId, appVersion, sessionMode?}` | met à jour `last_seen_at` (async), log si `sessionMode` diverge du mode serveur ; aucun ack |
| `playback.state` | `{jobId, state: playing\|paused, remainingMs?: number}` | throttle 1/s (excédent ignoré) ; `remainingMs` borné à `[0, 86_400_000]` ; transmet `PlaybackStateEvent` au moteur (busy lock + watchdog) ; `ended` ⇒ `1002 bad_playback_state` |
| `playback.ended` | `{jobId, reason: completed\|manual\|error}` | transmet `PlaybackEndedEvent` ; idempotent côté moteur ; `jobId` invalide ⇒ `1002 bad_playback_state` |
| `error` | `{jobId, code, message}` | log seulement (le watchdog libère le job) |
| `stop.request` | `{jobId?, source?: user\|hotkey\|admin}` | en `invite_read_only` : ack `read_only` ; throttle 1/s (ack `rate_limited`) ; sans `jobId` ⇒ tous les jobs de la guilde ; ack `ok` **puis** `engine.stop(...)` qui diffuse `stop {reason:"manual"}` |
| `meme.trigger` | `{itemId, trigger: shortcut\|ui}` | en lecture seule : ack `read_only` ; item inconnu ⇒ ack `item_not_found` ; média absent/non `READY`/expiré ⇒ ack `media_not_ready` ; ack `ok` puis job priorité 100 (`MEME_JOB_PRIORITY`) et `engine.enqueue` (préemption `stop {reason:"preempt"}` + `play`) |
| `peers.request` | `{}` | throttle 2 s (excédent ignoré **sans** ack) ; ack `ok` puis broadcast `peers` |

Codes d'ack (`error.code`) : `read_only`, `item_not_found`, `media_not_ready`, `invalid_payload`,
`rate_limited`, `internal_error`.

Règles de complétude :
- texte du mème = `item.message` trimé, tronqué à `MAX_MEME_TEXT_CHARS` (500) ;
- `showText = message non vide` ; `authorName = session.authorName || item.createdByName` ;
  `authorImage = session.authorImage` ; `durationSec = media.duration_sec` (0 si absent) ;
- ids de jobs/d'items parsés via `lc_domain` (`PlaybackJobId::parse`, `MemeBoardItemId::parse`),
  un id invalide produit une ack `invalid_payload`, jamais une panique.

## 5. Messages S→C

| `type` | Payload | Émission |
| --- | --- | --- |
| `welcome` | `{protocolVersion:2, guildId, clientId, sessionMode, serverVersion, serverTime, config:{showTextDefault, defaultMediaTime, maxMediaTime}, currentJob?}` | unicast, juste après hello |
| `settings` | `{guildId, sessionMode, showTextDefault, defaultMediaTime, maxMediaTime}` | unicast après `welcome`, puis broadcast sur changement (API admin) |
| `peers` | `{guildId, peers: [{clientId, label}]}` | broadcast room ; dédupliqué par `clientId`, trié (label puis clientId), tronqué à 8 |
| `play` | `{jobId, media: {assetId, url, mime, kind, durationSec, isVertical, startOffsetSec}\|null, text, author, tweetCard: ...\|null, durationSec}` | **unicast par socket** (URL signée pour son `clientId`) ; `startOffsetSec` et `tweetCard` toujours présents (null autorisé) |
| `stop` | `{reason: manual\|preempt\|revoked\|purge, jobId: string\|null}` | broadcast room (moteur / admin) |
| `ack` | `{ref, ok, error?: {code, message}}` | unicast en réponse aux requêtes le demandant |
| `error` | `{code, message, jobId?}` | réservé aux erreurs asynchrones non fatales (helper présent, aucune émission automatique pour l'instant) |

Les `id` serveur sont de la forme `srv-<préfixe>-NNNNNN` (`welcome`, `settings`, `peers`, `play`,
`stop`, `ack`, `error`), `ts` en millisecondes Unix, `v = 2`.

`currentJob` : présent seulement si un job `PLAYING` existe ; c'est un `PlayPayload` (URL re-signée
pour ce socket) + `startedAt` + `remainingMs`. Au plus un job `PLAYING` par guilde (invariant du
scheduler, §15.3 de la SSOT) ; si plusieurs sont détectés, le plus récent par `startedAt` est choisi.

## 6. Limites et robustesse

| Limite | Valeur | Comportement |
| --- | --- | --- |
| trame client | 64 Kio (`MAX_FRAME_BYTES`) | close `4408` |
| plafond d'upgrade axum | 256 Kio (`max_message_size`, `max_frame_size`) | garde-fou avant lecture applicative |
| messages C→S | 20/s glissant sur 1 s, par connexion | close `4409` |
| `playback.state` | 1/s | excédent ignoré silencieusement |
| `peers.request` | 1/2 s | excédent ignoré silencieusement |
| hello | 5 s (`HELLO_TIMEOUT_MS`) | close `4401 auth_timeout` |
| heartbeat / pong | ping 15 s, timeout 45 s (`WS_HEARTBEAT_TIMEOUT_SEC`) | close `1001 heartbeat_timeout` |
| `remainingMs` | `[0, 86_400_000]` | borne avant transmission au moteur |
| texte de mème | 500 caractères | troncature |
| peers | 8 | troncature (ordre déterministe) |

Le rate limiter est un simple `VecDeque<Instant>` par connexion (fenêtre glissante de 1 s) : sans
allocation par message, et suffisant pour un client overlay. Toute trame texte est comptée, y
compris les messages rejetés ensuite ; les Pong et les trames de contrôle ne sont pas comptés.

## 7. URLs média signées

Le hub ne sert pas les médias : il **signe** seulement les URLs de `play` et de `welcome.currentJob`,
par socket, via `crate::media_url` (module partagé du serveur) :

```
{API_URL}/overlay/media/{assetId}?clientId={clientId}&exp={exp}&sig={sig}
  [&startOffsetSec=N][#t=N]
sig = HMAC-SHA256(MEDIA_URL_SECRET, "assetId|clientId|exp")   (hex minuscule)
exp = media_url::expiry_for(media.kind, media.duration_sec, now_sec)
```

- `exp` suit la règle révision 2 : `clamp(durationSec + 120, 300, 3600)` pour vidéo/audio à durée
  connue, 300 s pour une image ou une durée inconnue.
- `startOffsetSec` et le fragment `#t=N` ne sont pas signés (comportement attendu du lecteur overlay).
- Un secret vide (`MEDIA_URL_SECRET` absent) empêche la signature : `unicast_play` renvoie 0, le
  moteur marque alors le job `FAILED` (plus aucun client servi) et un warning est loggé.
- La route `GET /overlay/media/{assetId}` (lane overlay) doit vérifier `exp`/`sig` avec
  `media_url::verify` et la clé du client ; le hub ne fait aucune vérification lui-même.

## 8. API AdminHub (contrat avec la lane admin / task-18)

Le trait `AdminHub` est défini par la lane admin dans `apps/lc-server/src/http/admin/hub.rs`
(avec `#[async_trait]`), accompagné de `NoopAdminHub`. Le hub expose **exactement** ces méthodes en
inhérent sur `Hub` ; le handle cloneable recommandé est `Arc<Hub>` (type alias conseillé :
`pub type AdminHubHandle = Arc<Hub>;`). La lane admin peut implémenter son trait pour `Arc<Hub>`
et déléguer sans conversion.

| Méthode | Sync/async | Sémantique |
| --- | --- | --- |
| `connected_count(&self) -> usize` | sync | nombre total de sockets enregistrés (toutes guildes) |
| `connected_for_guild(&self, guild_id: &GuildId) -> usize` | sync | sockets enregistrés pour la guilde |
| `is_connected(&self, client_id: &Uuid) -> bool` | sync | au moins un socket de ce client (`Uuid` = `OverlayClientId::as_uuid()`) |
| `close_client(&self, client_id: &Uuid, code: u16) -> bool` | sync | envoie `OutFrame::Close` à tous les sockets du client ; `true` si au moins un ciblé. Raison mappée : `4402 → revoked`, `1000 → client_shutdown`, `1001 → server_shutdown`, sinon `internal_error` |
| `broadcast_stop(&self, guild_id: &GuildId) -> usize` | async | `engine.stop_with_reason(guild, None, StopReason::Manual)` et retourne le nombre de jobs arrêtés (`0` sans moteur) |
| `broadcast_settings(&self, guild_id: &GuildId) -> usize` | async | relit les défauts de guilde (DB) et diffuse un `settings` à jour à chaque socket ; retourne le nombre de sockets servis (`0` sans moteur/DB) |

Câblage final (app.rs, propriété du Lead) :

```rust
use std::sync::Arc;

let hub = Arc::new(lc_server::ws::Hub::new());
let router = app::router_with_hub(state, Arc::clone(&hub)); // ou .merge(ws::router_with_hub(hub.clone()))
// http/admin : install_admin_hub(hub.clone()) → impl AdminHub pour Arc<Hub>
```

> ⚠️ `ws::router()` crée un `Arc<Hub>` privé (pratique pour les tests/outils) : en production,
> utiliser `ws::router_with_hub(hub.clone())` pour que la lane admin et les hooks ingest puissent
> partager le même registre. Le Lead effectue ce remplacement dans `app.rs` (hors de mon périmètre).

Signatures Rust exactes (déjà implémentées sur `Hub`) :

```rust
pub fn connected_count(&self) -> usize;
pub fn connected_for_guild(&self, guild_id: &GuildId) -> usize;
pub fn is_connected(&self, client_id: &Uuid) -> bool;
pub fn close_client(&self, client_id: &Uuid, code: u16) -> bool;
pub async fn broadcast_stop(&self, guild_id: &GuildId) -> usize;
pub async fn broadcast_settings(&self, guild_id: &GuildId) -> usize;
```

## 9. Intégration `lc-playback`

- `HubSink: PlaybackSink` : `on_play` → `unicast_play` (signature par client, retourne le nombre de
  sockets servis ; 0 ⇒ le job est marqué `FAILED` par le moteur, parité legacy « room vide ») ;
  `on_stop` → `broadcast_stop_frame` à la room.
- `PlaybackConfig` construit depuis `Config` : `api_base_url` = origine de `API_URL` (sans `/` final),
  `default_duration_sec` = `DEFAULT_DURATION` (5), `retention` = `PLAYBACK_JOB_RETENTION_HOURS` (72 h) ;
  le reste aux défauts du crate (tick 500 ms, padding 250 ms, grâce watchdog 10 s, etc.).
- Le moteur est créé une seule fois (premier hello), sa boucle `run` est liée à `state.shutdown` ;
  la purge tourne toutes les heures (`engine.purge(Utc::now())`) et s'arrête avec le serveur.
- `playback.state` met à jour le busy lock (`busy_until = now + max(remainingMs, 5000) + 250`) et le
  watchdog ; `playback.ended`/`error`/`stop.request` libèrent le job et lancent le suivant.
- Limites connues (cf. `docs/rust/lc-playback.md` §8) : le hub ne décode pas le rich payload legacy
  (pas de `tweetCard`, pas de `startOffsetSec` hérité hors `resume_offset_sec`), et ne recalcule pas
  `execution_date` hors dispatch/annulation.

## 10. Tests

`apps/lc-server/src/ws/tests.rs` (20 tests, `#[cfg(test)] mod tests`) :

Intégration (vrai serveur axum sur port éphémère + SQLite temporaire + client `tokio-tungstenite`) :

1. `hello_returns_welcome_then_settings_then_peers` — `welcome` conforme (v2, serverVersion,
   config par défaut, `currentJob` absent), puis `settings`, puis `peers`.
2. `empty_token_closes_with_4401`, 3. `unknown_token_closes_with_4401`,
   4. `revoked_client_closes_with_4402` (détection via SELECT ciblé),
   5. `outdated_app_version_closes_with_4403_and_min_version` (raison JSON parsée).
6. `non_hello_first_frame_closes_with_1002`.
7. `flooding_after_welcome_closes_with_4409` (21 heartbeats).
8. `oversized_frame_closes_with_4408` (65 537 octets).
9. `welcome_exposes_current_job_with_signed_url` (job `PLAYING`, URL signée ; `startOffsetSec`
   null/absent quand l'offset de reprise vaut 0).
10. `dispatched_play_carries_a_verifiable_signature` (signature re-vérifiée avec `media_url::verify`).
11. `stop_request_stops_the_job_and_broadcasts_stop` (ack puis `stop {reason:"manual"}` ; DB `DONE`).
12. `invite_read_only_refuses_meme_trigger` (ack `read_only`).
13. `heartbeat_is_accepted_and_updates_last_seen` (`last_seen_at` mis à jour en DB).
14. `peers_request_returns_ack_then_peers`.
15. `playback_state_is_persisted_for_a_corresponding_job` (`remaining_ms_snapshot` en DB).

Unitaires : `max_peers_is_frozen`, `sha256_token_hash_matches_the_reference_vector`,
`close_reason_mapping_is_stable`, `signed_play_uses_the_adaptive_media_ttl`,
`playback_state_event_and_stop_payload_shapes_match_the_protocol`.

Commandes de vérification (PowerShell) :

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_BUILD_JOBS = '4'
cargo fmt -p lc-server
cargo clippy -p lc-server --all-targets -- -D warnings
cargo test -p lc-server
```

État final de la vérification (2026-11-10) :

- `cargo fmt -p lc-server -- --check` : OK (exit 0) ;
- `cargo clippy -p lc-server --all-targets -- -D warnings` : OK (exit 0), aucune erreur dans `ws/**` ;
- `cargo test -p lc-server --lib` : **227 passed ; 0 failed ; 1 ignored** (dont les 20 tests `ws::tests`) ;
- `cargo test -p lc-server` (avec doctests) reste bloqué par un doctest hors périmètre,
  `apps/lc-server/src/discord/mod.rs` (appelle `Shutdown::wait` au lieu de `subscribe().wait()`),
  signalé à la lane discord. Aucun fichier hors `ws/**` n'est modifié par ce travail.

## 11. Écarts et décisions

1. **Hello tardif → `4401 auth_timeout`** : le prompt de la task-17 mentionnait `1008`/`4403` ;
   la SSOT `protocol-v2.md` (et la décision du Lead) impose `4401 auth_timeout`. Écart
   prompt↔SSOT signalé au Lead, SSOT appliquée.
2. **Plafond axum 256 Kio vs contrat 64 Kio** : `max_message_size`/`max_frame_size` à 256 Kio pour
   pouvoir lire une trame trop grosse et répondre proprement `4408` (le contrat applicatif reste
   64 Kio). Une trame > 256 Kio est fermée par axum sans close code applicatif.
3. **Rate limit `4409`** : la SSOT exige `4409` au-delà de 20 msg/s ; les excédents « ignorés » ne
   concernent que `playback.state` (1/s) et `peers.request` (1/2 s). Le compteur est par connexion,
   fenêtre glissante de 1 s.
4. **Détection des clients révoqués** : `lc-db::OverlayClientsRepository::get_by_token_hash` filtre
   déjà `revoked_at IS NULL`, d'où un `SELECT revoked_at FROM overlay_clients WHERE token_hash = ?`
   ciblé dans `ws/hub.rs` via `db.pool()` (accès public, approuvé par le Lead, dépendance `sqlx`
   ajoutée par lui). Promotion future conseillée : `get_by_token_hash_including_revoked` dans `lc-db`.
5. **Ordre ack/broadcast** : les acks de `stop.request`, `meme.trigger`, `peers.request` sont envoyés
   via la file du socket **avant** l'appel moteur, pour que le client observe `ack` puis `stop`/`play`.
6. **`sessionMode`** : celui du `hello` est purement diagnostique (log de divergence) ; le mode
   autoritaire est celui de la base, renvoyé dans `welcome`/`settings`.
7. **Peers** : maximum 8, dédupliqués par `clientId`, label = `default_author_name` sinon label du
   client sinon `unknown-device`, tri stable label puis `clientId`.
8. **`ServerMessage::Error`** : le payload existe (spec) mais le hub n'émet pas encore d'erreur
   asynchrone non fatale ; le helper `send_server_error` est conservé (allow dead_code) pour les
   hooks à venir (échec d'envoi média, purge en cours…).

## 12. Points d'intégration restants (hors propriétaire)

- **Lead / `app.rs`** : remplacer `ws::router()` par `ws::router_with_hub(Arc<Hub>)` et câbler
  `install_admin_hub(hub.clone())` ; `Hub` est le handle partageable (aucun changement d'`AppState`).
- **Lane overlay (Phase 3/8)** : implémente `GET /overlay/media/{assetId}` avec vérification
  `exp`/`sig` (`media_url::verify`), `HEAD`, `Range` (206/416) avant expiration ; le hub ne sert pas
  les octets.
- **Lane ingest/admin** : après purge/révocation, appeler `Hub::broadcast_stop`,
  `Hub::broadcast_settings`, `Hub::close_client` (via le trait `AdminHub`).
- **Observabilité** : les logs `tracing` couvrent handshake, authentification, dispatch, fermetures ;
  aucune métrique Prometheus pour l'instant (hors périmètre Phase 4b).
- **E2E serveur↔overlay** : test à ajouter quand l'overlay Tauri parlera le protocole v2 (Phase 8).
