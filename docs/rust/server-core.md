# Serveur core Rust — Phase 3a (`lc-server`)

Statut : **Phase 3a livrée** (socle du serveur axum : configuration, sécurité,
health, rate limiting, arrêt gracieux). Les routes métier `/overlay`, `/ingest`,
`/admin` et le hub WebSocket restent hors périmètre de cette phase (Phases 3b,
4 et 7).

Ce document décrit l'architecture, la configuration, le contrat HTTP et les
points d'extension de `apps/lc-server` pour les lanes suivantes.

---

## 1. Objectif et périmètre

Le serveur `lc-server` remplace le serveur Fastify legacy
(`server/src/server.ts`). La Phase 3a fournit un socle qui :

- démarre **sans** Discord ni base de données (exigence d'audit : le legacy
  refusait de démarrer sans Discord, en 7,05 s) ;
- applique la politique de sécurité cible (CSP, CORS, rate limit, erreurs
  normalisées, body limit) ;
- expose `GET /` et `GET /health`, et le garde-fou `426 overlay_outdated` sur
  `/socket.io/*` ;
- s'arrête gracieusement sur signal (`Ctrl+C`, `SIGTERM` Unix) avec drainage des
  connexions et fermeture du pool SQLite ;
- tient le budget de démarrage : **< 500 ms** côté processus.

Hors périmètre Phase 3a (ne pas ajouter ici sans phase dédiée) : `/overlay/*`,
`/ingest/*`, `/admin/*`, `/ws` (hub v2), workers de lecture, médias, pairing.

---

## 2. Architecture

`apps/lc-server` est à la fois une **bibliothèque** (`lc_server`) et un
**binaire** mince :

```text
apps/lc-server/
├── Cargo.toml
└── src/
    ├── lib.rs           # API publique, cycle de vie, bind, serve, drainage
    ├── main.rs          # configuration → run() → code de sortie (sans panic)
    ├── config.rs        # Config d'environnement typée/validée, Secret, tests
    ├── app.rs           # AppState, ouverture SQLite, router(), middlewares
    ├── shutdown.rs      # Shutdown/ShutdownSignal, signaux, DRAIN_TIMEOUT
    └── http/
        ├── mod.rs       # sous-routeurs + documentation des extensions
        ├── root.rs      # GET /
        ├── health.rs    # GET /health (HealthProbe, DiscordProbe)
        ├── socket_io.rs # 426 overlay_outdated (legacy socket.io)
        ├── security.rs  # CSP + en-têtes de sécurité
        ├── cors.rs      # origines explicites, tauri.localhost, loopback dev
        ├── rate_limit.rs# 100 req / 15 min par IP, exemptions
        └── error.rs     # normalisation JSON des erreurs, repli 404
```

La découpe bibliothèque/binaire est une exigence d'intégration : les lanes
suivantes greffent leurs sous-routeurs dans `app::router()` et testent le
serveur complet avec `tower::ServiceExt::oneshot`, sans toucher `main.rs`.

### Chaîne de middlewares

De l'intérieur vers l'extérieur (voir `app::router`) :

1. `DefaultBodyLimit` — corps limité à **1 Mio** (défaut Fastify legacy) ;
2. rate limit (`http::rate_limit::enforce`) ;
3. normalisation d'erreurs (`http::error::normalize_errors`) ;
4. compression gzip ;
5. CORS ;
6. en-têtes de sécurité (CSP…) ;
7. `TraceLayer` — span `http_request` corrélé par `x-request-id` ;
8. propagation puis génération de `x-request-id`.

Toute réponse d'erreur qui traverse les couches est donc déjà du JSON
`{"error": …}`.

### API publique (`lc_server`)

