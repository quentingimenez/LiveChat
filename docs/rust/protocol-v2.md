# Protocole v2 LiveChat — spécification GELÉE

> **FROZEN rev. 2 pour l'implémentation** — 2026-11-09 — owner : `protocole-v2` — statut : gelé (Phase 0)
>
> Révision 2 (2026-11-09, décisions Lead sur les 5 points ouverts) : TTL média adaptatif (§8.1),
> `stop.request` avec `jobId`/`source` (§5.6), invariant « un seul job `PLAYING` » (§6.1.1),
> suppression de `GET /overlay/config` (§6.5, §15.4), plancher `MIN_OVERLAY_VERSION` +
> `welcome.serverVersion` (§1.6, §15.5). Révision 1 (2026-11-09) : version initiale gelée.
>
> Toute modification de ce document est une rupture de contrat : elle exige l'accord du Lead,
> une mise à jour simultanée des fixtures golden (`crates/lc-protocol/tests/fixtures/`) et des
> implémentations serveur (`apps/lc-server`) et overlay (`apps/lc-overlay`).
> Le protocole v1 (socket.io 1.2.0) est remplacé en big-bang ; aucune négociation de version
> ascendante n'est prévue. Le résumé approuvé du plan est dans `docs/rust/migration-plan.md:61-72`.

---

## 0. Portée, sources et conventions

Cette spécification décrit le protocole WebSocket JSON natif **v2** qui remplace socket.io 1.2.0.
Elle est extraite du code TypeScript v1 réel (fichiers et lignes cités dans chaque section) et du
plan de migration approuvé (`docs/rust/migration-plan.md:61-72,147-150`).

Sources principales :

| Concern | Fichier v1 |
|---|---|
| Types de messages et version | `server/packages/overlay-protocol/src/index.ts:1-108` |
| Émission serveur (scheduler) | `server/src/services/playbackScheduler.ts:48-117,849-918,922-997` |
| Émission serveur (worker messages) | `server/src/components/messages/messagesWorker.ts:26-42,44-113` |
| Auth + handlers socket serveur | `server/src/loaders/socketLoader.ts:76-104,114-172,174-376` |
| Arrêt manuel / préemption / purge | `server/src/services/manualStop.ts:9-60` |
| Auth REST et hash de token | `server/src/services/overlayAuth.ts:28-143` |
| REST overlay (config, média, pairing) | `server/src/components/overlay/overlayRoutes.ts:147-198,202-291,293-315,317+` |
| Client socket overlay | `overlay/src/main/services/socket-service.ts:63-186` |
| Écouteurs socket overlay | `overlay/src/main/services/socket-listeners-service.ts:40-109` |
| Types/traits partagés overlay | `overlay/src/shared/socket-events.ts:1-9`, `overlay/src/shared/overlay-config.ts:3-63` |
| Renderer playback | `overlay/src/renderer/overlay/legacy-playback-session-utils.ts:77-172` |
| Carte tweet riche | `server/src/services/messages/richOverlayPayload.ts:1-33,169-213` |

Conventions :

- **camelCase intégral** sur le fil (parité renderer) ; les noms Rust sont en `snake_case` et
  renommés par serde.
- Tous les exemples JSON de ce document sont **littéralement** les fixtures de
  `crates/lc-protocol/tests/fixtures/` (même ids, mêmes timestamps) : un test rond-point lit ces
  fichiers et vérifie qu'ils se ré-sérialisent à l'identique.
- Les chemins `/ws`, `/socket.io/`, `/overlay/...` sont relatifs à `API_URL`.
- Les timestamps sont en **millisecondes epoch UTC** ; `exp` des URLs média en **secondes**.
- Les nombres de durée (`durationSec`, `remainingMs`, `defaultMediaTime`, `maxMediaTime`) sont
  des entiers ; `remainingMs` est en millisecondes.

---

## 1. Transport, handshake et cycle de vie

### 1.1 Endpoint

- `GET {API_URL}/ws` en HTTP/1.1 `Upgrade: websocket` (pas de sous-protocole négocié, pas de
  chemin socket.io).
- Le serveur ne lit **aucun** token depuis l'URL, un header ou le handshake HTTP : l'auth se fait
  dans la première trame `hello` (v1 utilisait `handshake.auth.token` ou `handshake.query.token`,
  `server/src/loaders/socketLoader.ts:114-172` — les deux sont abandonnés).
- Trames **texte UTF-8 JSON uniquement**. Une trame binaire → fermeture `1003`.
- Le client se connecte à `{API_URL}` avec `API_URL` tel qu'obtenu du pairing
  (`apiBaseUrl`, `server/src/components/overlay/overlayRoutes.ts:202-291`).
- `TRUST_PROXY_HEADERS=true` indique au serveur de lire l'IP réelle derrière un reverse proxy
  (utile au rate limit ; variable listée `docs/rust/migration-plan.md:150`).

### 1.2 Séquence de handshake applicatif

1. Ouverture WebSocket.
2. Dans les **5 s** (`HELLO_TIMEOUT_MS = 5000`), le client envoie **exactement** une trame
   `hello` :
   `{ token, appVersion, clientId?, sessionMode? }`.
   - `token` = `clientToken` 64 caractères hex du pairing (v1 : même token, `overlayAuth.ts:40-42`).
   - `appVersion` = version sémantique de l'overlay (`3.0.0`).
   - `clientId` = facultatif ; s'il est fourni, il doit correspondre au client résolu par le
     token, sinon fermeture `4401` (`invalid_token`).
   - `sessionMode` = croyance locale du client (`normal` | `invite_read_only`), fournie par
     `getConfig()`/`guestMode` (v1 envoyait la même valeur dans `handshake.auth.sessionMode`,
     `overlay/src/main/services/socket-service.ts:145-148`). **Informationnelle** : le serveur ne
     l'utilise pas pour l'autorisation (voir §9).
3. Le serveur authentifie : `SHA-256(token)` (hex minuscule) comparé à
   `overlay_clients.token_hash` avec `revoked_at IS NULL` (v1 : `overlayAuth.ts:36-38,116-143`).
   - token vide → `4401` (`missing_token`)
   - hash inconnu → `4401` (`invalid_token`)
   - hash trouvé mais `revoked_at` non nul → `4402` (`revoked`)
   - erreur DB/interne → `1011` (`internal_error`)
4. Le serveur envoie `welcome` puis immédiatement `settings` (même config, idempotent), diffuse
   `peers` à la room de guilde et démarre le heartbeat applicatif.
5. Le client envoie un `heartbeat` **immédiatement** après `welcome`, puis toutes les 15 s
   (v1 : `overlay/src/main/services/socket-listeners-service.ts:41-46`,
   `socket-service.ts:83-89`).

Trames autorisées avant `welcome` : `hello` uniquement. Toute autre trame → fermeture `1002`
(`protocol_error`). Un second `hello` après authentification → `1002`.

### 1.3 Liveness : ping/pong WebSocket + heartbeat applicatif

- Le **serveur** envoie un frame de contrôle `Ping` toutes les **15 s** ; le client doit répondre
  `Pong`. Sans `Pong` (ni aucune trame) pendant **45 s** (`WS_HEARTBEAT_TIMEOUT_SEC=45`), le
  serveur ferme avec `1001` (`heartbeat_timeout`). Le client n'émet pas de `Ping` applicatif.
- `heartbeat` applicatif (C→S) : toutes les 15 s, sert à la présence (`lastSeenAt`, parité v1
  `server/src/loaders/socketLoader.ts:199-219`) et au diagnostic du mode. Il ne remplace pas le
  ping/pong : un client qui envoie des `heartbeat` mais ne répond pas aux `Ping` est fermé.
- Le client peut envoyer d'autres messages à la place d'un `heartbeat` ; la présence est rafraîchie
  par toute trame valide.

### 1.4 Compatibilité v1 (`/socket.io/`)

- Toute requête sous `{API_URL}/socket.io/` (polling ou upgrade) reçoit **HTTP 426
  Upgrade Required** avec :
  ```json
  {"error":"overlay_outdated","protocolVersion":2,"minOverlayVersion":"3.0.0"}
  ```
  `minOverlayVersion` reflète l'env `MIN_OVERLAY_VERSION` (défaut `"3.0.0"`, §1.6).
- `GET {API_URL}/ws` **sans** `Upgrade: websocket` → `426` avec `{"error":"websocket_required"}`.
- Ces réponses permettent à un overlay v1 (socket.io) d'afficher « version obsolète, mettez à
  jour » au lieu de boucler silencieusement (plan : `docs/rust/migration-plan.md:71`).

