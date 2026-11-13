# POC Tauri 2 — Overlay LiveChat (Phase 0) — Rapport GO / NO-GO

- **Auteur** : `poc-tauri` (Phase 0, rebuild Rust)
- **Date** : 2026-11-09 (23:00–23:45 Europe/Paris)
- **Tâche board** : `task-6`
- **Périmètre** : spike autonome `spikes/poc-tauri/` — **aucun fichier hors ce dossier modifié** (config/dépôt/`overlay/` Electron intacts, rien commité).
- **Question** : Tauri 2 peut-il remplacer Electron 39 pour l'overlay (transparence, click-through, topmost, focus, vidéo WebView2, tray, installateur, coexistence) ?

## Verdict global

**GO conditionnel** : les 6 propriétés critiques de l'overlay sont **prouvées sur la machine du Lead** (transparence réelle, click-through effectif, topmost réaffirmé, aucun vol de focus, vidéo H.264/AAC avec audio+seek, tray, single-instance, events `overlay://play`). Tauri 2.12.5 / Windows 11 / WebView2 157 fonctionne sans patch fragile ni hack hors de contrôle.

**2 réserves bloquantes pour le plan actuel** (à trancher par le Lead, pas des échecs du POC) :

1. **Budget RAM** : l'overlay Tauri seul = 47,6 Mo WS, mais l'arbre complet (WebView2 = 6 process) = **≈ 355 Mo WS / 205 Mo private** au repos, **≈ 583 Mo WS / 475 Mo private** pendant la vidéo. Le plan §8 cible **< 150 Mo** pour l'overlay → **budget à revoir** (WebView2 est incompressible) ou accepter un dépassement justifié, et/ou utiliser un processus WebView2 partagé avec le board (Tauri 2 supporte plusieurs webviews ; à valider en Phase 8).
2. **Topmost en jeu plein écran exclusif** non testable ici (VALORANT tournait en fenêtré, pas en exclusive fullscreen) ; le re-assert topmost toutes les 2 s a été mesuré contre une fenêtre topmost concurrente, voir nuance §Topmost. Le repli Electron reste obligatoire pour l'exclusive fullscreen.

**Recommandation Phase 8** : GO pour l'overlay Tauri en **installation side-by-side** pendant la beta (productName/identifier distincts de l'Electron), puis bascule sur `LiveChat` après validation. Détails §Coexistence.

## Matrice par critère

| Critère | Verdict | Preuve principale |
|---|---|---|
| Transparence réelle (alpha) | **GO** | PNG + pixels : fond rouge visible à travers l'overlay ; `background_1200_200 = #FF0000` ; marker/alpha composites ; pas de rectangle noir |
| Click-through souris | **GO** | `set_ignore_cursor_events(true)` → Ok ; style `WS_EX_TRANSPARENT|WS_EX_LAYERED` posé ; clic synthétique (950,750) **traverse** l'overlay et incrémente le bouton de la fenêtre de fond |
| Topmost (réaffirmation 2 s) | **GO avec réserve** | 13 ticks ; overlay reste au-dessus de Discord/red form/jeu fenêtré ; voir nuance « fenêtre topmost concurrente » |
| Pas de vol de focus | **GO** | `foregroundSelf=0` sur 13 ticks ; foreground reste Discord / fenêtre du fond ; `WS_EX_NOACTIVATE` posé |
| Fenêtre config (transparent, no frame, topmost, skip taskbar, focus=false, click-through) | **GO** | `ex_style = 0x080C01B8` = LAYERED+TRANSPARENT+NOACTIVATE+TOOLWINDOW+TOPMOST ; `normalStyle 0x14CB0000` (pas de CAPTION visible) |
| Vidéo WebView2 H.264/AAC | **GO** | MP4 788 Ko sur HTTP : durée 10,03 s, `readyState=4`, audio décodé (172 331 o), seek 6,72 s OK, `error=null` ; captures |
| Tray icon + menu | **GO** | `TrayIconBuilder` + `Menu` construits (`tray icon + menu built OK`) ; capture |
| Single instance | **GO** | 2ᵉ instance : exit 134–203 ms, code 0, événement forwardé à la 1ʳᵉ (`singleInstanceForwarded=1`) |
| Events `overlay://play` (émission Rust → réception JS) | **GO** | 3 noms testés, émission `result:"OK"` et réception JS pour `overlay://play` |
| Origine WebView / mixed content | **GO (http par défaut)** | `http://tauri.localhost` par défaut ; fetch `http://127.0.0.1:8765/health` = 200 ; variante `useHttpsScheme:true` = `https://tauri.localhost` et fetch 127.0.0.1 **toujours OK** (loopback exempté) — voir §Lead-2 |
| Chemins config/appdata | **GO avec écart plan** | `app_config_dir = C:\Users\SuperKnacky\AppData\Roaming\com.livechat` (≠ `%APPDATA%\LiveChat`) ; fs par défaut **refusé**, capability exacte fournie §Lead-3 |
| Démarrage | **GO** | Release : process → fenêtre **436 ms**, JS prêt **480 ms** (budget < 500 ms) |
| Taille binaire release | **GO** | `poc-tauri.exe` = **9 875 968 o = 9,42 Mo** (budget < 25 Mo) |
| Installateur NSIS | **GO** | `cargo tauri build` → `LiveChatPoc_0.1.0_x64-setup.exe` **1,99 Mo** (budget < 20 Mo) ; non installé |
| Coexistence Electron | **Risques identifiés** | Side-by-side possible (dossiers/clés distincts) mais collisions Apps&Features, raccourci menu Démarrer, Run key autostart, single-instance (identifier `com.livechat` partagé) → recommandation §Coexistence |