| Élément | Rôle |
| --- | --- |
| `mod config` | `Config`, `ConfigError`, `LogLevel`, `VideoEncoder`, `Secret` |
| `mod app` | `AppState`, `router(AppState)`, `open_database(&Config)`, `MAX_REQUEST_BODY_BYTES` |
| `mod http` | sous-routeurs et middlewares (extensions Phases 3b+) |
| `mod shutdown` | `Shutdown`, `ShutdownSignal`, `install_signal_handlers`, `DRAIN_TIMEOUT` |
| `pub use app::{router, AppState}` | réexports pour les tests et les lanes |
| `pub use config::{Config, ConfigError}` | réexports |
| `pub use shutdown::{Shutdown, ShutdownSignal, DRAIN_TIMEOUT}` | réexports |
| `pub const LISTEN_ADDRESS` | `Ipv4Addr::UNSPECIFIED` (`0.0.0.0`) |
| `pub fn init_tracing(&Config)` | journalisation `tracing` (priorité `RUST_LOG` puis `LOG`) |
| `pub async fn run(Config)` | point d'entrée du binaire ; erreur = code 1 |

---

## 3. Démarrage mesuré

Méthode : build `--release`, base SQLite sur fichier temporaire
(`DATABASE_URL=file:…`), port éphémère, `NODE_ENV=production`, mesure du
lancement du processus jusqu'à l'acceptation de la première connexion TCP sur
`127.0.0.1:PORT`, sur 5 itérations. La mesure « interne » (`startup_ms`) est
prise entre l'entrée dans `serve()` et l'écoute effective (ouverture/migrations
SQLite incluses), d'après le log de démarrage.

| Mesure | Résultat (5 runs release) | Budget |
| --- | --- | --- |
| Démarrage interne `serve()` → écoute | **32–40 ms** (médiane 35 ms) | — |
| Processus → socket en écoute | **min 82 ms / médiane 83 ms / max 145 ms** | **< 500 ms** ✅ |
| Première réponse `GET /health` | `200` immédiat après l'écoute | — |

Références : legacy Fastify **3,67 s** (prod) / 14,42 s (dev) ; POC serveur
**0,53 s** process release. Le socle Rust est donc ~6× plus rapide que le POC et
~44× plus rapide que le legacy, très en dessous du budget.

## 4. Configuration d'environnement

`Config::from_env()` lit **uniquement l'environnement du processus** (pas de
`.env`, pas de `dotenvy` : le lanceur/installateur fournit les variables). Toute
erreur produit un `ConfigError` nommant la variable et la valeur fautive ;
`main` affiche `configuration invalide : …` et sort en **code 1**, sans panic.
Une variable vide est traitée comme absente.

`Config::from_pairs(…)` est la variante testable (aucun accès à l'environnement
global) : c'est elle qu'utilisent les tests.

### Variables

