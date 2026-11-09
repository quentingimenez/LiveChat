# Plan de migration Rust — LiveChat

> **Statut** : plan approuvé le 2026-11-09 ; exécution en cours (branche `feat/rust-rebuild`).
> **Périmètre** : serveur + overlay réécrits en Rust ; extension Chromium conservée en TypeScript.
> **Stratégie** : big-bang protocole v2 (WebSocket JSON natif, socket.io supprimé).
> **Documents liés** : `protocol-v2.md` (gel protocole), `audit-dynamique.md` (mesures), `overlay-bridge.md` (matrice renderer), `runbook.md` (bascule).

## 1. Décisions verrouillées

| Sujet | Décision |
|---|---|
| Serveur | Réécriture Rust : axum 0.8 + tokio + sqlx (SQLite) |
| Overlay | Réécriture Tauri 2 (Rust) ; renderer HTML/CSS/JS existant **conservé** |
| Extension | Inchangée (contrat REST conservé) ; version 0.1.1 cosmétique optionnelle |
| Protocole | v2 propriétaire : `GET /ws`, enveloppe JSON, `?token=` supprimé, URLs média signées |
| REST | Chemins, statuts et labels d'erreur **strictement conservés** |
| Base | SQLite migrée one-shot (fichier neuf + vérification), legacy intact pour rollback |
| Release | serveur 3.0.0 + overlay 3.0.0, protocole 2, tag `legacy-ts-2.0.2` |

Conséquence assumée : les overlays Electron 2.0.1 cessent de fonctionner à la bascule. Une release de transition Electron 2.0.2 (bandeau « mise à jour manuelle requise ») est obligatoire avant le cutover.

## 2. Audit — synthèse

| # | Sév. | Constat | Correctif cible |
|---|---|---|---|
| S1 | Élevé | SSRF via `/ingest` (URL arbitraire, redirections, DNS) | guard SSRF central (plages privées/loopback/link-local, redirections contrôlées) |
| S2 | Élevé | Tokens overlay acceptés en `?token=` | Bearer uniquement + URLs média signées HMAC courte durée |
| S3 | Élevé | TLS désactivé côté overlay (`rejectUnauthorized: false`) | rustls strict ; option `allow_insecure_tls` explicite |
| S4 | Élevé | Admin : token en query, CSP `unsafe-inline`, HTML inline 96 Ko | assets statiques + CSP sans inline + cookie HttpOnly/SameSite + CSRF |
| S5 | Moyen | CORS `origin: true` + credentials en dev | origines explicites, pas de credentials |
| S6 | Moyen | Rate limit global, exemptions admin/localhost, pas de limite `/ingest` | limites par route, `TRUST_PROXY_HEADERS` explicite |
| S7 | Moyen | Image prod = `pnpm dev`, Node 20, pas de migration au boot | binaire release, image slim non-root, healthcheck |
| S8 | Moyen | 5 fichiers de tests pour 56 sources, zéro test scheduler/socket/média | pyramide complète + harnais de parité |
| S9 | Moyen | Monolithes : adminRoutes 2 646 l., mediaIngestion 1 740 l., playbackScheduler 979 l. | crates + modules < 600 lignes |
| S10 | Moyen | Pipeline média sans concurrence bornée ni annulation | sémaphore (2), cancellation, kill groupe, timeouts |
| S11 | Faible | `as any` ×12, `@ts-ignore` ×8, console.error prod | types stricts, lints Rust, `thiserror` |
| S12 | Faible | Extension sans tests d'intégration | hors périmètre |
| S13 | Info | Overlay Electron : 3 fenêtres, tray, raccourcis, auto-start, updater | parité Tauri 2 |

## 3. Architecture cible

```
Cargo.toml                    workspace (resolver 2)
crates/lc-protocol            types v2 + serde + fixtures golden
crates/lc-domain              entités, ids, erreurs métier, temps
crates/lc-db                  sqlx, repositories, migrations SQL
crates/lc-media               resolver, yt-dlp/ffmpeg, transcode, cache, social
crates/lc-discord             bot serenity/poise, commandes, i18n
crates/lc-admin               routes admin + assets embarqués
apps/lc-server                binaire axum : REST + WS v2 + workers
apps/lc-overlay               app Tauri 2 + web/ (renderer existant + bridge)
static/admin                  index.html, app.js, styles.css
xtask                         migrateur, harnais de parité, release
docs/rust                     architecture, protocole, runbook, rapports
```

