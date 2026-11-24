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
- limites : trames **64 Kio** applicatives (close `4408`) avec borne de transport d'upgrade
  **1 Mio**, 20 messages/s, heartbeat 15 s, timeout 45 s mesuré sur les `Pong`, close `4408/4409` ;
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
   Limites d'upgrade : `max_message_size = max_frame_size = 1 Mio`
   (`MAX_TRANSPORT_MESSAGE_BYTES`). Le contrat applicatif reste **64 Kio** : une trame entre 64 Kio
   et 1 Mio est lue puis fermée `4408 frame_too_large` ; au-delà de 1 Mio, c'est la couche transport
   (axum/tungstenite) qui ferme (`1006`/`1009`) sans code applicatif garanti.
   **Origine (COR-06)** : si l'en-tête `Origin` est présent, il doit être autorisé par l'allowlist
   (§6) ; sinon la socket est fermée `1008 origin_not_allowed` **avant** toute lecture de trame.
   Une requête sans `Origin` (client natif) est acceptée.
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
6. **Boucle** : trames client (parsing `ClientEnvelope`, `v == 2`, `id` non vide et ≤ 64 caractères,
   sinon `1002 protocol_error`), file sortante **bornée par connexion** (256 trames,
   `OUTBOUND_QUEUE_CAPACITY`, production `try_send` non bloquante), chaque écriture socket bornée à
   **5 s** (`WRITER_SEND_TIMEOUT`), ping toutes les 15 s (timeout 45 s sans `Pong`), arrêt gracieux sur
   `state.shutdown`.
7. **Fermeture** : close frame si possible, désenregistrement du registre, broadcast `peers`. Un
   client lent (file pleine ou écriture socket > 5 s) est fermé seul en `4408 slow_client`, sans
   arrêter les autres sockets ni les jobs de la guilde.

Codes de fermeture émis par le serveur (cf. SSOT §3) :

