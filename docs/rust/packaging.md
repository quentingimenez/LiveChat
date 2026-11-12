# Packaging et déploiement du serveur Rust — Phase 9b

> Statut : Phase 9b (2026-11-10). Contrat de l'image de production `lc-server`, workflow
> de publication GHCR, procédure de migration et runbook de rollback.
> Source de vérité du comportement du serveur : `docs/rust/server-core.md` ; persistance :
> `docs/rust/lc-db.md` et `docs/rust/migrator.md` ; plan de bascule :
> `docs/rust/migration-plan.md` §5/§9.

## 1. Objectif et périmètre

L'image legacy `server/Dockerfile` (Node 20 + `pnpm dev`) est un des problèmes de l'audit
(migration-plan, S7) : image de développement, pas de healthcheck, pas de migration au
boot, exécution en root. La Phase 9b la remplace par une image de production :

- binaire Rust `lc-server` compilé en `--release --locked` (version workspace **3.0.0**,
  protocole **v2**) ;
- base d'exécution Debian slim, utilisateur **non-root**, `tini` comme PID 1,
  `HEALTHCHECK` sur `/health`, arrêt propre sur `SIGTERM` ;
- `ffmpeg`/`ffprobe` (sondage et transcodage médias) et `yt-dlp` épinglé (ingestion
  TikTok/Twitter) ;
- publication GHCR par CI (tags `server-v*`), avec job de smoke test.

**Hors périmètre Phase 9b** : la bascule de production elle-même (fenêtre de maintenance,
publication de l'overlay, surveillance 30 min) — voir migration-plan §9 ; les handlers
admin et médias complets (Phases 7b/3b) ; la terminaison TLS (assurée par le reverse
proxy, le serveur écoute en HTTP sur le réseau interne).

## 2. Livrables

| Fichier | Rôle |
| --- | --- |
| `deploy/server.Dockerfile` | Image multi-stage builder + runtime. |
| `deploy/server.Dockerfile.dockerignore` | Contexte Docker minimal (BuildKit) : le dépôt pèse ~23 Go. |
| `deploy/docker-compose.yml` | Compose de production (service `lc-server`, volume `./data`, healthcheck JSON) — voir §10. |
| `.github/workflows/server-image.yml` | CI : buildx amd64, push GHCR (**propriétaire unique des tags `livechat-server`**), provenance, smoke test ; actions épinglées par SHA. |
| `docs/rust/packaging.md` | Ce document (contrat, runbooks, mesures, limites). |

Le legacy `server/Dockerfile` et `server/docker-compose.yml` ne sont **pas** modifiés :
ils restent la référence de rollback (image 2.0.1).

Chaîne de release P0-4 : `server-image.yml` est le **seul** workflow autorisé à publier
l'image GHCR ; `server.yml` (legacy Node) ne fait que tester et les workflows de release
Electron (`overlay-release.yml`, `overlay-nightly.yml`) ont été retirés. Le canal overlay
3.0.0 a son auto-update **explicitement désactivé** (voir
[docs/rust/overlay-release.md](overlay-release.md) §6-§7).

## 3. Contrat de l'image

### 3.1 Stages et reproductibilité

| Élément | Valeur |
| --- | --- |
| Builder | `rust:1.99.1-bookworm` (digest `sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`) |
| Runtime | `debian:bookworm-slim` (digest `sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc`) |
| Toolchain Rust | `1.99.1` (host `x86_64-unknown-linux-gnu`), forcée via `RUSTUP_TOOLCHAIN` |
| Build cargo | `cargo build --release --locked -p lc-server --bin lc-server` |
| Parallélisme | `CARGO_BUILD_JOBS=4` (override du `.cargo/config.toml` du dépôt, `jobs=8`) |
| Verrouillage | `Cargo.lock` (`--locked`), base et toolchain épinglées par tag/digest |

**Cache de couches — pas de cargo-chef.** Le workspace contient `apps/lc-overlay`
(Tauri 2 : `build.rs` + dépendances WebKit/GTK). `cargo chef cook` travaille à l'échelle
du workspace et tenterait de compiler ces dépendances, qui ne sont pas installables dans
une image slim (et ne concernent pas le serveur). L'image utilise donc le pattern
« manifestes seuls + sources factices » :

1. copie de `Cargo.toml`/`Cargo.lock`/`rust-toolchain.toml`/`.cargo/config.toml` et des
   `Cargo.toml` de tous les membres du workspace ;
2. génération de sources factices (`src/lib.rs` vide, `src/main.rs` vide, `build.rs` vide
   pour `lc-overlay`) puis `cargo build --release -p lc-server` : seules les dépendances
   de `lc-server` sont compilées, dans une couche qui ne dépend que des manifestes ;
3. `COPY` des vraies sources puis rebuild : seules les crates du workspace local
   (`lc-*`, `lc-server`) sont recompilées.

Résultat : une modification de code ne recompile pas les ~600 dépendances (serenity,
axum, sqlx, tokio, rustls…). Le cache registre/git est monté via
`--mount=type=cache,target=/usr/local/cargo/registry` (+ `git`) ; il est local au démon et
n'est pas nécessaire en CI (cache GHA de `docker/build-push-action`).

### 3.2 Utilisateur, processus, signaux

| Élément | Valeur |
| --- | --- |
| Utilisateur | `livechat` (uid **10001**, gid **10001**), `/bin/false`, home `/app` |
| PID 1 | `tini -- /usr/local/bin/lc-server` (récolte des zombies, forward des signaux) |
| Arrêt | `STOPSIGNAL SIGTERM` → `lc-server` draine (`DRAIN_TIMEOUT = 10 s`) puis ferme SQLite et sort 0 |
| Binaire | `/usr/local/bin/lc-server` (strippé, `--release`) |

Le serveur **n'a pas de flag CLI** : `apps/lc-server/src/main.rs` appelle
`Config::from_env()` puis `lc_server::run(config)`. Toute la configuration est
environnementale ; `PORT` pilote le bind (`0.0.0.0:PORT`), il n'existe pas de `--bind`.
Une configuration invalide ⇒ message `configuration invalide : …` sur stderr et **exit 1**
(sans panic) ; un signal `SIGTERM` ⇒ arrêt propre (code 0 attendu).

### 3.3 Ports, volumes, système de fichiers

| Élément | Valeur |
| --- | --- |
| Port | `3000/tcp` (`EXPOSE 3000`), bind `0.0.0.0:3000` |
| Volume | `/app/data` (`VOLUME`), propriétaire `10001:10001` |
| Base SQLite | `/app/data/livechat.sqlite` (+ `-wal`, `-shm` en WAL) |
| Médias | `/app/data/media` (`MEDIA_STORAGE_DIR`) |

Le volume `/app/data` est **obligatoire** en production (base + médias). Sans volume,
les données vivent dans la couche inscriptible du conteneur et sont perdues au
remplacement. Un volume nommé vide hérite des droits de l'image (uid 10001) : le
processus non-root peut créer la base au premier démarrage (vérifié en smoke).

### 3.4 Variables d'environnement

Défauts posés **dans l'image** (surchargeables par `docker run -e` ou l'orchestrateur) :