## Environnement mesuré

| Élément | Valeur |
|---|---|
| OS | Windows 11 (NT 10.0.26200.0), 16 cœurs logiques, 63 136 Mo RAM |
| Écrans | DISPLAY1 2560×1440 @ (0,0) scale 1.0 (AppliedDPI 96) + DISPLAY2 2560×1440 @ (-2560,0) |
| Rust / cargo | 1.99.1 stable-x86_64-pc-windows-msvc |
| Tauri | **2.12.5** (tauri-runtime-wry 2.12.3, wry 0.57.1, tao 0.37.2) |
| WebView2 Runtime | **157.0.4341.42** (UA Edge `Chrome/157.0.0.0 Edg/157.0.0.0`) |
| tauri-cli | 2.12.4 (installé via `cargo install tauri-cli --locked` — 1ʳᵉ fois ~6 min 38 s) |
| Crate `windows` (Win32) | 0.62.2 (même version que la dépendance interne de Tauri → `HWND` directement réutilisable) |

## Méthode et artefacts

Scénarios harness PowerShell 5.1 (`scripts/harness.ps1`) : fenêtre de fond rouge « red form » plein écran (WinForms, bouton jaune à 950,750), serveur HTTP loopback local (`scripts/http-server.ps1`, routes `/health`, `/nocs`, `/mov_bbb.mp4` avec Range), POC lancé avec `--mode solid|video --auto-close 120`, captures recadrées + échantillons pixels, mesures `Get-Process`, parsing des logs JSONL (`report/rust.log`, `report/js-events.log`), nettoyage vérifié (0 process, 0 port). Une étape `Ensure-RedTop`/`Ensure-OverlayAboveRed` re-force le fond avant chaque capture et attend les ticks de 2 s (machine utilisée en parallèle par le Lead — captures restées minimales/recadrées pour la vie privée).

**Runs de référence** : `report/harness-A.json` (mode solid, exit 0) et `report/harness-V.json` (mode vidéo, exit 0). Logs bruts : `report/rust.log`, `report/js-events.log` (run V), `report/variant-b-js-events.log`, `report/tauri-build.log`, `report/build-release.log`.

### Captures (toutes dans `spikes/poc-tauri/report/`)

- `shot-A-1-baseline-red.png` — fond rouge plein écran avant overlay.
- `shot-A-2-overlay-top.png` — overlay transparent au-dessus du jeu/rouge (démo visuelle).
- `shot-A-3-overlay-background-red.png` — zone de fond à travers l'overlay = rouge pur.
- `shot-A-5-topmost-focus.png`, `shot-A-6-topmost-competition-before.png`, `shot-A-7-topmost-competition-after.png` — topmost/focus.
- `shot-A-8-clickthrough-button.png` — bouton du fond cliqué **à travers** l'overlay.
- `shot-V-4-overlay-videoarea.png` — vidéo H.264 lisible dans l'overlay (fond rouge autour = transparence).
- `shot-V-9-video-frame.png` — frame vidéo (crop).
- `shot-A-10-tray-area.png`, `shot-V-10-tray-area.png` — zone tray après minimisation du fond.

