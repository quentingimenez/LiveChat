# apps/lc-overlay — Phase 8b : client WebSocket v2, bridge réel, renderer legacy

Statut : terminé le 2026-11-10. Périmètre de cette phase : `apps/lc-overlay/**` + ce
document. Aucun commit. Le shell Tauri (Phase 8a) reste décrit dans
`docs/rust/lc-overlay-shell.md` ; ce document couvre la couche réseau v2, le bridge
réel, le portage du renderer legacy et les écarts/limites.

## 1. Vue d'ensemble

| Couche | Fichier(s) | Rôle |
| --- | --- | --- |
| Supervision WS | `src/net/ws.rs` | connexion `{serverUrl}/ws`, handshake `hello`, heartbeat, reconnexion backoff, adoption `welcome.currentJob`, mapping des codes de fermeture, snapshot d'état pour le tray |
| Appairage HTTP | `src/net/pairing.rs` | `POST {serverUrl}/overlay/pair/consume`, validation legacy, persistance de la config |
| État partagé | `src/state.rs` | config persistée + `NetSnapshot` + sender de commandes réseau + playback courant + throttle `playback.state` |
| Bridge commandes | `src/commands.rs` | 12 commandes Tauri : config, handshake renderer, télémétrie, appairage, memes, updater |
| Tray/statut | `src/tray.rs`, `src/windows.rs`, `src/shortcuts.rs` | ligne de connexion, reconnexion, raccourcis mémoire → trames `meme.trigger` |
| Renderer | `web/**` | pages legacy copiées telles quelles + `bridge.js` + `dist/renderer/**` (shims) |

Le renderer legacy (`overlay.js` / `board.js` / `pairing.js`) n'a pas été modifié :
il consomme déjà `window.livechatOverlay`, donc seule la façade `bridge.js` a été
injectée dans les pages.

## 2. Client WebSocket v2 (`src/net/ws.rs`)

- URL : `{serverUrl}` (http/ws → `ws://`, https/wss → `wss://`) + `lc_protocol::WS_PATH` (`/ws`),
  query/fragment purgés (testé). TLS strict par défaut ; option insecure legacy
  (`rejectUnauthorized=false`) derrière `LIVECHAT_INSECURE_TLS=1|true|yes|on`
  (via `native_tls::TlsConnector::danger_accept_invalid_certs`). Ce choix est
  explicite et documenté : il ne s'active jamais silencieusement.
- Un superviseur tokio possède la connexion et la boucle de reconnexion ; le reste
  de l'app lui parle via un canal de commandes (`NetCommand::Send`, `Reconnect`,
  `Shutdown`) exposé par `AppState::send_net_command`.
- Handshake : `hello` immédiat (token, `appVersion = CARGO_PKG_VERSION`,
  `clientId`, `sessionMode` déduit de `guestMode`), puis `welcome` → persistance
  éventuelle de `clientId`/`guildId`/`guestMode`, adoption de `currentJob`
  (seek calculé par `adopt_current_job`), `settings` immédiat, heartbeat toutes les 15 s.
- Trames sortantes : `hello`, `heartbeat`, `playback.state` (throttle 1/s au niveau
  `AppState`), `playback.ended`, `error` (télémétrie renderer), `stop.request`,
  `meme.trigger`, `peers.request`.
- Trames entrantes : `welcome`, `settings`, `play`, `stop`, `peers`, `ack`, `error`
  (les `error` serveur ne sont pas fatales). `play`/`stop` passent par le buffer
  borné existant (`events::deliver_or_buffer`, 5 s avant renderer-ready).
- Fermetures : `1000/1003` terminal sans boucle, `4401` purge token + réouverture
  du pairing, `4402` terminal révoqué + purge, `4403` terminal obsolète avec
  `minVersion` parsé dans le JSON de raison, `4409` backoff exponentiel 1→2→4…→30 s
  plafonné et jamais plus d'une tentative/s, `1001/1011/1002/4408/1006` reconnexion
  avec backoff 1→2→3 s (remis à zéro après `welcome`).
- `NetSnapshot` (status, reason, version serveur, `minVersion`, guild/client,
  session mode, pairs, dernière erreur) est la source du tray et des logs.
- **Keep-alive des `play` en file locale.** Quand un `play` arrive alors qu'une
  autre guilde occupe l'écran, il reste dans la file locale de l'overlay ; tant
  qu'il y est, la session envoie toutes les 3 s un `playback.state`
  `state: "paused"` avec `remainingMs = durationSec * 1000` (après `welcome`
  uniquement). Le serveur voit un job toujours en attente et son watchdog (15 s)
  ne le libère pas avant l'affichage.

## 3. Bridge réel (`src/commands.rs`)

- `overlay_get_config` : projection 8 champs de la config (pas de fuite de token
  dans les logs).
- `overlay_renderer_ready` : marque le renderer prêt, émet `overlay://settings`
  puis rejoue le buffer. Événements : `overlay://play`, `overlay://stop`,
  `overlay://settings` (noms conservés, `://` validés au POC).