| Variable | Défaut | Notes |
| --- | --- | --- |
| `NODE_ENV` | `production` | `development` active les exemptions loopback (CORS + rate limit) |
| `LOG` | `info` | `info` `debug` `error` `silent` `warning` (traduit en directive `tracing`) |
| `RUST_LOG` | — | prioritaire sur `LOG` (format `EnvFilter`) |
| `PORT` | `3000` | `0` accepté (port éphémère, utile aux tests) |
| `I18N` | `fr` | parité legacy |
| `API_URL` | **requis** | URL absolue http(s) ; son origine sert au CORS |
| `ADMIN_UI_TOKEN` | `""` | secret masqué dans les logs |
| `ADMIN_UI_LOCAL_ONLY` | `true` | booléen strict |
| `DISCORD_TOKEN` | — | optionnel ; avec `DISCORD_CLIENT_ID` ou aucun des deux |
| `DISCORD_CLIENT_ID` | — | optionnel ; les deux ou aucun (sinon erreur) |
| `INGEST_API_TOKEN` | `""` | secret masqué |
| `DATABASE_URL` | — | optionnel ; absent/vide ⇒ base désactivée (`/health` `db=disabled`) |
| `HIDE_COMMANDS_DISABLED` | `false` | booléen strict (legacy : chaîne `"false"`) |
| `DEFAULT_DURATION` | `5` | secondes |
| `PLAYBACK_JOB_RETENTION_HOURS` | `72` | |
| `MEDIA_STORAGE_DIR` | `./data/media` | |
| `MEDIA_CACHE_TTL_HOURS` | `12` | |
| `MEDIA_CACHE_MAX_TOTAL_MB` | `5120` | |
| `MEDIA_BOARD_MAX_TOTAL_MB` | `15360` | |
| `MEDIA_DOWNLOAD_TIMEOUT_MS` | `180000` | |
| `MEDIA_MAX_SIZE_MB` | `2048` | |
| `MEDIA_VIDEO_MAX_HEIGHT` | `1080` | |
| `MEDIA_VIDEO_ENCODER` | `auto` | `auto` `libx264` `h264_nvenc` |
| `MEDIA_VIDEO_NVENC_PRESET` | `p4` | |
| `MEDIA_VIDEO_PRESET` | `ultrafast` | |
| `MEDIA_AUDIO_NORMALIZE_ENABLED` | `true` | |
| `MEDIA_AUDIO_LOUDNORM_I` | `-18` | |
| `MEDIA_AUDIO_LOUDNORM_LRA` | `11` | |
| `MEDIA_AUDIO_LOUDNORM_TP` | `-1.5` | |
| `MEDIA_MAX_CONCURRENT_INGESTIONS` | `2` | annexe C migration-plan |
| `MEDIA_URL_TTL_SEC` | `300` | ≤ `MEDIA_URL_TTL_MAX_SEC` (3600) sinon erreur |
| `MEDIA_URL_SECRET` | `""` | secret masqué |
| `TWITTER_OEMBED_ENDPOINT` | `https://publish.twitter.com/oembed` | URL validée |
| `TWITTER_VIDEO_RESOLVER_BASE_URL` | `https://vxtwitter.com` | URL validée |
| `TWITTER_CARD_DURATION_SEC` | `15` | |
| `PAIRING_CODE_TTL_MINUTES` | `5` | |
| `YTDLP_BINARY` | `yt-dlp` | |
| `YTDLP_CONCURRENT_FRAGMENTS` | `1` | |
| `YTDLP_FORMAT` | format legacy long | valeur par défaut reprise telle quelle |
| `TIKTOK_COOKIE` | `""` | secret masqué |
| `FFMPEG_BINARY` | `ffmpeg` | |
| `FFPROBE_BINARY` | `ffprobe` | |
| `ADDITIONAL_CORS_ORIGINS` | `""` | origines séparées par des virgules |
| `MIN_OVERLAY_VERSION` | `3.0.0` | doit ressembler à `x.y.z` (contrat 426) |
| `TRUST_PROXY_HEADERS` | `false` | si `true`, `X-Forwarded-For` fait foi pour l'IP |
| `WS_HEARTBEAT_TIMEOUT_SEC` | `45` | réservé Phase 4 |

### Règles de validation

- booléens : `true`/`false` **uniquement** (casse insensible), toute autre valeur
  est une erreur explicite ;
- `API_URL`, `TWITTER_*` : URL http(s) absolues (validation `url`) ;
- `ADDITIONAL_CORS_ORIGINS` : origines pures (schéma + hôte + port, sans chemin) ;
- `MIN_OVERLAY_VERSION` : `majeure.mineure.correctif` (comparaison legacy
  inchangée, voir `lc-protocol`) ;
- Discord : token et client id ensemble ou aucun (erreur `PartialDiscord`
  sinon) ;
- `MEDIA_URL_TTL_SEC` : plafonné à 3600 s.

---

## 5. Surface HTTP (Phase 3a)