## Résultats détaillés

### 1. Fenêtre overlay (config déclarative + runtime)

`tauri.conf.json` : fenêtre `overlay` 2560×1440 @ (0,0), `transparent:true`, `decorations:false`, `alwaysOnTop:true`, `skipTaskbar:true`, `focus:false`, `visible:false`, `resizable:false`, `shadow:false`, `url:"index.html"`, `build.frontendDist:"web"` (HTML/CSS/JS existants servis embarqués), `withGlobalTauri:true`, CSP `null` pendant le POC.

Au setup Rust : `set_ignore_cursor_events(true)`, `set_focusable(false)`, `set_always_on_top(true)`, `set_skip_taskbar(true)` → tous `Ok`. Puis bloc Win32 isolé (`unsafe` confiné à un module) : `SetWindowLongPtrW(GWL_EXSTYLE, ...|WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW|WS_EX_LAYERED|WS_EX_TRANSPARENT)` + `SetWindowPos(HWND_TOPMOST, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)`. Mesuré : `ex_style` avant show `0x080C0138` (déjà LAYERED/TRANSPARENT/NOACTIVATE/TOPMOST via tao) → après `0x080C01B8` (+TOOLWINDOW). Thread de fond : toutes les 2 s, re-`ensure_ex_style` + `set_always_on_top(true)` + `SetWindowPos(HWND_TOPMOST)`. 13 ticks mesurés sans erreur, `foregroundSelf=0`.