| Variable | Défaut image | Obligatoire | Note |
| --- | --- | --- | --- |
| `PORT` | `3000` | non | port d'écoute |
| `API_URL` | `http://localhost:3000` | **oui** (défaut posé) | URL absolue http(s) ; origine CORS + base des URL médias (`{API_URL}/overlay/media/{assetId}`). **À surcharger avec l'origine publique réelle.** |
| `DATABASE_URL` | `sqlite:///app/data/livechat.sqlite` | non | absent/vide ⇒ base désactivée (`/health db=disabled`) ; si configurée mais KO ⇒ exit 1 |
| `MEDIA_STORAGE_DIR` | `/app/data/media` | non | stockage des médias ingérés |
| `RUST_LOG` | `info` | non | prioritaire sur `LOG` |
| `ADMIN_UI_LOCAL_ONLY` | `true` | non | AUD-COR-10 : l'API admin n'accepte que le loopback (défaut serveur). Dans un conteneur bridge, le serveur voit l'IP de la passerelle : passer à `false` pour utiliser le panneau (protégé par le bind et `ADMIN_UI_TOKEN`) — §10.4 |
| `MIN_OVERLAY_VERSION` | `3.0.0` | non | contrat 426 `/socket.io` |
| `ADMIN_UI_TOKEN` | `""` | **prod : oui** | secret ; vide ⇒ endpoints admin `503 admin_ui_disabled` |
| `INGEST_API_TOKEN` | `""` | **prod : oui** | secret d'ingestion |
| `MEDIA_URL_SECRET` | `""` | **prod : oui** | clé HMAC des URL médias signées |
| `DISCORD_TOKEN`/`DISCORD_CLIENT_ID` | — | non | optionnels, mais fournis **ensemble** ; absents ⇒ `discord=disabled` |