| Méthode | Chemin | Réponse |
| --- | --- | --- |
| `GET` | `/` | `200 {"service":"livechat-server","status":"ok"}` (identité legacy) |
| `GET` | `/health` | `200` JSON enrichi (voir ci-dessous) |
| *toute* | `/socket.io`, `/socket.io/`, `/socket.io/*` | `426 {"error":"overlay_outdated","protocolVersion":2,"minOverlayVersion":"3.0.0"}` |
| *toute* | autre | `404 {"error":"request_error"}` |

Toutes les réponses portent `x-request-id` (généré si absent, propagé sinon).
Le corps des requêtes est limité à **1 Mio** (`413` normalisé JSON).

### `GET /health`

Exemple (sans dépendance configurée) :

```json
{
  "status": "ok",
  "version": "3.0.0",
  "protocolVersion": 2,
  "db": "ok",
  "discord": "disabled",
  "uptimeSec": 0,
  "activeJobs": 0
}
```

| Champ | Valeurs | Sémantique |
| --- | --- | --- |
| `status` | `ok` \| `degraded` | `degraded` si `db` ou `discord` vaut `down` |
| `version` | `CARGO_PKG_VERSION` (`3.0.0`) | version du serveur |
| `protocolVersion` | `2` | `lc_protocol::PROTOCOL_VERSION` |
| `db` | `ok` \| `down` \| `disabled` | `ok` = `SELECT` de comptage des jobs `PLAYING` réussi (< 1 s) |
| `discord` | `ok` \| `down` \| `disabled` | `disabled` sans configuration ; `down` tant que le bot n'est pas connecté (Phase 6) |
| `uptimeSec` | entier ≥ 0 | secondes depuis le démarrage (horloge injectable) |
| `activeJobs` | entier \| `null` | jobs au statut `PLAYING` ; absent (`null`) si base indisponible |

Décision : **HTTP 200 même dégradé** (parité du legacy `/health` 200, utile aux
load-balancers) ; la dégradation est portée par `status`/`db`/`discord`. Une
base configurée mais impossible à ouvrir fait échouer le démarrage (code 1) au
lieu de servir silencieusement sans données.

### Normalisation des erreurs

- réponses `4xx/5xx` déjà en JSON : inchangées (assets/headers conservés) ;
- réponses d'erreur non JSON (extracteurs axum, `405`, `413`, `415`…) :
  remplacées par `{"error":"request_error"}` (4xx) ou
  `{"error":"internal_error"}` (5xx), **statut et en-têtes conservés** ;
- chaque erreur est journalisée dans le span `http_request` (request id) :
  `error!` pour 5xx, `warn!` pour 4xx.

---

## 6. Sécurité

### CSP et en-têtes

`Content-Security-Policy` :

```text
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline';
img-src 'self' data: https:; media-src 'self'; font-src 'self';
connect-src 'self'; object-src 'none'; frame-src 'none';
base-uri 'self'; form-action 'self'; frame-ancestors 'none'
```

Par rapport à Helmet legacy, `script-src` **n'autorise plus l'inline**
(correctif S4 du migration-plan) ; `style-src` garde `'unsafe-inline'` (styles
inline nécessaires à l'UI). En plus : `X-Content-Type-Options: nosniff`,
`X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`,
`Cross-Origin-Resource-Policy: cross-origin` (parité), `Permissions-Policy`
restrictive. COEP reste désactivé comme dans le legacy.

### CORS

Origines autorisées **explicitement** (aucun refléchissement arbitraire) :

- origine de `API_URL` ;
- `ADDITIONAL_CORS_ORIGINS` ;
- `http://tauri.localhost` et `https://tauri.localhost` (overlay Tauri) ;
- en `NODE_ENV=development` seulement : tout loopback
  (`localhost`, `127.0.0.0/8`, `::1`).

Méthodes : `GET PUT DELETE POST OPTIONS PATCH` ; en-têtes demandés (legacy
complet + `x-request-id`/`range`) ; en-têtes exposés :
`x-request-id`, `x-ratelimit-*`, `retry-after`. `Access-Control-Allow-Credentials`
n'est **jamais** émis (correctif S5 : origines explicites **sans** credentials,
contrairement au legacy qui combinait les deux). Préflight mis en cache 600 s.