- `overlay_report_playback_state` / `_stop` / `overlay_report_error` : traduction
  en trames `playback.state` / `playback.ended` / `error` (throttle 1/s, champs
  clampés, pas de panic si payload invalide).
- `pairing_consume` : appelle `pairing::consume` (HTTP), puis réapplique les
  raccourcis selon `guestMode`, émet les settings, ferme la fenêtre pairing,
  rafraîchit le tray, affiche l'overlay et demande une reconnexion WS.
- `meme_board_trigger` / `meme_board_stop` : trames `meme.trigger` /
  `stop.request`. Plus de stub `socket_unavailable` ; si le superviseur n'est pas
  lancé, la commande renvoie `ok:false` avec une raison explicite.
- `meme_board_get_bindings` / `meme_board_set_bindings` : conservés, avec rollback
  strict côté raccourcis.

## 4. Renderer porté (`web/**`)

- `overlay.html`, `board.html`, `pairing.html` + `overlay.css`, `board.css`,
  `overlay.js`, `board.js`, `pairing.js`, `styles/**` copiés depuis
  `overlay/renderer/` (legacy intouché).
- `web/dist/renderer/**` = shims ESM copiés depuis `overlay/dist/renderer/**`
  (loader legacy, utils, contrôleurs). Les pages chargent le module
  `../dist/renderer/<page>/index.js`, qui injecte les globals puis charge le
  script runtime relatif à la page.
- Ajouts aux 3 pages, avant le module : `<script src="./bridge.js">` puis
  `<script src="./renderer-errors.js">` (pas de script inline, compatible CSP
  `script-src 'self'`).
- Patch média v2 : `web/dist/renderer/overlay/legacy-media-dom-utils.js`
  `buildAuthorizedMediaUrl` retourne désormais l'URL **verbatim** (aucun `?token=`) :
  les URLs signées v2 (`?clientId&exp&sig[&startOffsetSec]`) sont la seule source.
  Le board conserve son `?token=` REST (`/overlay/meme-board/items` et
  `/media/:assetId`), le contrat REST v2 n'ayant pas changé — écart documenté.
- `web/renderer-errors.js` : remonte les erreurs JS (window error, unhandled
  rejection, `console.error`) vers `overlay_report_error` → trame `error` serveur.
  C'est ce qui a permis de détecter la course d'initialisation `state not managed`
  pendant le smoke run.

## 5. CSP et capabilities

CSP élargie dans `tauri.conf.json` et `tauri.beta.conf.json` :

```
default-src 'self';
script-src 'self' https://platform.twitter.com;
style-src 'self' 'unsafe-inline';
img-src 'self' data: blob: asset: http://asset.localhost http: https:;
media-src 'self' blob: http: https:;
connect-src 'self' ipc: http://ipc.localhost http: https: ws: wss:;
frame-src https://platform.twitter.com https://syndication.twitter.com;
object-src 'none'
```

Motifs de l'élargissement vs `docs/rust/overlay-bridge.md` §8.2 : l'API_URL et le
WS sont configurables à l'exécution (impossible d'injecter des origines statiquement
dans le manifeste), les médias sont signés sur des origines serveur variables, et
les cartes tweet chargent `platform.twitter.com/widgets.js` + iframes. `script-src`
reste limité à `'self'` + Twitter (pas de `'unsafe-inline'`/`'unsafe-eval'`).

Capabilities inchangées : `core:default` + `fs:default` + `fs:allow-read-text-file`
+ `fs:allow-write-text-file` + scopes `$APPDATA/LiveChat/**`, `$APPCONFIG/**/*`,
`$APPLOCALDATA/**/*`. Le bridge passe par des commandes applicatives (hors ACL),
comme prévu.

## 6. Tray / statut / raccourcis

- Menu tray : lignes désactivées `Connexion : …` / `Détail : …` / `Version : Overlay vX`,
  puis `Reconnecter` (enabled si overlay activé et appairé), `Afficher l'overlay`,
  `Ouvrir Meme Board`, `Appairer l'overlay`, toggles config, écrans, volume,
  `Vérifier les mises à jour`, `Quitter`. Le tooltip inclut le statut et la version serveur.
