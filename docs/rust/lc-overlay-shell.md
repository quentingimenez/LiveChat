# Coquille Tauri 2 — `apps/lc-overlay` (Phase 8a)

Document de conception et de passation de la Phase 8a : coquille applicative
Tauri 2 de l'overlay LiveChat, **sans** client WebSocket ni renderer définitif.
Le contrat exposé au renderer (`window.livechatOverlay`, 13 méthodes) et les
décisions de fenêtres/config sont ceux de [`overlay-bridge.md`](overlay-bridge.md)
et du [`poc-tauri-report.md`](poc-tauri-report.md).

Statut : shell compilé, testé et vérifié le 2026-11-10 (Windows 11, rustc/cargo
1.99.1, tauri 2.12.5, tauri-cli 2.12.4). Aucun commit effectué.

## 1. Périmètre

| Inclus en 8a | Réservé à 8b |
| --- | --- |
| Projet Tauri 2 complet (`cargo`, `build.rs`, `tauri.conf.json`, `tauri.beta.conf.json`) | Client WebSocket `tokio-tungstenite 0.29` + protocole v2 |
| Config legacy 16 champs, chemin forcé `%APPDATA%\LiveChat\config.json`, `.bak`, migration | Appairage HTTP réel (`pairing_consume`) |
| 3 fenêtres (overlay/pairing/board), click-through, topmost 2 s, styles Win32 | Renderer définitif (copie `overlay/renderer/**`, overlay.html/board.html/pairing.html) |
| Tray + menu complet (affichage, écran, volume, autostart, MAJ, quitter) | Hotkeys de memes actifs (émission socket) |
| Single-instance, autostart, squelette global-shortcuts | Updater signé (clé minisign réelle), vérification au démarrage |
| 10 commandes + 3 événements du contrat bridge (buffer 5 s) | Statut de connexion dynamique dans le tray |
| Updater plugin (stub), capabilities, CSP | Broadcast d'état playback/erreurs vers le board |

## 2. Arborescence

```text
apps/lc-overlay/
  Cargo.toml              # package lc-overlay (lib rlib + bin lc-overlay)
  build.rs                # tauri_build::build()
  tauri.conf.json         # LiveChat / com.livechat — build stable
  tauri.beta.conf.json    # LiveChat Next / com.livechat.next — build bêta side-by-side
  capabilities/default.json
  icons/icon.png|icon.ico # reprise de spikes/poc-tauri/icons (placeholders Phase 9)
  gen/schemas/            # généré par tauri-build (gitignore : /gen/schemas/)
  web/
    bridge.js             # CONTRAT EXISTANT, conservé tel quel
    bridge-test.html      # page de test Phase 0, conservée
    README.md             # consignes bridge Phase 0, conservées
    overlay.html          # placeholder 8a (badge bridge)
    board.html            # placeholder 8a (config JSON via bridge)
    pairing.html          # placeholder 8a (appairage en 8b)
    favicon.ico           # évite les asset not found de WebView2
  src/
    main.rs               # windows_subsystem release + appel run()
    lib.rs                # tracing, plugins, setup, run, boucle d'événements
    config.rs             # OverlayConfig 16 champs, ConfigStore, normalisations, tests
    display.rs            # DisplayInfo, sélection d'écran, helpers purs, tests
    state.rs              # AppState (config + Ready + buffer + bindings actifs), tests
    events.rs             # noms d'événements, buffer borné + timeout 5 s, tests
    win32.rs              # SEUL module unsafe : styles ex + SetWindowPos topmost
    windows.rs            # cycle de vie 3 fenêtres, topmost 2 s, second instance
    tray.rs               # icône + menu + handlers
    shortcuts.rs          # global-shortcuts / memeBindings (squelette)
    updater.rs            # check/install (stub)
    commands.rs           # 10 commandes bridge + 2 commandes update
```

## 3. Build et vérifications

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_BUILD_JOBS = '4'