### Rate limiting

- **100 requêtes / 15 minutes par IP**, fenêtre fixe, compteur en mémoire
  (`Arc<Mutex<…>>`), nettoyage opportuniste des fenêtres expirées ;
- IP lue depuis la connexion (`ConnectInfo`), ou premier `X-Forwarded-For`
  **seulement si** `TRUST_PROXY_HEADERS=true` ; les IPv4 encapsulées en IPv6
  (`::ffff:a.b.c.d`) sont normalisées ;
- exemptions : préfixe `/admin` (l'UI admin a ses propres contrôles) et, en
  `development`, les adresses loopback ;
- réponse `429 {"error":"request_error"}` avec `x-ratelimit-limit`,
  `x-ratelimit-remaining`, `x-ratelimit-reset` et `Retry-After` ; même
  informations sur les réponses acceptées.

Limite connue : compteur par processus (pas de partage multi-instance) —
suffisant pour un serveur local mono-processus ; à revoir si un déploiement
multi-instance apparaît (le legacy était également en mémoire).

### Journalisation et corrélation

`tracing` (fmt), niveau piloté par `RUST_LOG` puis `LOG` ; chaque requête a un
span `http_request` (`method`, `uri`, `request_id`) ; aucun en-tête n'est
journalisé (donc jamais `Authorization`/`Cookie`). Pas d'endpoint de debug ni de
shutdown exposé.

---

## 7. Arrêt gracieux

- `Shutdown` (watch booléen) est partagé entre `axum::serve`, les futurs
  workers et la fermeture SQLite ;
- signaux : `Ctrl+C` (toutes plateformes) et `SIGTERM` (Unix uniquement ;
  l'arrêt de service Windows viendra avec l'installateur Phase 7) ;
- après signal : arrêt de l'acceptation, drainage des connexions en cours avec
  `DRAIN_TIMEOUT = 10 s` (au-delà : arrêt forcé + `warn!`), puis fermeture du
  pool SQLite ;
- aucun `timeout` global n'enveloppe `axum::serve` (recommandation POC : ne pas
  couper les téléchargements/ingestions longs).

---

## 8. Tests

Suites vertes : `cargo fmt -p lc-server -- --check`,
`cargo clippy -p lc-server --all-targets -- -D warnings`,
`cargo test -p lc-server` → **71 tests**, 0 échec.

| Module | Tests | Couverture |
| --- | --- | --- |
| `config` | 24 | défauts legacy, overrides complets, erreurs (PORT/LOG/URL/bool/encodeur/TTL/Discord partiel), secrets masqués, `from_env` |
| `app` | 16 | contrat HTTP complet via `tower::ServiceExt::oneshot` : `/`, 404, 405, `/health` (disabled/db ok/dégradé/uptime), 426 exact, CSP, request id, CORS tauri, rate limit (100 puis 429, admin, loopback dev), body limit 413, ouverture SQLite optionnelle |
| `http::cors` | 8 | tauri http(s), API_URL, origines additionnelles, inconnue/null refusées, loopback dev, absence de credentials |
| `http::rate_limit` | 8 | limite puis refus, clients séparés, reset de fenêtre, purge, seau inconnu, exemptions, XFF fiable seulement si activé, normalisation IPv4-mapped |
| `http::error` | 4 | normalisation 4xx/5xx, JSON intact, 2xx intact |
| `http::health` | 3 | sérialisation lowercase, DiscordProbe disabled/down |
| `http::security` | 2 | en-têtes présents, CSP sans inline script |
| `shutdown` | 5 | trigger/wait, idempotence, abonné tardif, sender disparu, arrêt du serveur HTTP |
| `lib` | 1 | `Shutdown` → l'`axum::serve` se termine proprement |