- `tray rafraîchi status=… detail=…` est journalisé en debug à chaque changement.
- Raccourcis : `reapply_for_session` recale les bindings sur `guestMode` après
  `welcome`/purge ; un raccourci mémoire trouvé envoie `meme.trigger` au serveur
  (plus d'émission directe « locale »).

## 7. Configuration et cycle de vie

- Chemin forcé `%APPDATA%\LiveChat\config.json` (ou `LIVECHAT_CONFIG_PATH`) ; la
  projection 8 champs reste la seule vue renderer.
- Correctif de course important : l'`AppState` est désormais géré **via
  `Builder::manage`** (avant la construction des fenêtres), et non plus dans
  `setup`. Les webviews appellent `overlay_get_config`/`overlay_renderer_ready`
  dès leur chargement, avant `setup` ; l'ancien ordre produisait
  `state not managed for field state on command overlay_get_config` (panic
  non-unwinding). La migration `app_config_dir()` et le `.bak` sont faits dans
  `setup` via `finalize_store`.

## 8. Vérifications

| Commande | Résultat |
| --- | --- |
| `cargo fmt --check -p lc-overlay` | OK |
| `cargo clippy -p lc-overlay --all-targets -- -D warnings` | OK |
| `cargo test -p lc-overlay` | **51 tests OK** (config, display, events, state, pairing, ws mock tokio-tungstenite, updater, win32) |
| `cargo tauri build --debug --no-bundle` | OK → `target/debug/lc-overlay.exe` |
| `cargo tauri build --debug --no-bundle --config tauri.beta.conf.json` | OK |

Smoke run (9 s, config temporaire `%TEMP%\lc-overlay-8b-smoke\config.json`
`{"enabled":true}`, `RUST_LOG=lc_overlay=debug`) :

- `fichier de configuration forcé chemin=…\lc-overlay-8b-smoke\config.json`
- `overlay_get_config demandé par le renderer` (overlay puis board)
- `renderer overlay prêt : envoi des settings et rejeu du buffer`
- `sauvegarde .bak créée`, `icône et menu de la zone de notification créés`
- `overlay positionné sur l'écran sélectionné display_index=0 …`,
  `styles étendus overlay appliqués before=0x080C0138 after=0x080C01B8`
  (`WS_EX_LAYERED|WS_EX_TRANSPARENT|WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW`)
- `raccourcis de mémoire réappliqués applied=0 failed=0`
- `tray rafraîchi status=non appairé detail=Détail : appairage requis`
- process vivant à 9 s, tué ensuite : `leftover_lc_overlay=0`, stderr vide.

Le test `net::ws::tests::session_handshake_heartbeat_play_and_close` couvre le
handshake réel contre un mock WS (hello reçu, heartbeat reçu, `play` reçu, close 1000).

## 9. Écarts et limites

1. **4401** : la description de phase demande purge du token + réouverture du
   pairing ; `docs/rust/protocol-v2.md` §10 prévoit une reconnexion avec backoff en
   gardant le token. Implémentation conforme à la consigne produit (purge +
   pairing) ; divergence documentée ici.
2. **Board** : le renderer board conserve `?token=` pour les appels REST
   meme-board (contrat REST v2 inchangé) alors que l'overlay n'envoie plus de
   token sur les médias signés.
3. **CSP** : élargie (http/https/ws/wss) ; une future config statique d'API
   permettrait de la resserrer.
4. **TLS insecure** : uniquement via `LIVECHAT_INSECURE_TLS` (jamais par défaut).
5. **Updater** : pubkey minisign encore placeholder → `overlay_check_updates` /
   `overlay_install_update` renvoient une erreur explicite, à remplacer en Phase 9.
6. **`displayId` Electron** n'est pas résolu (seuls `displayKey`/`displayIndex`/
   primaire le sont) ; hérité de la Phase 8a.
7. **Non testé bout-en-bout** contre le vrai serveur v2 (les crates serveur sont
   hors périmètre) : couverture par mock + tests unitaires uniquement.
8. **Pas de mesure mémoire/CPU refaite** en 8b ; se référer au POC Tauri
   (`docs/rust/poc-tauri-report.md`) et à la révision de budget du plan §12.7.
9. **Peers** : seuls les `clientId` sont affichés (label serveur pas encore
   disponible dans `PeerInfo` du protocole).
10. **Multi-écrans** : déplacement à chaud non traité (l'écran est choisi à
    l'initialisation, comme en 8a).

## 10. Build / release

```
# debug de développement (vérifié)
cargo tauri build --debug --no-bundle

# release (installeur NSIS currentUser, à exécuter en Phase 9)
cargo tauri build

# variante bêta side-by-side (LiveChat Next / com.livechat.next)
cargo tauri build --config tauri.beta.conf.json
```

Prérequis : `tauri-cli 2.12.4` (`cargo tauri --version`), PATH
`%USERPROFILE%\.cargo\bin`, `CARGO_BUILD_JOBS=4` pour limiter la contention.

## 11. APIs à réutiliser en Phase 9

- `net::ws::{start, request_reconnect, send_command, request_shutdown}` et
  `AppState::send_net_command` pour toute nouvelle trame.
- `events::{deliver_or_buffer, emit_settings, flush_pending}` pour le contrat
  renderer (`overlay://play|stop|settings`).
- `shortcuts::reapply_for_session` après toute modification de `guestMode`/token.
- `tray::refresh_tray` + `AppState::net_snapshot()` pour tout statut UI.
- `pairing::consume` reste la seule entrée d'appairage (pas de fetch renderer).