Dépendances : axum 0.8, tokio, tower 0.5, tower-http 0.6, tower_governor, serde, sqlx 0.8 (sqlite, migrate, offline), reqwest 0.12 (rustls), scraper, regex, url, sha2, hmac, subtle, rand, uuid, base64, serenity + poise, rust-i18n, tracing, thiserror/anyhow, clap 4, tauri 2 (+ single-instance, global-shortcut, autostart, updater, notification), dev : tokio-tungstenite, wiremock, insta, rstest, tempfile, criterion.

Règles : `#![forbid(unsafe_code)]` sauf `apps/lc-overlay` (bloc Win32 isolé) ; `clippy` interdit `unwrap_used`/`expect_used`/`panic` hors tests ; `cargo-deny` + `cargo-audit` en CI.

## 4. Protocole v2 (résumé — spec gelée dans `protocol-v2.md`)

- `GET /ws` ; première trame `hello` sous 5 s : `{token, appVersion, clientId?, sessionMode?}` ; auth = SHA-256(token) vs `overlay_clients.token_hash` + `revoked_at IS NULL`.
- Enveloppe : `{v:2, type, id, ts, payload}` ; clés métier en camelCase (parité renderer).
- `welcome` : `{protocolVersion:2, guildId, clientId, sessionMode, serverTime, config{showTextDefault,defaultMediaTime,maxMediaTime}, currentJob?}`.
- C→S : `hello`, `heartbeat` (15 s), `playback.state` (≤1/s), `playback.ended`, `error`, `stop.request`, `meme.trigger`, `peers.request`.
- S→C : `welcome`, `play`, `stop` (manual/preempt/revoked/purge), `peers`, `settings`, `ack`, `error`.
- Close : 4401 auth, 4402 révoqué, 4403 version obsolète, 1011 interne.
- Limites : trame 64 Kio, 20 msg/s, ping 15 s, timeout pong 45 s.
- Média : `{API_URL}/overlay/media/{assetId}?exp&sig`, `sig = HMAC-SHA256(secret, "assetId|clientId|exp")`, TTL 300 s ; `?token=` supprimé ; `Accept-Ranges` + `HEAD`.
- `/socket.io/` → 426 `overlay_outdated`.
- Modes : `normal` | `invite_read_only` (meme/stop refusés en lecture seule — à confirmer Phase 0).

## 5. Persistance et migration

- Schéma : mêmes entités/colonnes que Prisma ; timestamps `INTEGER` epoch ms ; booléens 0/1 ; tables snake_case ; `WAL`, `foreign_keys=ON`, `busy_timeout=5000`.
- `lc-migrate legacy-sqlite --from --to --dry-run` puis exécution ; lecture seule du legacy, écriture fichier neuf.
- `lc-migrate verify` : comptes par table, somme des tailles, échantillon de 20 lignes champ à champ.
- Formats de dates legacy : à prouver en Phase 0 (probe Prisma) puis parser avec repli multi-format ; toute valeur non parsable = échec explicite.
- Rollback : relancer l'image 2.0.1 avec le `sqlite.db` sauvegardé (jamais touché).

## 6. Overlay Tauri

- 3 fenêtres : overlay (transparent, sans frame, always-on-top, skip taskbar, click-through `set_ignore_cursor_events(true)`, topmost réaffirmé toutes les 2 s + `WS_EX_NOACTIVATE`), pairing 460×580, board 1180×780 (min 960×640).
- Renderer existant conservé ; bridge injecté fournissant `window.livechatOverlay` (13 méthodes → `invoke`/`listen`) ; mode mock pour test navigateur.
- Config : mêmes champs ; migration depuis `%APPDATA%\LiveChat\config.json` avec `.bak`.
- Tray, raccourci global, auto-start, single-instance ; updater signé (minisign) + `latest.json` GitHub.
- POC go/no-go Phase 0 : transparence + click-through + topmost sur app plein écran ; vidéo WebView2 ; tray ; coexistence installateurs. Repli : Electron conservé pour l'overlay, serveur seul migré.

## 7. Phases (estimations j-h)