| Code | Raisons | Déclencheurs |
| --- | --- | --- |
| `1000` | `client_shutdown` | close client, socket fermé par le client, fin normale |
| `1001` | `server_shutdown`, `heartbeat_timeout` | arrêt du serveur, 45 s sans activité |
| `1002` | `protocol_error`, `bad_playback_state` | JSON/type/v inconnu, payload invalide, `id` vide ou > 64, `playback.state` incohérent |
| `1003` | `unsupported_frame` | trame binaire |
| `1008` | `origin_not_allowed` | en-tête `Origin` hors allowlist (fermeture pré-hello, COR-06) |
| `1011` | `internal_error` | erreur DB au handshake/runtime, base indisponible |
| `4401` | `auth_timeout`, `missing_token`, `invalid_token` | hello absent/tardif, jeton vide/inconnu, `clientId` incohérent |
| `4402` | `revoked` | client révoqué (y compris fermeture admin) |
| `4403` | `outdated_protocol` | `v != 2` ou `appVersion` < minimum (raison JSON) |
| `4408` | `frame_too_large`, `slow_client` | trame texte > 64 Kio (lue jusqu'à la borne de transport de 1 Mio), ou client trop lent (file sortante pleine, écriture socket > 5 s) |
| `4409` | `rate_limited` | > 20 trames C→S/s |

## 4. Messages C→S

Toutes les trames sont des enveloppes JSON `lc_protocol::ClientEnvelope`
(`{v:2, type, id, ts, payload}`). Les champs inconnus sont tolérés ; le serveur ne renvoie jamais
d'erreur pour un `id` inconnu. Le champ `id` est **validé** avant tout traitement : non vide
(refusé s'il est vide ou uniquement composé d'espaces) et ≤ 64 caractères, sinon close
`1002 protocol_error` (prend effet avant les débits et les acks).

| `type` | Payload | Traitement |
| --- | --- | --- |
| `hello` | `{token, appVersion, clientId?, sessionMode?}` | handshake uniquement (§3) |
| `heartbeat` | `{clientId, guildId, appVersion, sessionMode?}` | met à jour `last_seen_at` (async), log si `sessionMode` diverge du mode serveur ; aucun ack |
| `playback.state` | `{jobId, state: playing\|paused, remainingMs?: number}` | throttle 1/s (excédent ignoré) ; `remainingMs` borné à `[0, 86_400_000]` ; transmet `PlaybackStateEvent` au moteur (busy lock + watchdog) ; `ended` ⇒ `1002 bad_playback_state` |
| `playback.ended` | `{jobId, reason: completed\|manual\|error}` | transmet `PlaybackEndedEvent` ; idempotent côté moteur ; `jobId` invalide ⇒ `1002 bad_playback_state` |
| `error` | `{jobId, code, message}` | log seulement (le watchdog libère le job) |
| `stop.request` | `{jobId?, source?: user\|hotkey\|admin}` | en `invite_read_only` : ack `read_only` ; throttle 1/s (ack `rate_limited`) ; sans `jobId` ⇒ tous les jobs de la guilde ; ack `ok` **puis** `engine.stop(...)` qui diffuse `stop {reason:"manual"}` |
| `media.trigger` | `{itemId, trigger: shortcut\|ui}` | en lecture seule : ack `read_only` ; item inconnu ⇒ ack `item_not_found` ; média absent/non `READY`/expiré ⇒ ack `media_not_ready` ; ack `ok` puis job `source = BOARD` et `engine.enqueue` (dispatch quand la file arrive à lui, **sans préemption**) |
| `peers.request` | `{}` | throttle 2 s (excédent ignoré **sans** ack) ; ack `ok` puis broadcast `peers` |

Le type legacy `meme.trigger` n'est **plus accepté** : la désérialisation échoue
comme pour tout type inconnu (close `1002`). Les clients envoient uniquement
`media.trigger`.

**Coalescence (B-02).** Une session ne garde qu'une exécution de `media.trigger` en vol. Une
répétition du **même `itemId`** dans une fenêtre glissante de 1 s est idempotente : le serveur
ré-acquitte le job déjà créé (`ack {ok:true}` avec le même `jobId`, `outcome` et `position`)
**sans** nouvelle requête SQL ni nouveau job, et la fenêtre glisse à chaque répétition. Un
`itemId` différent passe immédiatement (un vrai changement de média n'est jamais perdu) ; le même
`itemId` repasse une fois la fenêtre écoulée. Limites : la garde est **par connexion**, pas
globale à la guilde — deux sockets (ou deux overlays) de la même guilde ont chacun leur fenêtre
et peuvent créer chacun un job ; un job arrêté ou terminé pendant la fenêtre peut être ré-acquitté
tel quel.

Codes d'ack (`error.code`) : `read_only`, `item_not_found`, `media_not_ready`, `invalid_payload`,
`rate_limited`, `internal_error`.

Règles de complétude :
- texte du média = `item.message` trimé, tronqué à `MAX_MEDIA_TEXT_CHARS` (500) ;
- `showText = message non vide` ; `authorName = profil membre courant || item.createdByName` ;
  `authorImage = avatar du membre courant` ; `durationSec = media.duration_sec` (0 si absent) ;
- **profil d'auteur lu à l'instant du trigger** : `guild_members.display_name` /
  `guild_members.avatar` du membre propriétaire de l'overlay. Un
  `PATCH /overlay/profile` est donc pris en compte immédiatement, sans
  reconnexion. Si l'overlay n'a aucun membre rattaché (ancien appairage), repli
  sur `overlay_clients.default_author_name` / `default_author_image` ;
- ids de jobs/d'items parsés via `lc_domain` (`PlaybackJobId::parse`, `MediaBoardItemId::parse`),
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
| `error` | `{code, message, jobId?}` | erreur **non fatale** émise effectivement (rév. 4) : `dispatch_failed` (aucun socket de la room n'a pu recevoir un `play` média) et `internal_error` (échec moteur après `ack` de `stop.request`/`media.trigger`) |
| `media.ingestion` | `{ingestionId, ok, created?, itemId?, error?: {code, message?}}` | **unicast à toutes les sockets d'un client overlay** : fin d'une ingestion asynchrone du board demandée par ce client (`202 {ingestionId}`) |
| `board.changed` | `{change: added\|updated\|deleted, itemId}` | **broadcast room** : ajout/modification/suppression d'un item du board (overlay, `/ingest` `saveToBoard`, Discord `/media-add`, panneau admin) |

Les `id` serveur sont de la forme `srv-<préfixe>-NNNNNN` (`welcome`, `settings`, `peers`, `play`,
`stop`, `ack`, `error`, `ingestion`, `board`), `ts` en millisecondes Unix, `v = 2`.

`currentJob` : présent seulement si un job `PLAYING` existe ; c'est un `PlayPayload` (URL re-signée
pour ce socket) + `startedAt` + `remainingMs`. Au plus un job `PLAYING` par guilde (invariant du
scheduler, §15.3 de la SSOT) ; si plusieurs sont détectés, le plus récent par `startedAt` est choisi.

## 6. Limites et robustesse

| Limite | Valeur | Comportement |
| --- | --- | --- |
| trame client | 64 Kio (`MAX_FRAME_BYTES`) | close `4408` |
| borne de transport d'upgrade | 1 Mio (`MAX_TRANSPORT_MESSAGE_BYTES`) | 64 Kio..1 Mio → `4408` applicatif ; > 1 Mio → close transport `1006`/`1009` |
| file sortante | **256 trames par connexion** (`OUTBOUND_QUEUE_CAPACITY`) | `try_send` non bloquant ; pleine → close `4408 slow_client` de cette connexion seule, sauf close explicite déjà demandé qui garde son code |
| écriture socket | **5 s** (`WRITER_SEND_TIMEOUT`) | `sink.send` bloqué → close `4408 slow_client` (ou close explicite en attente), tâche writer terminée |
| `id` de trame C→S | non vide (espaces refusés), ≤ 64 caractères (`MAX_REQUEST_ID_CHARS`) | close `1002 protocol_error` avant traitement |
| messages C→S | 20/s glissant sur 1 s, par connexion | close `4409` |
| `playback.state` | 1/s | excédent ignoré silencieusement |
| `peers.request` | 1/2 s | excédent ignoré silencieusement |
| hello | 5 s (`HELLO_TIMEOUT_MS`) | close `4401 auth_timeout` |
| heartbeat / pong | ping 15 s, timeout 45 s (`WS_HEARTBEAT_TIMEOUT_SEC`) | timeout mesuré sur le dernier **Pong** reçu ; close `1001 heartbeat_timeout` |
| sockets par client | **3** (`MAX_SOCKETS_PER_CLIENT`) | nouvelle socket : close `4409 too_many_connections` avant `welcome` |
| sockets par guilde | **16** (`MAX_SOCKETS_PER_GUILD`) | nouvelle socket : close `4409 too_many_connections` avant `welcome` |
| origine `/ws` | allowlist : origine de `API_URL`, `http(s)://tauri.localhost`, `ADDITIONAL_CORS_ORIGINS`, env `WS_ALLOWED_ORIGINS` ; loopback seulement en dev | `Origin` présent et hors liste : close `1008 origin_not_allowed` ; `Origin` absent : accepté |
| `remainingMs` | `[0, 86_400_000]` | borne avant transmission au moteur |
| texte du média | 500 caractères | troncature |
| peers | 8 | troncature (ordre déterministe) |

Le rate limiter est un simple `VecDeque<Instant>` par connexion (fenêtre glissante de 1 s) : sans
allocation par message, et suffisant pour un client overlay. Toute trame texte est comptée, y
compris les messages rejetés ensuite ; les Pong et les trames de contrôle ne sont pas comptés.

La file sortante est créée par `handle_socket` (`tokio::sync::mpsc::channel(OUTBOUND_QUEUE_CAPACITY)`)
et son `Sender` est enregistré dans `Hub::Connection`. Tous les producteurs (broadcasts, `play`,
`stop`, `settings`, acks) passent par `Connection::enqueue_text`, donc par `try_send` : jamais
d'attente du hub sur une socket. `Connection` porte une machine à états `OPEN` / `SLOW` / `EXPLICIT`
(`AtomicU8`, transitions sérialisées par le verrou `Hub::inner`) : une file pleine passe la
connexion à `SLOW`, loggue une fois (client, guilde, code, raison, capacité — pas de secret) et
publie `4408 slow_client` dans un `watch::channel` propre à la connexion ; la boucle de session la
lit et ferme sans tenir le verrou du hub. Un close explicite (révocation `4402`, arrêt admin) passe
la connexion à `EXPLICIT` : s'il arrive après un `4408`, il publie son code hors file et le client
reçoit bien le code explicite ; s'il est le premier, aucun `4408` ultérieur ne le remplace. Avant de
diffuser le `stop` d'une fermeture explicite, le hub marque la connexion « explicite en
préparation » : un débordement pendant cette fenêtre ne publie pas de `4408`, le close explicite
reste donc le premier événement observé par la session. Cette priorité s'applique au hub : si la
session a déjà lu et commencé à fermer sur un `4408` publié **avant** la demande explicite, elle ne
peut pas le remplacer ; la fenêtre `stop` d'une révocation, elle, est neutralisée par l'annonce
`explicit_requested`, donc aucun `4408` n'y est publié. La profondeur est observable en test via
`outbound_queue_depth`/`queue_depth` (`max_capacity() - capacity()`).

La borne est en **trames**, pas en octets : 256 trames de texte peuvent peser jusqu'à ~16 Mio par
socket (64 Kio/trame), plafond de fait non mesuré — ce n'est pas un budget mémoire enforced.

Le writer borne chaque `sink.send` (texte, ping, close) avec `timeout(WRITER_SEND_TIMEOUT, ...)`.
Un dépassement ferme le client en `4408 slow_client`, sauf close explicite déjà en attente (dans le
`watch` ou dans la file) qui conserve son code et sa raison ; la tâche de session termine par le
chemin habituel (`unregister`, `broadcast_peers`). Limite connue : si le timeout survient au milieu
d'une grande trame, le close peut être illisible côté client (trame tronquée) ; le client verra
alors une fermeture de transport sans code applicatif.

Les plafonds de sockets (3/client, 16/guilde) sont vérifiés **sous le verrou du registre** au moment
de `Hub::register`, après authentification et avant l'envoi de `welcome` : un client refusé ne reçoit
donc jamais de `welcome`, et les compteurs sont libérés par `Hub::unregister` dans la sortie
normale de `handle_socket` (close, erreur, shutdown).

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
| `send_to_client(&self, client_id: &Uuid, message: ServerMessage) -> bool` | sync | envoie un message protocolaire à **toutes** les sockets du client (comparaison `client_uuid`, plusieurs sockets possibles) ; `true` si au moins une socket l'a accepté. Utilisé pour `media.ingestion` |
| `broadcast_to_guild(&self, guild_id: &GuildId, message: ServerMessage) -> usize` | sync | diffuse un message protocolaire à toutes les sockets de la guilde ; retourne le nombre de sockets servies. Utilisé pour `board.changed` (et disponible pour l'admin) |
| `broadcast_stop(&self, guild_id: &GuildId) -> usize` | async | `engine.stop_with_reason(guild, None, StopReason::Manual)` et retourne le nombre de jobs arrêtés (`0` sans moteur) |
| `broadcast_settings(&self, guild_id: &GuildId) -> usize` | async | relit les défauts de guilde (DB) et diffuse un `settings` à jour à chaque socket ; retourne le nombre de sockets servis (`0` sans moteur/DB) |
| `broadcast_purge(&self, guild_id: &GuildId) -> usize` | async | `engine.stop_all_with_reason(guild, StopReason::Purge)` : diffuse `stop {reason:"purge", jobId:null}` ; retourne le nombre de jobs arrêtés (`0` sans moteur). Implémentation par défaut du trait = repli sur `broadcast_stop` ; `Hub` la surcharge |

Câblage final (fait dans `serve()` / `app.rs`, intégration Lead) :

```rust
use std::sync::Arc;

let hub = Arc::new(lc_server::ws::Hub::new());
let hub_handle: Arc<Hub> = Arc::clone(&hub);
let admin_hub: Arc<dyn AdminHub> = hub_handle;
state = state.with_admin_hub(admin_hub); // AUD-COR-14 : plus de OnceLock
let router = app::router_with_hub(state, Arc::clone(&hub));
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
pub fn send_to_client(&self, client_id: &Uuid, message: ServerMessage) -> bool;
pub fn broadcast_to_guild(&self, guild_id: &GuildId, message: ServerMessage) -> usize;
pub async fn broadcast_stop(&self, guild_id: &GuildId) -> usize;
pub async fn broadcast_stop_with_reason(
    &self,
    guild_id: &GuildId,
    job_id: Option<PlaybackJobId>,
    reason: StopReason,
) -> usize;
pub async fn broadcast_purge(&self, guild_id: &GuildId) -> usize;
pub async fn broadcast_settings(&self, guild_id: &GuildId) -> usize;
```

`broadcast_purge` est exposé par le trait `AdminHub` (méthode à implémentation par défaut, repli sur
`broadcast_stop`) et surchargé par `Hub` pour émettre `stop {reason:"purge", jobId:null}`. La route
`POST /admin/api/guilds/{id}/purge` l'appelle désormais à la place de `broadcast_stop`. La route
`.../stop` continue d'utiliser `broadcast_stop` (`reason:"manual"`, `jobId` renseigné si un seul job
était en cours). La révocation admin (`close_client(..., 4402)`) diffuse elle aussi
`stop {reason:"revoked", jobId:null}` **avant** la close frame (cf. §9).

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

`apps/lc-server/src/ws/tests.rs` — `cargo test -p lc-server ws::` exécute **80 tests**
(2026-11-23, `hub.rs` et `session.rs` inclus) ; liste non exhaustive des cas structurants :

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
   null/absent quand l'offset de départ vaut 0).
10. `dispatched_play_carries_a_verifiable_signature` (signature re-vérifiée avec `media_url::verify`).
11. `stop_request_stops_the_job_and_broadcasts_stop` (ack puis `stop {reason:"manual"}` ; DB `DONE`).
12. `invite_read_only_refuses_media_trigger` (ack `read_only`).
13. `heartbeat_is_accepted_and_updates_last_seen` (`last_seen_at` mis à jour en DB).
14. `peers_request_returns_ack_then_peers`.
15. `playback_state_is_persisted_for_a_corresponding_job` (`last_playback_state_at` en DB).
16. `origin_tauri_localhost_is_allowed` (Origin `http://tauri.localhost` → `welcome`).
17. `unknown_origin_is_closed_with_1008` (Origin étrangère → close `1008 origin_not_allowed`).
18. `too_many_sockets_per_client_are_rejected_with_4409` (3 sockets OK, 4e → `4409
    too_many_connections` sans `welcome` ; compteur libéré à la déconnexion).
19. `socket_caps_are_enforced_unit` (`Hub::register` : `ClientLimit` puis `GuildLimit`, 3/16).
20. `text_only_job_is_played_and_marked_playing` (job sans média → `play` émis, `media:null`, DB
    `PLAYING`).
21. `welcome_exposes_text_only_current_job` (`currentJob` aplati sans média, `remainingMs` u64).
22. `revocation_emits_stop_revoked_before_close_4402` (stop `revoked` `jobId:null` PUIS close `4402`,
    broadcast aux autres sockets, job libéré en DB).
23. `purge_stop_has_a_dedicated_reason` (`broadcast_purge` → reason `"purge"`, `jobId:null`, DB `DONE`).
24. `oversized_frame_is_closed_with_4408_instead_of_1006` (trame de 300 Kio sous la borne transport
    de 1 Mio → close `4408 frame_too_large`, pas `1006`).
25. `empty_request_id_is_closed_with_1002` (`heartbeat` avec `id:""` → close `1002`).
26. `oversized_request_id_is_closed_with_1002` (`id` de 65 caractères → close `1002`).
27. `dispatch_failure_emits_non_fatal_error_frame` (serveur sans `MEDIA_URL_SECRET` : le `play`
    média ne peut être signé → trame `error` `dispatch_failed` avec `jobId`, puis `peers.request`
    acquitté et connexion toujours ouverte).
28. `slow_client_is_closed_with_4408_and_healthy_client_keeps_receiving` (client qui ne lit pas sa
    socket : 300 `rights.changed` ciblés saturent sa file de 256 → close `4408 slow_client`, aucun
    `stop`, job toujours `PLAYING`, le second client de la guilde reçoit encore la diffusion).

Unitaires : `max_peers_is_frozen`, `sha256_token_hash_matches_the_reference_vector`,
`close_reason_mapping_is_stable`, `signed_play_uses_the_adaptive_media_ttl`,
`playback_state_event_and_stop_payload_shapes_match_the_protocol` ; dans `tests.rs` :
`outbound_queue_is_bounded_and_full_producers_do_not_block`,
`close_frame_that_does_not_fit_is_routed_out_of_band`,
`explicit_close_wins_over_slow_close_when_stop_fills_the_queue` (255/256 : le `stop` remplit la
file, le close `4402 revoked` gagne),
`explicit_close_wins_over_slow_close_when_the_queue_is_full` (256/256 : le `stop` est absorbé par
l'annonce explicite, aucun `4408` n'est publié),
`explicit_close_replaces_an_already_published_slow_close` (un `4408` déjà publié est remplacé par
`4402 revoked`),
`explicit_close_announcement_suppresses_slow_close_during_stop` (256/256 avec annonce : le watch
reste vide pendant le débordement, puis `4402`),
`first_explicit_close_wins_over_line_revocation_then_rate_limit` et
`..._rate_limit_then_revocation` (le premier code explicite est conservé, le second n'est pas
déposé) ; dans `session.rs` :
`request_id_validation_accepts_64_chars_and_rejects_others`,
`pong_liveness_is_not_refreshed_by_application_heartbeats`,
`writer_send_is_bounded_even_when_the_socket_stalls`,
`writer_timeout_keeps_a_pending_explicit_close`,
`writer_timeout_falls_back_to_the_slow_client_close`,
`pending_explicit_close_scans_the_queue_behind_text_frames`,
`pending_explicit_close_is_none_when_only_the_slow_request_is_pending`.

Commandes de vérification (PowerShell) :

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_BUILD_JOBS = '4'
cargo fmt -p lc-server
cargo clippy -p lc-server --all-targets -- -D warnings
cargo test -p lc-server
```

État final de la vérification (2026-11-10, task-40) :

- `cargo fmt --all -- --check` : OK (exit 0) ;
- `cargo clippy --workspace --all-targets -- -D warnings` : OK (exit 0) ;
- `cargo test -p lc-server -p lc-playback --no-fail-fast` : OK (exit 0) — `lc-server` **251 passed /
  0 failed / 1 ignored** (dont les 28 tests `ws::tests`) + doctest `lc-server` 1 passed ; `lc-playback`
  `3+3+13 passed / 0 failed` ;
- Correctif test-only hors périmètre (autorisé par le Lead) : `crates/lc-playback/src/store.rs:414`
  décale de 1 s la `submission_date` du second job de `claim_est_unique_pour_une_guilde`
  (départage par UUID v4 non déterministe lorsque deux jobs tombaient dans la même milliseconde).

État de la vérification (2026-11-10, task-48 / P2-A) :

- `cargo test -p lc-server --no-fail-fast` : OK (exit 0) — **284 passed / 0 failed / 1 ignored**
  (dont les **34 tests `ws::tests`**) + doctest `lc-server` 1 passed ;
- `cargo test -p lc-protocol` : OK (exit 0) — **16 passed / 0 failed** (29 fixtures golden
  inchangées + `unknown_play_and_tweet_card_fields_are_preserved`) ;
- `cargo clippy -p lc-server -p lc-protocol --all-targets -- -D warnings` : OK (exit 0) ;
- `rustfmt --check` sur les fichiers modifiés de cette lane : OK ;
- execution de la ligne de commande ci-dessus relancée après stabilisation des lanes parallèles ;
  le `cargo fmt --all -- --check` global reste à la charge de l'intégration (fichiers d'autres
  lanes en cours d'écriture).

Vérification B-01 (2026-11-23, branche `fix/ws-outbound-queue`) :

- `cargo test -p lc-server ws::` : **80 passed / 0 failed / 0 ignored** ;
- `cargo test -p lc-server` : **726 passed / 0 failed / 11 ignored** (lib) + `contracts` 3 passed +
  `kill_recover` 1 passed + doctest 1 passed / 1 ignored ;
- `cargo clippy -p lc-server --all-targets -- -D warnings` : OK (exit 0) ;
- `cargo fmt --all -- --check` : OK (exit 0) ;
- morsure de la borne : `OUTBOUND_QUEUE_CAPACITY` temporairement portée à 512 →
  `slow_client_is_closed_with_4408_and_healthy_client_keeps_receiving` échoue
  (`tests.rs:267`, timeout d'attente de la fermeture) ; capacité restaurée à 256 → passe ;
- morsure P2-1 : `enqueue_close` muté pour absorber l'explicite quand l'état est `SLOW` et pour
  déclencher `slow_client` au lieu de publier hors file → les deux tests
  `explicit_close_wins_over_slow_close_*` échouent (`Some(4408 slow_client)` / `None` au lieu de
  `Some(4402 revoked)`) ; code restauré → passent ;
- morsure P2-2 : `pending_explicit_close` muté en `None` → `writer_timeout_keeps_a_pending_explicit_close`
  et `pending_explicit_close_scans_the_queue_behind_text_frames` échouent (« le close explicite en
  attente doit être conservé », « le close explicite de la file doit gagner ») ; code restauré →
  passent ;
- morsure P3-1a : branche `previous == SLOW` de `enqueue_close` remplacée par `return true` →
  `explicit_close_replaces_an_already_published_slow_close` échoue (le watch reste
  `4408 slow_client`) ; restaurée → passe ;
- morsure P3-1b : `request_slow_close` ne consulte plus `explicit_requested` →
  `explicit_close_announcement_suppresses_slow_close_during_stop` échoue (un `4408` est publié
  pendant le `stop`) ; restauré → passe ;
- morsure P3-2 : `timeout_close_info` remplacé par un `4408` inconditionnel →
  `writer_timeout_keeps_a_pending_explicit_close` échoue ; restauré → passe ;
- morsure P3-3 : `claim_explicit_close` renvoie `Some(EXPLICIT)` au lieu de `None` → les deux tests
  `first_explicit_close_wins_*` échouent (deux closes déposés) ; restauré → passent.

## 11. Écarts et décisions

1. **Hello tardif → `4401 auth_timeout`** : le prompt de la task-17 mentionnait `1008`/`4403` ;
   la SSOT `protocol-v2.md` (et la décision du Lead) impose `4401 auth_timeout`. Écart
   prompt↔SSOT signalé au Lead, SSOT appliquée.
2. **Borne transport 1 Mio vs contrat 64 Kio** : `max_message_size`/`max_frame_size` à 1 Mio pour
   pouvoir lire une trame trop grosse et répondre proprement `4408` (le contrat applicatif reste
   64 Kio). Une trame > 1 Mio est fermée par le transport sans close code applicatif — la marge de
   1 Mio couvre largement une trame `Text` surdimensionnée réaliste tout en bornant la mémoire lue
   par trame. La validation de `id` (non vide, ≤ 64) suit immédiatement le parsing de l'enveloppe,
   avant tout effet de bord.
3. **Rate limit `4409`** : la SSOT exige `4409` au-delà de 20 msg/s ; les excédents « ignorés » ne
   concernent que `playback.state` (1/s) et `peers.request` (1/2 s). Le compteur est par connexion,
   fenêtre glissante de 1 s.
4. **Détection des clients révoqués** : `lc-db::OverlayClientsRepository::get_by_token_hash` filtre
   déjà `revoked_at IS NULL`, d'où un `SELECT revoked_at FROM overlay_clients WHERE token_hash = ?`
   ciblé dans `ws/hub.rs` via `db.pool()` (accès public, approuvé par le Lead, dépendance `sqlx`
   ajoutée par lui). Promotion future conseillée : `get_by_token_hash_including_revoked` dans `lc-db`.
5. **Ordre ack/broadcast** : les acks de `stop.request`, `media.trigger`, `peers.request` sont envoyés
   via la file du socket **avant** l'appel moteur, pour que le client observe `ack` puis `stop`/`play`.
6. **`sessionMode`** : celui du `hello` est purement diagnostique (log de divergence) ; le mode
   autoritaire est celui de la base, renvoyé dans `welcome`/`settings`.
7. **Peers** : maximum 8, dédupliqués par `clientId`, label = `default_author_name` lu à la
   connexion, sinon label du client sinon `unknown-device`, tri stable label puis `clientId`. Le
   profil vivant du membre ne sert qu'aux payloads `play` (relu à chaque `media.trigger`), pas au
   libellé des pairs.
8. **`ServerMessage::Error`** : le hub émet désormais les erreurs asynchrones non fatales
   (rév. 4) : `dispatch_failed` depuis `HubSink::on_play` quand `unicast_play` ne délivre à aucun
   socket alors que la room en contient, et `internal_error` après un `ack` lorsque l'appel moteur
   (`stop.request`/`media.trigger`) échoue. `media_unavailable` reste réservé aux hooks média.
9. **Origine `1008` plutôt que HTTP 403 (COR-06)** : le refus d'`Origin` intervient après l'upgrade
   (une réponse HTTP n'est alors plus possible) ; on ferme donc la socket `1008 origin_not_allowed`,
   avant `hello`. L'allowlist est configurable sans redéploiement de code (`WS_ALLOWED_ORIGINS`,
   séparée par virgules) et réutilise les origines déjà connues (`API_URL`, Tauri, CORS additionnels).
   Absence d'`Origin` = client natif accepté (le token reste obligatoire).
10. **Plafonds de sockets 3/16 (COR-06)** : constants du crate, comptées sous le verrou du registre
    à `register` (client puis guilde), fermeture `4409 too_many_connections` sans `welcome`. Les
    compteurs sont libérés par `unregister`, appelé inconditionnellement à la sortie de
    `handle_socket` (y compris en cas d'erreur ou de shutdown).
11. **Jobs texte-seul (COR-07)** : `Hub::signed_play_payload` et `signed_current_job` ne renvoient
    plus `None` quand `media` est absent : ils renvoient le payload tel quel. `unicast_play` compte
    ces sockets comme servis, donc le moteur ne marque plus le job `FAILED` (`recipients == 0`) —
    `play` est émis avec `media:null`, `text`/`author`/`tweetCard` restant porteurs du contenu.
12. **Révocation (COR-07)** : `close_client(..., 4402)` diffuse d'abord `stop {reason:"revoked",
    jobId:null}` sur la room (donc le client ciblé ET les autres), puis envoie la close frame, puis
    libère le job côté moteur via un `spawn` de `release_playing_without_stop` (le broadcast a déjà
    prévenu les clients ; on ne rediffuse pas de stop manuel). Si la file du client est saturée, le
    close explicite part hors file et reste prioritaire (`4402`), sauf si la session a déjà lu un
    `4408` publié avant l'annonce explicite et commencé à fermer ; le `stop` diffusé juste avant est
    best-effort : l'ordre `stop` puis close n'est alors plus garanti.
13. **Purge admin (COR-07)** : nouvelle raison dédiée `StopReason::Purge` via `broadcast_purge`
    (`AdminHub` + `Hub`), avec `jobId:null` imposé par `engine.stop_all_with_reason` ; la route
    `/stop` conserve `manual`. Le trait `AdminHub` est modifié hors `ws/**` avec l'accord du Lead
    (`http/admin/hub.rs`, `http/admin/guilds.rs`, `wiring.rs`).
14. **Liveness sur Pong (AUD-A1-008)** : le timeout de 45 s est mesuré sur le dernier `Pong` reçu,
    pas sur n'importe quelle trame ; une socket qui reçoit des Ping serveur mais ne répond jamais
    est fermée `1001 heartbeat_timeout`. Depuis la rév. 4, le tracker `PongLiveness` (session.rs) ne
    peut pas être rafraîchi par un `heartbeat` applicatif : seule une trame `Pong` réelle remet la
    fenêtre à zéro, ce qui est couvert par un test unitaire dédié.
15. **Validation de `id` (AUD-A1-007)** : `validate_request_id` refuse un `id` vide, uniquement
    composé d'espaces, ou de plus de 64 caractères (`MAX_REQUEST_ID_CHARS`), avant tout traitement
    du message ; close `1002 protocol_error`. Testé en unitaire (64 acceptés, 65 refusés) et en
    intégration (`empty_request_id_is_closed_with_1002`, `oversized_request_id_is_closed_with_1002`).
16. **Champs inconnus `play`/`tweetCard` (AUD-COR-12)** : `PlayPayload` et `TweetCardPayload` du
    crate `lc-protocol` portent un `#[serde(flatten)] extra: serde_json::Map<String, Value>` ;
    les clés non modélisées sont conservées puis ré-émises à l'identique (round-trip exact vérifié
    par `unknown_play_and_tweet_card_fields_are_preserved`, qui étend une fixture existante). Les
    29 fixtures golden restent inchangées ; `PlayPayload::default()` (map vide) évite d'imposer
    `serde_json` aux crates consommateurs.
17. **Timeout HTTP global (COR-06, nit V2 « exemptions par préfixe »)** : le diff de `§12` n'exempte
    plus le préfixe `/overlay/media-board/` en bloc : seules les routes réellement de streaming sont
    exemptées (`/ws` exact, `/ingest` exact, préfixes `/overlay/media/` et
    `/overlay/media-board/media/`). Les routes d'écriture du board
    (`POST /overlay/media-board/items` et `/upload`) gardent leur budget historique, mais
    l'ingestion longue ne dépend plus de la requête HTTP (tâche détachée). Les anciens chemins
    `/overlay/meme-board/...` n'existent plus. Le reste de l'API
    (admin, board JSON, config, pairing, health) reçoit `408 request_timeout` à 30 s. Le rate-limit
    appliqué au streaming (`Range`)/uploads est un nit séparé, hors de cette lane : à traiter par
    l'owner du rate limiter.
18. **Budget `/ingest` propre (remplace l'exemption du point 17)** : `/ingest` et `/ingest/*` ne
    sont plus exemptées du timeout global ; elles reçoivent `TimeoutPolicy::ingest =
    MEDIA_INGEST_TIMEOUT_MS + INGEST_TIMEOUT_GRACE` (60 s de marge, **300 s** par défaut) et
    répondent `504 ingest_timeout` au-delà. Les exemptions restantes sont `/ws`, les préfixes
    `/overlay/media/` et `/overlay/media-board/media/`, et `PUT
    /overlay/media-board/uploads/<id>/parts` ; tout le reste garde `408 request_timeout` à 30 s.
    Le timeout couvre la lecture du corps, donc une ingestion bloquée (résolution DNS sans borne,
    hook de lecture sur un verrou du moteur) ne peut plus laisser la requête ouverte
    indéfiniment. Bornes d'appui : `lc_media::guard::DNS_TIMEOUT` (5 s) pour la résolution DNS et
    `media::hook::NOTIFY_TIMEOUT` (5 s) pour la notification de lecture — au-delà, le job reste
    `PENDING` et sera joué au tick suivant au lieu de retarder la réponse HTTP.

## 12. Points d'intégration restants (hors propriétaire)

- **Lead / `app.rs`** : intégré — `ws::router_with_hub(Arc<Hub>)` + admin hub
  injecté dans `AppState` (`with_admin_hub`), plus de `OnceLock` ; le timeout
  (COR-06) est en place sous forme de middleware `enforce_timeout` paramétré par
  `app::TimeoutPolicy`, avec exemptions resserrées (`/ws` et préfixes média) et
  un budget propre pour `/ingest` (`MEDIA_INGEST_TIMEOUT_MS + 60 s`) qui répond
  `504 ingest_timeout` ; `408 request_timeout` 30 s ailleurs, plus de chemin legacy.
  Extrait de la forme actuelle (`apps/lc-server/src/app.rs`) :
  ```rust
  fn timeout_scope(method: &Method, path: &str) -> Option<TimeoutScope> {
      if path == "/ws"
          || path.starts_with("/overlay/media/")
          || path.starts_with("/overlay/media-board/media/")
      {
          return None; // streaming : aucune borne globale
      }
      if method == Method::PUT && path.ends_with("/parts") {
          return None; // fragment d'upload du board
      }
      if path == "/ingest" || path.starts_with("/ingest/") {
          return Some(TimeoutScope::Ingest); // 300 s par défaut, 504 ingest_timeout
      }
      Some(TimeoutScope::Request) // 30 s, 408 request_timeout
  }

  // dans router_with_hub, après normalize_errors / avant compression :
  .layer(middleware::from_fn_with_state(
      TimeoutPolicy::from_config(&config),
      enforce_timeout,
  ))
  ```
- **Lane overlay (Phase 3/8)** : implémente `GET /overlay/media/{assetId}` avec vérification
  `exp`/`sig` (`media_url::verify`), `HEAD`, `Range` (206/416) avant expiration ; le hub ne sert pas
  les octets.
- **Lane ingest/admin** : après purge/révocation, appeler `Hub::broadcast_stop`,
  `Hub::broadcast_settings`, `Hub::close_client` (via le trait `AdminHub`).
- **Observabilité** : les logs `tracing` couvrent handshake, authentification, dispatch, fermetures ;
  aucune métrique Prometheus pour l'instant (hors périmètre Phase 4b).
- **E2E serveur↔overlay** : test à ajouter quand l'overlay Tauri parlera le protocole v2 (Phase 8).