Les tests « app-level » utilisent le vrai routeur et de vraies requêtes HTTP
in-process : c'est l'équivalent d'une suite d'intégration, sans port ni serveur
externe.

---

## 9. Intégration Phases 3b / 4 / 7

Le socle a été conçu pour que les lanes suivantes n'aient **pas** à modifier
`main.rs` ni `lib.rs`.

### Brancher de nouvelles routes

```rust
// apps/lc-server/src/app.rs — app::router()
Router::new()
    .merge(http::root::router())
    .merge(http::health::router())
    .merge(http::socket_io::router())
    // Phases 3b+ : .merge(http::overlay::router()) etc.
    .fallback(http::error::fallback)
    …
```

Chaque module de `http/` suit le même contrat : `pub fn router() -> Router<AppState>`
(sous-routeur) ou `pub async fn middleware(State, Request, Next) -> Response`.
L'état partagé est `AppState` (cloneable) : `config`, `db`, `clock`,
`started_at`, `shutdown`, `discord`. Un handler accède à la base via
`state.db.as_ref()` (repositories lc-db) et doit utiliser `state.clock` (pas
`Utc::now()`) pour rester testable.

### Points d'extension précis

| Besoin | Point d'extension |
| --- | --- |
| Routes `/overlay/*` (8), `/ingest/*` (2), `/admin/*` (14) | nouveaux modules `http::overlay`, `http::ingest`, `http::admin` mergés dans `app::router()` |
| Auth admin (`ADMIN_UI_TOKEN`) | couche ou extracteur dédié, `Secret::expose_secret` uniquement au point de comparaison |
| Tokens ingest (`INGEST_API_TOKEN`) | idem |
| Hub WebSocket v2 `/ws` | route axum dédiée dans `http::ws` ; `tokio-tungstenite = "0.29"` (aligné axum 0.8.9), `max_message_size` **256 Kio** (4 × 64 Kio pour ne pas masquer le close 4408) |
| Workers playback | `tokio::spawn` + `AppState.shutdown.subscribe()` pour l'arrêt |
| État Discord réel (Phase 6) | implémenter `http::health::HealthProbe` et `AppState::with_discord_probe(Arc<dyn HealthProbe>)` |
| Dépendance externe à sonder | trait `HealthProbe` (`fn state(&self) -> HealthState`) |
| Fermeture propre | `shutdown.trigger()` puis attendre `ShutdownSignal::wait()` dans les workers |
| Assets/médias | `tower-http::services::ServeFile` (Range/206) en Phase 3b/7 |
| Base partagée | `app::open_database(&Config)` uniquement au démarrage ; `Db` est cloneable |

### Recommandations POC déjà prises en compte

- pas de timeout global autour de `axum::serve` (drain explicite) ;
- pas d'endpoints debug/shutdown ;
- présence/last_seen en écriture asynchrone (`synchronous=NORMAL` côté SQLite à
  valider Phase 3b, gain ×7 mesuré au welcome) ;
- `max_message_size` WS à 256 Kio le jour où le hub arrive.

### Ce que la Phase 3a ne fournit pas encore

- montée en charge WebSocket (heartbeat, close codes) : Phase 4 ;
- limitation de débit « par route » (le legacy y réfléchissait) : le socle
  applique une limite par client, tuning possible en Phase 3b ;
- support IPv6 complet : écoute IPv4 (`0.0.0.0`), voir risques ;
- TLS : terminé par le reverse proxy (comme le legacy).

---

## 10. Écarts et décisions par rapport au legacy