Secrets : jamais dans l'image ; fournis par l'orchestrateur (variables d'environnement,
secret manager). `Config` masque les secrets dans ses `Debug` et ne lit pas de `.env`.

### 3.5 Healthcheck

`HEALTHCHECK --interval=15s --timeout=3s --start-period=15s --retries=3` exécute
`curl -fsS http://127.0.0.1:${PORT}/health`. Le contrat `/health` répond **toujours
HTTP 200**, y compris `degraded` : le healthcheck Docker valide donc la disponibilité du
processus HTTP, pas la santé métier. Les sondes d'orchestration qui doivent distinguer
`degraded` lisent le JSON (`status`, `db`, `discord`).

### 3.6 Migrations SQLite

`apps/lc-server/src/app.rs` ouvre la base avec `DbConfig::default()` : `run_migrations =
true`, `check_schema_version = true`, pool max 5, `busy_timeout` 5 s, WAL + FK. Les
migrations embarquées (`sqlx::migrate!` compilé dans le binaire) sont appliquées **au
démarrage**, de façon idempotente, puis `app_metadata.schema_version` doit valoir
`3.0.0`, sinon le démarrage échoue (exit 1).

Conséquences opérationnelles :

- pas de commande/sous-commande de migration dans le binaire serveur (aucun `--migrate`) :
  le démarrage du conteneur migre le schéma ; c'est le comportement actuel de la Phase 3a,
  documenté ici pour ne pas le découvrir en prod ;
- ne **jamais** pointer `DATABASE_URL` vers la base legacy Prisma : le serveur 3.0.0
  appliquerait ses migrations dans ce fichier. La migration de données legacy → schéma 3.0
  est un outil **hors runtime** (`cargo xtask migrate-legacy`, one-shot, base legacy
  ouverte en lecture seule), voir `docs/rust/migrator.md` ;
- mettre à jour une instance = nouveau conteneur sur le même volume : les migrations sqlx
  passent à jour et sont tracées dans `_sqlx_migrations`.

## 4. Build local

Depuis la racine du dépôt (Docker 28.5.2, BuildKit) :

```powershell
docker build -f deploy/server.Dockerfile -t livechat-server:3.0.0 .
docker run --detach --name lc-server-test `
  --publish 127.0.0.1:3000:3000 `
  --volume lc-server-test-data:/app/data `
  livechat-server:3.0.0
