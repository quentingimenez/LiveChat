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
| 7 | admin (15 endpoints + panel statique) | 5 |
| 8 | overlay Tauri (shell, bridge, WS, packaging) | 12 |
| 9 | packaging serveur, parité, soak, bascule | 6 |

Total ≈ 69–70 j-h (+15 % buffer ≈ 80 j-h) ; solo 14–16 semaines ; multi-agents ≈ 30–35 jours ouvrés. Chemin critique : 0 → 1 → 2 → 4 → 5 → 9 ; Phase 8 démarre après 1+3.

## 8. Tests, parité, budgets

- Unitaires : protocole (fixtures), domaine, db, scheduler (horloge simulée `tokio::time::pause`), média (fixtures), Discord (HTTP mocké).
- Intégration : routes via `tower::oneshot`, WS réel (`tokio-tungstenite`), SQLite temporaire par test.
- Contrat : `xtask parity` rejoue ~20 scénarios REST/WS contre legacy et nouveau, diff JSON tolérant ids/dates.
- E2E overlay, charge (100 overlays, 24 h), sécurité (SSRF, auth, rate limit, CSP, cargo-deny/audit).
- Parité : 22 endpoints REST, 13 types WS, 13 slash + 5 message commands, 15 endpoints admin, 3 fenêtres overlay, 100 % des lignes migrées.
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

- `/overlay` : `POST /pair/consume`, `GET /config`, `GET /media/:assetId`, `GET|POST /meme-board/items`, `GET /meme-board/media/:assetId`, `DELETE|PATCH /meme-board/items/:itemId`.
- `/ingest` : `POST /pair/consume`, `POST /`.
- `/admin` : `GET /`, `GET /api/overview`, `GET|PATCH /api/runtime-settings`, `GET /api/ingest-authors`, `POST /api/ingest-clients`, `PATCH /api/guilds/:guildId`, `POST /api/guilds/:guildId/stop`, `POST /api/guilds/:guildId/purge`, `POST /api/overlay-clients/:clientId/revoke`, `POST /api/ingest-clients/:clientId/revoke`, `GET /api/pairing-codes`, `DELETE /api/pairing-codes/expired`, `POST /api/pairing-codes/:code/revoke`.
- Racine : `GET /`, `GET /health`.

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