| Phase | Contenu | j-h |
|---|---|---|
| 0 | toolchain, audit dynamique, POC serveur/Tauri, gel protocole | 4–5 |
| 1 | workspace, protocole, domaine, CI | 3 |
| 2 | db + migrateur + tests | 5 |
| 3 | serveur HTTP, auth, pairing, config, média Range, health | 6 |
| 4 | scheduler + WS v2 + meme board | 9 |
| 5 | pipeline média (resolver/download/transcode/cache/social) | 12 |
| 6 | Discord (13 slash + 5 message commands, i18n) | 7 |
| 7 | admin (14 endpoints + panel statique) | 5 |
| 8 | overlay Tauri (shell, bridge, WS, packaging) | 12 |
| 9 | packaging serveur, parité, soak, bascule | 6 |

Total ≈ 69–70 j-h (+15 % buffer ≈ 80 j-h) ; solo 14–16 semaines ; multi-agents ≈ 30–35 jours ouvrés. Chemin critique : 0 → 1 → 2 → 4 → 5 → 9 ; Phase 8 démarre après 1+3.

## 8. Tests, parité, budgets

- Unitaires : protocole (fixtures), domaine, db, scheduler (horloge simulée `tokio::time::pause`), média (fixtures), Discord (HTTP mocké).
- Intégration : routes via `tower::oneshot`, WS réel (`tokio-tungstenite`), SQLite temporaire par test.
- Contrat : `xtask parity` rejoue ~20 scénarios REST/WS contre legacy et nouveau, diff JSON tolérant ids/dates.
- E2E overlay, charge (100 overlays, 24 h), sécurité (SSRF, auth, rate limit, CSP, cargo-deny/audit).
- Parité : 26 endpoints REST v1 (25 en v2 après suppression de `GET /overlay/config`), 13 types WS, 13 slash + 5 message commands, 14 endpoints admin, 3 fenêtres overlay, 100 % des lignes migrées.
- Budgets : démarrage < 500 ms ; RSS repos < 30 Mo ; p95 REST local < 10 ms ; latence play < 20 ms ; binaire < 25 Mo ; RAM overlay < 150 Mo ; installeur < 20 Mo.

## 9. CI/CD et bascule (détail dans `runbook.md`)

- `rust.yml` : fmt, clippy `-D warnings`, nextest (Linux+Windows), deny/audit, `sqlx prepare --check`, build release.
- `server-image.yml` : multi-stage (`rust:bookworm` → `debian:bookworm-slim` + ffmpeg + yt-dlp), non-root, healthcheck, GHCR.
- `overlay-release.yml` : `tauri build` (NSIS + MSI + zip, mac dmg), signature, `latest.json`.
- Runbook : release transition Electron 2.0.2 → gel des envois → backup `data/` → pull image 2.0.1 de secours → stop → `lc-migrate` dry-run puis réel → start 3.0.0 + `/health` → publication overlay 3.0.0 → smoke tests → 30 min de surveillance → rollback possible (image 2.0.1 + `sqlite.db`) → nettoyage après 7 jours.

## 10. Risques et hypothèses

Risques majeurs : Tauri click-through/topmost en jeu plein écran (POC Phase 0, repli Electron) ; coexistence installateurs Electron/Tauri ; utilisateurs qui ne mettent pas à jour ; fragilité du scraping plateformes ; dérive de parité ; format des dates Prisma ; panne de bascule ; sous-estimation du scheduler.

Hypothèses : aucune nouvelle fonctionnalité ; Windows-first (mac via runner GitHub) ; extension et raccourcis iOS inchangés ; admin en cookie+Bearer ; base de prod dans `./data` ; ffmpeg/yt-dlp externes ; nouvelle clé de signature updater ; `invite_read_only` réellement restrictif.

## 11. Organisation multi-agents

- Capacité : 5–6 coéquipiers durables, 5 écrivains simultanés max ; `target/` partagé, agents en `cargo check -p`, Lead lance `nextest`/`clippy` globaux ; poste 8 cœurs/16 threads, 61,7 Go RAM.
- Règles : protocole gelé avant fan-out ; 1 owner par crate/répertoire ; `Cargo.toml` racine = Lead ; write scopes déclarés au board ; pas de parallélisme sur la sémantique scheduler et le mapping de migration ; POC risqués en worktree/spike ; revue du Lead avant merge ; aucun agent ne committe.
- Calendrier : P0 3–4 j ; P1–2 5–6 j ; P3–4 8–10 j ; P5 7–9 j ; P6–7 5–6 j (chevauche) ; P8 7–9 j ; P9 4–5 j → 30–35 jours ouvrés.