```

Rebuild exact (après changement de code, cache deps conservé) :
`docker build -f deploy/server.Dockerfile -t livechat-server:<version> .`

Mise à jour de `yt-dlp` : bumper `ARG YTDLP_VERSION` et `ARG YTDLP_SHA256` en tête du
stage `runtime` (hash officiel dans `SHA2-256SUMS` de la release), rebuild, smoke.

## 5. Mesures locales

<!-- MESURES:DÉBUT -->
Mesures du 2026-11-10, Docker 28.5.2 (BuildKit, context `desktop-linux`, machine Windows 11
8 cœurs/16 threads), image finale `livechat-server:3.0.0`
(id `sha256:6f12a09d2aac18ecf1d0cad29c49cfefa95b738d89bc55b2186a1316e117cf15`).
Deux builds ont été mesurés : un build dit « snapshot » (validation, sources du commit
`068face`) puis le build final depuis l'arbre de travail une fois la lane `http-core`
stabilisée (voir la note en fin de section).

| Mesure | Valeur |
| --- | --- |
| Build à froid, toutes couches (snapshot HEAD) | **116 s** — deps builder 101 s, build des sources réelles 9,7 s, apt/ffmpeg 53 s (stages builder et runtime en parallèle) |
| Rebuild cache chaud (arbre de travail, deps en cache) | **27,6 s** (aucune dépendance recompilée : couches manifests/deps réutilisées ; seules les crates `lc-*` et le binaire sont rebuildés) |
| Contexte Docker transféré | **1,45 Mo** (dépôt complet ~23 Go ; `.dockerignore` BuildKit) |
| Taille image | **575,2 MiB (603,1 Mo — 603 149 752 octets)** — cible < 250 Mo **dépassée ×2,3** (voir ci-dessous) |
| Couche de base | `debian:bookworm-slim` 74,9 Mo |
| Couche ffmpeg/apt | **468 Mo** (`ffmpeg` Debian 5.1.9 + dépendances : libLLVM-15 112 Mo, libicudata 30 Mo, mesa/dri 25 Mo, libz3 23 Mo, x265 16 Mo… ; `--no-install-recommends` déjà utilisé) |
| Couche yt-dlp | 41,2 Mo (binaire standalone épinglé, SHA-256 vérifié au build) |
| Binaire `lc-server` | 19,37 Mo (release strippé, arbre de travail complet — 9,98 Mo sur le snapshot HEAD, la lane Discord étant alors incomplète) |
| Démarrage `docker run` → `/health` 200 | **432 ms** au premier run à froid (page cache vide), **35 ms** à chaud ; `startup_ms` interne du serveur = **30 ms** |
| `/health` (runtime réel) | 200 `{"status":"ok","version":"3.0.0","protocolVersion":2,"db":"ok","discord":"disabled","uptimeSec":0,"activeJobs":0}` ; base SQLite ouverte, migrations sqlx appliquées, `schema_version=3.0.0` |
| Utilisateur | `uid=10001(livechat) gid=10001(livechat)` — non-root ; PID 1 = `tini` |
| `ffmpeg`/`ffprobe` | 5.1.9-0+deb12u1 |
| `yt-dlp` | 2026.10.26 |
| HEALTHCHECK | `healthy` constaté après 20 s (`start_period` 15 s, interval 15 s, exit 0) |
| Données | `/app/data/livechat.sqlite` + `-wal`/`-shm` créés au premier boot, propriétaire 10001:10001, média dir `/app/data/media` ; volume inscriptible (write test OK) ; après arrêt propre le WAL est checkpointé à 0 |
| Arrêt SIGTERM (`docker stop`) | **exit code 0 en 412–456 ms** (drain < 10 s) ; logs : « signal d'arrêt reçu ; arrêt gracieux demandé » → « base SQLite fermée » → « serveur HTTP arrêté proprement » |
| Redémarrage sur le même volume | base persistée (`db:"ok"`) et `/health` 200 en **moins d'1 s** (559 ms au run à froid ; re-mesure à chaud trop rapide pour être significative) |
| `GET /` | `{"service":"livechat-server","status":"ok"}` |
| `GET /socket.io/` | 426 `overlay_outdated` (`minOverlayVersion: 3.0.0`) |
| Mémoire au repos | **≈ 5,9 MiB RSS** (`docker stats`, 12 échantillons : 5,55–5,93 MiB), serveur + `tini` |

**Budget taille : cible < 250 Mo non atteinte (575,2 MiB).** Le surcoût vient à ~78 % de
`ffmpeg` Debian et de ses dépendances d'accélération (LLVM, Mesa/Vulkan, codecs). Pistes,
par ordre de préférence :

1. **ffmpeg statique minimal** compilé dans le builder (seuls les codecs réellement
   utilisés par `lc-media` : H.264/AAC, VP9/AV1 en lecture, remux MP4/WebM) : supprime
   ~400 Mo de bibliothèques partagées ; image estimée ≈ 150–220 Mo. Coût : +5–10 min de
   build (couche mise en cache), options `--disable-everything` + `--enable-*`.
2. **Séparer le worker média** (`ffmpeg`/`yt-dlp`) du serveur HTTP : image serveur
   ≈ 90 Mo, worker dédié (déjà envisagé par l'architecture Phase 4 : ingestion
   asynchrone) — préférable à moyen terme.
3. Accepter 575 Mo en connaissance de cause : budget de stockage/registre GHCR
   confortable, redémarrages fréquents (couples registry→nœud) pénalisés.

Le choix n'est pas tranché en Phase 9b (hors périmètre) ; le contrat ci-dessous est
indépendant de la variante retenue.

**Note sur les builds mesurés.** Un premier build de validation a été fait sur un snapshot
`git archive HEAD` (commit `068face`) car la lane `http-core` modifiait `apps/lc-server` à
cet instant (modules `discord/event_handler.rs` et `discord/runtime.rs` absents → E0583) ;
`deploy/` avait été copié dans le snapshot. Une fois la lane stabilisée (`cargo check -p
lc-server --locked` OK en 32 s), l'image a été rebuildée depuis l'arbre de travail en
27,6 s de cache chaud : c'est cette image (id `6f12a09d…`) qui est mesurée ci-dessus. Le
Dockerfile est inchangé ; en CI, le build se fait toujours sur un checkout propre du tag.
<!-- MESURES:FIN -->

## 6. Publication CI et politique de tags

**Propriétaire unique des tags GHCR.** Depuis P0-4 (A7-003), `server.yml` (Node legacy)
ne publie plus d'image : seul `.github/workflows/server-image.yml` pousse
`ghcr.io/<owner>/livechat-server`. Toutes les actions tierces des workflows CI/release
sont épinglées par commit SHA.

`.github/workflows/server-image.yml` :

- déclencheurs : push de tag `server-v*` (release) et `workflow_dispatch`
  (input `version` au format `X.Y.Z`, option `update_latest`) ;
- permissions : `contents: read`, `packages: write` (GHCR via `GITHUB_TOKEN`) ;
- buildx `linux/amd64` (aucun besoin arm64 aujourd'hui), `provenance: true` (attestation
  SLSA du build), cache GHA (`type=gha,mode=max`) ;
- image : `ghcr.io/<owner>/livechat-server` ;
  - tag push `server-v3.0.0` → tags `server-v3.0.0` **et** `latest` ;
  - `workflow_dispatch` → tag `server-v<version>` (et `latest` seulement si
    `update_latest=true`) ;
- job `smoke` (après build) : pull de l'image publiée, `docker run`, attente `/health`
  (≤ 60 s), vérification `status`/`version`/`protocolVersion`/`db`, uid/gid **10001**
  non-root, `ffmpeg`/`ffprobe`/`yt-dlp`, persistance SQLite sous `/app/data`, puis
  `docker stop` (SIGTERM) et vérification `ExitCode == 0`.

Politique de rétention : conserver les tags `server-v*` de production (pas de `latest`
comme référence de rollback — **toujours** épingler une version) ; la visibilité du
package GHCR est à aligner sur celle du dépôt ; un tag publié est immuable (pas de
réécriture).

## 7. Runbook de déploiement et rollback

### 7.1 Déploiement d'une nouvelle version (résumé ; bascule complète : migration-plan §9)

1. vérifier la publication de l'image (`docker pull ghcr.io/<owner>/livechat-server:server-vX.Y.Z`) ;
2. sauvegarder le volume : arrêter l'instance, copier `/app/data` (SQLite + médias) —
   en WAL, préférer un arrêt propre ou `sqlite3 .backup` pour un instantané cohérent ;
3. démarrer le nouveau conteneur sur le même volume et le même port, avec les secrets
   (procédure compose : §10) ;
4. attendre `/health` à 200 avec `db:"ok"`, `version` attendue, `protocolVersion:2` ;
5. smoke fonctionnel (overlay : connexion WS, `welcome`, `play` ; admin : page et une
   action) ; surveiller 30 min (migration-plan §9 ligne 121) ;
6. conserver l'ancienne image et la sauvegarde pendant 7 jours.

### 7.2 Rollback

Rollback = **image 2.0.1 (legacy Node) + base legacy sauvegardée**, conformément à
migration-plan §5 et §9 : la migration de données écrivant dans un fichier neuf laisse la
base legacy intacte. Procédure :

1. arrêter le conteneur 3.0.0 ;
2. remettre en place la base legacy sauvegardée (fichier `sqlite.db` + éventuels
   `-wal`/`-shm`) dans le volume attendu par l'image 2.0.1 ;
3. redémarrer l'image `2.0.1` (legacy) et vérifier `/health` ;
4. conserver les données 3.0.0 (ne pas écraser) pour analyse ; publier un correctif
   avant toute nouvelle tentative.

Le rollback ne dépend pas de la base 3.0.0 : c'est la base legacy qui fait foi.

## 8. Limites, risques et dette

| Sujet | Constat | Mitigation / suite |
| --- | --- | --- |
| Taille d'image | Voir §5 ; `ffmpeg` Debian tire des dépendances d'accélération (LLVM/Mesa/Vulkan) lourdes | Cible < 250 Mo à réévaluer ; options : build ffmpeg statique minimal, ou image média séparée (worker) |
| `ffmpeg` | version Debian bookworm `7:5.1.9-0+deb12u1`, mise à jour liée à la distro | acceptée (codecs H.264/AAC/VP9/AV1 présents) ; monter de base = rebuild |
| `yt-dlp` | binaire standalone épinglé, mise à jour manuelle (ARG + SHA256) | cadence de revue trimestrielle ou à incident ; vérifier le hash au build |
| TLS | le serveur écoute en HTTP | terminaison TLS au reverse proxy ; `API_URL` doit être l'URL publique https — procédure §10.4 |
| `ADMIN_UI_LOCAL_ONLY` | `true` par défaut depuis AUD-COR-10 ; en conteneur bridge le serveur voit la passerelle | bind `127.0.0.1:3000` par défaut (non joignable LAN) ; pour le panneau : proxy TLS + `ADMIN_UI_LOCAL_ONLY=false` + `ADMIN_UI_TOKEN` long, ou `network_mode: host` — §10.4 |
| Secrets | fournis par l'environnement (pas de fichiers/dotenv) | secret manager / variables d'orchestrateur ; jamais dans l'image ni les logs |
| SQLite | un seul écrivain, volume local | une seule réplique ; sauvegardes régulières ; pas de multi-nœuds partagés |
| Migrations auto au boot | comportement Phase 3a | documenté §3.6 ; sauvegarde avant upgrade ; ne jamais pointer une base legacy |
| Attestations/provenance | `provenance: true` en CI | vérifier l'attestation au pull (`docker buildx imagetools inspect`) |
| Smoke CI | vérifie `/health` et le non-root, pas le comportement fonctionnel complet | tests de parité Phase 9a (harnais) en complément |

## 9. Écarts constatés par rapport au brief initial

- **`API_URL` n'était pas listé dans les variables du brief** : c'est pourtant la seule
  variable *requise* de `Config::from_env()`. L'image pose un défaut
  `http://localhost:3000` (démarrage possible) et la documentation impose de la
  surcharger avec l'origine publique.
