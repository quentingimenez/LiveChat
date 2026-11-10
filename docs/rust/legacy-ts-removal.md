# Suppression du legacy TypeScript — inventaire, mapping et checklist (Phase 9c)

> **Statut :** stratégie prête, **aucune suppression effectuée** (task-25, 2026-11-10).
> **Périmètre de ce document :** préparation de la suppression réversible de `server/` et
> `overlay/` après la bascule 3.0.0, avec inventaire, mapping vers Rust, références
> d'outillage/CI/docs et critères go/no-go à 7 jours.
> **Sources :** `docs/rust/migration-plan.md` (§1, §9, annexe D), `docs/rust/parity-harness.md`,
> `docs/rust/lc-overlay-8b.md`, `docs/rust/packaging.md` (§6, §7), `docs/rust/server-*.md`,
> `docs/rust/lc-*.md`, et mesures directes du dépôt (git, comptages de lignes).
> **Contraintes de la phase :** aucun commit, aucune suppression, aucun autre fichier modifié.

## 1. Objet et principes

La cible Rust (`crates/*`, `apps/lc-server`, `apps/lc-overlay`, `static/admin`, `xtask`) est
fonctionnelle et documentée. Le legacy TypeScript/Electron reste dans l'arbre pour deux
raisons : il est la **référence de rollback** tant que la fenêtre de 7 jours n'est pas
close, et il est encore **nécessaire au build de l'overlay** via les shims
`apps/lc-overlay/web/dist/renderer/**` (voir §4).

Principes retenus pour la suppression :

1. **Réversibilité totale par tag** : un tag `legacy-ts-2.0.2` archive le dernier état
   contenant le legacy ; restaurer se fait par `git checkout <tag> -- server overlay`, sans
   dépendre d'un build local antérieur.
2. **Suppression après la fenêtre de survie**, avant le nettoyage final : on ne supprime
   qu'une fois la bascule 3.0.0 validée et la fenêtre de 7 jours écoulée (migration-plan §9).
3. **Capturer la parité avant de supprimer** : les goldens `xtask parity` doivent être
   capturés depuis le serveur legacy **avant** le tag de suppression ; ensuite, seule
   l'image 2.0.1 de secours ou le tag permettent encore de recapturer.
4. **Aucune perte de contrat** : chemins REST, payloads, statuts HTTP et format des
   messages WS v2 ne changent pas du fait de la suppression (ils sont déjà figés en Rust).
5. **Historique git préservé par tag**, pas par sous-dossier vivant : `git log -- server/`
   reste consultable sur le tag, mais disparaît du `HEAD` après suppression (assumé).

## 2. Inventaire du legacy restant (mesuré le 2026-11-10)

### 2.1 `server/` — serveur Node/TypeScript

| Mesure | Valeur |
|---|---|
| Fichiers (hors `node_modules/`, `dist/`) | 92 fichiers, ≈ 12 971 lignes |
| `server/src/` | 57 fichiers, 12 656 lignes |
| `server/prisma/` | 14 fichiers, 324 lignes (schéma + 11 migrations + lock) |
| `server/packages/overlay-protocol/` | 3 fichiers, 113 lignes |
| Racine du paquet | `package.json`, `pnpm-lock.yaml`, `tsconfig.json`, `vitest.config.ts`, `Dockerfile`, `docker-compose.yml`, `.env.example`, `SECURITY_EXCEPTIONS.md`, configs eslint/prettier/node/npm, `.vscode/` |

Sous-ensembles fonctionnels :

- `server/src/index.ts` (33 l.), `server/src/server.ts` (202 l.) : bootstrap Fastify.
- `server/src/loaders/` : `RESTLoader.ts` (23 l.), `socketLoader.ts` (320 l.),
  `DiscordLoader.ts` (142 l.), `discordUserSyncLoader.ts` (13 l.).