cargo fmt --check -p lc-overlay
cargo clippy -p lc-overlay --all-targets -- -D warnings
cargo test -p lc-overlay
# shell complet avec les assets web, sans installateur :
cd apps/lc-overlay; cargo tauri build --debug --no-bundle
```

Résultats mesurés (2026-11-10) :

- `cargo fmt --check -p lc-overlay` : OK.
- `cargo clippy -p lc-overlay --all-targets -- -D warnings` : OK (0 warning).
- `cargo test -p lc-overlay` : **35 tests, 35 OK** (`config` 17, `display` 8,
  `events` 4, `state` 4, `win32` 2 ; `shortcuts` 1).
- `cargo tauri build --debug --no-bundle` : OK, binaire
  `target/debug/lc-overlay.exe` (23,58 Mo en debug, non significatif du
  binaire release).
- Smoke run du binaire debug (~6 s, aucun crash) : `tray` créé, styles étendus
  overlay appliqués, topmost actif, fenêtre positionnée sur l'écran mémorisé.

### Contrat de lints

Le package n'hérite pas de `[lints] workspace = true` (Cargo interdit de
surcharger `workspace.lints` tout en gardant `lints.workspace = true`) : les
lints du workspace sont recopiés explicitement dans `apps/lc-overlay/Cargo.toml`,
à la seule différence que `unsafe_code = "deny"` remplace `"forbid"` (le module
`win32` est le seul à porter `#![allow(unsafe_code)]`). Les tests unitaires
peuvent utiliser `unwrap/expect/panic` et les littéraux de test via
`#![cfg_attr(test, allow(...))]` dans `lib.rs` : le contrat « pas de
unwrap/expect/panic/dbg » s'applique au code de production.

## 4. Configuration

### 4.1 Chemin forcé et migration

Ordre de résolution (`config::resolve_config_path_from_env`) :

1. variable d'environnement `LIVECHAT_CONFIG_PATH` (tests/diagnostics) ;
2. `%APPDATA%\LiveChat\config.json` — **chemin legacy imposé** (jamais
   `app_config_dir()`, qui renverrait `%APPDATA%\com.livechat\config.json`) ;
3. repli `app_config_dir()/config.json` si `%APPDATA%` est indisponible.

Au démarrage (`lib.rs::prepare_store`) :

- `migrate_from(app_config_dir()/config.json)` : copie unique si le fichier
  forcé n'existe pas encore ;
- `ensure_backup()` : copie `config.json` → `config.json.bak` **une seule fois**,
  avant toute écriture Tauri, pour permettre le rollback Electron ;
- lecture tolérante : fichier absent/corrompu/`[]` → configuration par défaut +
  avertissements (jamais de panic).

> ⚠ Le smoke run 8a a créé `%APPDATA%\LiveChat\config.json.bak` à partir du
> `config.json` Electron existant sur la machine de test (comportement attendu
> de la migration, aucune réécriture du fichier Electron).

### 4.2 Schéma

`OverlayConfig` = les 16 champs legacy exacts (camelCase JSON) :
`displayId`, `displayIndex`, `displayKey`, `displayLabel`, `volume`, `enabled`,
`autoStart`, `guestMode`, `showText`, `serverUrl`, `clientToken`, `clientId`,
`guildId`, `authorName`, `deviceName`, `memeBindings` ; les clés inconnues sont
conservées dans `extra` (`#[serde(flatten)]`) pour ne pas perdre de champ
Electron lors d'un rollback.

Normalisations identiques au legacy : volume borné `[0,1]` (non fini → 1),
`autoStart`/`guestMode` seulement si `=== true`, `memeBindings` trimmés et
entrées vides supprimées. `serverUrl` est **conservé tel quel** dans le fichier :
comme dans le legacy, `normalizeServerUrl` (trim + suppression des `/` finaux)
ne s'applique qu'à l'usage (appairage/connexion, Phase 8b).

Projection renderer (`OverlayConfigView`, 8 champs) :
`serverUrl`, `clientToken`, `guildId`, `clientId`, `guestMode`, `showText`,
`volume`, `memeBindings` — `display*`, `enabled`, `autoStart`, `authorName` et
`deviceName` restent côté shell.

## 5. Fenêtres

| Label | Taille | Politique |
| --- | --- | --- |
| `overlay` | écran cible plein (2560×1440 ×1.0 dans le POC) | transparente, sans décorations, always-on-top, skip taskbar, focusable=false, click-through, `WS_EX_LAYERED|TRANSPARENT|NOACTIVATE|TOOLWINDOW` |
| `pairing` | 460×580 | non redimensionnable, centrée, cachée au démarrage |
| `board` | 1180×780 (min 960×640) | centrée, cachée au démarrage |

Détails d'implémentation :

- `overlay` est positionnée sur l'écran mémorisé (`displayKey` → `displayIndex`
  → primaire → premier) et la sélection est réécrite dans la config si elle a
  changé (`windows::persist_display_selection`).