### 1.5 Cycle de vie d'une connexion

```
client                                   serveur
  |--- WS open --------------------------->|
  |--- hello ----------------------------->|  (≤ 5 s, sinon 4401 auth_timeout)
  |<--------------------------- welcome ---|
  |<-------------------------- settings ---|
  |<----------------------------- peers ---|
  |--- heartbeat ------------------------->|  (toutes les 15 s)
  |<--- ping ------------------------------|  (toutes les 15 s)
  |--- pong ------------------------------>|
  |<------------------------------ play ---|  (media signé pour CE client)
  |--- playback.state -------------------->|  (≤ 1/s)
  |--- playback.ended -------------------->|  (fin locale d'un job)
  |<------------------------------ stop ---|  (manual/preempt/revoked/purge)
  |<------------------------------ ack ----|  (réponse aux requêtes)
  |<-------------------------- error -----|  (non fatal)
```

Ordre garanti : les messages S→C sont livrés dans l'ordre d'émission (WebSocket). Un `play` ne
précède jamais le `welcome` correspondant ; un `stop` ne précède jamais le `play` du job arrêté.

### 1.6 Versions d'application (`serverVersion`, `MIN_OVERLAY_VERSION`)

- `welcome.serverVersion` (string) : version sémantique du serveur, **toujours émise** (ex.
  `"3.0.0"`). Elle informe l'overlay, sans contrôle de version serveur côté client.
- `hello.appVersion` (string) : version de l'overlay. Le serveur la compare à
  `MIN_OVERLAY_VERSION` (env, défaut `"3.0.0"`).