- **`--bind` n'existe pas** : le brief proposait `lc-server --bind 0.0.0.0:3000` ; le
  binaire n'a aucun flag, le bind vient de `PORT` + `LISTEN_ADDRESS = 0.0.0.0`.
- **Migrations** : pas de sous-commande ; auto-migration idempotente au boot
  (`sqlx::migrate!` + `schema_version`), documentée §3.6.
- **Healthcheck `degraded`** : `/health` répond 200 même dégradé ; le healthcheck Docker
  ne détecte donc pas `db=down` (les sondes métier doivent lire le JSON).
- **Budget taille d'image** : 575,2 MiB mesurés pour une cible < 250 Mo ; le surcoût est
  celui de `ffmpeg` Debian et de ses dépendances (~468 Mo). **Accepté pour la 3.0.0 par
  le Lead** (déviation documentée, voir §11) ; follow-up 3.1 : ffmpeg statique minimal ou
  worker média séparé.
- Le legacy `server/Dockerfile` et `server/docker-compose.yml` n'ont pas été modifiés
  (périmètre) ; la suppression/dépréciation se fera à la bascule.

## 10. Compose de production (v3)

`deploy/docker-compose.yml` décrit le service `lc-server` en Compose Specification
(Docker Compose v2.40+ ; pas de clé `version:` obsolète). Il ne remplace pas le legacy
`server/docker-compose.yml` (référence de rollback 2.0.1).