- `apply_overlay_flags` : `set_ignore_cursor_events(true)`,
  `set_focusable(false)`, `set_always_on_top(true)`, `set_skip_taskbar(true)`,
  puis `GetWindowLongPtrW(GWL_EXSTYLE)` + `SetWindowLongPtrW` pour forcer les
  styles Win32, `show()`, `SetWindowPos(HWND_TOPMOST, SWP_NOMOVE|SWP_NOSIZE|
  SWP_NOACTIVATE)`.
- Thread `spawn_topmost_guard` : toutes les 2 s (cadence legacy
  `setInterval(2000)`), `set_always_on_top(true)` + réaffirmation des styles et
  de `HWND_TOPMOST`, jusqu'à ce que la fenêtre disparaisse.
- `CloseRequested` sur les 3 fenêtres → `api.prevent_close()` + `hide()` : le
  shell ne détruit jamais ses fenêtres (comme l'`auxiliary-window-service`
  Electron).
- Single-instance : le plugin est enregistré **en premier** ; une deuxième
  instance transmet `argv`/cwd à l'instance principale puis quitte.
  `--pairing` ouvre la fenêtre d'appairage, sinon le board.

## 6. Tray

`TRAY_ID = "main-tray"`, icône = `default_window_icon()` (placeholder Phase 9).
Menu reconstruit via `app.tray_by_id("main-tray").set_menu(...)` après tout
changement (config/écran/autostart).

Ordre et identifiants :

1. `tray.show_overlay` — Afficher l'overlay
2. `tray.open_board` — Ouvrir Meme Board (activé si `enabled &&
   hasPairingConfig`)
3. `tray.open_pairing` — Appairer l'overlay
4. `tray.toggle_enabled` — Overlay activé (case)
5. `tray.toggle_autostart` — Lancer au démarrage (case, pilotée par le plugin)
6. `tray.toggle_show_text` — Afficher le texte (case)
7. `tray.display.header` — « Écran sélectionné : … » (désactivé) +
   `tray.display.<index>` — radios « Écran N - label »
8. `tray.volume` — sous-menu : `Volume +`, `Volume -`, `Volume 100 %`
9. `tray.check_updates` — Vérifier les mises à jour
10. `tray.quit` — Quitter