## Annexe A — Inventaire REST conservé

- `/overlay` : `POST /pair/consume`, `GET /media/:assetId`, `GET|POST /meme-board/items`, `GET /meme-board/media/:assetId`, `DELETE|PATCH /meme-board/items/:itemId`. (`GET /config` supprimé en v2 : la config est livrée dans la trame `welcome`.)
- `/ingest` : `POST /pair/consume`, `POST /`.
- `/admin` : `GET /`, `GET /api/overview`, `GET|PATCH /api/runtime-settings`, `GET /api/ingest-authors`, `POST /api/ingest-clients`, `PATCH /api/guilds/:guildId/settings`, `POST /api/guilds/:guildId/stop`, `POST /api/guilds/:guildId/purge`, `POST /api/overlay-clients/:clientId/revoke`, `POST /api/ingest-clients/:clientId/revoke`, `GET /api/pairing-codes`, `DELETE /api/pairing-codes/expired`, `POST /api/pairing-codes/:code/revoke`.
- Racine : `GET /`, `GET /health`.
- Total vérifié sur le code v1 : **26** routes (8 `/overlay` + 2 `/ingest` + 14 `/admin` + 2 racine), **25** en v2.

## Annexe B — Commandes Discord

Slash : `alive`, `client`/`overlayCode` (option `mode`), `help`, `info`, `memeAdd` (url, media, title, forceRefresh), `overlays`, `setDefaultTime`, `setDisplayFull`, `setMaxTime`, `send`.
Commands message : `hidesend`, `hidetalk`, `talk`, `stop`.

## Annexe C — Variables d'environnement

Noms v1 conservés (`PORT`, `API_URL`, `ADMIN_UI_TOKEN`, `ADMIN_UI_LOCAL_ONLY`, `DISCORD_TOKEN`, `DISCORD_CLIENT_ID`, `INGEST_API_TOKEN`, `DATABASE_URL`, `LOG`, `I18N`, `HIDE_COMMANDS_DISABLED`, `DEFAULT_DURATION`, `PLAYBACK_JOB_RETENTION_HOURS`, `MEDIA_*`, `TWITTER_*`, `PAIRING_CODE_TTL_MINUTES`, `YTDLP_*`, `TIKTOK_COOKIE`, `FFMPEG_BINARY`, `FFPROBE_BINARY`, `ADDITIONAL_CORS_ORIGINS`).
Ajouts : `RUST_LOG`, `MEDIA_MAX_CONCURRENT_INGESTIONS=2`, `MEDIA_URL_TTL_SEC=300`, `MEDIA_URL_SECRET`, `TRUST_PROXY_HEADERS=false`, `WS_HEARTBEAT_TIMEOUT_SEC=45`.

## Annexe D — Devenir des fichiers

| Existant | Devenir |
|---|---|
| `server/` | Remplacé par `crates/*` + `apps/lc-server` + `static/admin` ; supprimé après cutover |
| `overlay/src/main`, `preload`, `shared` | Remplacés par `apps/lc-overlay` (Rust + bridge) |
| `overlay/renderer/**` | Conservé, copié dans `apps/lc-overlay/web/` |
| `extension/` | Inchangé |
| workflows server/overlay | Remplacés par `rust.yml`, `server-image.yml`, `overlay-release.yml` |

## 12. Addendum — Phase 0 exécutée (2026-11-09)

État d'avancement : Phase 0 (outillage, audit dynamique, protocole gelé, POC, contrat de schéma) réalisée. Commits : `75952df` (workspace + CI + crate `lc-protocol` + spec v1 du protocole), `7181c44` (audit dynamique + bridge overlay) ; révision protocole 2 committée ensuite.

### 12.1 Outillage validé

Rust 1.99.1 / cargo 1.99.1 (host `x86_64-pc-windows-msvc`), rustfmt + clippy, cargo-nextest 0.9.151, cargo-deny 0.20.2, cargo-audit 0.22.2 ; VS 2022 BuildTools 17.14.44 (toolset 14.44.35207, `cl.exe`/`link.exe`), vswhere ; WebView2 157.0.4341.42 ; node v24.12.0. `cargo check/test/fmt/clippy -D warnings/deny/audit` verts sur le workspace squeletté ; CI `rust.yml` en place.