### 10.1 Contrat du service

| Élément | Valeur |
| --- | --- |
| Image | `${LIVECHAT_IMAGE:-ghcr.io/quentingimenez/livechat-server:3.0.0}` |
| `container_name` / projet | `livechat-server` / projet `livechat` |
| Redémarrage | `restart: unless-stopped` |
| Environnement | `env_file: .env` + surcharges explicites (`PORT=3000`, `DATABASE_URL=sqlite:///app/data/livechat.sqlite`, `MEDIA_STORAGE_DIR=/app/data/media`, `ADMIN_UI_LOCAL_ONLY=true`, `RUST_LOG=info`) |
| `API_URL` | **obligatoire** : `${API_URL:?API_URL requis …}` ⇒ `config`/`up` échouent sans elle (pas de défaut silencieux en production) |
| Ports | `127.0.0.1:3000:3000` (AUD-COR-10) : exposition sur l'hôte uniquement, jamais le LAN par défaut ; le reverse proxy hôte assure TLS et l'exposition publique — §10.4 |
| Données | bind mount `./data:/app/data` (SQLite `livechat.sqlite` + `-wal`/`-shm` + `media/`) |
| Healthcheck | `curl /health` **+ vérification du JSON `db:"ok"`** (un 200 seul ne suffit pas, `/health` répond 200 même dégradé) — interval 15 s, timeout 3 s, start_period 15 s, 3 essais |
| Arrêt | `stop_grace_period: 20s` (drain serveur 10 s + marge, exit 0 attendu) |
| Durcissement | `security_opt: no-new-privileges:true` ; pas de `read_only` (SQLite et l'ingestion média écrivent dans `/app/data`) |
| Logs | `json-file`, rotation `max-size: 10m`, `max-file: 3` |
| Réplicabilité | **une seule instance** (SQLite mono-écrivain) ; limites mémoire commentées (POC ≈ 6 MiB au repos) |

Fichier `deploy/.env` (non versionné, permissions `600`) — modèle :

```dotenv
# deploy/.env — production, ne jamais committer
LIVECHAT_IMAGE=ghcr.io/quentingimenez/livechat-server:3.0.0
API_URL=https://livechat.example.com
ADMIN_UI_TOKEN=<secret admin>
INGEST_API_TOKEN=<secret ingest>
MEDIA_URL_SECRET=<secret URLs médias>
# optionnels (défauts du compose) :
# DATABASE_URL=sqlite:///app/data/livechat.sqlite
# MEDIA_STORAGE_DIR=/app/data/media
# RUST_LOG=info
# paire complète ou aucune :
# DISCORD_TOKEN=<secret>
# DISCORD_CLIENT_ID=<id>
```

`ADMIN_UI_TOKEN`, `INGEST_API_TOKEN` et `MEDIA_URL_SECRET` doivent être fournis par le
secret manager de l'hôte ; `deploy/.env` n'est qu'un point d'injection. **Attention** :
`docker compose config` affiche les valeurs interpolées — ne pas rediriger sa sortie vers
des logs partagés. Follow-up hors périmètre : ajouter `deploy/.env` et `deploy/data/` au
`.gitignore` racine.

### 10.2 Procédure d'exploitation

Depuis la racine du dépôt :

```powershell
# préparation (Linux : mkdir -p data && chown 10001:10001 data ; Docker Desktop : automatique)
docker compose -f deploy/docker-compose.yml config -q      # validation + interpolation
docker compose -f deploy/docker-compose.yml pull           # image GHCR (ou build local, §4)
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml ps             # attendre "healthy"
docker compose -f deploy/docker-compose.yml logs -f --tail=100 lc-server
```

Vérification : `/health` doit renvoyer `status:"ok"`, `db:"ok"`, `version:"3.0.0"`,
`protocolVersion:2` ; `docker exec livechat-server id` doit afficher `uid=10001`.
Arrêt/upgrade : `down` (les données de `deploy/data` sont conservées ; le compose n'a
aucun volume nommé), upgrade = `pull` + `up -d` (recréation du conteneur, données
conservées), rollback = §7.2. Sauvegarde : arrêt propre puis copie de `deploy/data`
(ou `sqlite3 deploy/data/livechat.sqlite ".backup 'sauvegarde.sqlite'"`).

Smoke local sans GHCR : `LIVECHAT_IMAGE=livechat-server:3.0.0 docker compose -f deploy/docker-compose.yml up -d`
(l'image locale validée au §5).

### 10.3 Vérification locale du 2026-11-10

- Le fichier est un YAML valide (parse `yaml.safe_load` : projet `livechat`, service
  `lc-server`, healthcheck conformé) et `${API_URL:?...}` est bien contraignant : sans
  `API_URL`, `docker compose config` échoue avec
  `required variable API_URL is missing a value: API_URL requis (origine publique, …)`.
- La commande de healthcheck du compose a été validée contre l'image réelle
  (`docker exec` + script via stdin, pour éviter les artefacts de quoting PowerShell) :
  elle sort en **exit 0** sur le `/health` réel et en **exit 1** sur un JSON
  `{"status":"degraded","db":"down"}` — elle détecte donc bien la dégradation SQLite,
  contrairement au seul code HTTP 200.
- La validation complète (`config` rendu, `up -d`, smoke `/health`, `down`) **n'a pas pu
  être exécutée dans cette session** : l'environnement de l'agent interdit l'accès au
  fichier de secrets `deploy/.env` (safety net), et Compose refuse de rendre la config
  tant que ce fichier n'existe pas (`env file … not found`). Ce n'est pas un défaut du
  compose : les commandes de §10.2 sont à exécuter par l'opérateur (ou en CI) après
  création de `deploy/.env`. L'image `livechat-server:3.0.0` est présente sur le poste et
  déjà validée (smoke §5), l'override `LIVECHAT_IMAGE` permet ce smoke sans GHCR.

### 10.4 Exposition réseau, TLS et accès au panneau admin (AUD-COR-10)

Le compose par défaut n'expose le serveur que sur le loopback de l'hôte
(`ports: "127.0.0.1:3000:3000"`) et le serveur garde
`ADMIN_UI_LOCAL_ONLY=true` : le panneau `/admin/` n'est donc **pas joignable
depuis le LAN**, et le reverse proxy est le seul point d'entrée public.

- **Exposition publique avec TLS (recommandé)** : conserver le bind
  `127.0.0.1`, laisser un reverse proxy sur l'hôte (Caddy, nginx, Traefik)
  écouter sur l'IP publique et relayer vers `http://127.0.0.1:3000`. Le serveur
  reste en HTTP (limite assumée, §8) et `API_URL` doit être l'URL **publique
  https**. Exemple Caddy : `livechat.example.com { reverse_proxy 127.0.0.1:3000 }`
  (certificat automatique).
- **Accès au panneau admin via le proxy** : réserver `/admin*` au réseau
  d'administration (règle réseau, basic auth ou mTLS du proxy), puis passer
  `ADMIN_UI_LOCAL_ONLY=false` **avec** `ADMIN_UI_TOKEN` long et aléatoire. Le
  panneau n'accepte que `Authorization: Bearer` (AUD-COR-05) et est soumis au
  rate limit global (AUD-COR-10).
- **Rate limit** : 100 requêtes par fenêtre de 15 min et par IP pour toutes les
  routes, sauf les téléchargements de médias (`GET`/`HEAD` sur
  `/overlay/media/{id}` et `/overlay/meme-board/media/{id}`), comptés à part
  avec un budget de 1 000. Les vignettes de la mème board et les requêtes
  `Range` des vidéos épuisaient sinon le budget commun. Plusieurs overlays
  derrière la même IP (NAT) partagent ces deux budgets ; derrière un reverse
  proxy, activer `TRUST_PROXY_HEADERS=true` pour compter par client réel.
- **Accès admin depuis l'hôte, sans proxy** : dans un conteneur bridge, le
  serveur voit l'IP de la passerelle Docker et non `127.0.0.1` ; avec
  `ADMIN_UI_LOCAL_ONLY=true`, l'API admin répond `403 admin_local_only`. Deux
  options : `ADMIN_UI_LOCAL_ONLY=false` (le bind `127.0.0.1` et le token
  protègent toujours l'accès) ou `network_mode: host`.
- **LAN direct (déconseillé)** : remplacer `127.0.0.1:3000:3000` par
  `3000:3000` ou l'IP LAN, avec TLS au proxy et pare-feu ; ne jamais exposer
  l'API admin hors d'un réseau d'administration.

## 11. Décisions Lead (Phase 9b, 2026-11-10)

| Sujet | Décision | Conséquence |
| --- | --- | --- |
| Budget image | **575,2 MiB accepté pour la 3.0.0** (déviation documentée) ; follow-up 3.1 = ffmpeg statique minimal ou worker média séparé ; pas d'optimisation lourde maintenant | §5, §8 et §9 marquent la déviation comme acceptée ; piste 3.1 conservée |
| `API_URL` requise | acceptée ; l'image garde un défaut `http://localhost:3000` pour le dev, mais le **compose l'exige** (`${API_URL:?...}`) | §3.3, §10.1, §10.3 |
| Migrations sqlx au boot | acceptées et documentées ; **ne jamais pointer la base legacy** ; sauvegarde avant upgrade | §3.6, §8, §9 |
| `/health` 200 même `db=down` | accepté ; les sondes métier lisent le champ JSON `db` (pas le code 200) ; le HEALTHCHECK du compose vérifie `db:"ok"` | §3.5, §8, §10.1 |
| Bind `0.0.0.0` + `PORT` | accepté (le binaire n'a aucun flag CLI) | §3.2, §9 |
| Durcissement admin/compose (AUD-COR-10) | bind `127.0.0.1` + `ADMIN_UI_LOCAL_ONLY=true` par défaut, `/admin*` soumis au rate limit, audit minimal des mutations | §8, §10.1, §10.4 |

Suites hors Phase 9b : follow-up 3.1 taille d'image ; ajout de `deploy/.env` et
`deploy/data/` au `.gitignore` racine ; exécution du smoke compose par l'opérateur/CI.

## 12. Références

- `docs/rust/server-core.md` — contrat serveur Phase 3a (env, `/health`, arrêt gracieux).
- `docs/rust/lc-db.md` — schéma et contrat de persistance SQLite.
- `docs/rust/migrator.md` — migration one-shot des données legacy.
- `docs/rust/migration-plan.md` §5 (rollback), §9 (CI/CD et bascule), S7 (audit).
- `deploy/server.Dockerfile`, `deploy/docker-compose.yml`,
  `.github/workflows/server-image.yml`.
