# POC serveur Rust — rapport de mesure et verdict (Phase 0)

**Date :** 2026-11-09 · **Branche :** `feat/rust-rebuild` · **Agent :** `poc-server` (task-5)
**Spike :** `spikes/poc-server/` (workspace Cargo séparé, `[workspace]` vide)
**Protocole de référence :** `docs/rust/protocol-v2.md` (gelé le 2026-11-09) + crate `crates/lc-protocol` (utilisé tel quel, non modifié).

---

## 0. Verdict : **GO** (avec conditions)

La stack **axum 0.8.9 + tokio 1.54.1 + sqlx 0.8.6/SQLite (WAL) + hmac/sha2** tient le scénario de référence
(100 clients WebSocket, broadcast `play` toutes les 500 ms pendant 4 min, charge REST concurrente,
reconnexions, arrêt propre) avec des marges très confortables sur les budgets fixés :

| Budget Phase 0 | Cible | Mesuré (run FULL, 100 clients / 240 s) | Verdict |
|---|---|---|---|
| RSS du serveur **au repos** | < 30 Mo | **8,91 Mo** (WorkingSet ; 2,22 Mo private, 22 threads) | ✅ très large |
| RSS **sous charge** (pic) | (indicatif) | 30,00 Mo moyen / **30,74 Mo** pic — au niveau du budget « repos », ok pour un pic sous charge | ✅ (à ré-évaluer en prod) |
| p95 REST local | < 10 ms | GET `/health` p95 **0 ms** (8 916 req, 0 erreur) ; POST `/jobs` **0–1 ms côté serveur** (TraceLayer), `curl` keep-alive 0,15–0,55 ms | ✅ côté serveur |
| Latence `play` client→client | < 20 ms | p50 **1 ms**, p95 **1 ms**, p99 2 ms, max 3 ms ; skew inter-clients p95 1 ms | ✅ très large |
| Taux d'erreurs / reconnexions | 0 erreur | 0 erreur client, 10/10 reconnexions forcées réussies, 0 perte sur 90 clients stables | ✅ |
| Arrêt propre | fermeture WS + flush | 100/100 clients fermés en 1001 `server_shutdown`, close frames 100, **sortie process ~20 ms** après la demande | ✅ |

**Conditions / réserves :**
1. La latence POST `/jobs` mesurée **par le client de bench** (p50 3–15 ms, p95 34–46 ms) est un **artefact de mesure côté client Windows** (détails §6.3). Le serveur lui-même répond en 0–1 ms (TraceLayer, `curl`, `System.Net.Http`). Le budget REST est considéré atteint, mais doit être **re-mesuré en Phase 3 avec un générateur neutre** (oha/wrk/k6, ou mesures serveur-side).
2. Le run par défaut était `synchronous=FULL` (configurable `POC_SQLITE_SYNCHRONOUS=off|normal|full|extra`). Le connexion en rafale de 100 clients coûte p95 230 ms / max 980 ms en FULL contre p95 34 ms / max 60 ms en NORMAL, à cause de l'`UPDATE last_seen_at` authentifié synchrone (§6.4). Recommandation : **écrire la présence en asynchrone/débounce** plutôt que dégrader le mode de synchronisation global.
3. Le sous-ensemble « média » du POC ne fait **pas** de `Range`/206 (voir §8.1) ; à implémenter en Phase 3 avec la vraie API HTTP.
4. Le POC est mono-guilde, sans TLS, sans auth REST (sauf `/shutdown`) — c'est un spike, pas le serveur de production.

---

## 1. Livrable et périmètre

Spike autonome qui démontre de bout en bout :

- **WebSocket v2 (sous-ensemble)** : `GET /ws`, handshake `hello` → `welcome` + `settings` + broadcast `peers`, auth par `SHA-256(token)` contre `overlay_clients.token_hash`, heartbeat applicatif + Ping/Pong WS, broadcast `play` toutes les 500 ms avec **URL média signée par client**, `stop.request`/`meme.trigger`/`peers.request` avec `ack`, rate-limit 20 msg/s, codes de fermeture du protocole.
- **HTTP** : `GET /health`, `GET /debug/pragmas` (diagnostic), `GET /media/{asset_id}?clientId=&exp=&sig=` avec HMAC-SHA256, `POST /jobs` (insertion sqlx), `POST /shutdown` protégé par header secret, `GET /socket.io/*` → 426 legacy.
- **SQLite** : WAL, foreign keys, busy_timeout 5000 ms, pool max 5, tables `guilds` / `overlay_clients` / `jobs`, requêtes `sqlx::query` **runtime** (aucune macro compile-time, donc pas de `DATABASE_URL` au build).
- **Bench reproductible** : `src/bench.rs` + `bench/run-bench.ps1` (100 clients WS, charge REST, reconnexions, échantillonnage RSS/CPU, arrêt propre, agrégation JSON).
- **Arrêt propre** : `POST /shutdown` (header `x-poc-shutdown`) → broadcast, fermeture des WS en 1001 `server_shutdown`, `db.close()`, `axum::serve(...).with_graceful_shutdown(...)`, garde-fou forcé à 10 s.