Écarts assumés vs menu Electron : pas encore de lignes « Statut connexion /
Mise à jour / Version » (dépendent du client WS et de l'updater réels, Phase 8b).

## 7. Autostart et coexistence

- Plugin `tauri-plugin-autostart`, arg `--from-autostart` (transmis au
  lancement, exploitable en 8b pour ne pas voler le focus).
- Nom de la valeur `Run` : `autostart_app_name()` = env
  `LIVECHAT_PRODUCT_NAME` sinon `"LiveChat"`.
- **Risque coexistence** : Windows compare les noms de valeur de registre sans
  tenir compte de la casse ; l’Electron utilise déjà
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\livechat`. Activer
  l'autostart Tauri **avant la désinstallation de l'Electron** écraserait cette
  valeur. Mitigation : build bêta avec
  `$env:LIVECHAT_PRODUCT_NAME = 'LiveChat Next'`, ou n'activer l'autostart
  stable qu'après bascule.
- Rappel bundler (POC) : NSIS Tauri crée la clé de désinstallation
  `…\Uninstall\LiveChat` (l'Electron a un GUID) et installe dans
  `%LOCALAPPDATA%\LiveChat` (l'Electron dans `%LOCALAPPDATA%\Programs\livechat`)
  → side-by-side mécaniquement possible, mais même nom dans « Applications et
  fonctionnalités » et même raccourci menu Démarrer `LiveChat.lnk` si le
  productName est identique — d'où la variante `LiveChat Next`.

## 8. Updater

- Plugin `tauri-plugin-updater` + config `plugins.updater.endpoints =
  ["https://github.com/Babz-Discord/LiveChat/releases/latest/download/latest.json"]`
  (bêta : `.../releases/download/next/latest.json`).
- `plugins.updater.pubkey` = **placeholder**
  `RWQPLACEHOLDER_MINISIGN_PUBLIC_KEY_A_REMPLACER_EN_PHASE_9=` : la clé
  minisign réelle est générée en Phase 9. Tant que le placeholder est en place,
  `overlay_check_updates` renvoie un état `error` explicite et
  `overlay_install_update` échoue proprement (pas de panic).
- Commandes : `overlay_check_updates` → `UpdateStatus {state: "up_to_date" |
  "available" | "error", currentVersion, availableVersion?, message}` ;
  `overlay_install_update` → `download_and_install` puis état `installed`
  (sous Windows, l'installateur ferme l'app).
- Pas encore de vérification automatique au démarrage ni de lignes de statut
  dans le tray (Phase 8b/9).

## 9. Global shortcuts (squelette memeBindings)

- Accélérateur de rechargement manuel (legacy) :
  `CommandOrControl+Shift+Alt+R` → `emitManualStopSignal` sera branché sur le
  socket en 8b ; pour l'instant l'action recharge la webview overlay
  (`window.eval("window.location.reload()")`) et journalise.
- `apply_meme_bindings(app, bindings, strict, persist)` reproduit la sémantique
  legacy : mode invité → aucun binding ; `strict` → rollback complet si un
  accélérateur est refusé, réponse `{ok, appliedBindings, failedAccelerators}`.
- Commandes bridge `meme_board_get_bindings` / `meme_board_set_bindings`
  opérationnelles ; `meme_board_trigger` / `meme_board_stop` renvoient
  `{ok:false, reason:"socket_unavailable"}` jusqu'au client WS (8b).

## 10. Événements et commandes (contrat bridge)

Événements ciblés (jamais `app.emit`) :

| Nom | Cible | Payload |
| --- | --- | --- |
| `overlay://play` | fenêtre `overlay` | `OverlayPlayPayload` complet |
| `overlay://stop` | fenêtre `overlay` | `OverlayStopPayload` |
| `overlay://settings` | fenêtres `overlay` + `board` | projection 8 champs |

Bufferisation (décision §8.1 de `overlay-bridge.md`) : tant que l'overlay n'a
pas appelé `overlay_renderer_ready`, les événements play/stop sont coalescés
(durée bornée à 2 : dernier play + dernier stop), puis rejoués dans l'ordre au
handshake ; timeout de 5 s → purge + `warn`. `overlay://settings` n'est jamais
bufferisé vers le board (il n'appelle pas `renderer_ready`) mais est émis
immédiatement aux deux fenêtres.

Commandes exposées (via `tauri::generate_handler!`) :
`overlay_get_config`, `overlay_renderer_ready`, `overlay_report_error`,
`overlay_report_playback_state`, `overlay_report_playback_stop`,
`pairing_consume`, `meme_board_get_bindings`, `meme_board_set_bindings`,
`meme_board_trigger`, `meme_board_stop` (+ `overlay_check_updates`,
`overlay_install_update`).

`bridge.js` (Phase 0, inchangé) détecte `__TAURI_INTERNALS__.invoke` puis
`__TAURI__.core.invoke` ; `app.withGlobalTauri: true` est activé, `core:default`
couvre les commandes applicatives (hors ACL) et les événements.

## 11. Capabilities et CSP

`capabilities/default.json` — fenêtres `overlay`, `board`, `pairing` :

```json
["core:default", "fs:default", "fs:allow-read-text-file",
 "fs:allow-write-text-file",
 { "identifier": "fs:scope",
   "allow": ["$APPDATA/LiveChat/**", "$APPCONFIG/**/*", "$APPLOCALDATA/**/*"] }]
```

(valeur exacte validée au POC ; `fs:default` = dossiers app en lecture,
`fs:allow-read-text-file`/`fs:allow-write-text-file` = commandes explicites,
`fs:scope` = chemins additionnels, dont le fichier de config legacy).
Pas de `shell`/`opener` : le shell 8a n'ouvre pas de process externe.

CSP (`tauri.conf.json`) :

```text
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline';
img-src 'self' asset: http://asset.localhost data:; media-src 'self' blob:;
connect-src 'self' ipc: http://ipc.localhost;
frame-src https://platform.twitter.com; object-src 'none'
```

En 8b, `connect-src` et `media-src` devront être élargis avec `{API_URL}` et
`{WS_API_URL}` injectées depuis la config (cf. §8.2 d'`overlay-bridge.md`), les
URLs média restant verbatim. Origine WebView : `http://tauri.localhost` (pas de
`useHttpsScheme`), donc le serveur devra autoriser cette origine en CORS
(action Phase 3).

## 12. Variante bêta et packaging

- `tauri.beta.conf.json` : `productName = "LiveChat Next"`,
  `identifier = "com.livechat.next"`, version `3.0.0-beta.1`, endpoint updater
  `next`.
- Build bêta :
  `cargo tauri build --config tauri.beta.conf.json` (avec
  `$env:LIVECHAT_PRODUCT_NAME = 'LiveChat Next'` pour une clé Run distincte).
- Bundle stable : NSIS, `installMode: currentUser`,
  `webviewInstallMode.downloadBootstrapper`, icônes placeholder.
- Installateur NSIS vérifié au POC (2,0 Mo, démarrage release 436 ms) ; le
  binaire release 8a n'a pas été produit (hors critère 8a) mais la config est
  identique à celle du POC.

## 13. Écarts, risques et limites connues

1. `displayId` (identifiant Electron du moniteur) n'est pas résolu : la
   sélection utilise `displayKey`/`displayIndex`/primaire, comme décidé au POC.
2. `pairing_consume` renvoie une erreur explicite (« Phase 8b ») — l'UI
   d'appairage réelle n'est pas encore servie.
3. `meme_board_trigger/stop` : `socket_unavailable` jusqu'au client WS.
4. Updater : clé minisign placeholder + pas de check au démarrage.
5. Tray : pas de statut de connexion/version dans le menu (Phase 8b).
6. `serverUrl` non normalisé à l'écriture (parité legacy volontaire).
7. Autostart : collision potentielle de la valeur `Run` `livechat` avec
   l'Electron (voir §7).
8. Le shell 8a affiche `show()` l'overlay au démarrage (transparent) même avant
   le handshake renderer : conforme au comportement « overlay toujours prêt »,
   mais le renderer définitif devra gérer l'état vide.
9. Le smoke run a écrit `%APPDATA%\LiveChat\config.json.bak` (migration, §4.1).
10. `gen/schemas/` est généré par tauri-build et ignoré via
    `apps/lc-overlay/.gitignore`.
11. Limitation plateforme héritée du POC : le topmost ne couvre pas le vrai
    plein écran exclusif (comme Electron) ; repli documenté dans le plan.

## 14. Checklist Phase 8b

- [ ] Client WS `tokio-tungstenite 0.29` : connexion depuis `serverUrl` +
      `clientToken`, backoff, événements protocole v2 → `overlay://play/stop`.
- [ ] Host bridge : brancher `overlay_renderer_ready` (flusher le buffer),
      `overlay_report_*` → télémétrie/socket, `pairing_consume` → appairage HTTP
      réel + `normalizeServerUrl`.
- [ ] Renderer : copier `overlay/renderer/**` (HTML/CSS/JS existants) dans
      `web/overlay.html`, `web/board.html`, `web/pairing.html` et brancher
      `bridge.js` (script classique, `<head>`, avant les modules).
- [ ] Hotkeys memes actifs : `meme_board_trigger/stop` → socket, feedback
      `failedAccelerators` réel.
- [ ] Cibles `overlay://settings` après `overlay_renderer_ready` + à chaque
      changement de config (toggles tray, écran, volume).
- [ ] CSP élargie `{API_URL}`/`{WS_API_URL}` ; CORS serveur pour
      `http://tauri.localhost`.
- [ ] Updater : clé minisign réelle, `latest.json`, check au démarrage +
      lignes de statut tray.
- [ ] Single-instance : `--pairing`/`--from-autostart` et remontée au board.
- [ ] Coexistence bêta : build `tauri.beta.conf.json` + `LIVECHAT_PRODUCT_NAME`,
      test side-by-side avec l'Electron, migration/rollback `.bak`.

## 15. APIs exactes à réutiliser (Phase 8b et suivi)

Win32 (module `src/win32.rs`, seul `unsafe` autorisé) :

```rust
GetWindowLongPtrW(hwnd, GWL_EXSTYLE)                  // styles étendus
SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED
    | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW)
SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0,
    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)
```

Tauri 2.12 : `WebviewWindow::set_ignore_cursor_events(true)`,
`set_focusable(false)`, `set_always_on_top(true)`, `set_skip_taskbar(true)`,
`show()`, `hide()`, `set_size`/`set_position` (physiques), `hwnd()`,
`app.available_monitors()` / `primary_monitor()`, `app.get_webview_window`,
`AppHandle::tray_by_id`, `Emitter::emit_to(EventTarget::labeled(...))`,
`tauri_plugin_global_shortcut::{Builder, GlobalShortcutExt, Shortcut}`,
`tauri_plugin_autostart::{Builder, ManagerExt}`, `tauri_plugin_updater::UpdaterExt`.

Tout ajout devra conserver : `#![deny(unsafe_code)]` hors `win32`,
`clippy -D warnings`, `cargo fmt --check`, tests unitaires et
`cargo tauri build --debug --no-bundle`.