- `server/src/components/overlay/` : `overlayRoutes.ts` (480 l.).
- `server/src/components/ingest/` : `ingestRoutes.ts` (473 l.).
- `server/src/components/admin/` : `adminRoutes.ts` (2 646 l. — monolithe S9 de l'audit).
- `server/src/components/discord/` : 9 commandes slash (16 à 119 l. chacune).
- `server/src/components/messages/` : `messagesWorker.ts` (356 l.), `sendCommand.ts`
  (260 l.), `hidesendCommand.ts` (258 l.), `talkCommand.ts`, `stopCommand.ts`, etc.
- `server/src/services/` : `playbackScheduler.ts` (979 l.), `media/mediaIngestion.ts`
  (1 740 l.), `media/mediaTranscode.ts` (408 l.), `media/mediaErrors.ts` (248 l.),
  `media/mediaSourceResolver.ts` (230 l.), `media/mediaCache.ts` (221 l.),
  `social/twitterVideoResolver.ts` (641 l.), `social/twitterOEmbed.ts` (125 l.),
  `memeBoard.ts` (327 l.), `overlayAuth.ts` (116 l.), `ingestAuth.ts` (134 l.),
  `playbackJobs.ts` (167 l.), `runtimeSettings.ts`, `pairingCodes.ts`, `env.ts`,
  `i18n/{en,fr,loader}.ts`, `gtts.ts`, `manualStop.ts`, `discord-utils(.test).ts`,
  `discordUserSync(.test).ts`, `stringUtils(.test).ts`, `utils.ts`.
- `server/src/services/prisma/`, `server/src/types/module.d.ts`, `logger/prettyTransport.cjs`.
- 5 fichiers de tests `*.test.ts` (audit S8 : 5 fichiers pour 56 sources).

### 2.2 `overlay/` — app Electron/TypeScript

| Mesure | Valeur |
|---|---|
| Fichiers (hors `node_modules/`, `dist/`) | 197 fichiers, ≈ 23 542 lignes |
| `overlay/src/` | 140 fichiers, 13 853 lignes (`main/` 49, `renderer/` 80, `shared/` 7, `preload/` 2, `types/` 2) |
| `overlay/renderer/` (HTML/CSS/JS runtime) | 16 fichiers, 1 652 lignes |
| `overlay/tests/` | 16 fichiers, 1 764 lignes |
| `overlay/scripts/` | 4 fichiers (`assert-renderer-esm.mjs`, `assert-win-artifacts.mjs`, `check-max-lines.sh`, `verify.sh`) |
| Racine | `main.js`, `main.runtime.js`, `preload.js`, `protocol.js`, `package.json` (v2.0.1), `package-lock.json`, 4 tsconfigs, configs eslint/prettier, `icon.ico`/`icon.png`/`icon-tray.png` (≈ 2,9 Mo), `README.md`, `SECURITY_EXCEPTIONS.md` |

Statut de conservation :

- `overlay/src/main`, `preload`, `shared`, `types` : **remplacés** par `apps/lc-overlay/src/**`
  et `apps/lc-overlay/web/bridge.js` ; il n'existe plus de code Electron à conserver.
- `overlay/renderer/**` : **déjà copié à l'identique** dans `apps/lc-overlay/web/`
  (3 pages, CSS, JS, `styles/**`) par la Phase 8b — la source `overlay/renderer/` devient
  redondante. Note : `overlay/renderer/pairing.js` est un artefact mort non copié
  (décision §8.7 de `docs/rust/overlay-bridge.md`).
- `overlay/src/renderer/**` (80 fichiers TS) : **partiellement conservé sous forme
  générée** — voir le risque §4.2 (`apps/lc-overlay/web/dist/renderer/**`).
- `overlay/tests/**`, `overlay/scripts/**`, tsconfigs et configs lint : remplacés par
  `rust.yml` + `cargo test -p lc-overlay` (51 tests) ; à supprimer avec le reste.

### 2.3 `server/packages/overlay-protocol/`

Package local `@livechat/overlay-protocol` (3 fichiers, 113 lignes) : constantes et types
du protocole v1 (version de protocole, événements socket, payloads play/stop). Il est
importé par 8 fichiers legacy (admin, messagesWorker, overlayRoutes, socketLoader,
mediaTranscode, manualStop, overlayAuth, types). **Remplacement :** `crates/lc-protocol`
(protocole v2 gelé : enveloppe, types, fixtures golden). Aucun consommateur hors
`server/` (l'`extension/` ne l'importe pas) ⇒ supprimable avec `server/`.

### 2.4 Ce qui n'est pas supprimé

- `extension/` : inchangée (27 fichiers, 8 283 lignes) — périmètre hors bascule.
- `static/admin/` : nouveau panneau servi par `apps/lc-server/src/http/admin/static_files.rs`.
- `docs/rust/` : documentation de référence et archives historiques (voir §6.3).
- `xtask/`, `deploy/`, `crates/`, `apps/`, `Cargo.*`, `rust-toolchain.toml`,
  `.github/workflows/rust.yml`, `server-image.yml`, `extension.yml`.

## 3. Mapping legacy → Rust

Les chemins Rust cités sont relatifs à la racine du dépôt. Les documents `docs/rust/*.md`
correspondants sont la référence détaillée de chaque lane.

### 3.1 HTTP / REST

| Legacy | Rust | Écarts notables |
|---|---|---|
| `server/src/server.ts` (bootstrap Fastify, port, health) | `apps/lc-server/src/main.rs`, `src/lib.rs`, `src/app.rs`, `src/http/{root,health}.rs` | Axum 0.8, `/health` JSON compatible ; docs `server-core.md` |
| `server/src/loaders/RESTLoader.ts` | `apps/lc-server/src/http/mod.rs` (merge des sous-routeurs) | Chaîne de middlewares identique (CORS, sécurité, rate-limit, normalisation d'erreurs) |
| `server/src/components/overlay/overlayRoutes.ts` | `apps/lc-server/src/http/overlay/{mod,auth,pair,media,meme_board}.rs` | Bearer only (`?token=` supprimé), `GET /overlay/config` supprimé, `protocolVersion: 2` additif ; docs `server-overlay.md` |
| `server/src/components/ingest/ingestRoutes.ts` | `apps/lc-server/src/http/ingest/{mod,auth,body,pair,submit}.rs` | Ingest client DB-backed uniquement ; docs `server-ingest.md` |
| `server/src/components/admin/adminRoutes.ts` | `apps/lc-server/src/http/admin/**` + `static/admin/{index.html,app.js,styles.css}` | 13 endpoints ; docs `server-admin.md`, `admin-panel.md` (le squelette `crates/lc-admin` a été supprimé le 2026-11-10) |
| `server/src/loaders/socketLoader.ts` (`/socket.io/` → 426) | `apps/lc-server/src/http/socket_io.rs` | Réponse 426 `overlay_outdated` conservée |
| `server/src/services/{ingestAuth,overlayAuth,pairingCodes}.ts` | `apps/lc-server/src/http/{ingest,overlay}/auth.rs`, `overlay/pair.rs` | SHA-256 des tokens, `overlay_clients.token_hash`, apps/lc-server `media/tokens.rs` |

### 3.2 WebSocket

| Legacy | Rust | Écarts |
|---|---|---|
| `server/src/loaders/socketLoader.ts` (auth, heartbeat, dispatch, peers) | `apps/lc-server/src/ws/{mod,hub,session}.rs` | Protocole v2 (`GET /ws`, enveloppe JSON, 64 Kio, 20 msg/s) ; docs `server-ws.md`, `protocol-v2.md` |
| `server/src/services/overlayPeers`-like (dans socketLoader) | `apps/lc-server/src/ws/hub.rs` + `crates/lc-protocol` `PeersPayload` | Labels de peers conservés côté Rust |

### 3.3 Ordonnanceur de playback

| Legacy | Rust | Écarts |
|---|---|---|
| `server/src/services/playbackScheduler.ts` | `crates/lc-playback` (`engine.rs`, `store.rs`, `payload.rs`, `sink.rs`, `config.rs`) + `apps/lc-server/src/ws/hub.rs` (intégration) | Ordonnanceur extrait en crate, horloge injectée ; docs `lc-playback.md` |
| `server/src/services/playbackJobs.ts` | `crates/lc-db/src/playback_jobs.rs` + `apps/lc-server/src/media/jobs.rs` | Dates ms epoch ; migrations lc-db |
| `server/src/services/manualStop.ts`, `services/overlayAuth` (stop) | `apps/lc-server/src/ws/hub.rs` (stop manuel) + `crates/lc-playback` | `overlay:stop` conservé |

### 3.4 Média, social, cache

| Legacy | Rust | Écarts |
|---|---|---|
| `server/src/services/media/mediaIngestion.ts` | `crates/lc-media/{ingest,download,probe,transcode,storage,cache,source,config,error}.rs` | Pipeline sans cache de ré-ingestion (Phase 5b) ; docs `lc-media.md` |
| `server/src/services/media/mediaTranscode.ts` | `crates/lc-media/src/transcode.rs`, `process.rs` | ffmpeg/ffprobe externes, mêmes contraintes de taille |
| `server/src/services/media/mediaSourceResolver.ts` | `crates/lc-media/src/{source,resolve,html}.rs` | Résolveur conservé |
| `server/src/services/media/mediaErrors.ts` | `crates/lc-media/src/error.rs` (`MediaErrorCode` 11 codes, `legacy_http_status()=422`) | Parité de codes/labels |
| `server/src/services/media/mediaCache.ts` | `crates/lc-media/src/cache.rs` | Éviction/purge Phase 5b |
| `server/src/services/social/twitterVideoResolver.ts`, `twitterOEmbed.ts` | `crates/lc-media/src/twitter.rs` (+ `tiktok.rs`, `youtube.rs`) | Résolveurs par plateforme |
| `server/src/services/gtts.ts` | `crates/lc-discord/src/context.rs` (« Synthétise un texte en MP3 et l'ingère ») | TTS intégré à la glue Discord |
| `server/src/services/memeBoard.ts` | `apps/lc-server/src/http/overlay/meme_board.rs`, `apps/lc-server/src/media/board.rs`, `crates/lc-db/src/meme_board.rs` | Ingestion inline + quota ; docs `server-overlay.md` §7 |

### 3.5 Discord (bot)

| Legacy | Rust | Écarts |
|---|---|---|
| `server/src/loaders/DiscordLoader.ts`, `discordUserSyncLoader.ts` | `apps/lc-server/src/discord/{mod,runtime,registration,event_handler}.rs`, `crates/lc-discord/src/serenity_glue.rs` | Serenity + poise ; docs `server-discord.md`, `lc-discord.md` |
| `server/src/components/discord/*.ts` (9 commandes : alive, client, help, info, memeAdd, overlays, setDefaultTime, setDisplayFull, setMaxTime) | `crates/lc-discord/src/{catalog,handlers}.rs` | 13 slash commands + 5 message commands |
| `server/src/components/messages/*.ts` (send, hidesend, talk, hidetalk, stop, worker) | `apps/lc-server/src/discord/message_commands.rs`, `crates/lc-discord/src/handlers.rs`, `rich_payload.rs` | Même format de texte overlay riche |
| `server/src/services/i18n/{en,fr,loader}.ts` | `crates/lc-discord/src/i18n.rs` | Mêmes clés fr/en, placeholders `{{…}}` |
| `server/src/services/{discord-utils,discordUserSync,stringUtils,utils}.ts` | `crates/lc-discord/src/{text,inputs,serenity_glue}.rs` | — |

### 3.6 Configuration, données, protocole

| Legacy | Rust | Écarts |
|---|---|---|
| `server/src/services/env.ts`, `.env.example` | `apps/lc-server/src/config.rs` (`Config::from_pairs`, variables documentées) + `crates/lc-media/src/config.rs`, `crates/lc-discord` | Validation typée ; `.env.example` à convertir (§4.3) |
| `server/prisma/schema.prisma` + 11 `migrations/**` | `crates/lc-db/migrations/0001_init.sql` (schéma 3.0.0) | Dates INTEGER ms, enums TEXT majuscules ; docs `lc-db.md`, `migrator.md` |
| `server/src/services/prisma/{loadPrisma,prismaEnums}.ts` | `crates/lc-db/src/{db,convert}.rs` | Mapping enums/ids |
| `server/src/services/runtimeSettings.ts` | `apps/lc-server/src/http/admin/runtime_settings.rs`, `apps/lc-server/src/media/settings.rs` | Réglages runtime persistés |
| `server/packages/overlay-protocol/src/index.ts` | `crates/lc-protocol/src/lib.rs` | Protocole v2 gelé, fixtures golden (§16 du doc protocole) |
| `server/src/types/module.d.ts` | — (types Rust) | Supprimé sans remplacement |
| `server/src/index.ts` (`pnpm start` cassé sur `dist/src/index.js`) | `apps/lc-server/src/main.rs` | Démarrage mesuré 0,53 s release (migration-plan §12) |

### 3.7 Overlay desktop (Electron → Tauri 2)

| Legacy | Rust / web | Écarts |
|---|---|---|
| `overlay/src/main/app/runtime-legacy-entry.ts` (orchestration, ordre de démarrage, garde TLS/autoplay) | `apps/lc-overlay/src/main.rs`, `src/lib.rs`, `src/state.rs` | Démarrage Tauri ; docs `lc-overlay-shell.md` |
| `overlay/src/main/app/runtime-paths.ts` (userData, `runtime-assets/`, icônes) | `apps/lc-overlay/src/config.rs`, `src/windows.rs` | Config forcée `%APPDATA%\LiveChat\config.json` + `.bak` (`finalize_store`) |
| `overlay/src/main/services/config-service.ts` (16 champs, normalisations) | `apps/lc-overlay/src/config.rs` | Projection 8 champs vers le renderer |
| `overlay/src/preload/index.ts` (`contextBridge`, 13 méthodes) | `apps/lc-overlay/web/bridge.js` + `apps/lc-overlay/src/commands.rs` (12 commandes Tauri) | Même API `window.livechatOverlay`, transport invoke/listen ; docs `overlay-bridge.md` |
| `overlay/src/shared/ipc.ts` (canaux IPC) | `apps/lc-overlay/src/events.rs` + `web/bridge.js` (table `COMMANDS`/`EVENTS`) | Canaux → commandes snake_case, événements `overlay://play\|stop\|settings` |
| `overlay/src/main/services/socket-service.ts`, `socket-listeners-service.ts` | `apps/lc-overlay/src/net/ws.rs` | WS v2, handshake `hello`, reconnexion, buffer 5 s |
| `overlay/src/main/services/pairing-consume-service.ts` | `apps/lc-overlay/src/net/pairing.rs` | `POST /overlay/pair/consume`, mode `invite_read_only` |
| `overlay/src/main/windows/{window-factory,overlay-window-service,auxiliary-window-service}.ts` | `apps/lc-overlay/src/windows.rs`, `src/win32.rs` | 3 fenêtres, click-through, topmost 2 s |
| `overlay/src/main/services/{display-service,meme-bindings-service,shortcut-service,auto-start-service,auto-update-service}.ts` | `apps/lc-overlay/src/{display,shortcuts}.rs`, `src/tray.rs`, `src/updater.rs` | `displayId` Electron non résolu (displayKey/Index/primaire) ; updater à clé réelle |
| `overlay/src/main/services/tray-*.ts`, `overlay-peers-service.ts` | `apps/lc-overlay/src/tray.rs` | Tooltip/peers conservés |
| `overlay/src/main/ipc/*`, `runtime-*.ts`, `infra/*` | `apps/lc-overlay/src/{commands,state,events}.rs` | Répartition équivalente, différée si besoin |
| `overlay/renderer/**` (16 fichiers) | `apps/lc-overlay/web/**` (copie Phase 8b) | Patch média v2 : URL verbatim ; board conserve `?token=` REST |
| `overlay/src/renderer/**` (80 fichiers TS) | `apps/lc-overlay/web/dist/renderer/**` (shims générés) | **Dépendance de build à sécuriser (§4.2)** |
| `overlay/tests/**`, `overlay/scripts/**` | `apps/lc-overlay` tests Rust (51 tests), `rust.yml`, `apps/lc-overlay/src/*` | `assert-renderer-esm.mjs` obsolète après suppression des TS renderer |
| `overlay/{main.js,main.runtime.js,preload.js,protocol.js}` (shims CJS) | — | Supprimés avec `overlay/` |
| `overlay/package.json` (v2.0.1) + workflows Electron | Tauri 2 (`tauri.conf.json`, `frontendDist:"web"`) + `overlay-release.yml` cible | Transition Electron 2.0.2 obligatoire avant cutover (migration-plan §1) |

## 4. Dépendances à sécuriser AVANT toute suppression

### 4.1 Goldens de parité (`xtask parity`)

`docs/rust/parity/golden-v1/README.md` indique explicitement qu'**aucun golden n'est encore
capturé** : la capture exige un serveur legacy démarré avec une base représentative
(guilde, média `READY` rattaché à un job, clients overlay/ingest), puis relecture Lead et
commit. Sans goldens, `parity compare` ne peut plus constituer une preuve de non-régression
après suppression.

**Action bloquante :** capturer `docs/rust/parity/golden-v1/**` (34 cas, `cases/v1.json`)
depuis le legacy **avant** le tag `legacy-ts-2.0.2`, puis committer le dossier après revue
(procédure : `docs/rust/parity-harness.md`).

### 4.2 Shims renderer de l'overlay (`apps/lc-overlay/web/dist/renderer/**`)

Mesures :

- `git ls-files apps/lc-overlay/web/dist` → **0 fichier** (non versionné).
- `apps/lc-overlay/.gitignore` n'ignore que `/gen/schemas/` : le dossier est simplement
  absent du suivi git.
- `apps/lc-overlay/build.rs` = `tauri_build::build()` ; `tauri.conf.json` a
  `"frontendDist": "web"` : **aucune génération** de `web/dist/renderer/**` par le build.
- Les 3 pages chargent `../dist/renderer/<page>/index.js` (80 fichiers générés depuis
  `overlay/src/renderer/**` via `tsc`), et la Phase 8b les a copiés depuis
  `overlay/dist/renderer/**`.

**Conséquence :** supprimer `overlay/` aujourd'hui casserait un build propre de
`apps/lc-overlay` (CI `overlay-release.yml`, poste neuf) car les shims sont introuvables.

**Actions (au choix, à trancher dans la PR de suppression) :**
1. committer `apps/lc-overlay/web/dist/renderer/**` (80 fichiers) en l'état — solution
   simple, fige la dette (les TS sources disparaissent) ;
2. ou déplacer/porter les sources TS minimales (`legacy-loader`, utils renderer) dans
   `apps/lc-overlay/web/src/` avec un petit script de build (`tsc`) exécuté avant
   `tauri build` — solution propre, plus de travail ;
3. ou vérifier que les 80 shims sont réellement nécessaires (certains peuvent être
   remplacés par du JS direct) et réduire la copie.

Tant que ce point n'est pas réglé, **la suppression de `overlay/` est bloquée**
(go/no-go §7).

### 4.3 `.env.example` et configuration

`server/.env.example` (48 variables : `PORT`, `API_URL`, `DISCORD_TOKEN`, `DATABASE_URL`,
`MEDIA_*`, `YTDLP_*`, `FFMPEG_*`, `TWITTER_*`, `PAIRING_CODE_TTL_MINUTES`, etc.) est le
seul inventaire des variables encore attendues par le serveur Rust : `Config::from_pairs`
lit les mêmes noms (`API_URL` obligatoire, `MEDIA_STORAGE_DIR`,
`MEDIA_BOARD_MAX_TOTAL_MB`, `MEDIA_URL_SECRET`, `PAIRING_CODE_TTL_MINUTES`, …).

**Action :** avant suppression, créer `deploy/.env.example` (ou documenter dans
`docs/rust/packaging.md` §3.4) avec les variables Rust uniquement (retirer `PORT`,
`ADMIN_UI_*`, `INGEST_API_TOKEN` déprécié, `DISCORD_CLIENT_ID` si inutile en v3),
puis supprimer `server/.env.example`. `deploy/server.Dockerfile` et
`docs/rust/packaging.md` §10 documentent déjà le compose de production.

### 4.4 Références historiques à conserver

- Les images GHCR (`ghcr.io/.../livechat-server:2.0.1`) et les releases GitHub ne
  disparaissent pas avec les sources : le rollback peut s'appuyer dessus.
- Les docs `docs/rust/overlay-bridge.md`, `server-*.md`, `lc-*.md` contiennent des
  références au legacy (tableau §6.3) : elles deviennent **historiques** ; ne pas les
  réécrire en masse, ajouter une note « legacy supprimé au tag `legacy-ts-2.0.2` ».

## 5. Checklist de suppression (ordonnée, réversible)

### 5.1 Pré-requis (avant toute action)

- [ ] Serveur 3.0.0 et overlay 3.0.0 publiés, bascule faite, fenêtre de 7 jours close
      (voir §7).
- [ ] Release de transition Electron **2.0.2** publiée (bandeau « mise à jour manuelle
      requise ») et `overlay/package.json` à `2.0.2` au moment du tag — cf. migration-plan §1.
- [ ] Goldens de parité capturés et commités (§4.1) ; `parity compare` vert sur l'image 2.0.1
      ou la cible.
- [ ] Shims `web/dist/renderer/**` sécurisés (§4.2).
- [ ] `.env.example` converti (§4.3).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features
      --locked -- -D warnings`, `cargo nextest run --workspace --all-features --locked`,
      `cargo test --workspace --doc`, `cargo deny check`/`cargo audit` verts sur `HEAD`.
- [ ] Build Tauri de l'overlay vert depuis un clone propre (preuve que les shims sont là).
- [ ] `cargo xtask parity compare` documenté (base de référence) et rapport archivé.

### 5.2 Tag d'archivage

```powershell
# à faire sur le dernier commit contenant server/ + overlay/ (après capture golden)
git fetch --tags
git tag -a legacy-ts-2.0.2 -m "Archive legacy TypeScript/Electron avant suppression (Phase 9c)"
git push origin legacy-ts-2.0.2
```

Notes :
- Le nom reprend la version legacy de référence (`2.0.2`), aligné sur la release de
  transition de l'overlay ; il ne doit pas exister deux tags pour deux contenus différents.
- Vérifier les tags **locaux** (état mesuré : `overlay-v*` jusqu'à `overlay-v2.0.1`,
  aucun `server-v*`, aucun `legacy-*`) et, si `overlay-v2.0.2` est publié côté distant,
  taguer le commit exact du contenu archivé.

### 5.3 Branche et PR de suppression

```powershell
git switch -c chore/remove-legacy-ts
```

Ordre des commits (chacun revertable indépendamment) :

1. **build/web** : sécuriser `apps/lc-overlay/web/dist/renderer/**` (commit ou build) ;
   mettre à jour `apps/lc-overlay/web/README.md` et `docs/rust/lc-overlay-8b.md` (chemin
   des shims).
2. **config** : ajouter `deploy/.env.example` converti, mettre à jour
   `docs/rust/packaging.md` §3.4 si nécessaire.
3. **CI/tooling/docs** (avant de supprimer les dossiers, pour éviter une CI rouge) :
   supprimer `.github/workflows/{server.yml,overlay-ci.yml,overlay-nightly.yml}`,
   réécrire `overlay-release.yml` en flux Tauri (ou supprimer si déjà couvert),
   supprimer les commandes legacy de `.husky/pre-commit` et `.husky/pre-push`,
   nettoyer `package.json` racine (`install:all`, `server:dev`, `server:test`,
   `overlay:start`, `overlay:verify`), mettre à jour `README.md` (§6.2) et
   `.gitignore` si besoin.
4. **suppression `server/`** : `git rm -r server` (le dossier `deploy/` et les docs
   nécessaires sont ailleurs ; `server/Dockerfile` et `server/docker-compose.yml`
   restent accessibles via le tag).
5. **suppression `overlay/`** : `git rm -r overlay` (après vérification §4.2).
6. **finitions** : notes historiques dans les docs concernées (§6.3), mise à jour
   `docs/rust/migration-plan.md` annexe D, entrée de changelog/`README`.
7. **directory `docs/rust/parity/`** : conserver les goldens ; retirer les mentions
   « à capturer ».

PR : titre `chore(legacy): supprimer le serveur TS et l'overlay Electron après bascule 3.0.0`,
description avec les preuves (tag, goldens, builds), labels `phase-9` ; **ne pas squash**
les 3 commits majeurs si l'on veut un revert sélectif (ou documenter le revert du merge).

### 5.4 Vérifications post-suppression (sur la branche)

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run --workspace --all-features --locked --no-fail-fast --no-tests=pass
cargo test --workspace --doc --locked
cargo deny check
cargo run -p xtask -- parity compare --base-url http://127.0.0.1:8080 `
  --golden docs/rust/parity/golden-v1 --report target/parity/rapport.json
cargo tauri build --debug --no-bundle      # dans apps/lc-overlay (ou via workspace)
git grep -n -E "server/src|overlay/src|pnpm --dir server|npm --prefix overlay" -- `
  ':!docs/rust/legacy-ts-removal.md' ':!docs/rust/migration-plan.md'
```

Attendu : build Tauri OK avec `web/dist/renderer/**` présent dans un clone neuf ;
`git grep` ne renvoie que des références historiques explicitement listées (§6.3) ;
CI `rust.yml` verte ; `server-image.yml` (image `server-v*`) et `extension.yml` inchangés.

### 5.5 Fusion et promotion

- Merger sur `main` après revue Lead ; conserver le tag `legacy-ts-2.0.2` poussé avant.
- Surveiller les runs CI post-merge (rust.yml, server-image.yml, extension.yml) pendant
  24 h ; vérifier que `overlay-release.yml` (Tauri) produit NSIS/MSI/zip et `latest.json`.
- Aucune release 3.0.x ne doit dépendre d'un artefact legacy.

## 6. Références legacy dans l'outillage, la CI et les docs

### 6.1 CI et outillage

| Référence | Ligne / contenu | Action |
|---|---|---|
| `.github/workflows/server.yml` | paths `server/**`, tag `server-v*`, Node 20, pnpm, image GHCR legacy | **Supprimer** dans la PR (image Rust publiée par `server-image.yml`) |
| `.github/workflows/overlay-ci.yml` | `working-directory: overlay`, Node 22, `npm run verify` | **Supprimer** |
| `.github/workflows/overlay-nightly.yml` | `working-directory: overlay`, prerelease `overlay-nightly` Electron | **Supprimer** (remplacé par le flux Tauri nightly, à ajouter si voulu) |
| `.github/workflows/overlay-release.yml` | tag `overlay-v*`, `working-directory: overlay`, electron-builder | **Réécrire** en flux Tauri (ou supprimer si non retenu) dans la PR CI |
| `.github/workflows/rust.yml` | paths `crates/**`, `apps/**`, `xtask/**` ; fmt/clippy/nextest/audit | **Conserver** (déjà zéro dépendance legacy) |
| `.github/workflows/server-image.yml` | build `deploy/server.Dockerfile`, tag `server-v*` | **Conserver** |
| `.github/workflows/extension.yml` | paths `extension/**` | **Conserver** |
| `.husky/pre-commit` | `git diff --cached --name-only -- server/` → `pnpm --dir server test` | **Remplacer** par un check Rust (fmt/clippy au choix) ou supprimer |
| `.husky/pre-push` | `pnpm --dir server audit --audit-level=critical --prod` | **Remplacer** par `cargo audit`/`cargo deny` |
| `.husky/commit-msg` | commitlint | **Conserver** |
| `package.json` racine | scripts `install:all`, `server:dev`, `server:test`, `overlay:start`, `overlay:verify` | **Nettoyer** ; garder `extension:*` et `prepare` |
| `deploy/server.Dockerfile` | image Rust 3.0.0 | **Conserver** (aucune ref legacy) |
| `server/Dockerfile`, `server/docker-compose.yml` | image legacy 2.0.1 (rollback) | **Supprimés avec `server/`** ; restent disponibles via le tag ; ne pas modifier avant |
| `.gitignore` racine | `node_modules/`, `/target`, `/spikes/`, `/.phase0/` | **Conserver** (extension utilise `node_modules/`) ; `node_modules/` ne vise plus `server/` |
| `xtask/src/parity/*` | `--base-url` legacy (documentation) | **Conserver** : le harnais est agnostique ; après suppression, `compare` se fait contre l'image 2.0.1 |
| `xtask/src/migrate/*` | migreur one-shot (`cargo xtask migrate-legacy`) | **Conserver** : outil hors runtime (`docs/rust/migrator.md`) |

### 6.2 README racine

| Lignes | Contenu | Action |
|---|---|---|
| 10 | schéma mermaid `server -->|socket.io| overlay` | Remplacer par cible Rust |
| 17-18 | table `server/` et `overlay/` (Node/Electron) | Remplacer par `apps/lc-server`/`apps/lc-overlay` |
| 25 | prérequis Node 22/pnpm 10 | Réécrire (Rust + Node seulement pour l'extension) |
| 36-37 | scripts `server:dev`/`server:test` | Supprimer/remplacer par commandes cargo |
| 49-53 | table workflows (server, overlay-ci, overlay-nightly, overlay-release) | Remplacer par `rust.yml`, `server-image.yml`, `extension.yml` (et le workflow overlay Tauri) |
| 59-61 | politique de tags (`server-v*`, `overlay-v*`) | Mettre à jour (tags Rust 3.0.0) |
| 63 | auto-update Electron (`electron-updater`) | Remplacer par updater Tauri |
| 70, 73 | `docker pull`, `server/docker-compose.yml` | Remplacer par `deploy/` (compose v3) |
| 78-79 | hooks husky (server/pnpm audit) | Synchroniser avec §6.1 |
| 83-89 | historique git importé + table provenance des 3 dépôts | Conserver en note historique (« sources archivées sous tag `legacy-ts-2.0.2` ») |

### 6.3 Docs et code Rust citant le legacy

Occurrences mesurées de `server/src`, `overlay/src`, `overlay/renderer`,
`pnpm --dir server`, `npm --prefix overlay`, `server/packages` :

| Fichier | Occurrences | Traitement |
|---|---|---|
| `docs/rust/overlay-bridge.md` | 45 | **Version publiée à archiver** (historique) ; reste la référence du bridge Phase 0 |
| `docs/rust/protocol-v2.md` | 20 | Parité v1→v2 : conserver les mentions, ajouter note « sources legacy supprimées » |
| `docs/rust/server-overlay.md` | 15 | Conserver (écarts legacy) ; les chemins legacy deviennent historiques |
| `docs/rust/server-ingest.md` | 8 | Idem |
| `docs/rust/lc-discord.md` | 8 | Idem |
| `docs/rust/server-ws.md` | 5 | Idem |
| `docs/rust/server-discord.md` | 5 | Idem |
| `docs/rust/server-admin.md` | 3 | Idem |
| `docs/rust/lc-overlay-8b.md` | 1 | Mettre à jour le chemin des shims (§4.2) |
| `docs/rust/migration-plan.md` | 3 | Annexe D : passer de « supprimé après cutover » à « supprimé au tag `legacy-ts-2.0.2` » |
| `docs/rust/{admin-panel,lc-overlay-shell,lc-playback,lc-media,migrator,packaging,server-core}.md` | 1-3 chacun | Conserver ; notes historiques |
| `docs/rust/parity-harness.md`, `docs/rust/parity/**` | capture legacy | Conserver la procédure ; marquer la capture comme faite |
| `apps/lc-overlay/src/config.rs:8` | commentaire « reproduisent `overlay/src/main/services/config-service.ts` » | Conserver comme traçabilité (chemin historique) |
| `apps/lc-overlay/web/bridge.js:623` | commentaire « Signatures : `overlay/src/types/livechat-overlay.d.ts:19-33` » | Conserver (traçabilité du contrat) |
| `apps/lc-overlay/web/README.md` (l.18, 26, 37, 121) | arborescence cible et références `overlay/` | Mettre à jour dans le commit « build/web » |
| `static/admin/*` | — | Aucune référence legacy |

Politique : **ne pas réécrire l'historique** dans les docs de lane (les chemins legacy y
sont des preuves de parité) ; seule la documentation d'exploitation (README, packaging,
migration-plan) doit refléter l'absence des dossiers.

## 7. Risques et critères go/no-go

### 7.1 Risques

| # | Risque | Impact | Probabilité | Mitigation |
|---|---|---|---|---|
| R1 | Goldens de parité non capturés avant suppression | Perte de la preuve de non-régression REST ; impossible de recapturer sans serveur legacy | **Élevée** (état actuel : `golden-v1/` vide) | Capture bloquante §4.1 avant tag |
| R2 | `web/dist/renderer/**` non versionné | Build Tauri cassé sur clone neuf / CI release overlay | **Élevée** (0 fichier suivi) | Committer les 80 shims ou créer un build script (§4.2) |
| R3 | Historique git des sous-dossiers non consultable depuis `HEAD` | Perte de traçabilité quotidienne | Certaine | Tag `legacy-ts-2.0.2` + note README ; `git log <tag> -- server/` |
| R4 | CI rouge après suppression (workflows paths `server/**`, `overlay/**`) | Blocage des merges | Moyenne | Supprimer/réécrire les workflows dans le même PR, avant la suppression (§5.3 ordre) |
| R5 | Rollback nécessaire après suppression | Restauration manuelle | Faible (fenêtre 7 j close) | Tag + image GHCR 2.0.1 + sauvegarde `sqlite.db` ; procédure §8 |
| R6 | Auto-update Electron 2.0.1 → 3.0.0 inopérant | Parc obsolète non mis à jour | Moyenne (déjà traitée) | Release de transition 2.0.2 obligatoire avant cutover (migration-plan §1) |
| R7 | Vulnérabilités legacy non corrigées après suppression | Risque sécurité résiduel si un serveur 2.0.1 reste exposé | Faible (fin de support) | Fin de support documentée ; 3.0.0 seul maintenu ; image 2.0.1 non republiée |
| R8 | Références docs/scripts mortes (`pnpm`, `npm --prefix overlay`) | Confusion contributeurs / CI | Moyenne | `git grep` de contrôle §5.4 + nettoyage §6 |
| R9 | `.env` legacy perdu | Perte de la liste des variables attendues | Faible | Conversion `deploy/.env.example` avant suppression (§4.3) |
| R10 | Extension impactée par le nettoyage racine | CI extension cassée | Faible | `extension.yml` et scripts `extension:*` inchangés ; vérifier `install:all` |

### 7.2 Critères go/no-go pour la suppression (au terme des 7 jours)

Go si **tous** les critères suivants sont vrais :

- [ ] Serveur 3.0.0 en production depuis ≥ 7 jours ; `/health` vert en continu ; aucun
      rollback déclenché ; aucune régression P0/P1 ouverte.
- [ ] `cargo xtask parity compare` vert (goldens capturés §4.1) sur un échantillon
      représentatif, rapport archivé.
- [ ] Overlay 3.0.0 publié, aucun crash-report bloquant, parc majoritairement migré
      (télémétrie updater / releases téléchargées), release de transition 2.0.2 en place.
- [ ] Image `ghcr.io/.../livechat-server:2.0.1` toujours disponible et testée (pull +
      `/health` en local) comme référence de rollback.
- [ ] Sauvegarde de la base legacy (`sqlite.db` + `data/media`) vérifiée et restaurable
      (exercice de restauration sur copie).
- [ ] CI Rust verte sur `main` (fmt, clippy `-D warnings`, nextest, audit, deny) ;
      `server-image.yml` et `extension.yml` verts.
- [ ] §4.1/§4.2/§4.3 réglés (goldens, shims renderer, `.env.example`).
- [ ] PR de suppression relue par le Lead, tag `legacy-ts-2.0.2` poussé et vérifié
      (`git ls-remote --tags`).

No-go (reporter) si : régression serveur ou overlay non résolue, goldens indisponibles,
build Tauri non reproductible depuis un clone propre, rollback non testé, ou fenêtre de
7 jours non écoulée.

## 8. Restauration après suppression (rollback)

Deux niveaux :

1. **Revert de la PR** (si la suppression vient d'être mergée) :
   `git revert -m 1 <merge_commit>` sur `main`, puis re-vérifier la CI. Les 3 commits
   pouvant être séparés (§5.3), un revert partiel est possible.
2. **Restauration depuis le tag** (si le revert n'est plus praticable) :

```powershell
git checkout legacy-ts-2.0.2 -- server overlay
git checkout legacy-ts-2.0.2 -- package.json .husky/pre-commit .husky/pre-push
# puis restaurer les workflows supprimés si nécessaire
git checkout legacy-ts-2.0.2 -- .github/workflows/server.yml .github/workflows/overlay-ci.yml `
  .github/workflows/overlay-nightly.yml .github/workflows/overlay-release.yml
```

3. **Rollback de production** (indépendant du dépôt) : redémarrer l'image legacy 2.0.1
   et restaurer la base legacy sauvegardée (procédure `docs/rust/packaging.md` §7.2 et
   `docs/rust/migration-plan.md` §9).

## 9. Points ouverts et handoffs

- **P1 — Goldens de parité :** capture à réaliser par la lane parité (Lead) sur machine
  legacy ; condition bloquante de la suppression (§4.1).
- **P2 — Shims renderer :** décision Lead sur « committer les 80 shims » vs « porter les
  sources TS + script de build » (§4.2) ; impacte `apps/lc-overlay/web/README.md` et
  `docs/rust/lc-overlay-8b.md`.
- **P3 — Release de transition Electron 2.0.2 :** à publier avant le tag ; vérifier le
  numéro dans `overlay/package.json` et `overlay-release.yml` (validation tag ↔ version).
- **P4 — Updater Tauri :** clé minisign encore placeholder (Phase 8b) ; la release
  overlay 3.0.0 doit la remplacer avant de considérer la suppression définitive (§7.1 R6).
- **P5 — `README.md`/CI :** réécriture à faire dans la PR de suppression (ordre §5.3) ;
  aucune modification maintenant.
- **P6 — `server/packages/overlay-protocol` :** supprimé avec `server/` ; vérifier qu'aucun
  script npm/pnpm racine ne le référence après nettoyage (`git grep overlay-protocol`).
- **P7 — `deploy/.env.example` :** création et revue (§4.3) ; documenter dans
  `docs/rust/packaging.md` §3.4.
- **P8 — Extension :** hors périmètre ; vérifier seulement que le nettoyage de
  `package.json` racine ne casse pas `install:all` pour `extension/`.

## 10. Références

- `docs/rust/migration-plan.md` — décisions, §9 bascule/CI/runbook, annexe D.
- `docs/rust/parity-harness.md` — harnais `xtask parity`, 34 cas, procédure de capture.
- `docs/rust/parity/README.md`, `docs/rust/parity/golden-v1/README.md`.
- `docs/rust/lc-overlay-8b.md`, `docs/rust/lc-overlay-shell.md`, `docs/rust/overlay-bridge.md`.
- `docs/rust/packaging.md` (§6 politique de tags, §7 rollback), `docs/rust/migrator.md`.
- `docs/rust/server-core.md`, `server-ws.md`, `server-overlay.md`, `server-ingest.md`,
  `server-discord.md`, `server-admin.md`, `admin-panel.md`, `protocol-v2.md`.
- `docs/rust/lc-db.md`, `lc-media.md`, `lc-playback.md`, `lc-discord.md`, `baseline.md`,
  `audit-dynamique.md`.
- `README.md` (racine), `.github/workflows/`, `.husky/`, `package.json`, `deploy/`.
- Mesures de ce document : comptages `Measure-Object -Line` et `git ls-files` du
  2026-11-10, sur la branche `feat/rust-rebuild`.