| Sujet | Legacy Fastify | Rust Phase 3a | Raison |
| --- | --- | --- | --- |
| Discord au démarrage | requis (exit 1) | optionnel ; `down`/`disabled` dans `/health` | exigence d'audit : démarrer sans Discord |
| Base de données | `DATABASE_URL` requise | optionnelle ; configurée mais KO ⇒ exit 1 | développement local + dégradation explicite |
| CORS credentials | `credentials: true` + origines explicites | jamais de credentials | correctif S5 (combinaison interdite) |
| CORS Tauri | origines API seulement | + `http(s)://tauri.localhost` | overlay Tauri |
| CSP script | `'self' 'unsafe-inline'` | `'self'` sans inline | correctif S4 |
| `/health` | `{"status":"ok"}` | enrichi (`db`, `discord`, `uptimeSec`, `activeJobs`, version, protocole) ; 200 même dégradé | observabilité |
| Rate limit | plugin `@fastify/rate-limit` (100/15 min, allowList `/admin` + loopback dev) | compteur maison en mémoire, mêmes valeurs et exemptions, en-têtes `x-ratelimit-*` + `Retry-After` | moins de dépendances, tests déterministes |
| Erreurs | handler Fastify (`internal_error`/`request_error`) | normalisation de toute réponse non JSON, mêmes corps | cohérence avec les réponses axum |
| `.env` | `dotenv` | environnement du processus uniquement | le lanceur fournit l'environnement ; pas de dépendance |
| `HIDE_COMMANDS_DISABLED` | chaîne `"false"` interprétée par le code Discord | booléen strict `false` | typage |
| Écoute | `::` dual-stack | `0.0.0.0` IPv4 | démarrage garanti si IPv6 absent (voir risques) |
| Journalisation | `pino` (`LOG`) | `tracing` (`RUST_LOG` prioritaire, sinon `LOG` traduit) | parité des niveaux + flexibilité Rust |

---

## 11. Risques connus et points Phase 3b

1. **IPv4 seul** : le legacy écoutait en dual-stack. Un client qui résout
   `localhost` en `::1` d'abord doit basculer en IPv4 (comportement standard
   des clients HTTP), mais un déploiement 100 % IPv6 ne fonctionnera pas.
   À trancher en Phase 7 (installateur/reverse proxy) ; un double listen
   IPv4+IPv6 est possible sans casser l'API.
2. **Compression + Range** : `CompressionLayer` est global ; les réponses
   médias (ServeFile, 206/`Range`) ne doivent pas être compressées, sinon le
   client reçoit un flux invalide. À exclure explicitement lors de l'ajout des
   routes médias (Phase 3b/7).
3. **Rate limit et IP réelle** : `TRUST_PROXY_HEADERS=true` fait confiance à
   `X-Forwarded-For` ; ne l'activer que derrière un proxy de confiance, sinon
   l'IP est falsifiable (documenté).
4. **`/health` `discord=down`** tant que la Phase 6 n'injecte pas l'état réel :
   un overlay qui attend `ok` verra `down` si Discord est configuré. À
   connecter au bot en Phase 6.
5. **`activeJobs`** ne compte que `PLAYING` ; les statuts `PENDING/FAILED`
   pourront enrichir `/health` en Phase 3b si utile.
6. **Body limit 1 Mio** : parité Fastify, mais les routes d'ingestion qui
   accepteraient des corps plus gros devront relever la limite localement
   (`DefaultBodyLimit::max` par sous-routeur), pas globalement.
7. **CORS sans credentials** : si l'admin UI a besoin de cookies, il faudra
   revoir le correctif S5 (préférer un token d'en-tête, déjà prévu par
   `ADMIN_UI_TOKEN`).
8. **Compteur de rate limit en mémoire** : perdu au redémarrage et non partagé
   entre instances (acceptable pour un serveur local).

---

## 12. Références

- `docs/rust/migration-plan.md` — architecture cible, annexe C (variables), S4/S5/S6.
- `docs/rust/protocol-v2.md` — version de protocole, contrat 426, close codes.
- `docs/rust/audit-dynamique.md` — comportement legacy mesuré (démarrage, RSS, `/health`).
- `docs/rust/lc-db.md` — API `Db`, schéma, migrations.
- `docs/rust/poc-server-report.md` — recommandations POC reprises ici.
- `crates/lc-protocol` — constantes de protocole partagées.