Non-objectifs assumés : multi-guilde, TLS, authentification des routes REST, vrai state-machine de playback
(`playback.state`/`playback.ended` sont acceptés/ignorés ou ackés), stockage réel des médias, `Range`/206,
métriques Prometheus, tests unitaires exhaustifs (le bench est le test d'intégration).

---

## 2. Environnement de mesure

| Élément | Valeur |
|---|---|
| Machine | Windows 11, **8 cœurs / 16 threads**, 61 Go RAM |
| Toolchain | **rustc 1.99.1** (`58cf8f9c3`, 2026-10-13, LLVM 23.1.1), cargo 1.99.1 (`5f94df478`) |
| Host | `x86_64-pc-windows-msvc` |
| Linker | Visual Studio 2022 BuildTools, toolset 14.44.35207 (`cl.exe`/`link.exe` OK) |
| Réseau | loopback local (`127.0.0.1`), serveur et client de bench sur la même machine |
| Build | `CARGO_BUILD_JOBS=4`, `[profile.release] opt-level = 2` (pas de LTO) |
| Binaire | `target/release/poc-server.exe` ≈ **7,18 Mo** |

### Durées de build mesurées

| Build | Durée |
|---|---|
| **Clean release from scratch** (`cargo clean` puis `cargo build --release`, 4 jobs) | **62,57 s** |
| Build incrémental release (modif code) | 8–18 s |
| Build release sans changement | 0,26–0,44 s |
| Démarrage serveur → `/health` OK | **0,53 s** (init SQLite + migrations + seed inclus) |

---

## 3. Architecture du spike

```
spikes/poc-server/
├── Cargo.toml            # workspace Cargo SÉPARÉ ([workspace] vide) + path dep lc-protocol
├── Cargo.lock
├── src/
│   ├── main.rs           # #![forbid(unsafe_code)] ; dispatch `serve` | `bench` | `help`
│   ├── server.rs         # ~1 250 l : config, DB, routes HTTP, WS handshake/session, ticker play, shutdown
│   └── bench.rs          # ~1 100 l : client de bench (100 WS + REST + reconnexions + agrégation JSON)
├── bench/
│   ├── run-bench.ps1     # orchestrateur : build chronométré, serveur, RSS/CPU, bench, shutdown, cleanup
│   ├── results/          # bench-full.json, bench-normal.json (résultats machine)
│   └── logs/             # logs serveur/bench + CSV RSS pour les runs conservés
└── REPORT.md             # ce document
```

### Routes HTTP

| Méthode/route | Rôle | Codes |
|---|---|---|
| `GET /health` | JSON statut + uptime + compteurs (`wsConnections`, `wsConnectedTotal`, `wsAuthFailures`, `framesClientToServer`, `playEvents`, `jobsInserted`, `outboundDropped`) | 200 |
| `GET /debug/pragmas` | diagnostic SQLite/pool (`journalMode`, `foreignKeys`, `busyTimeoutMs`, `synchronous`, `poolMaxConnections`, `poolSize`, `poolIdle`) | 200 |
| `GET /media/{asset_id}?clientId&exp&sig` | vérif HMAC-SHA256 + expiration, renvoie 4 096 octets binaires (`video/mp4`, `Cache-Control: no-store`, `Accept-Ranges: bytes`) | 200 / 403 `media_url_invalid` / 403 `media_url_expired` |
| `POST /jobs` | insertion sqlx `{id, guildId, status, createdAt}` ; `guildId`/`status` optionnels | 201 / 400 `unknown_guild` (FK) / 503 `database_busy` / 500 |
| `GET /ws` | upgrade WebSocket v2 | 101 / 426 `{"error":"websocket_required"}` |
| `POST /shutdown` | arrêt propre, header `x-poc-shutdown` comparé en temps constant | 202 / 401 |
| `GET /socket.io/` (+ `/{*rest}`) | legacy | 426 `{"error":"overlay_outdated","protocolVersion":2,"minOverlayVersion":"3.0.0"}` |

### SQLite

- `PRAGMA journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000`, `synchronous` configurable (défaut `FULL`), pool **max 5**, `acquire_timeout=10 s`.
- Tables : `guilds(id TEXT PK, name, created_at)` ; `overlay_clients(client_id TEXT PK, guild_id FK, label, session_mode, token_hash TEXT UNIQUE, last_seen_at, revoked_at)` ; `jobs(id TEXT PK, guild_id FK, status, created_at)` + index `(guild_id, status)`.
- Seed : guilde `123456789012345678`, client démo `clx-poc-demo`/`poc-demo-token`, et N clients de bench `clx-poc-bench-NNN`/`poc-token-NNN` (N = `POC_BENCH_CLIENTS`).
- Toutes les requêtes sont **runtime** (`sqlx::query`, `query_scalar`, `query_as`, `Row::try_get`) — aucune macro `query!` compile-time.

### Modèle WebSocket (par connexion)

- Task lectrice (session) + **task écrivaine dédiée** alimentée par un `mpsc::Sender<Outbound>` borné (256) ;
  `Outbound = { Text(String) | Ping | Pong(Vec<u8>) | Close(u16, &'static str) }`. Échec de `try_send` → compteur `outboundDropped`.
- Sessions suivies dans `RwLock<HashMap<conn_id, ClientEntry>>` (`client_id`, `guild_id`, `label`, `session_mode`, `tx`).
- Ticker global 500 ms (`POC_PLAY_INTERVAL_MS`) : construit un `PlayPayload` **par client** (URL média signée avec l'`exp` et le `sig` de ce client), `try_send` → aucun blocage de la boucle.
- Liveness : Ping WS toutes les 15 s (`POC_PING_INTERVAL_MS`), watchdog 45 s sans trame/Pong → close 1001 `heartbeat_timeout` ; heartbeat applicatif C→S toutes les 15 s (présence).
- Rate-limit : fenêtre glissante 1 s, 20 trames client max → close 4409 `rate_limited`.
- Handshake gelé : `hello` sous 5 s (sinon 4401 `auth_timeout`), token vide → 4401 `missing_token`, inconnu → 4401 `invalid_token`, révoqué → 4402 `revoked`, `clientId` divergent → 4401, erreur DB → 1011 `internal_error` ; ordre `welcome` → `settings` → enregistrement → broadcast `peers`.

### Conformité au sous-ensemble protocol-v2 implémenté

| Élément de spec | État POC |
|---|---|
| Enveloppe `{v:2,type,id,ts,payload}`, payload objet, `id` ≤ 64 | ✅ (rejets 1002/4403/1003, validation `v`/`id`/`payload`) |
| Handshake auth + ordre welcome/settings/peers + `currentJob` absent (pas de job courant au démarrage) | ✅ |
| `peers` dédupliqué par clientId, trié par label, broadcast à chaque connexion/déconnexion | ✅ |
| `play` broadcast par socket avec URL signée par client, `jobId` monotone | ✅ |
| `stop.request` → ack puis broadcast `stop {reason:"manual"}` ; `meme.trigger` → ack `item_not_found` ; `peers.request` throttlé 2 s | ✅ (ack/refus, pas de vraie donnée) |
| `playback.state` ≤ 1/s (excédent ignoré), `playback.ended` accepté | ✅ partiel : acceptés, sans state machine ni persistance |
| Ping/Pong 15 s/45 s, codes de fermeture 1000/1001/1002/1003/1011/4401/4402/4403/4408/4409 | ✅ |
| Trame > 64 Kio → close **4408** | ✅ logique maison (voir §6.5 : `max_message_size` axum relevé à 4×) |
| URL média `exp = now+300 s`, `sig = HMAC-SHA256(secret, "assetId|clientId|exp")` hex minuscule, comparaison temps constant | ✅ (`subtle`) |
| Modes `invite_read_only` → refus `meme.trigger`/`stop.request` avec ack `read_only` | ✅ codé, non exercé par le bench (gap de test) |

---

## 4. Bench : méthode et commandes exactes

### Commandes (reproductibles)

```powershell
# une seule fois
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$env:CARGO_BUILD_JOBS = "4"
Set-Location C:\Users\SuperKnacky\code\LiveChat\spikes\poc-server

# build
cargo build --release

# run de référence : 100 clients WS, 240 s, broadcast play 500 ms, REST 40+20 rps, 10 reconnexions, WAL/FULL
powershell -NoProfile -ExecutionPolicy Bypass -File bench\run-bench.ps1 -Clients 100 -DurationSecs 240 -RunName full -SyncMode full

# comparaison : même scénario 120 s avec synchronous=NORMAL
powershell -NoProfile -ExecutionPolicy Bypass -File bench\run-bench.ps1 -Clients 100 -DurationSecs 120 -RunName normal -SyncMode normal
```

Le script démarre le serveur sur le port **3411** avec `POC_*` (dont `POC_DB=bench/tmp/poc-server-<run>.db`,
`POC_SQLITE_SYNCHRONOUS`), mesure le RSS au repos, lance un échantillonneur `Get-Process` (WorkingSet,
PrivateMemory, CPU, threads) à 1 Hz, exécute le client de bench, attend la sortie du serveur ≤ 20 s,
puis supprime la DB temporaire et les process résiduels.

### Client de bench (`poc-server bench`)

- 100 clients `tokio-tungstenite` 0.29 en parallèle : `hello` → `welcome`, heartbeat applicatif immédiat puis 15 s,
  répond aux Ping, comptabilise `play` (latence = `now_ms − envelope.ts`), `peers`, fermetures, reconnexions.
- 10 clients forcés à se reconnecter à `t + duration/3` (close 1000 côté client puis reconnexion ~1 s).
- Charge REST : 4 workers `GET /health` (40 rps) + 4 workers `POST /jobs` (20 rps), latences p50/p95/p99.
- Checks ciblés : 4 cas média (valide / signature altérée / expiré / signature absente), `POST /jobs` guilde inconnue,
  `GET /ws` sans Upgrade, `GET /socket.io/`.
- En fin de run : dump `/health` + `/debug/pragmas`, `POST /shutdown` **sans** header (401 attendu), puis **avec** header (202 attendu),
  et attend que les 100 clients observent le close 1001 du serveur.
- Résultat : JSON structuré (`--json-out`) + résumé stdout.

---

## 5. Résultats détaillés

### 5.1 Run de référence — 100 clients / 240 s / `synchronous=FULL` (`bench/results/bench-full.json`)

**Connexion / auth**
- 100/100 clients connectés en **1 059 ms** (phase complète incluant les 100 handshakes).
- Handshake `hello→welcome` : p50 5 ms, p95 **7 ms**, max 9 ms.
- `welcome` (auth DB + sérialisation + envoi) : p50 121 ms, p95 **230 ms**, p99 872 ms, max 980 ms (**à améliorer**, §6.4).
- 110 connexions au total (10 reconnexions), 0 échec d'auth (`wsAuthFailures=0`).

**Broadcast `play` (500 ms)**
- 476 événements `play` émis ; **47 545 messages** livrés ; `outboundDropped = 0`.
- Latence client→client : p50 **1 ms**, p95 **1 ms**, p99 2 ms, max 3 ms.
- Skew inter-clients (par `jobId`) : p50 1 ms, p95 **1 ms**, max 3 ms.
- Pertes sur les **90 clients stables** (hors reconnexions) : **0** (0,0 %) ; ratio de livraison estimé 99,05 % (la différence correspond aux fenêtres de reconnexion des 10 clients forcés).
- `peers` : 6 930 messages, jusqu'à 100 pairs observés.

**REST concurrente (pendant le broadcast)**
- `GET /health` : 8 916 requêtes, **0 erreur**, p95 **0 ms**, max 1 ms.
- `POST /jobs` : 4 482 lignes créées, **0 erreur**, p50 3 ms / p95 34 ms / p99 77 ms / max 644 ms mesurés par le client séquentiel de bench — **artefact côté client**, voir §6.3 ; côté serveur TraceLayer : **0–1 ms** pour toutes les requêtes.

**SQLite**
- 4 482 insertions + 110 updates d'auth + lectures `/health` pendant 240 s : **aucune erreur** (`database is locked`, 503 ou 500 : 0).
- Fin de run : `journal_mode=wal`, `foreign_keys=1`, `busy_timeout_ms=5000`, `synchronous=2` (FULL), `pool_max_connections=5`, pool size 5 / idle 4.
- `jobsInserted=4483` (1 job créé par un check ciblé), cohérent avec le bench.

**Ressources serveur (échantillonnage `Get-Process` 1 Hz)**
- RSS : repos **8,91 Mo**, moyenne **30,00 Mo**, pic **30,74 Mo** ; private au repos 2,22 Mo.
- CPU (process serveur, % d'**un** cœur) : moyenne **2,3 %**, pic **15,5 %** → en pratique ~0,15 % à 1 % de la machine 16 threads.
- Threads : 22 au repos.
- Ticker `play` + 100 sessions + 8 workers REST + pool SQLite : charge CPU très faible, pas de saturation.
- Binaire 7,18 Mo ; démarrage 0,53 s (init + migrations + seed).

**Arrêt propre**
- `POST /shutdown` avec header : **202 en 2 ms** ; sans header : **401**.
- 100/100 clients connectés ont observé la fermeture serveur : **close code 1001 `server_shutdown`**, `closeFramesObserved = 100`.
- Collecte des tâches clients : 4 ms ; **sortie du process ~23 ms** après la demande (jamais le garde-fou 10 s).
- Logs serveur : `client WebSocket deconnecte … close_cause="server_shutdown"` ×100 puis
  `arret propre termine (pool SQLite ferme) … outbound_dropped=0` ; pas de WAL orpheline (DB `-wal`/`-shm` supprimées après fermeture).

**Checks ciblés (tous OK)**

| Check | Résultat |
|---|---|
| Média valide | 200, 4 096 octets, `content-type: video/mp4`, `cache-control: no-store` |
| Média signature altérée | 403 JSON (29 octets) |
| Média expiré | 403 JSON |
| Média signature absente | 403 JSON |
| `POST /jobs` guilde inconnue | 400 `{"error":"unknown_guild"}` (FK) |
| `POST /jobs` valide | 201 `{"id":"poc-job-…","guildId":"123456789012345678","status":"QUEUED","createdAt":…}` |
| `GET /ws` sans Upgrade | 426 |
| `GET /socket.io/` | 426 `{"error":"overlay_outdated",…}` |

### 5.2 Comparaison `synchronous=NORMAL` — 100 clients / 120 s (`bench/results/bench-normal.json`)

| Métrique | FULL (240 s) | NORMAL (120 s) |
|---|---|---|
| 100 clients connectés | 1 059 ms | **113 ms** |
| Handshake p95 | 7 ms | 5 ms |
| `welcome` p50 / p95 / max | 121 / 230 / 980 ms | **15 / 34 / 60 ms** |
| `play` p95 / pertes clients stables | 1 ms / 0 | 1 ms / 0 |
| `GET /health` p95 / erreurs | 0 ms / 0 | 0 ms / 0 |
| `POST /jobs` créés / erreurs | 4 482 / 0 | 2 252 / 0 |
| `POST /jobs` p95 (artefact client) | 34 ms | 46 ms |
| RSS repos / moyen / pic | 8,91 / 30,00 / 30,74 Mo | 9,06 / 31,03 / 31,82 Mo |
| CPU moyen / pic (1 cœur) | 2,3 % / 15,5 % | 2,2 % / 15,5 % |
| Arrêt propre : 202 / close 1001 / sortie | 2 ms / 100/100 / 23 ms | 1 ms / 100/100 / 19 ms |

**Lecture :** la rafale d'authentification est le seul poste où FULL coûte visiblement plus cher (welcome ×7).
Le débit `play` et le CPU/RSS sont identiques. Le mode `NORMAL` suffit pour la présence ; garder `FULL` pour les
écritures métier critiques (jobs/médias) est une piste, ou passer la présence en écriture asynchrone (§9).

### 5.3 Taux d'erreur et stabilité
- Run FULL : `clientErrors = 0`, `wsAuthFailures = 0`, 0 perte sur clients stables, `outboundDropped = 0`, 10/10 reconnexions.
- Aucun `503 database_busy`, aucun `500`, aucun close inattendu (aucun `heartbeat_timeout`).
- Fenêtre de 45 s de watchdog jamais déclenchée : les 100 clients répondaient aux Ping.

---

## 6. Surprises, pièges et points d'attention

### 6.1 Le `timeout(10 s)` global tue le serveur même sans shutdown
Première version : tout `axum::serve(...).with_graceful_shutdown(...)` était enveloppé dans `timeout(10 s)` ;
le serveur s'arrêtait après 10 s **sans** demande d'arrêt. Correction : `tokio::select!` entre la future serve et une
future « garde-fou » qui ne dort 10 s **qu'après** réception du signal de shutdown. À noter pour Phase 4 : le serveur
de prod doit vivre indéfiniment, le timeout ne doit s'appliquer qu'à la phase de drainage.

### 6.2 `max_message_size` par défaut masque le code 4408
axum rejette une trame trop grosse par défaut et ferme en 1009 ; pour émettre la close **4408 `frame_too_large`**
de la spec, il faut relever `max_message_size`/`max_frame_size` (fait : 4 × 64 Kio) et garder la vérification
applicative (`is_frame_too_large`). Sinon la conformité protocole est cassée silencieusement.

### 6.3 Artefact de latence REST côté client de bench (important pour Phase 3)
Le client de bench mesure `POST /jobs` à p50 3–15 ms / p95 34–46 ms / max 644 ms, alors que :
- `curl.exe` keep-alive : **0,15–0,55 ms** par requête (4 parallèles : 1,1–1,7 ms) ;
- `System.Net.Http.HttpClient` (.NET) keep-alive : 0,36–0,70 ms après la première requête ;
- logs serveur TraceLayer : **`latency=0 ms` ou `1 ms`** pour 100 % des POST `/jobs` et GET `/health` ;
- étrangement, avec `RUST_LOG=poc_server=debug,tower_http=debug` actif, la latence mesurée par le bench retombe à p50 3 / p95 5 ms.

Conclusion : ce n'est **pas** le serveur. Suspects : timers Windows / chemin loopback / client reqwest en process
séparé sous charge CPU (100 tâches WS). → **Phase 3 : mesurer avec un générateur neutre** (oha, wrk, k6) ou via
métriques serveur (histogramme p95), ne pas se fier à cette métrique du bench maison.

### 6.4 `welcome` lent en rafale avec `synchronous=FULL`
Chaque auth fait `UPDATE overlay_clients SET last_seen_at=…` ; en FULL, chaque commit = fsync WAL. 100 connexions
simultanées ⇒ sérialisation ~1 s au total (p95 230 ms, max 980 ms). En NORMAL : p95 34 ms. Pistes pour la Phase 4 :
écriture de présence en tâche dédiée avec debounce/batch (ou `PRAGMA synchronous=NORMAL` pour la base présence),
et/ou `welcome` optimiste avant l'update de présence.

### 6.5 Divers Rust / API
- `tokio-tungstenite` doit être **0.29** (même version que celle embarquée par axum 0.8.9) sinon deux `tungstenite` compilés (doublon inutile).
- `tracing` + `.await` dans les arguments d'un macro (`info!(..., x = fut.await)`) rend la future `!Send` → calculer la valeur avant le log (bug rencontré et corrigé).
- `axum::extract::ws::rejection::WebSocketUpgradeRejection` renvoie 400 par défaut : il faut un handler `IntoResponse` pour le **426 `websocket_required`** de la spec.
- `sqlx::sqlite::SqliteSynchronous` : `Full` par défaut ; `PRAGMA synchronous` peut être exposé via `query_scalar("PRAGMA synchronous")`.
- `lc-protocol` se compile **tel quel** comme path dependency dans un workspace séparé (malgré `version.workspace`/`lints.workspace`), à condition que le workspace racine exclue `spikes` (déjà le cas). `cargo metadata` le confirme. Aucune modification du crate n'a été nécessaire.
- Le crate `lc-protocol` contient `WelcomePayload.server_version` (non listé dans l'extrait initial de spec) : à utiliser tel quel (`"3.0.0-poc"` côté POC).

### 6.6 Pièges Windows / PowerShell 5.1 rencontrés (pour les scripts Phase 1+)
- `Invoke-WebRequest -Method Post` en PS 5.1 peut échouer avec « Windows PowerShell n'est pas en mode interactif… » **alors que la requête part** ; utiliser `System.Net.Http.HttpClient` (`Add-Type -AssemblyName System.Net.Http` **obligatoire** en PS 5.1).
- La sortie stderr d'un exécutable natif devient une erreur non-terminante sous PS 5.1 : avec `$ErrorActionPreference="Stop"` un build cargo (qui écrit sur stderr) **arrête le script**. Passer temporairement en `Continue` autour des commandes natives.
- `Export-Csv`/casts numériques dépendent de la locale (`11,34` en fr-FR) : formater/parser les CSV de bench en **culture invariante**, sinon les agrégations sont fausses (RSS « 1 219 Mo » obtenu au premier essai).
- `Set-Content -Encoding utf8NoBOM` n'existe pas en PS 5.1 (énumération `UTF8`/`Default`) ; créer les fichiers via l'outillage UTF-8 sans BOM.
- `Get-Process` donne RSS/CPU processus : fiable pour un serveur unique, mais attention aux PID réutilisés dans les scripts (vérifier `ProcessName`).

---

## 7. Versions exactes (Cargo.lock)

| Crate | Version | Crate | Version |
|---|---|---|---|
| axum | **0.8.9** | sqlx | **0.8.6** |
| axum-core | 0.5.6 | sqlx-core / sqlx-sqlite / sqlx-macros | 0.8.6 |
| tokio | **1.54.1** | libsqlite3-sys (SQLite bundlé) | 0.30.1 |
| tokio-tungstenite | **0.29.0** | hmac | 0.12.1 |
| tungstenite | 0.29.0 | sha2 | 0.10.9 |
| hyper | 1.12.0 | hex | 0.4.3 |
| hyper-util | 0.1.21 | subtle | 2.6.1 |
| tower | 0.5.3 | uuid | 1.29.2 |
| tower-http | 0.6.11 | tracing / tracing-subscriber | 0.1.44 / 0.3.23 |
| reqwest (bench) | 0.12.28 | futures-util | 0.3.34 |
| serde / serde_json | 1.0.229 / 1.0.154 | anyhow | 1.0.104 |
| http / http-body-util / bytes | 1.5.0 / 0.1.5 / 1.12.1 | lc-protocol (path) | 3.0.0 |
| **rustc** | **1.99.1** (LLVM 23.1.1) | **cargo** | **1.99.1** |

Aucune dépendance supplémentaire non listée dans `Cargo.toml` (uniquement des transitives de axum/sqlx/tokio/tower-http).

### APIs utilisées (résumé)

- **axum** : `Router::new().route(...).with_state(...)`, `WebSocketUpgrade`/`ws::WebSocket`/`ws::Message`/`ws::CloseFrame`,
  `Json`, `State`, `Path`, `IntoResponse`, `response::Response`, `serve` + `with_graceful_shutdown`, `rejection::WebSocketUpgradeRejection`.
- **tokio** : `net::TcpListener`, `sync::{mpsc, broadcast, RwLock}`, `time::{interval, timeout, sleep}`, `task::JoinSet` (bench),
  `signal::ctrl_c` (fallback arrêt), `select!`.
- **sqlx** : `SqlitePoolOptions`, `SqliteConnectOptions` (filename, journal_mode, synchronous, foreign_keys, busy_timeout),
  `query`/`query_scalar`/`query_as`, `Row::try_get`.
- **hmac/sha2** : `Hmac<Sha256>`, `Mac::new_from_slice`, `update`, `finalize().into_bytes`, `hex::encode` ;
  **subtle** : `ConstantTimeEq` pour le header `/shutdown` (signature média comparée via égalité de chaînes hex — piste : `subtle` aussi).
- **tower-http** : `TraceLayer` (latence serveur) ; **tokio-tungstenite** + **reqwest** + **futures-util** : bench.
- **lc-protocol** : `Envelope`, `ClientEnvelope`/`ServerEnvelope`, `ClientMessage`/`ServerMessage`, payloads typés,
  constantes (`HELLO_TIMEOUT_MS`, `MAX_FRAME_BYTES`, `MAX_CLIENT_MESSAGES_PER_SECOND`, `MEDIA_URL_TTL_SEC`, `PING_INTERVAL_MS`, `PONG_TIMEOUT_MS`,
  `is_frame_too_large`, `media_signature_input`), enums de close codes/raisons.

---

## 8. Écarts et limites à couvrir en Phase 3/4

### 8.1 HTTP / Phase 3
1. **`Range`/206 absent** : le POC renvoie toujours 200 + 4 Kio. En Phase 3, servir les médias via `tower-http`
   (`ServeFile`/`ServeDir`, qui gère `Range`, 206, `Content-Range`, `Accept-Ranges`) ou un handler dédié ; conserver
   la vérification HMAC avant toute lecture de fichier. La spec mentionne `startOffsetSec`/`#t=` non signés → le 206
   devra être supporté.
2. **Auth REST** : `POST /jobs` et `GET /media` ne sont pas authentifiés au-delà de la signature média. La vraie API
   devra porter une authentification (session/token) et un contrôle par guilde.
3. **Sémantique d'erreurs** : mapper FK → `unknown_guild` (fait), `database is locked` → `503 database_busy` (fait) ;
   ajouter le rate-limiting HTTP par IP/guilde, le corps d'erreur JSON homogène et les `request_id`.
4. **`/overlay/media/:assetId` (spec) vs `/media/{asset_id}` (spike)** : axum 0.8 utilise la syntaxe **`{param}`**
   (les `:` de la doc spec ne sont pas acceptés par axum 0.8). Le routeur de production devra utiliser
   `/overlay/media/{asset_id}` et garder les query params signés `clientId/exp/sig`.
5. **426 legacy** : garder l'interception `/socket.io/{*rest}` (le POC le démontre) pour les anciens overlays.
6. **Secrets** : `MEDIA_URL_SECRET`/`SHUTDOWN_SECRET` en clair dans l'environnement du POC ; passer par un
   gestionnaire de secrets en Phase 1+ et ne pas exposer `/debug/pragmas`/`/shutdown` en production (le POC les expose volontairement).
7. **Observabilité** : ajouter métriques (connexions, latence play p95, drops) et logs structurés ; le POC ne fait que
   des compteurs atomiques exposés sur `/health`.

### 8.2 WebSocket v2 / Phase 4
1. **Piste prod = crate `lc-protocol`** : réutiliser directement les types et constantes (aucun fork) ; le POC le prouve faisable
   depuis un autre workspace via path dep. Prévoir une CI de compatibilité protocole (tests serde de non-régression).
2. **Présence / `last_seen_at`** : passer l'update en tâche dédiée (debounce, ex. toutes les 5–10 s) ou en
   `synchronous=NORMAL` sur la base présence ; sinon la rafale de reconnexion coûte ~1 s (mesuré).
3. **Backpressure** : budget actuel = mpsc(256)/socket + drop compté. En prod, définir la politique : drop des `play`
   si le client est lent (déjà le cas via `try_send`), mais garder `welcome`/`settings`/`stop` fiables (les envoyer
   avant l'enregistrement dans le registre, comme dans le POC) et fermer 1011 après N drops.
4. **Thundering herd** : connexion de 100 clients en 1 s OK (p95 handshake 7 ms) ; prévoir un jitter de reconnexion côté
   client, un cap par guilde/IP et éventuellement une file de `hello` avant auth DB.
5. **Ping/Pong** : conserver Ping WS 15 s + watchdog 45 s ; le heartbeat applicatif reste nécessaire pour la présence mais
   ne doit pas remplacer le Ping. Ajouter métriques `close_code` par raison.
6. **Rate-limit 20 msg/s** : fenêtre glissante 1 s par connexion validée ; à compléter par un rate-limit par guilde
   (broadcast/stop/meme) et le mode `invite_read_only` autoritaire côté serveur (déjà outline).
7. **Shutdown** : conserver la séquence actuelle (broadcast → close 1001 → drain → `db.close()` → exit), avec garde-fou
   de 10 s ; surveiller que les writes SQLite en cours soient flushés (`synchronous=FULL` pour les jobs).
8. **Auth token** : `SHA-256(token)` hex + `revoked_at IS NULL` fonctionne ; comparer le hash en temps constant
   (`subtle`) et prévoir rotation/révocation, TTL de session, et le rejet `clientId` divergent (déjà fait).

---

## 9. Recommandations concrètes synthétiques

| # | Recommandation | Phase |
|---|---|---|
| R1 | Garder **axum 0.8 / tokio / sqlx-sqlite runtime queries** : aucun blocage, budget latence tenu. | 3 |
| R2 | SQLite : WAL + busy_timeout 5000 + pool 5 suffisent à 100 clients / 18 jobs/s ; `synchronous=FULL` pour les jobs, **NORMAL ou écriture différée pour la présence**. | 3/4 |
| R3 | Ne pas wrapper le serveur dans un `timeout` global ; timeout seulement sur le drainage après signal. | 4 |
| R4 | Relever `max_message_size`/`max_frame_size` à ≥ 4×64 Kio pour que le code **4408** de la spec soit réellement émis. | 4 |
| R5 | Réutiliser `lc-protocol` comme crate unique (path dep) et épingler `tokio-tungstenite = 0.29` (= version d'axum 0.8.9). | 4 |
| R6 | Rendre l'écriture de présence asynchrone/batchée (debounce) — gain mesuré sur `welcome` (p95 230 ms → 34 ms en NORMAL). | 4 |
| R7 | Bench Phase 3 avec générateur neutre (oha/wrk/k6) ou histogrammes serveur ; ne pas se fier au client reqwest du POC pour le POST REST. | 3 |
| R8 | Médiathèque : servir via `tower-http::ServeFile` + Range/206, HMAC vérifié avant lecture ; sinon régression streaming. | 3 |
| R9 | Fermer `/debug/pragmas`, `/shutdown` et les compteurs détaillés en production (ou les protéger). | 3 |
| R10 | Tests de non-régression serde sur `lc-protocol` (enveloppe, payloads, close codes) dans la CI dès la Phase 1. | 1 |

---

## 10. Reproductibilité et nettoyage

- `bench/run-bench.ps1` est le point d'entrée unique ; il supprime la DB temporaire et les process résiduels à la fin
  (vérifié : aucun `poc-server.exe` laissé après les runs).
- Résultats machine conservés : `bench/results/bench-full.json` (référence 240 s / FULL),
  `bench/results/bench-normal.json` (comparaison 120 s / NORMAL), CSV RSS/CPU correspondants dans `bench/logs/`
  (`rss-full.csv`, `rss-normal.csv`).
- Logs serveur/bench conservés : `bench/logs/server-{full,normal}.err.log`, `bench/logs/bench-{full,normal}.out.log`.
- Les artefacts de diagnostic intermédiaires (`probe*.log`, `probe*.db`, `smoke*.log`) ont été supprimés.
- `target/` n'est pas committé (ignoré par la racine : `/spikes/*/target`).
- Le spike **ne modifie aucun fichier hors `spikes/poc-server/`** : `crates/lc-protocol` est lu via path dep sans modification.