Limite couleur observée (non bloquante) : le marker CSS `rgb(0,255,0)` est capturé `#7AF940` et l'alpha bleu `rgba(0,0,255,.4)` → `#9F0E63` (au lieu de ~`#990066`). Une fenêtre WinForms native verte capturée au même moment donne `#00FF00` exact → l'écart vient du **compositing/color management du layer WebView2**, pas de la capture. À valider en Phase 8 sur les vrais assets (couleurs d'overlay peuvent être légèrement décalées selon le moniteur/HDR).

### 2. Transparence

Pixels des captures (recadrées) : `background_1200_200 = #FF0000`, `trayAreaProbe_2200_1390 = #FF0000`, `markerCenter = #7AF940`, `alphaCenter = #9F0E63`, `labelArea = #730000`. Le fond rouge est **visible à travers** la fenêtre (aucun rectangle noir/opaque) ; les éléments CSS se composent par-dessus. `Ensure-OverlayAboveRed` confirme l'overlay au-dessus du fond rouge.

### 3. Click-through

API : `set_ignore_cursor_events(true)` → `Ok` ; `GetWindowLongW(GWL_EXSTYLE)` = `0x080C01B8` → `WS_EX_TRANSPARENT (0x20)` + `WS_EX_LAYERED (0x80000)` présents. Pratique (run V) : `WindowFromPoint(950,750)` renvoie la red form (pas l'overlay), clic synthétique `mouse_event` down/up → `clickLandedOnRedFormButton=true`, compteur du bouton incrémenté, foreground inchangé (red form). → **les clics traversent l'overlay**.

### 4. Topmost / focus (nuance à connaître)

- Activation d'une autre fenêtre (Discord, foreground forcé) : l'overlay reste visible au-dessus (`markerPixel=#7AF940`), `overlayIsForeground=false`, `foregroundSelf=0` sur les 13 ticks → **pas de vol de focus, topmost tient**.
- Compétition avec une fenêtre **topmost concurrente** : le scénario a tenté de masquer l'overlay en remettant la red form au premier plan ; `ForceForeground` sur une fenêtre déjà topmost+foreground **ne la réordonne pas** (Windows ne réordonne pas une fenêtre déjà au sommet de sa bande) → `otherWindowHidOverlay=false`, l'overlay reste au-dessus. Le re-assert 2 s n'a donc pas été mis en échec, mais le test **n'a pas non plus prouvé** qu'il bat une vraie fenêtre topmost tierce qui se réafficherait au-dessus (ex. autres overlays type Medal/Discord). Risque résiduel à lever en Phase 8 avec un second topmost contrôlé (`SetWindowPos` par script) et un test en jeu borderless/exclusive fullscreen.
- Tests **non réalisés** : exclusive fullscreen natif (le jeu du Lead tournait en fenêtré), multi-écrans (DISPLAY2 non exercé), changement de résolution/HDR.

### 5. Vidéo WebView2 (H.264/AAC)

MP4 `mov_bbb.mp4` (788 493 o, 320×176) servi en HTTP avec Range : événements `loadedmetadata`, `playing`, seek à 6 s → `currentTime 6,72 s`, playback avancé de 6,76 s, `duration=10,026 s`, `readyState=4`, `paused=false`, `buffered=[0,10.026]`, `seekable=[0,10.026]`, **`webkitAudioDecodedByteCount=172 331`** (audio AAC décodé), `webkitVideoDecodedByteCount=306 205`, `error=null`. Capture `shot-V-4-overlay-videoarea.png` : frame vidéo lisible, zone hors vidéo laisse voir le fond rouge → transparence compatible vidéo.

Mémoire/CPU : voir tableau ci-dessous. Le décodage vidéo crée un process WebView2 supplémentaire et fait passer l'arbre de ≈355 Mo WS à ≈583 Mo WS (≈475 Mo private) — conséquence directe du moteur Chromium, pas de fuite (WS stable du process principal : 47,6 Mo constant sur 8 échantillons/500 ms).

| Métrique | Repos (solid) | Vidéo |
|---|---|---|
| Process Tauri (poc-tauri.exe) — WS / private | 47,6 Mo / 20,2 Mo | 47,6 Mo / 20,2 Mo |
| Threads / handles (process Tauri) | 30 / 334 | 30 / — |
| Arbre complet (Tauri + conhost + WebView2 ×6) — WS | **355,2 Mo** | **582,7 Mo** |
| Arbre complet — private | **205,5 Mo** | **475,4 Mo** |
| CPU idle (process Tauri, % de tous les cœurs) | ~0,02 % | ~0 % (échantillonné) |
| Démarrage release (process → fenêtre / JS prêt) | **436 ms / 480 ms** | — |
| Taille exe release (`target/release/poc-tauri.exe`) | **9 875 968 o = 9,42 Mo** | — |

Note : 47,6 Mo WS pour le process applicatif seul respecte le budget plan (< 30 Mo) ; c'est WebView2 (≈ 294 Mo WS au repos) qui dépasse le budget overlay < 150 Mo. Le plan doit être mis à jour (ou mutualiser la WebView2 avec le board).

### 6. Tray + single instance + autostart

- Tray : `TrayIconBuilder::new().menu(...).tooltip(...).icon(default_window_icon())` + `MenuItem::with_id("quit","Quitter le POC")` / `"show"` → construit sans erreur, capture zone tray ; le menu est natif (Windows 11).
- Single-instance : plugin 2.5.5 ; 2ᵉ lancement → forward argv à la 1ʳᵉ instance, exit 0 en 134–203 ms.
- Auto-start : plugin autostart 2.7.0 initialisé, `is_enabled=false` interrogé sans activer (pour ne pas toucher à l'autostart Electron existant) → **non testé** (enable/disable volontairement non exécuté) ; en Phase 8, app_name par défaut = productName, donc valeur Run key distincte de `livechat` si productName différent.

### 7. Installateur NSIS

`cargo build --release` : exit 0, 3 min 47 s (lto=true, codegen-units=1), exe **9 875 968 o = 9,42 Mo** (budget < 25 Mo → GO). `cargo tauri build` (tauri-cli 2.12.4) : rebuild release 2 min 11 s puis bundler NSIS 3.13 (téléchargé et hash-vérifié automatiquement) + `nsis_tauri_utils.dll` v0.5.3 → **installateur construit** :

- `target/release/bundle/nsis/LiveChatPoc_0.1.0_x64-setup.exe` = **2 083 497 o ≈ 1,99 Mo** (budget < 20 Mo → GO)
- `target/release/poc-tauri.exe` après packaging (patché infos de bundle) = 9 875 456 o ≈ 9,42 Mo
- log complet : `report/tauri-build.log` (exit 0, 140 s).

L'installateur n'a **pas été exécuté/installé** (aucune modification du système au-delà des fichiers de build) ; la désinstallation NSIS est donc à valider en Phase 8 (VM/profil jetable).

## Réponses aux 3 ajouts du Lead (m00048)

### Lead-1 — Nom d'événement `overlay://play` (émission Rust → réception JS)

**Accepté sans erreur.** Rust : `app_handle.emit("overlay://play", json!(...))` → log `emit event {"name":"overlay://play","result":"OK"}` pour les 3 noms testés (`overlay://play`, `overlay:play`, `play`). JS : `listen('overlay://play', cb)` → `"ok"`, et les 3 réceptions ont été enregistrées (`event_received`, payload `source:"rust-emitter"`). Aucun `illegal event name`. **Décision Phase 8** : garder `overlay://play` fonctionne, mais préférer `overlay:play` (même validité, pas de `//` ambigu dans les logs/serialisation) — si le protocole v2 impose `overlay://play`, il est prouvé valide.

### Lead-2 — Origine WebView et mixed content

| Config | `window.location.origin` | `window.location.href` | fetch `http://127.0.0.1:8765/health` | fetch `/nocs` (sans CORS) |
|---|---|---|---|---|
| **Défaut (pas de `useHttpsScheme`)** — config finale | `http://tauri.localhost` | `http://tauri.localhost/` | **200** `{"ok":true,...}` | `TypeError: Failed to fetch` |
| Variante `"useHttpsScheme": true` | `https://tauri.localhost` | `https://tauri.localhost/` | **200** (loopback `127.0.0.1` traité comme potentiellement de confiance → non bloqué) | `TypeError: Failed to fetch` |

`secureContext=true`, `cookieEnabled=true`, dpr=1 quel que soit le schéma. Le CORS est bien appliqué (route sans `Access-Control-Allow-Origin` → refusée). **Mixed content** : en `useHttpsScheme:true`, un `fetch` vers une IP LAN en `http://` serait bloqué (le loopback est exempté) ; le serveur du plan étant local, les deux configs fonctionnent pour la vidéo et le WS `ws://127.0.0.1`. Le plan doit utiliser **explicitement** `http://tauri.localhost` par défaut (ou `useHttpsScheme:false`), car un accès au serveur via IP LAN impose HTTPS.

### Lead-3 — Chemins config/appdata et permissions fs

Chemins réels pour `identifier: "com.livechat"` :

| API | Valeur réelle |
|---|---|
| `app.path().app_config_dir()` | `C:\Users\SuperKnacky\AppData\Roaming\com.livechat` |
| `app.path().app_data_dir()` | idem (Roaming\com.livechat) |
| `app.path().app_local_data_dir()` | `C:\Users\SuperKnacky\AppData\Local\com.livechat` |
| `app.path().app_log_dir()` | `...\Local\com.livechat\logs` |
| `%APPDATA%\LiveChat\config.json` (Electron) | existe (config Electron actuelle) |
| `%LOCALAPPDATA%\Programs\livechat` (Electron) | existe |

→ **Écart avec le plan** (§6 : config `%APPDATA%\LiveChat\config.json`) : par défaut Tauri utilise `%APPDATA%\com.livechat`. Phase 8 doit soit **migrer** (lire l'ancien chemin via `%APPDATA%\LiveChat` puis écrire dans `app_config_dir`), soit forcer un chemin custom (`app.path().resolve("C:\\...", BaseDirectory::...)` / `dirs`), et prévoir la migration `.bak` du plan.

Permissions fs **par défaut refusées** (capability `core:default` seule) :
`fs.read_text_file not allowed. Permissions associated with this command: fs:allow-app-read, ... fs:default, ...` (idem write). Capability **exacte nécessaire** (JSON `capabilities/default.json`, validée par un run build+runtime) :

```json
{
  "identifier": "default",
  "windows": ["overlay"],
  "permissions": [
    "core:default",
    "fs:default",
    "fs:allow-read-text-file",
    "fs:allow-write-text-file",
    { "identifier": "fs:scope", "allow": ["$APPCONFIG/**/*", "$APPLOCALDATA/**/*", "$APPDATA/**/*"] }
  ]
}
```

Avec cette capability : lecture `%APPDATA%\com.livechat\config.json` **autorisée** (échec = fichier absent, pas permission denied) et **écriture autorisée** (`fsWrite:"OK"`, fichier créé) — le scope `$APPCONFIG/**/*` suffit pour config/logs du plan. Lecture du chemin Electron `%APPDATA%\LiveChat\**` nécessiterait `$APPDATA/LiveChat/**/*` dans `fs:scope`. Equivalent TOML possible :
`[permissions] identifier = "fs:scope"` + `allow = ["$APPCONFIG/**/*"]`. Préférer **une permission dédiée** `fs:allow-read-text-file`/`fs:allow-write-text-file` plutôt que `fs:allow-app-read/write-*` (moindre privilège) ; ne pas utiliser `fs:allow-*` globales.

## Coexistence avec l'Electron installé (NE RIEN DÉSINSTALLÉ)

État existant : `electron-builder` HKCU `...\Uninstall\ec5c36a6-04f8-5099-a257-77071e875a50`, DisplayName **`LiveChat 2.0.1`**, Publisher `Babz`, UninstallString `"...\AppData\Local\Programs\livechat\Uninstall LiveChat.exe" /currentuser`, install dir `%LOCALAPPDATA%\Programs\livechat`, Run key HKCU `livechat`, aucune entrée HKLM.

Comportement Tauri (tauri-bundler 2.10.4, NSIS `installer.nsi`) : clé de désinstallation `Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}`, install dir currentUser `$LOCALAPPDATA\${PRODUCTNAME}`, raccourci Démarrer `${PRODUCTNAME}.lnk`, `MAINBINARYNAME` = nom du binaire cargo, publisher = `settings.publisher()`.

Conséquences :
- **Upgrade in-place Electron → Tauri impossible proprement** : clés/dossier/shortcut différents (GUID vs `LiveChat`), NSIS Tauri installerait une 2ᵉ app ; l'Electron resterait installé (désinstallation manuelle requise).
- **Side-by-side possible** : dossier `%LOCALAPPDATA%\LiveChat` ≠ `%LOCALAPPDATA%\Programs\livechat` ; clés distinctes. Collisions à éviter : **même productName** → entrées Apps & Features ambiguës + raccourci menu Démarrer portant le même nom ; **même identifier `com.livechat`** → même dossier de config, même single-instance mutex (2ᵉ instance forwardée vers l'autre app), même autostart app_name.
- **Recommandation Phase 8** : beta en side-by-side avec `productName: "LiveChat Next"` (ou `LiveChat (Tauri)`) et `identifier: "com.livechat.next"` → aucun conflit (config, Run key, mutex, Apps&Features). Puis bascule finalisée : désinstaller LiveChat 2.0.1 (NSIS `/currentuser`, ou script plan), installer LiveChat Tauri avec productName/identifier définitifs, **migrer `%APPDATA%\LiveChat\config.json`** vers `app_config_dir()` avec `.bak`, prendre la main sur la Run key `livechat` (supprimer l'ancienne valeur ou réutiliser le même nom pour une transition transparente), tester la désinstallation propre.

## APIs exactes à réutiliser (Phase 8)

- Fenêtre : `WebviewWindow::set_ignore_cursor_events(true)`, `set_focusable(false)`, `set_always_on_top(true)`, `set_skip_taskbar(true)`, `show()` (fenêtre créée `visible:false` pour éviter tout flash) ; `win.hwnd() -> Result<HWND>` (crate `windows` 0.62 aligné avec Tauri).
- Bloc Win32 isolé (`unsafe`, un seul module) : `GetWindowLongPtrW(hwnd, GWL_EXSTYLE)`, `SetWindowLongPtrW(...)`, `SetWindowPos(hwnd, HWND_TOPMOST, 0,0,0,0, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)` — constantes `windows::Win32::UI::WindowsAndMessaging` ; réaffirmation toutes les 2 s dans un thread dédié (`std::thread::spawn` + `app_handle.clone()`), + `set_always_on_top(true)`.
- Config : `app.path().app_config_dir()/app_data_dir()/app_local_data_dir()/app_log_dir()` ; `tauri.conf.json` : `build.frontendDist:"web"`, fenêtre avec `transparent/decorations/alwaysOnTop/skipTaskbar/focus/visible/shadow` ; `app.withGlobalTauri:true` (ou bundler JS) ; CSP à durcir en Phase 8 (POC : `null`).
- Events : Rust `AppHandle::emit(name, payload)` ; JS `listen(name, cb)` — `overlay://play` validé de bout en bout ; payload JSON sérialisé serde.
- Tray : `TrayIconBuilder::new()`, `MenuItem::with_id`, `Menu::with_items`, `.on_menu_event(|app, event| match event.id().as_ref() { "quit" => app.exit(0), ... })`, `app.default_window_icon()`; menu natif.
- Single instance : `tauri_plugin_single_instance::init(|app, argv, cwd| {...})` **en premier** dans `Builder`.
- Autostart : `tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--from-autostart"]))` puis `app.autolaunch().enable()/disable()/is_enabled()` ; app_name = productName (à fixer distinct en beta).
- fs : capability JSON ci-dessus (`fs:default` + 2 commandes + `fs:scope`) ; résoudre les chemins via `AppHandle::path()`.
- Installer : `cargo build --release` puis `cargo tauri build` (tauri-cli 2.12.4 ; NSIS 3.13 auto-téléchargé) ; `bundle.targets:["nsis"]`, `bundle.windows.nsis.installMode:"currentUser"`.

## Limites du POC / risques résiduels

1. **RAM WebView2** : dépassement du budget < 150 Mo du plan (≈ 355 Mo WS repos) — décision Lead requise.
2. **Topmost en exclusive fullscreen** non testé (impossible sans lancer un jeu en fullscreen natif ici) ; Windows ne garantit pas topmost au-dessus d'une surface exclusive → repli Electron / hook de jeu à conserver pour ce cas.
3. Test de compétition topmost partiel (fenêtre déjà topmost non réordonnée) ; ajouter en Phase 8 un test avec un vrai second topmost contrôlé et un jeu borderless.
4. Couleur/compositing WebView2 (`#7AF940` au lieu de `#00FF00`) : à revalider sur les assets réels (HDR/moniteur).
5. `useHttpsScheme` non retenu ; accès serveur via IP LAN en `http://` bloquerait (loopback OK).
6. Tray : construction et capture OK mais **interaction menu non cliquée** (pas de clic automatisé dans la zone tray, risque UI Automation) — à tester en Phase 8.
7. Auto-start : plugin initialisé mais `enable()` non exécuté (protection de la config Electron du Lead) → à tester dans un profil vierge.
8. Installateur NSIS : construit mais **non installé/désinstallé** ; à valider sur VM/clean profile.
9. Machine utilisée en parallèle (jeu/overlays du Lead) : captures et timings ponctuellement perturbés ; harness durci (re-forçage du fond avant chaque capture) pour garantir la validité des pixels.
10. Un seul écran exercé (DISPLAY1) ; pas de test multi-écrans/DPI scaling (scale 100 % ici).
11. POC en debug pour les runs ; les chiffres release n'ont été pris que pour démarrage + taille binaire + confirmations (les deux profils donnent les mêmes bits de style/mêmes events).

## Fichiers du spike

- `spikes/poc-tauri/Cargo.toml` (`[workspace]` vide, tauri 2 `tray-icon`, windows 0.62, plugins single-instance/autostart/fs, release lto/codegen-units=1/strip)
- `spikes/poc-tauri/build.rs` (`tauri_build::build()`), `tauri.conf.json`, `capabilities/default.json`
- `spikes/poc-tauri/src/main.rs` (~560 lignes : setup fenêtre, Win32 isolé, thread topmost, emit test, tray, single-instance, autostart, commands invoke, logs JSONL, args `--mode solid|video --auto-close N`)
- `spikes/poc-tauri/web/index.html` (overlay de test : marker/alpha/label/status/vidéo, listener events, fetch CORS, fs, metrics vidéo, report vers Rust)
- `spikes/poc-tauri/scripts/{http-server,red-form,harness}.ps1` (PowerShell 5.1)
- `spikes/poc-tauri/assets/mov_bbb.mp4` (788 493 o, échantillon public w3schools)
- `spikes/poc-tauri/report/` : captures ci-dessus + `harness-A.json`, `harness-V.json`, `rust.log`, `js-events.log`, `variant-b-js-events.log`, `variant-b-rust.log`, `tauri.conf.A.default.json`, `capabilities.A.default.json`, `build-debug.log`, `build-release.log`, `tauri-build.log`, logs server/red-form.
- `spikes/poc-tauri/target/` : builds debug/release (+ `bundle/nsis` si présent).

**Nettoyage** : aucun process `poc-tauri`, aucune 2ᵉ instance, aucun serveur HTTP résiduel, window red form fermée (vérifié par harness : `leftoversOk=true`, 0 process, 0 listener port). Aucune modification hors `spikes/poc-tauri/` ; aucun commit.