### 12.2 Audit dynamique du legacy (mesuré, remplace les estimations)

- Tests : server 50/50 OK, overlay 62/62 OK, extension 109/109 OK.
- Serveur cassé hors tests : `tsc` exit 2 (3× TS2339 `server/src/loaders/socketLoader.ts:160,169,186`) ; lint ESLint crashe (override `ajv` 8 : `TypeError: Cannot set properties of undefined (setting 'defaultMeta')`) ; `pnpm start` pointe `dist/src/index.js` alors que le build émet `dist/index.js`.
- Vulnérabilités production : server **61** (32 high / 22 mod / 7 low ; `undici` 29, `fast-uri` 12, `fastify` 8, `engine.io` 3, `socket.io-parser` 2, `lodash` 2, `ws` 2, `zod` 1, `fast-copy` 1, `find-my-way` 1) — `SECURITY_EXCEPTIONS.md` était donc périmé ; overlay **6** (5 high, `electron-updater`…) ; extension **0**.
- Baseline dev : boot → `/health` 22,71 s à froid / 14,42 s à chaud (dont 32 ms d'init applicative, dominée par `tsx`), prod-like 3,67 s, RSS 145,2 Mo, `/health` `{"status":"ok"}` mean 1,06 ms / p95 1,82 ms (n=50). Discord est obligatoire au boot (zod exit 1 en 7,05 s) — le serveur Rust devra démarrer sans Discord pour les tests/CI.
- Dates Prisma : SQLite stocke les `DateTime` en `INTEGER` epoch ms UTC ; du TEXT `"YYYY-MM-DD HH:MM:SS"` (UTC seconde, via `DEFAULT CURRENT_TIMESTAMP` d'un INSERT brut) est possible et doit être toléré ; Prisma rejette `T` sans fuseau, espace+fraction et REAL ; une valeur invalide casse toute requête sélectionnant la colonne. Le migrateur écrira en i64 ms avec parseur tolérant.

### 12.3 Décisions de protocole rev. 2 (détail `protocol-v2.md`)

- TTL média : `exp = now + clamp(durationSec + 120, 300, 3600)` ; 300 s pour une image sans durée ; `HEAD`/`Range` à faire avant `exp` ; pause > TTL non supportée (nouvelle URL au prochain `play`).
- `stop.request { jobId?, source?: "user"|"hotkey"|"admin" }` ; absent = tout arrêter.
- Invariant scheduler : un seul `PLAYING` par guilde ; anomalie = plus récent `started_at` + warn. Reprise : `remaining_ms_snapshot` + `resume_offset_sec`.
- `GET /overlay/config` supprimé : config uniquement dans `welcome`.
- `MIN_OVERLAY_VERSION` (défaut 3.0.0), `serverVersion` dans `welcome`, close 4403 `{"minVersion":"x.y.z"}`, comparaison semver simple ; 4409 = reconnexion ≤ 1/s.
- Helpers dans `lc-protocol` : `media_url_ttl_sec(kind, durationSec)`, `outdated_protocol_reason`, `parse_simple_version`, `is_version_below` ; 29 fixtures golden, 15 tests round-trip.

### 12.4 Overlay Tauri — décisions bridge

`bridge.js` (719 l., 13 méthodes) livré ; CSP `script-src 'self'` strict (`style-src` tolère l'inline de `pairing.html`) ; origine WebView `http://tauri.localhost` ⇒ CORS `/overlay` et `/admin` à compléter avec `http(s)://tauri.localhost` ; URL média v2 utilisée verbatim (token legacy supprimé en Phase 8) ; événements `overlay://play|stop|settings` conservés ; config forcée `%APPDATA%\LiveChat\config.json` (pas de `app_config_dir()`), `.bak` one-shot, rollback Electron possible ; bufferisation Rust des play/stop avant `overlay_renderer_ready` (file bornée, 5 s puis drop), board sans buffer.

### 12.5 Contrat de schéma v3

`docs/rust/schema-v3.sql` : tables STRICT snake_case (`app_metadata`, `media_assets`, `meme_board_items`, `playback_jobs`, `guilds`, `overlay_clients`, `ingest_clients`, `pairing_codes`), dates INTEGER ms, booléens 0/1, contraintes CHECK, index alignés Prisma. C'est le seul contrat dont dépend le migrateur ; toute évolution passe par le Lead.

### 12.6 Corrections d'inventaire et suite

- Inventaire REST vérifié : **26** routes v1 / **25** v2 (18.1) ; admin 14 et non 15 ; le route de réglages guilde est `PATCH /api/guilds/:guildId/settings`.
- Phase 2 prête : `task-7` (lc-db + lc-domain) et `task-8` (migrateur `xtask migrate-legacy`) sont sur le board, en attente des verdicts POC ; POCs serveur/Tauri en build (`spikes/`).
- Propos suivant : verdicts POC → go/no-go Tauri → commit d'intégration Phase 0 → lancer Phase 2 → puis Phases 3–9.

### 12.7 Verdicts POC et décision overlay Tauri (2026-11-09)

**POC Tauri — verdict GO conditionnel retenu : GO.** Les 6 propriétés critiques sont prouvées sur la machine (Windows 11 10.0.26200, Tauri 2.12.5, WebView2 157.0.4341.42) : transparence réelle, click-through effectif (`WS_EX_TRANSPARENT|WS_EX_LAYERED`, clic synthétique traversant), topmost réaffirmé (13 ticks / 2 s, aucun vol de focus `WS_EX_NOACTIVATE`), vidéo H.264/AAC (10,03 s, seek 6,72 s, `readyState=4`, `error=null`), tray + single-instance (2ᵉ instance 134–203 ms), démarrage fenêtre 436 ms / JS 480 ms, exe 9,42 Mo, NSIS 1,99 Mo. Rapport complet conservé en `docs/rust/poc-tauri-report.md` (sources et captures restent hors dépôt sous `spikes/`, gitignoré).

Décisions prises :
- **Budget RAM révisé.** Le process Tauri seul fait 47,6 Mo WS / 20,2 Mo private (dans le budget < 30 Mo) ; l'arbre complet incluant les process WebView2 incompressibles (browser/gpu/renderer/utility) est mesuré à 355 Mo WS / 205 Mo private au repos et 583 / 475 Mo en lecture vidéo. Le budget « overlay < 150 Mo » du plan est remplacé par : *process applicatif Tauri ≤ 60 Mo WS ; coût WebView2 mutualisé accepté (non contraint, non régressif vs Electron qui embarque déjà Chromium)*. La mutualisation d'une WebView2 entre board et overlay sera évaluée en Phase 8 (option, pas exigence).
- **Topmost et plein écran.** Windows ne garantit pas `WS_EX_TOPMOST` au-dessus d'une surface exclusive fullscreen. Aucun test exclusive fullscreen n'est concluant (le POC a été validé en fenêtré). Le repli retenu est identique au comportement Electron 2.0.1 actuel : fenêtres de jeu sans bordure/fenêtrées OK, exclusive fullscreen non supporté par l'overlay (limitation de plateforme, pas une régression). Phase 8 doit le documenter et le tester en condition réelle.
- **Événements et origine.** `overlay://play|stop|settings` conservés (émis Rust et reçus JS, validé) ; origine WebView par défaut `http://tauri.localhost` (fetch loopback OK, CORS à compléter côté serveur) ; `useHttpsScheme` non retenu (LAN http bloqué) ; nommage `overlay:play` reste préféré dans le protocole v2 pour les messages serveur (les deux sont techniquement valides, décision finale en Phase 8).
- **Chemins de config.** `app_config_dir()` = `Roaming\com.livechat` ≠ `%APPDATA%\LiveChat` du legacy. Phase 8 force le chemin `%APPDATA%\LiveChat\config.json` (comme décidé §12.4) ou migre avec `.bak` one-shot ; l'option `fs:scope` exacte validée au POC est reprise dans le rapport.
- **Coexistence et bascule.** Upgrade in-place Electron → Tauri impossible proprement (clés de désinstallation différentes : GUID `ec5c36a6-…` vs `${PRODUCTNAME}`). Bêta side-by-side avec `productName: "LiveChat Next"` et `identifier: "com.livechat.next"`, puis bascule finale : désinstallation Electron 2.0.1, installation `LiveChat` Tauri, migration `%APPDATA%\LiveChat\config.json` → chemin cible, reprise de la Run key, test de désinstallation.
- **Suite.** Phase 0 close. Phase 2 (`lc-db` + `lc-domain`, migrateur `xtask`) en cours ; Phase 3 (serveur HTTP) et Phase 5 (pipeline média) démarrent en parallèle après les objets `lc-db`.