- Règle de comparaison gelée (rév. 2) : analyse **semver simple** `major.minor.patch` — suffixes
  de pré-version (`-beta.1`) et de build (`+sha`) ignorés, les trois composantes doivent être
  numériques. Comparaison lexicographique `(major, minor, patch)`. Une version **illisible** est
  traitée comme obsolète (refus : plus sûr qu'une acceptation silencieuse).
- Si `appVersion < MIN_OVERLAY_VERSION` : close `4403` avec `reason` JSON
  `{"minVersion":"<x.y.z>"}` — aucun `welcome`, aucune autre trame. Le client affiche la version
  minimale et **ne retente pas** (§10).
- Le même `MIN_OVERLAY_VERSION` alimente `minOverlayVersion` du corps HTTP 426 de §1.4.

---

## 2. Enveloppe

Chaque trame est **un objet JSON unique** :

```json
{
  "v": 2,
  "type": "play",
  "id": "srv-play-0001",
  "ts": 1794182400000,
  "payload": { }
}
```

| Clé | Type | Règle |
|---|---|---|
| `v` | entier | **2**. Toute autre valeur sur la première trame → `4403` (`reason` JSON `{"minVersion":"x.y.z"}`, §1.6) ; sur une trame ultérieure → `4403` également (protocole non négociable). |
| `type` | chaîne | Nom exact du message (voir §5/§6). Type inconnu → `1002` (`protocol_error`). |
| `id` | chaîne | Non vide, ≤ 64 caractères. Unique par connexion et par sens (UUID v4 recommandé côté client ; le serveur génère les siens). `ack.ref` référence l'`id` de la requête. |
| `ts` | entier | Epoch ms UTC de l'émission. Côté client, informatif : le serveur utilise sa propre horloge. |
| `payload` | objet | Obligatoire (objet vide `{}` autorisé). Absent/null/tableau → `1002`. |

Règles générales :

- Trame > **64 Kio** → `4408` (`frame_too_large`). Les messages S→C sont garantis < 64 Kio.
- JSON invalide, objet non unique, clé manquante, `payload` non-objet → `1002`
  (`protocol_error`, `close_reason` précisé si possible).
- **Champs inconnus** dans `payload` : tolérés et ignorés (compatibilité ascendante douce). Le
  crate `lc-protocol` conserve ces champs dans `extra` pour les structures `play`/`tweetCard`
  afin qu'un aller-retour serde reste fidèle.
- Aucune sérialisation d'`undefined` : les champs optionnels réellement absents sont **omis**
  (pas de `null` ajouté), sauf ceux explicitement décrits « toujours émis, `null` autorisé »
  ci-dessous (`media`, `tweetCard`, `image`, `authorUrl`, `caption`, `maxMediaTime`,
  `remainingMs`, `jobId` de `stop`).

---

## 3. Codes de fermeture

| Code | Nom Rust | Sens | `reason` (texte ASCII) | Reconnexion client |
|---|---|---|---|---|
| 1000 | `NORMAL` | fermeture propre demandée par le client | `client_shutdown` | non (arrêt volontaire) |
| 1001 | `GOING_AWAY` | arrêt/redémarrage serveur, ou timeout de liveness | `server_shutdown`, `heartbeat_timeout` | oui (backoff §10) |
| 1002 | `PROTOCOL_ERROR` | trame/état invalide (JSON, type inconnu, second hello, état interdit) | `protocol_error`, `bad_playback_state` | oui, mais bug à corriger |
| 1003 | `UNSUPPORTED_FRAME` | trame binaire | `unsupported_frame` | non |
| 1011 | `INTERNAL_ERROR` | erreur interne inattendue (DB, dispatch) | `internal_error` | oui (backoff) |
| **4401** | `UNAUTHORIZED` | auth impossible : token absent/invalide, `hello` trop tardif ou absent, clientId incohérent | `missing_token`, `invalid_token`, `auth_timeout`, `hello_required` | non (sauf config modifiée) |
| **4402** | `REVOKED` | token révoqué (`revoked_at` non nul) ou révocation pendant la connexion | `revoked` | non (re-pairing requis) |
| **4403** | `OUTDATED_PROTOCOL` | `v` ≠ 2, `hello` v1, `appVersion` < `MIN_OVERLAY_VERSION` | JSON `{"minVersion":"x.y.z"}` (§1.6) | non (mise à jour overlay) |
| 4408 | `FRAME_TOO_LARGE` | trame > 64 Kio | `frame_too_large` | oui, bug à corriger |
| 4409 | `RATE_LIMITED` | > 20 trames C→S dans la seconde glissante | `rate_limited` | oui (backoff) |

Les codes 4401/4402/4403/1011 sont exigés par le plan (`docs/rust/migration-plan.md:68`).
4408/4409/1002/1003 sont des précisions de cette spec (comportement nécessaire des limites §4).
Pour `4403`, la `reason` de fermeture est **le JSON compact** `{"minVersion":"<x.y.z>"}` (UTF-8,
< 123 octets), et non le texte `outdated_protocol` ; c'est la seule exception au tableau.
Le client mappe :
- `4401` → état `error` avec message « jeton invalide » ;
- `4402` → état terminal « appareil révoqué » (pas de retry, re-pairing) ;
- `4403` → état terminal « overlay obsolète » (pas de retry, mise à jour) ; le client **parse la
  `reason` JSON** pour afficher `minVersion` ;
- `1001`/`1011`/`4409`/`1002` → état `reconnecting` avec backoff §10.

---

## 4. Limites et quotas

| Limite | Valeur | Application |
|---|---|---|
| Taille de trame | **64 Kio (65 536 octets)** | C→S et S→C ; dépassement C→S → close `4408` |
| Débit C→S | **20 messages/s** fenêtre glissante 1 s | dépassement → close `4409` |
| Délai `hello` | **5 s** après l'ouverture | sinon close `4401` (`auth_timeout`) |
| `playback.state` | **≤ 1 msg/s** par connexion | excédent ignoré (pas de close), v1 : `socketLoader.ts:229-254` + plan |
| Heartbeat applicatif | **15 s** | présentiel ; `lastSeenAt` |
| Ping WS serveur | **15 s** | contrôle |
| Timeout pong/liveness | **45 s** (`WS_HEARTBEAT_TIMEOUT_SEC`) | close `1001` |
| `peers.request` | ≤ 1 par 2 s | excédent ignoré |
| TTL URL média | image ou durée inconnue : **300 s** ; audio/vidéo : `clamp(durationSec + 120, 300, 3600)` | voir §8.1 |
| Version overlay minimale | `MIN_OVERLAY_VERSION` (défaut `"3.0.0"`, semver simple) | close `4403` — voir §1.6 |
| `remainingMs` accepté | entier fini, borné **[0, 86 400 000]** | sinon `null` (v1 : clamp identique `socketLoader.ts:229-254`) |
| Longueur texte de mème | 500 caractères max (tronqué serveur) | v1 : `socketLoader.ts:277-338` |
| `id` de message | ≤ 64 caractères | sinon `1002` |

Le serveur ne ferme pas pour un `heartbeat` en retard tant que le transport répond ; seul le
timeout de 45 s ferme.

---

## 5. Messages client → serveur (C→S)

| `type` | `payload` | Réponse serveur associée |
|---|---|---|
| `hello` | `HelloPayload` | `welcome` + `settings` + `peers` |
| `heartbeat` | `HeartbeatPayload` | aucune (met à jour la présence) |
| `playback.state` | `PlaybackStatePayload` | aucune |
| `playback.ended` | `PlaybackEndedPayload` | aucune |
| `error` | `ClientErrorPayload` | aucune (log) |
| `stop.request` | `StopRequestPayload` (`{}`) | `ack` puis `stop` (reason `manual`) |
| `meme.trigger` | `MemeTriggerPayload` | `ack` puis `stop`(preempt)+`play` si accepté |
| `peers.request` | `PeersRequestPayload` (`{}`) | `ack` puis `peers` |

### 5.1 `hello`

```json
{"v":2,"type":"hello","id":"cli-hello-0001","ts":1794182400000,"payload":{"token":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","appVersion":"3.0.0","clientId":"clx7q2x0b0000abcd1234efgh","sessionMode":"normal"}}
```

Variantes : `v2_hello_invite_read_only.json` (`sessionMode:"invite_read_only"`),
`v2_hello_minimal.json` (sans `clientId` ni `sessionMode` — champs omis).

### 5.2 `heartbeat`

```json
{"v":2,"type":"heartbeat","id":"cli-heartbeat-0001","ts":1794182400000,"payload":{"clientId":"clx7q2x0b0000abcd1234efgh","guildId":"123456789012345678","appVersion":"3.0.0","sessionMode":"normal"}}
```

Parité v1 exacte (`overlay-protocol/src/index.ts:54-59`, émission
`socket-service.ts:63-74`).

### 5.3 `playback.state`

```json
{"v":2,"type":"playback.state","id":"cli-state-0001","ts":1794182400000,"payload":{"jobId":"clx7q2x0b0001abcd1234efgh","state":"playing","remainingMs":84000}}
```

- `state` ∈ `playing` | `paused`. **`ended` n'existe plus** : la fin locale se signale par
  `playback.ended` (§5.4). Une valeur inconnue → close `1002` (`bad_playback_state`).
- `remainingMs` : entier ou `null` ; le serveur borne à `[0, 86 400 000]` puis met à jour
  `busyUntil = now + max(remainingMs, 5000) + 250` et le watchdog (v1 :
  `playbackScheduler.ts:178-237`).
- Throttle 1/s : les trames excédentaires sont ignorées silencieusement.

### 5.4 `playback.ended`

```json
{"v":2,"type":"playback.ended","id":"cli-ended-0001","ts":1794182400000,"payload":{"jobId":"clx7q2x0b0001abcd1234efgh","reason":"completed"}}
```

- `reason` ∈ `completed` | `manual` | `error`.
- Mapping serveur : `completed`/`error` → `onPlaybackEnded` (v1 `playbackScheduler.ts:294-346`) ;
  `manual` → `onPlaybackStopped` (v1 `playbackScheduler.ts:239-292`). Libération **idempotente**
  (si le job n'est plus `PLAYING`, sans effet, pas d'ack, pas d'erreur).
- Remplace `overlay:playback-state {state:'ended'}` **et** `overlay:stop {jobId}` émis par le
  renderer en fin de session (v1 : `legacy-playback-session-utils.ts:154-172`,
  `socket-service.ts:119-123`). Ne jamais envoyer `jobId:"manual-stop"` (sentinelle v1
  supprimée) : utiliser `stop.request`.

### 5.5 `error`

```json
{"v":2,"type":"error","id":"cli-error-0001","ts":1794182400000,"payload":{"jobId":"clx7q2x0b0001abcd1234efgh","code":"render_failed","message":"Impossible de lire le média (MEDIA_ERR_SRC_NOT_SUPPORTED)."}}
```

Parité v1 (`index.ts:61-65`) ; seul code produit par le renderer actuel : `render_failed`
(`legacy-play-flow-utils.ts:102-106`). Le serveur journalise (v1 :
`socketLoader.ts:221-227`) et ne modifie aucun état : le watchdog libère le job.

### 5.6 `stop.request`

```json
{"v":2,"type":"stop.request","id":"cli-stop-request-0002","ts":1794182400000,"payload":{"jobId":"clx7q2x0b0001abcd1234efgh","source":"user"}}
```

Payload (rév. 2, §15.2) : `{ "jobId"?: string, "source"?: "user"|"hotkey"|"admin" }`.
`jobId` absent ou `null` = arrêt de **tous** les jobs de la guilde (la fixture
`v2_stop_request.json` porte `{}`) ; `source` est purement informatif (statistiques), défaut
serveur `user`. Demande d'arrêt manuel déclenchée par l'overlay/le board (v1 : `stopMemePlayback`
émettait `overlay:stop {jobId:'manual-stop'}`, `meme-bindings-service.ts:51-76`). Serveur : si mode
lecture seule → `ack {ok:false, error:{code:"read_only"}}` sans arrêt ; sinon
`executeManualStopForGuild` (v1 `manualStop.ts:9-60`) ciblé sur `jobId` s'il est fourni → `ack
{ok:true}` puis `stop {reason:"manual", jobId}` à la room. Une demande par seconde max (excédent →
ack `rate_limited`).

### 5.7 `meme.trigger`

```json
{"v":2,"type":"meme.trigger","id":"cli-meme-trigger-0001","ts":1794182400000,"payload":{"itemId":"clx7q2x0b0004abcd1234efgh","trigger":"shortcut"}}
```

Parité v1 (`index.ts:73-76`, `socketLoader.ts:277-338`). `trigger` ∈ `shortcut` | `ui`.
Serveur : mode lecture seule → ack `read_only` ; item/média absent ou non `READY` → ack
`item_not_found`/`media_not_ready` ; sinon crée un job priorité 100
(`text = item.message.trim().slice(0, 500)`, `showText = !!message`,
`authorName = session.authorName || item.createdByName`, `authorImage = session.authorImage`),
`ack {ok:true}`, puis préemption : `stop {reason:"preempt"}` du job actif puis `play` du mème
(v1 : `suspendPlayingJobForPreemption`, `playbackScheduler.ts:922-997`).

### 5.8 `peers.request`

```json
{"v":2,"type":"peers.request","id":"cli-peers-request-0001","ts":1794182400000,"payload":{}}
```

Force un rafraîchissement : `ack {ok:true}` puis `peers` à la room. Utile au tray overlay et après
reconnexion (v1 : broadcast uniquement à la connexion/déconnexion, `socketLoader.ts:76-86`).

---

## 6. Messages serveur → client (S→C)

| `type` | `payload` | Déclencheur |
|---|---|---|
| `welcome` | `WelcomePayload` | handshake réussi |
| `settings` | `SettingsPayload` | juste après `welcome`, puis à chaque changement de config/mode |
| `peers` | `PeersPayload` | connexion/déconnexion d'un overlay, `peers.request`, gracieux |
| `play` | `PlayPayload` | dispatch d'un job (media signé par client) |
| `stop` | `StopPayload` | arrêt manuel, préemption, révocation, purge |
| `ack` | `AckPayload` | réponse à `stop.request`, `meme.trigger`, `peers.request` |
| `error` | `ServerErrorPayload` | erreur non fatale (dispatch, média indisponible) |

### 6.1 `welcome`

```json
{"v":2,"type":"welcome","id":"srv-welcome-0001","ts":1794182400000,"payload":{"protocolVersion":2,"guildId":"123456789012345678","clientId":"clx7q2x0b0000abcd1234efgh","sessionMode":"normal","serverVersion":"3.0.0","serverTime":1794182400000,"config":{"showTextDefault":true,"defaultMediaTime":30,"maxMediaTime":600}}}
```

- `protocolVersion` = **2** (entier). v1 exposait `protocolVersion:"1.2.0"` en REST
  (`index.ts:102-108`) : le champ devient numérique et la version de protocole n'est plus une
  version sémantique d'application.
- `sessionMode` : **autorité serveur** (normalisé `normal`|`invite_read_only`, v1
  `overlayAuth.ts:28-34`).
- `serverVersion` : version sémantique du serveur (rév. 2, §1.6/§15.5), toujours émise.
- `serverTime` : epoch ms serveur (alignement des compte à rebours).
- `config` : mêmes clés que la réponse REST v1 `GET /overlay/config`
  (`overlayRoutes.ts:293-315`), endpoint **supprimé** en v2 (§6.5, §15.4) : `showTextDefault:true`,
  `defaultMediaTime` (sec, `guild.defaultMediaTime ?? env.DEFAULT_DURATION`), `maxMediaTime`
  (sec ou `null`).
- `currentJob` : **présent uniquement s'il y a un job `PLAYING`** au moment du welcome
  (reconnexion, §10). Absent sinon (ne pas émettre `null`). Voir §6.1.1. Invariant serveur : au
  plus **un** job `PLAYING` par guilde (§6.1.1).

#### 6.1.1 `currentJob` (reprise après reconnexion)

```json
{"v":2,"type":"welcome","id":"srv-welcome-0002","ts":1794182424000,"payload":{"protocolVersion":2,"guildId":"123456789012345678","clientId":"clx7q2x0b0000abcd1234efgh","sessionMode":"normal","serverVersion":"3.0.0","serverTime":1794182424000,"config":{"showTextDefault":true,"defaultMediaTime":30,"maxMediaTime":600},"currentJob":{"jobId":"clx7q2x0b0001abcd1234efgh","media":{"assetId":"clx7q2x0b0003abcd1234efgh","url":"https://api.livechat.example/overlay/media/clx7q2x0b0003abcd1234efgh?clientId=clx7q2x0b0000abcd1234efgh&exp=1794182700&sig=9f2c4e6a8b1d3f50791b2d4e6f8a0c1e3b5d7f9012a4c6e8b0d2f4a6c8e0b2d4","mime":"video/mp4","kind":"video","durationSec":120,"isVertical":true,"startOffsetSec":12},"text":{"value":"Extrait du direct","enabled":true},"author":{"name":"SuperKnacky","image":null,"enabled":true},"tweetCard":null,"durationSec":108,"startedAt":1794182400000,"remainingMs":84000}}}
```

`currentJob` = **exactement** un `PlayPayload` + deux champs de reprise :

| Champ | Type | Sens |
|---|---|---|
| `startedAt` | entier ms | début de lecture de ce job (horloge serveur) |
| `remainingMs` | entier ms | restant estimé au moment du welcome |

Formule client de reprise : `elapsedSec = max(0, durationSec - ceil(remainingMs/1000))` puis
`seekSec = (media.startOffsetSec ?? 0) + elapsedSec`. Si `currentJob` est absent : le client
efface tout état de lecture local et attend le prochain `play`.

**Invariant (rév. 2, §15.3)** : au plus **un** job `PLAYING` par guilde, garanti par le scheduler
(Phase 4). Si le serveur détecte malgré tout plusieurs jobs `PLAYING` (anomalie de données), il
choisit celui dont `startedAt` est le plus récent et journalise un `warn` — il n'émet qu'un seul
`currentJob`. Aucun changement de type n'est associé à cette décision.

### 6.2 `play`

```json
{"v":2,"type":"play","id":"srv-play-0001","ts":1794182400000,"payload":{"jobId":"clx7q2x0b0001abcd1234efgh","media":{"assetId":"clx7q2x0b0003abcd1234efgh","url":"https://api.livechat.example/overlay/media/clx7q2x0b0003abcd1234efgh?clientId=clx7q2x0b0000abcd1234efgh&exp=1794182700&sig=9f2c4e6a8b1d3f50791b2d4e6f8a0c1e3b5d7f9012a4c6e8b0d2f4a6c8e0b2d4&startOffsetSec=12#t=12","mime":"video/mp4","kind":"video","durationSec":120,"isVertical":true,"startOffsetSec":12},"text":{"value":"Extrait du direct","enabled":true},"author":{"name":"SuperKnacky","image":null,"enabled":true},"tweetCard":null,"durationSec":108}}
```

Le payload `play` est **strictement** celui de v1 (`index.ts:26-48`,
`messagesWorker.ts:44-113`, copie scheduler `playbackScheduler.ts:48-117`), à trois exceptions
près :

1. `media.url` est une **URL signée** (§8), valable 300 s, signée pour le `clientId` de la
   connexion destinataire — donc `play` est émis **par socket** (`socket.emit`), pas en broadcast
   de room.
2. `media.startOffsetSec` reste **toujours émis** (entier ou `null`) comme en v1
   (`messagesWorker.ts:81-91`).
3. Les champs inconnus de `tweetCard` sont admis (voir §6.2.1).

Rappels de construction v1 conservés :
- `media.kind = asset.kind.toLowerCase()` (base en MAJUSCULES `IMAGE`/`AUDIO`/`VIDEO`) ;
  `media` = `null` si le job n'a pas de média (`messagesWorker.ts:81-91`).
- `media.durationSec` = durée de l'asset (ou `null`), `play.durationSec` = durée effective du job
  = `job.durationSec - startOffsetSec` pour un média non `IMAGE`, minimum 1
  (`messagesWorker.ts:26-42`).
- `text.value` = caption tweet si carte tweet, sinon caption riche média, sinon `job.text ?? ""` ;
  `text.enabled` = `caption.length > 0` si carte tweet, sinon `job.showText`
  (`messagesWorker.ts:66-72`).
- `author.enabled` = `false` si carte tweet, sinon `!!job.authorName` ; `author.name` =
  `job.authorName ?? ""` ; `author.image` = `job.authorImage ?? null` (`messagesWorker.ts:100-104`).
- `tweetCard` toujours émis : objet ou `null` (`messagesWorker.ts:105-110`).

#### 6.2.1 `tweetCard` riche (parité de forme, clés optionnelles tolérées)

Le type v1 `OverlayTweetCard` (`index.ts:18-24`) ne décrit que 5 clés, mais le payload réellement
émis est le spread du `TweetCardPayload` serveur
(`server/src/services/messages/richOverlayPayload.ts:3-19`) : `url`, `html`, `authorName`,
`authorUrl` (nullable), `caption` (nullable, ajouté à l'émission), plus, optionnellement :
`currentStatusId`, `videoUrl`, `videoMime`, `videoIsVertical`, `videos[]`
(`{url, mime|null, isVertical|null, sourceStatusId|null, durationSec|null}`).
Le renderer, conservé, lit ces clés : la spec les **gèle** (parité obligatoire). Le crate
`lc-protocol` type les 5 clés de base et conserve les autres dans `extra` (round-trip exact).

Exemples : `v2_play_tweet_card.json` (forme minimale) et `v2_play_tweet_card_rich.json`
(avec `videoUrl`/`videos[]`).

### 6.3 `stop`

```json
{"v":2,"type":"stop","id":"srv-stop-0001","ts":1794182400000,"payload":{"reason":"manual","jobId":"clx7q2x0b0001abcd1234efgh"}}
```

`reason` :

| `reason` | Sens v1 | `jobId` |
|---|---|---|
| `manual` | arrêt utilisateur/commande/admin (v1 sentinelle `jobId:"manual-stop"` émise par `manualStop.ts:9-60`, consommée par `socketLoader.ts:256-275`) | id du job `PLAYING` si unique, sinon `null` |
| `preempt` | job suspendu pour laisser passer un mème (v1 `STOP {jobId: activeJob.id}`, `playbackScheduler.ts:922-997`) | id du job suspendu (toujours connu) |
| `revoked` | jeton révoqué (v1 : simple `socket.disconnect(true)`, `socketLoader.ts:88-104` ; admin `adminRoutes.ts:2884-2892`) | `null` ; fermeture `4402` juste après |
| `purge` | purge admin de la guilde (v1 : arrêt manuel loggué « Admin panel purge », `adminRoutes.ts:2747,2776-2831`) | `null` |

Comportement client sur `stop` : vider immédiatement média/texte/compte à rebours, **sans**
envoyer `playback.ended` en réponse (l'état serveur est déjà libéré ; une émission tardive serait
idempotente mais inutile). En mode lecture seule, `stop` reste reçu et appliqué (c'est le serveur
qui décide).

### 6.4 `peers`

```json
{"v":2,"type":"peers","id":"srv-peers-0001","ts":1794182400000,"payload":{"guildId":"123456789012345678","peers":[{"clientId":"clx7q2x0b0000abcd1234efgh","label":"Overlay salon"},{"clientId":"clx7q2x0b0009abcd1234efgh","label":"unknown-device"}]}}
```

Parité v1 (`index.ts:78-86`) : dédupliqué par `clientId`, `label = authorLabel || deviceLabel ||
'unknown-device'`, trié par `label` puis `clientId` (`socketLoader.ts:76-86`). `v2_peers_empty.json`
représente une room sans overlay.

### 6.5 `settings`

```json
{"v":2,"type":"settings","id":"srv-settings-0001","ts":1794182400000,"payload":{"guildId":"123456789012345678","sessionMode":"normal","showTextDefault":true,"defaultMediaTime":30,"maxMediaTime":600}}
```

- Émis juste après `welcome` puis à chaque changement : configuration de guilde
  (`defaultMediaTime`/`maxMediaTime`/`showTextDefault`) ou `sessionMode` du client.
- `GET /overlay/config` est **supprimé** du protocole v2 (rév. 2, §15.4) : la config n'est
  délivrée que par `welcome.config` puis `settings`. `/overlay/pair/consume` renvoie uniquement
  `clientToken`, `guildId` et `sessionMode` ; l'overlay ne fait plus aucune requête REST de config
  (`overlayRoutes.ts:293-315` pour l'ancien contrat v1).
- `showTextDefault` v1 est constant `true` ; conservé pour parité.
- Client : apply idempotent (les `settings` peuvent être dupliqués).

### 6.6 `ack`

```json
{"v":2,"type":"ack","id":"srv-ack-0001","ts":1794182400000,"payload":{"ref":"cli-stop-request-0001","ok":true}}
```

```json
{"v":2,"type":"ack","id":"srv-ack-0003","ts":1794182400000,"payload":{"ref":"cli-meme-trigger-0002","ok":false,"error":{"code":"read_only","message":"Session en lecture seule : déclenchement refusé."}}}
```

- `ref` = `id` de la trame C→S acquittée.
- `ok:true` → `error` omis. `ok:false` → `error {code, message}`.
- Codes d'erreur d'ack : `read_only`, `item_not_found`, `media_not_ready`, `invalid_payload`,
  `rate_limited`, `internal_error`.
- Les messages de flux (`playback.state`, `playback.ended`, `heartbeat`, `error`) ne sont
  **jamais** acquittés (économie de trames) ; seules les requêtes (`stop.request`,
  `meme.trigger`, `peers.request`) le sont.

### 6.7 `error` (S→C)

```json
{"v":2,"type":"error","id":"srv-error-0001","ts":1794182400000,"payload":{"code":"dispatch_failed","message":"Aucun overlay actif n'a pu charger le média.","jobId":"clx7q2x0b0001abcd1234efgh"}}
```

Erreur **non fatale** : la connexion reste ouverte. `ref` et `jobId` sont facultatifs (omis si
absents). Codes : `dispatch_failed`, `media_unavailable`, `internal_error`. Utilisation : le
serveur signale un échec de dispatch/chargement média que l'overlay peut afficher discrètement
(v1 n'avait aucun équivalent S→C : l'`overlay:error` v1 est C→S uniquement
`server/src/types/module.d.ts:38-61`). Le client ne doit jamais fermer ni réessayer à cause de ce
message.

---

## 7. Structures de payload — référence

Notation : `T?` = champ optionnel omis si absent ; `T|null` = toujours émis, `null` autorisé ;
`T[]` = tableau JSON.

### 7.1 `HelloPayload`
`{ token: string, appVersion: string, clientId?: string, sessionMode?: SessionMode }`
(v1 : `handshake.auth.token` + `handshake.auth.sessionMode`, `socket-service.ts:145-148` ;
`appVersion` = `getAppVersion()` v1 heartbeat `socket-service.ts:63-74`).

### 7.2 `HeartbeatPayload`
`{ clientId: string, guildId: string, appVersion: string, sessionMode?: SessionMode|null }`
(`index.ts:54-59`).

### 7.3 `PlaybackStatePayload`
`{ jobId: string, state: "playing"|"paused", remainingMs: number|null }`
(`index.ts:67-71`, `legacy-playback-session-utils.ts:77-152` : émission immédiate `playing` puis
1 Hz).

### 7.4 `PlaybackEndedPayload`
`{ jobId: string, reason: "completed"|"manual"|"error" }` (remplace la fin v1
`state:"ended"` + `overlay:stop`, `legacy-playback-session-utils.ts:154-172`).

### 7.5 `ClientErrorPayload`
`{ jobId: string, code: string, message: string }` (`index.ts:61-65`).

### 7.6 `StopRequestPayload` / `PeersRequestPayload`
`StopRequestPayload = { jobId?: string, source?: "user"|"hotkey"|"admin" }` (rév. 2) : `jobId`
absent/null = tous les jobs de la guilde ; `source` informatif, défaut serveur `user`.
`v2_stop_request.json` = `{}` ; `v2_stop_request_user.json` = `{jobId, source:"user"}`.
`PeersRequestPayload = {}` (objet vide obligatoire).

### 7.7 `MemeTriggerPayload`
`{ itemId: string, trigger: "shortcut"|"ui" }` (`index.ts:73-76`,
`meme-bindings-service.ts:78-116`).

### 7.8 `WelcomePayload`
`{ protocolVersion: 2, guildId: string, clientId: string, sessionMode: SessionMode,
serverVersion: string, serverTime: number, config: WelcomeConfig, currentJob?: CurrentJob }`
`WelcomeConfig = { showTextDefault: boolean, defaultMediaTime: number, maxMediaTime: number|null }`
`CurrentJob = PlayPayload & { startedAt: number, remainingMs: number }`
`serverVersion` (rév. 2) : semver simple du serveur, toujours émis ; `currentJob` : au plus un job
`PLAYING` par guilde (§6.1.1). (v1 config : `overlayRoutes.ts:293-315`, endpoint supprimé §15.4.)

### 7.9 `PlayPayload`
```
{
  jobId: string,
  media: MediaPayload|null,
  text: { value: string, enabled: boolean },
  author: { name: string, image: string|null, enabled: boolean },
  tweetCard: TweetCardPayload|null,
  durationSec: number,
  ...champs inconnus tolérés
}
MediaPayload = {
  assetId: string, url: string, mime: string,
  kind: "image"|"audio"|"video",
  durationSec: number|null, isVertical: boolean,
  startOffsetSec: number|null
}
TweetCardPayload = {
  url: string, html: string, authorName: string,
  authorUrl: string|null, caption: string|null,
  currentStatusId?, videoUrl?, videoMime?, videoIsVertical?, videos?: TweetCardVideo[],
  ...champs inconnus tolérés
}
TweetCardVideo = { url: string, mime: string|null, isVertical: boolean|null,
                   sourceStatusId: string|null, durationSec: number|null }
```
(v1 `index.ts:18-48`, `messagesWorker.ts:44-113`, carte riche
`richOverlayPayload.ts:3-19`).

### 7.10 `StopPayload`
`{ reason: "manual"|"preempt"|"revoked"|"purge", jobId: string|null }`.

### 7.11 `PeersPayload`
`{ guildId: string, peers: { clientId: string, label: string }[] }` (`index.ts:78-86`).

### 7.12 `SettingsPayload`
`{ guildId: string, sessionMode: SessionMode, showTextDefault: boolean,
defaultMediaTime: number, maxMediaTime: number|null }`.

### 7.13 `AckPayload`
`{ ref: string, ok: boolean, error?: { code: string, message: string } }`.

### 7.14 `ServerErrorPayload`
`{ code: string, message: string, jobId?: string, ref?: string }`.

### 7.15 `SessionMode`
`"normal" | "invite_read_only"` (`index.ts:16`).

---

## 8. URLs média signées (HMAC)

### 8.1 Format

```
{API_URL}/overlay/media/{assetId}?clientId={clientId}&exp={exp}&sig={sig}[&startOffsetSec={n}][#t={n}]
```

- `exp` : epoch **secondes** UTC = `now + ttl`, avec (rév. 2, §15.1) :
  - `ttl = 300` s pour une **image** (ou une durée inconnue) ;
  - `ttl = clamp(durationSec + 120, 300, 3600)` pour une **vidéo/audio** dont `durationSec` est
    connu.
  Constantes : `MEDIA_URL_TTL_SEC=300` (plancher), `MEDIA_URL_EXPIRY_MARGIN_SEC=120`,
  `MEDIA_URL_TTL_MAX_SEC=3600` (plafond).
- `sig` : `HMAC-SHA256(MEDIA_URL_SECRET, "assetId|clientId|exp")` en **hex minuscule** (64
  caractères). Message signé exactement `{assetId}|{clientId}|{exp}`, sans espaces, `exp` en
  décimal.
- `clientId` : id du client destinataire ; **la signature lie l'URL à ce client**.
- `startOffsetSec` et le fragment `#t=` reproduisent l'URL v1
  (`messagesWorker.ts:74-79`) pour le renderer conservé ; ils ne sont **pas** couverts par la
  signature (impact uniquement sur le seek local, pas sur l'autorisation).
- `MEDIA_URL_SECRET` : secret serveur (env, `docs/rust/migration-plan.md:150`), jamais exposé.
- Le jeton `?token=` v1 est supprimé ; les URLs signées sont la seule authentification du média.

### 8.2 Émission

- `play` et `welcome.currentJob` portent une URL signée **par client** : ces deux messages sont
  donc émis **socket par socket** (`clientId` du socket) et non en broadcast de room.
- À chaque émission de `play` (y compris reprise après préemption ou reconnexion), `exp` est
  recalculé : un job suspendu longtemps est re-signé au moment du dispatch.
- `stop`, `peers`, `settings`, `welcome` (hors `currentJob`) ne contiennent pas d'URL et peuvent
  être diffusés à la room.
- Un `HEAD`/`Range` doit arriver **avant `exp`**. Une pause prolongée au-delà du TTL n'est **pas**
  supportée en v2 : le client reprend la lecture avec une **nouvelle** URL signée émise par un
  nouveau dispatch `play` ; l'URL existante n'est jamais prolongée silencieusement (rév. 2,
  §15.1).

### 8.3 Vérification serveur (`GET /overlay/media/:assetId`)

| Condition | Réponse |
|---|---|
| `clientId`/`exp`/`sig` absents ou malformés | `403 {"error":"media_url_invalid"}` |
| `sig` invalide | `403 {"error":"media_url_invalid"}` |
| `exp` dépassé | `403 {"error":"media_url_expired"}` |
| asset absent, expiré ou non `READY` | `404 {"error":"media_not_found"}` |
| fichier absent du disque | `404 {"error":"media_not_found_on_disk"}` |
| OK | `200`/`206` stream, `Cache-Control: no-store`, `Accept-Ranges: bytes`, `Content-Type: <mime>` |

Comparaison HMAC en temps constant. `HEAD` supporté (mêmes headers, sans corps). `Range`
invalide → `416` (parité v1 `overlayRoutes.ts:147-198,317+`).

> **Décision gelée (rév. 2, §15.1)** : TTL **adaptatif** (formule §8.1). Un `HEAD`/`Range` arrivant
> après `exp` reçoit `403 media_url_expired` et n'est **jamais** ré-autorisé en silence. Une pause
> ou un seek au-delà de `exp` n'est pas supporté : le serveur re-signe l'URL lors du prochain
> dispatch `play` (nouveau message, nouvelle URL), ce qui couvre les jobs longs sans allonger
> indéfiniment la fenêtre de validité.

---

## 9. Modes `normal` / `invite_read_only`

- Mode autoritaire = serveur (`overlay_clients.session_mode` / `sessionMode` du welcome et des
  `settings`), normalisé par `overlayAuth.ts:28-34`.
- `invite_read_only` : le serveur **refuse** `meme.trigger` et `stop.request` avec
  `ack {ok:false, error:{code:"read_only"}}`. Les autres messages restent acceptés : `heartbeat`,
  `playback.state`, `playback.ended`, `error`, `peers.request`, et les flux S→C (`play`, `stop`,
  `settings`, `peers`) continuent normalement.
- `hello.sessionMode` et `heartbeat.sessionMode` sont **diagnostiques** : le serveur journalise
  une divergence mais ne modifie pas le mode sur la foi du client (contrairement à v1 où le
  heartbeat écrasait `socket.data.overlaySessionMode`, `socketLoader.ts:199-219`). Un changement
  de mode est poussé par `settings`.
- L'overlay applique aussi localement le mode invité (désactivation préventive des raccourcis,
  v1 `meme-bindings-service.ts:51-116` : refus `guest_mode`), mais **la sécurité est serveur**.

---

## 10. Reconnexion et reprise

- Délais : `1 s → 2 s → 3 s → 3 s → …` (parité v1 socket.io
  `reconnectionDelay:1000, reconnectionDelayMax:3000`,
  `overlay/src/main/services/socket-service.ts:138-149`), sans jitter, compteur remis à 1 s après
  un `welcome` reçu.
- Fermetures `4402` et `4403` : **ne pas retenter** (état terminal, message utilisateur : appareil
  révoqué / overlay obsolète). `4401` : retenter avec backoff mais afficher l'erreur.
- `4403` (rév. 2) : la `reason` est le JSON `{"minVersion":"x.y.z"}` (§1.6) ; le client l'affiche
  et ne retente pas.
- `4409` (rate limit, précision rév. 2) : le client applique un **backoff exponentiel** (1 s, 2 s,
  4 s… plafonné à 30 s) et **jamais plus d'une tentative de reconnexion par seconde** — une boucle
  de reconnexion > 1/s est interdite (protection serveur et respect du quota 20 msg/s).
- À la reconnexion, le client :
  1. ouvre `/ws`, envoie `hello` (mêmes token/appVersion/clientId/sessionMode) ;
  2. attend `welcome` ; si `currentJob` présent, il **adopte** ce job (seek §6.1.1) et remet son
     compte à rebours à `remainingMs` ;
  3. si `currentJob` absent, il efface tout état de lecture local ;
  4. il **jette** les trames `playback.state`/`playback.ended` en attente d'envoi (v1 les
     rejouait après connexion, `socket-service.ts:105-124`) : le `welcome.currentJob` fait foi et
     un `playback.ended` périmé tuerait le job repris ;
  5. il envoie un `heartbeat`, puis reprend la boucle 15 s.
- Plusieurs sockets pour un même `clientId` sont tolérés (parité v1) : `peers` déduplique par
  `clientId`, `play`/`stop` atteignent tous les sockets de la room. Le serveur ne déconnecte pas
  l'ancien socket ; il disparaît par timeout `45 s` ou déconnexion du client.
- Redémarrage serveur : close `1001` (`server_shutdown`) → reconnexion 1 s, `welcome` sans
  `currentJob` (le scheduler repart de la base), état local effacé.
- Révocation pendant la connexion : le serveur envoie `stop {reason:"revoked", jobId:null}` puis
  ferme `4402` (v1 : `disconnectOverlayClient`, `socketLoader.ts:88-104`, appelé par
  l'admin/ingest après mise à jour de `revoked_at`).

---

## 11. Mapping v1 → v2

| v1 (socket.io / REST) | v2 | Delta de payload |
|---|---|---|
| `connect` avec `handshake.auth.{token,sessionMode}` + `connect_error` | `hello` → `welcome` + close 4401/4402/4403 | token déplacé dans la trame ; erreurs explicites |
| — (nouveau) | `hello.appVersion` < `MIN_OVERLAY_VERSION` → close `4403` + `{"minVersion":"x.y.z"}` | plancher de version overlay (§1.6) |
| `overlay:play` | `play` | mêmes clés ; `media.url` signée et par client |
| `overlay:stop` `{jobId}` | `stop` `{reason, jobId}` | `reason` ajouté ; sentinelle `manual-stop` supprimée |
| `overlay:heartbeat` | `heartbeat` | identique |
| `overlay:error` | `error` (C→S) | identique |
| `overlay:playback-state` (`playing`/`paused`/`ended`) | `playback.state` (`playing`/`paused`) + `playback.ended` | `ended` extrait dans un message dédié |
| `overlay:playback-stop` (renderer → serveur) | `playback.ended` | `reason` ajouté |
| `overlay:meme-trigger` | `meme.trigger` | identique |
| `overlay:peers` | `peers` | identique ; `peers.request` ajouté |
| `overlay:stop {jobId:'manual-stop'}` (client) | `stop.request {jobId?, source?}` | `jobId` ciblé, `source` informatif (rév. 2) |
| `overlay:stop {jobId:'manual-stop'}` (serveur) | `stop {reason:'manual', jobId}` | plus de sentinelle |
| révocation = `socket.disconnect(true)` | `stop {reason:'revoked'}` + close `4402` | explicite |
| `GET /overlay/config` (Bearer) | `welcome.config` + `settings` | endpoint **supprimé** (§15.4) ; `serverVersion` dans `welcome` |
| `GET /overlay/media/:assetId` (Bearer ou `?token=`) | URL signée `?clientId&exp&sig` | plus de jeton ; HMAC, TTL adaptatif §8.1 |
| reconnexion socket.io 1 s→3 s | reconnexion native 1 s→3 s + `welcome.currentJob` | reprise d'état ajoutée |
| `/socket.io/` accepté | `426 overlay_outdated` | échec rapide v1 |

---

## 12. Parité des clés JSON (camelCase conservé)

Aucune clé ne change de casse ; le renderer conservé lit les mêmes noms qu'en v1.

| Structure | Clés (camelCase) | Source v1 |
|---|---|---|
| Enveloppe | `v`, `type`, `id`, `ts`, `payload` | nouvelle |
| `hello` | `token`, `appVersion`, `clientId`, `sessionMode` | handshake.auth (`socket-service.ts:145-148`) |
| `play` | `jobId`, `media`, `text`, `author`, `tweetCard`, `durationSec` | `index.ts:26-48` |
| `play.media` | `assetId`, `url`, `mime`, `kind`, `durationSec`, `isVertical`, `startOffsetSec` | `index.ts:28-36` |
| `play.text` | `value`, `enabled` | `index.ts:37-40` |
| `play.author` | `name`, `image`, `enabled` | `index.ts:41-45` |
| `play.tweetCard` | `url`, `html`, `authorName`, `authorUrl`, `caption`, `currentStatusId`, `videoUrl`, `videoMime`, `videoIsVertical`, `videos` | `index.ts:18-24` + `richOverlayPayload.ts:3-19` |
| `videos[]` | `url`, `mime`, `isVertical`, `sourceStatusId`, `durationSec` | `richOverlayPayload.ts:3-19` |
| `stop` | `reason`, `jobId` | `index.ts:50-52` + nouveau |
| `heartbeat` | `clientId`, `guildId`, `appVersion`, `sessionMode` | `index.ts:54-59` |
| `error` C→S | `jobId`, `code`, `message` | `index.ts:61-65` |
| `playback.state` | `jobId`, `state`, `remainingMs` | `index.ts:67-71` |
| `playback.ended` | `jobId`, `reason` | nouveau |
| `meme.trigger` | `itemId`, `trigger` | `index.ts:73-76` |
| `stop.request` | `jobId` (optionnel), `source` (optionnel : `user`/`hotkey`/`admin`) | nouveau (rév. 2) |
| `peers` | `guildId`, `peers[].clientId`, `peers[].label` | `index.ts:78-86` |
| `welcome` | `protocolVersion`, `guildId`, `clientId`, `sessionMode`, `serverVersion`, `serverTime`, `config`, `currentJob` | nouveau + `index.ts:102-108` |
| `welcome.config` | `showTextDefault`, `defaultMediaTime`, `maxMediaTime` | `index.ts:102-108` |
| `currentJob` | clés `play` + `startedAt`, `remainingMs` | nouveau |
| `settings` | `guildId`, `sessionMode`, `showTextDefault`, `defaultMediaTime`, `maxMediaTime` | `index.ts:102-108` |
| `ack` | `ref`, `ok`, `error.code`, `error.message` | nouveau |
| `error` S→C | `code`, `message`, `jobId`, `ref` | nouveau |
| URL média (query) | `clientId`, `exp`, `sig`, `startOffsetSec` (option) | remplace `?token=`/`?startOffsetSec=` |

REST conservé hors WS (rév. 2 : inventaire **22 → 21 endpoints**, `GET /overlay/config` supprimé) :
`POST /overlay/pair/consume` (renvoie `clientToken`, `guildId`, `sessionMode` ;
`overlayRoutes.ts:202-291`), `GET /overlay/meme-board/items`,
`GET /overlay/meme-board/media/:assetId`, `/health`, admin
(`docs/rust/migration-plan.md:135-140` — l'annexe A du plan doit être alignée sur ce retrait).

---

## 13. Traitement serveur (mapping vers le scheduler v1)

| Message v2 | Effet serveur v1 équivalent |
|---|---|
| `hello` | auth (§1.2), `join('overlay-guild-'+guildId)`, broadcast `peers`, envoie `welcome`+`settings` |
| `heartbeat` | `updateMany lastSeenAt` ; log divergence de mode (`socketLoader.ts:199-219`) |
| `playback.state` | `scheduler.onPlaybackState` : `playing`/`paused` → `busyUntil` + watchdog (`playbackScheduler.ts:178-237`) |
| `playback.ended` | `completed`/`error` → `onPlaybackEnded` ; `manual` → `onPlaybackStopped` (`playbackScheduler.ts:239-346`) |
| `error` | log structuré uniquement (`socketLoader.ts:221-227`) |
| `stop.request` | `{jobId?, source?}` (source défaut `user`, sans `jobId` = tous les jobs) → `executeManualStopForGuild` ciblé → `stop {reason:'manual'}` (`manualStop.ts:9-60`) |
| `meme.trigger` | validation item/média + `createPlaybackJob(priority=100)` + `preemptWithJob` → `stop{preempt}` puis `play` (`socketLoader.ts:277-338`, `playbackScheduler.ts:922-997`) |
| `peers.request` | broadcast `peers` (`socketLoader.ts:76-86`) |
| envoi `stop {reason:'revoked'}` | après `revoked_at`, puis close `4402` ; conserve `disconnectOverlayClient` pour compat admin |
| purge admin | `stop {reason:'purge'}` + libération de tous les jobs (`adminRoutes.ts:2747,2776-2831`) |
| dispatch d'un job | `play` émis **par socket** avec URL signée (cf. §8.2) ; `busyUntil = startedAt + durationSec*1000 + 250` (parité `playbackScheduler.ts:849-918`) |

---

## 14. Fixtures golden

Dossier : `crates/lc-protocol/tests/fixtures/`. Chaque fichier est une trame complète
(enveloppe comprise) ; les tests `tests/fixtures_roundtrip.rs` vérifient syntaxe, type, version 2
et round-trip serde exact.

| Fichier | Sens | `type` | Particularité |
|---|---|---|---|
| `v2_hello_normal.json` | C→S | `hello` | token + clientId + normal |
| `v2_hello_invite_read_only.json` | C→S | `hello` | sessionMode invité |
| `v2_hello_minimal.json` | C→S | `hello` | `clientId`/`sessionMode` omis |
| `v2_welcome_initial.json` | S→C | `welcome` | `serverVersion`, sans currentJob |
| `v2_welcome_reconnect_current_job.json` | S→C | `welcome` | `currentJob` + `startedAt`/`remainingMs` |
| `v2_play_media_resume.json` | S→C | `play` | vidéo, `isVertical:true`, `startOffsetSec:12`, URL signée + fragment |
| `v2_play_text_author.json` | S→C | `play` | `media:null`, texte + auteur actifs |
| `v2_play_tweet_card.json` | S→C | `play` | carte tweet minimale, `media:null` |
| `v2_play_tweet_card_rich.json` | S→C | `play` | carte riche `videoUrl`/`videos[]` (extras `tweetCard`) |
| `v2_stop_manual.json` | S→C | `stop` | `reason:"manual"`, jobId |
| `v2_stop_preempt.json` | S→C | `stop` | `reason:"preempt"`, jobId suspendu |
| `v2_stop_revoked.json` | S→C | `stop` | `reason:"revoked"`, `jobId:null` |
| `v2_stop_purge.json` | S→C | `stop` | `reason:"purge"`, `jobId:null` |
| `v2_peers.json` | S→C | `peers` | 2 overlays triés par label |
| `v2_peers_empty.json` | S→C | `peers` | room vide |
| `v2_settings_normal.json` | S→C | `settings` | mode normal |
| `v2_settings_invite_read_only.json` | S→C | `settings` | mode invité, `maxMediaTime:null` |
| `v2_ack_stop_request_ok.json` | S→C | `ack` | `ok:true` |
| `v2_ack_meme_trigger_ok.json` | S→C | `ack` | `ok:true` |
| `v2_ack_read_only.json` | S→C | `ack` | `ok:false`, `error.code:"read_only"` |
| `v2_error_render_failed.json` | C→S | `error` | code renderer `render_failed` |
| `v2_error_dispatch_failed.json` | S→C | `error` | non fatal, `jobId` |
| `v2_heartbeat.json` | C→S | `heartbeat` | parité v1 |
| `v2_playback_state.json` | C→S | `playback.state` | `playing`, remainingMs |
| `v2_playback_ended.json` | C→S | `playback.ended` | `completed` |
| `v2_meme_trigger.json` | C→S | `meme.trigger` | `shortcut` |
| `v2_stop_request.json` | C→S | `stop.request` | payload vide = tous les jobs |
| `v2_stop_request_user.json` | C→S | `stop.request` | `jobId` + `source:"user"` (rév. 2) |
| `v2_peers_request.json` | C→S | `peers.request` | payload vide |

Constantes partagées par les fixtures : `GUILD=123456789012345678`,
`CLIENT=clx7q2x0b0000abcd1234efgh`, `JOB=clx7q2x0b0001abcd1234efgh`,
`ASSET=clx7q2x0b0003abcd1234efgh`, `ts=1794182400000` ms, `exp=1794182700` s
(`ts` + 300 s), signature factice de 64 hex (non vérifiée par les tests de round-trip).

---

## 15. Décisions

Décisions prises en Phase 0 (rév. 1, validées par le Lead) :

1. **`playback.state` sans `ended`** : `ended` est extrait dans `playback.ended` avec `reason`.
   Une valeur `ended` reçue → close `1002` (`bad_playback_state`). Le renderer doit être adapté.
2. **`stop.request` remplace `overlay:stop {jobId:'manual-stop'}`** ; la sentinelle disparaît du
   fil.
3. **`welcome.currentJob` enrichi** de `startedAt` + `remainingMs` (le plan ne disait que
   `currentJob?`) pour permettre le seek de reprise.
4. **`settings` poussé après `welcome` et à chaque changement** ; l'overlay ne fait plus
   `GET /overlay/config`.
5. **URL média par client** : `clientId` en query (en plus de `exp`/`sig`) car le serveur doit
   vérifier la signature, et signature `HMAC-SHA256(secret, "assetId|clientId|exp")` hex minuscule.
   `startOffsetSec`/fragment `#t=` non signés (seek local uniquement).
6. **`play`/`welcome.currentJob` émis en unicast par socket** (URL signée par client) ;
   `stop`/`peers`/`settings` en broadcast de room.
7. **Codes de fermeture ajoutés** : `1002`, `1003`, `4408`, `4409`, `1001` (timeout liveness et
   arrêt serveur), en plus des 4401/4402/4403/1011 du plan.
8. **Rate limit 20 msg/s = close 4409** (fail-fast) ; `playback.state` excédentaire =
   ignoré ; `peers.request` excédentaire = ignoré. Le client applique un backoff exponentiel et ne
   reconnecte jamais plus d'une fois par seconde (§10).
9. **`hello.sessionMode`/`heartbeat.sessionMode` informationnels** ; mode autoritaire serveur,
   poussé par `settings` (v1 laissait le client écraser `socket.data`).
10. **Pas de remplacement de socket** : plusieurs sockets par clientId tolérés (parité v1), peers
    dédupliqué.

### 15.1 TTL média adaptatif (rév. 2)

Décision Lead : `exp = now + clamp(durationSec + 120, 300, 3600)` pour une vidéo/audio dont
`durationSec` est connu ; `300` s fixes pour une image sans durée. Les requêtes `HEAD`/`Range`
doivent arriver **avant `exp`** ; une pause prolongée au-delà du TTL n'est pas supportée en v2 —
le client redémarre le média avec une **nouvelle URL signée** sur un nouveau dispatch `play`.
La signature reste `HMAC-SHA256(MEDIA_URL_SECRET, "assetId|clientId|exp")` (§8.1, §8.2).
Implémenté par `media_url_ttl_sec` dans `crates/lc-protocol/src/lib.rs` et couvert par un test.

### 15.2 `stop.request` : `jobId` et `source` (rév. 2)

Décision Lead : payload `{ "jobId"?: string, "source"?: "user"|"hotkey"|"admin" }`. `source` est
purement informatif (défaut serveur `"user"`) ; `jobId` absent/null = arrêt de **tous** les jobs
de la guilde. Fixtures : `v2_stop_request.json` (payload vide) et
`v2_stop_request_user.json` (`jobId` + `source:"user"`). Type Rust `StopRequestPayload`
(+ enum `StopRequestSource`).

### 15.3 Invariant « au plus un job `PLAYING` » (rév. 2)

Décision Lead : invariant documenté et garanti par le scheduler (Phase 4) : **au plus un job
`PLAYING` par guilde**. Si le serveur détecte plusieurs `PLAYING` (anomalie), il choisit le plus
récent par `startedAt` et journalise un `warn`. Aucun changement de type : `welcome.currentJob`
reste un objet unique (§6.1.1).

### 15.4 `GET /overlay/config` supprimé (rév. 2)

Décision Lead : `GET /overlay/config` est **supprimé** du protocole v2 ; la config n'est délivrée
que par `welcome.config` puis `settings`. `POST /overlay/pair/consume` renvoie uniquement
`clientToken`, `guildId` et `sessionMode` ; `apiBaseUrl`/config ne sont plus dupliqués. L'inventaire
REST passe de **22 à 21 endpoints** — à répercuter en Phase 3 (REST serveur) et Phase 8 (bridge
overlay) et dans l'annexe A de `docs/rust/migration-plan.md` (owner du plan).

### 15.5 `MIN_OVERLAY_VERSION` et `welcome.serverVersion` (rév. 2)

Décision Lead : env `MIN_OVERLAY_VERSION` (défaut `"3.0.0"`, parse semver simple), close `4403`
avec `reason` JSON `{"minVersion":"<x.y.z>"}`, et `welcome` gagne `serverVersion` (string).
Règle de comparaison : `major.minor.patch` lexicographique, suffixe de pré-version ignoré, version
illisible = obsolète (§1.6). Constantes Rust `DEFAULT_MIN_OVERLAY_VERSION`,
`outdated_protocol_reason`, `parse_simple_version`, `is_version_below`.

Aucun point ouvert : les 5 points (a)–(e) de la rév. 1 sont soldés par §15.1–§15.5.

---

## 16. Implémentation Rust (`crates/lc-protocol`)

- `PROTOCOL_VERSION: u8 = 2` ; constantes : `HELLO_TIMEOUT_MS=5_000`, `MAX_FRAME_BYTES=65_536`,
  `MAX_CLIENT_MESSAGES_PER_SECOND=20`, `PLAYBACK_STATE_MIN_INTERVAL_MS=1_000`,
  `HEARTBEAT_INTERVAL_MS=15_000`, `PING_INTERVAL_MS=15_000`, `PONG_TIMEOUT_MS=45_000`,
  `MEDIA_URL_TTL_SEC=300`, `MEDIA_URL_EXPIRY_MARGIN_SEC=120`, `MEDIA_URL_TTL_MAX_SEC=3_600`,
  `DEFAULT_MIN_OVERLAY_VERSION="3.0.0"`, `PEERS_REQUEST_MIN_INTERVAL_MS=2_000`,
  `MAX_PLAYBACK_REMAINING_MS=86_400_000`, `MAX_MEME_TEXT_CHARS=500`, `WS_PATH="/ws"`,
  `LEGACY_SOCKET_IO_PATH="/socket.io/"`, `HTTP_UPGRADE_REQUIRED=426`.
- Types : `Envelope<M>` (implémentation manuelle de `Serialize`/`Deserialize` : les champs
  `type`/`payload` de l'enum sont aplatis au même niveau que `v`/`id`/`ts`), `ClientMessage`/
  `ServerMessage` (enums adjacents `#[serde(tag="type", content="payload")]`), payloads en
  structs camelCase, mode `SessionMode`, `MediaKind`, `PlaybackState`, `PlaybackEndedReason`,
  `StopReason`, `MemeTrigger`, `StopRequestSource`. `WelcomePayload` porte `server_version`
  (obligatoire) ; `StopRequestPayload` porte `job_id`/`source` optionnels ;
  `CurrentJobPayload` aplatit `PlayPayload` + `startedAt`/`remainingMs`.
- Aides : `close_code::*`, `close_reason::*`, `msg_type::*`,
  `media_signature_input(assetId, clientId, exp)`, `media_url_ttl_sec(kind, durationSec)`,
  `outdated_protocol_reason(minVersion)`, `parse_simple_version`, `is_version_below`,
  `is_frame_too_large(len)`.
- Vérification : `cargo test -p lc-protocol` — `tests/fixtures_roundtrip.rs` vérifie le round-trip
  exact des 29 fixtures (table fichier → sens → `type`, égalité de `serde_json::Value`), la
  couverture exhaustive du dossier `tests/fixtures/`, les constantes, les codes de fermeture, la
  séparation des sens de message, la reprise `welcome.currentJob`, le TTL média adaptatif, la
  règle semver et `welcome.serverVersion`.
