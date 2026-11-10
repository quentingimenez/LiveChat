# Pont renderer ↔ main de l'overlay — matrice IPC et contrat Tauri 2

> Statut : **Phase 0** (document de référence, aucun code Rust de production).
> Portée : l'app Electron `overlay/` est remplacée par une app **Tauri 2** ; le renderer HTML/CSS/JS est conservé tel quel
> et reçoit un bridge qui réexpose `window.livechatOverlay`.
> Livrables associés : `apps/lc-overlay/web/bridge.js`, `apps/lc-overlay/dev/README.md`,
> `apps/lc-overlay/dev/bridge-test.html` (harnais de dev, hors bundle depuis P1-11, cf. §12.6).
> Références de plan : `docs/rust/migration-plan.md` (§4 protocole v2 l.63-72, §6 overlay Tauri l.82-88, risques l.29) et
> `overlay/docs/architecture.md:152-170` (compatibilité « noms des channels / payloads / API preload inchangés »).
> Toutes les références `fichier:ligne` renvoient au dépôt `C:\Users\SuperKnacky\code\LiveChat`, branche `feat/rust-rebuild`.
> Décisions Lead du 2026-11-09 intégrées : §8 (bufferisation play/stop, CSP, origine WebView/CORS, URLs média v2, noms
> d'événements, chemin de config).

## 1. Vue d'ensemble

Chaîne actuelle (Electron) :

```
HTML (renderer/{overlay,board,pairing}.html)
  └─ module ESM ../dist/renderer/<page>/index.js   (compile overlay/src/renderer/**)
       └─ legacy-loader.ts:1-8 charge ./<page>.js  (script classique, relatif à l'URL de la page)
            └─ window.livechatOverlay              (contextBridge, overlay/src/preload/index.ts:56)
                 └─ ipcRenderer invoke/send/on     (canaux overlay/src/shared/ipc.ts:19-33)
                      └─ ipcMain handlers          (overlay/src/main/ipc/*)
                           └─ services main (config, socket, tray, fenêtres, mémoires)
```

Chaîne cible (Tauri 2, Phase 8) :

```
HTML + bridge.js (script classique, apps/lc-overlay/web/bridge.js)
  └─ window.livechatOverlay (mêmes 13 méthodes, mêmes payloads)
       └─ invoke(<commande Tauri>, { payload }) / listen("overlay://play|stop|settings")
            └─ commandes/événements Rust (apps/lc-overlay/src/**)
                 └─ mêmes services métier portés en Rust
```

Le renderer **ne change pas** : c'est le bridge qui absorbe le changement de transport. Le tableau de correspondance des
noms se trouve en §6.

## 2. Matrice des 13 méthodes de `window.livechatOverlay`

Sources : API `overlay/src/preload/index.ts:40-54`, types `overlay/src/types/livechat-overlay.d.ts:19-39`, canaux
`overlay/src/shared/ipc.ts:19-33`, handlers `overlay/src/main/ipc/register-overlay-ipc-handlers.ts:42-74` et
`overlay/src/main/ipc/register-pairing-ipc-handler.ts:36-38`.

| # | Méthode JS | Signature exacte (`livechat-overlay.d.ts:19-33`) | Canal IPC | Handler main | Payload → retour | Sémantique / ordre d'appel |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | `getConfig` | `getConfig(): Promise<OverlayConfig>` | `overlay:get-config` | `ipcMain.handle` → `overlay-config-ipc-service.getConfig` (`register-overlay-ipc-handlers.ts:42-44`, service `overlay-config-ipc-service.ts:25-47`) | `undefined` → `OverlayConfig` (projection 8 champs) | Lecture de la config au démarrage ; **1er appel** de chaque page, `await` avant d'installer les listeners. |
| 2 | `rendererReady` | `rendererReady(): void` | `overlay:renderer-ready` | `ipcMain.on` → `sendOverlaySettingsToRenderer()` (`:60-62`) | `undefined` | Handshake : à appeler **en dernier**, après `getConfig` + les 3 abonnements ; le main renvoie alors `overlay:settings`. |
| 3 | `onPlay` | `onPlay(callback: (payload: OverlayPlayPayload) => void): () => void` | `overlay:play` | émis par `runtime-window-socket-service.ts:130-137` (envoi l.136) depuis l'événement socket `overlay:play` | `OverlayPlayPayload` → fonction de désabonnement | Abonnement aux ordres de lecture ; installé après `getConfig`, avant `rendererReady`. |
| 4 | `onStop` | `onStop(callback: (payload: OverlayStopPayload) => void): () => void` | `overlay:stop` | `runtime-window-socket-service.ts:138-145` (envoi l.144) depuis le socket `overlay:stop` | `OverlayStopPayload` → désabonnement | Abonnement aux stops (dont stop manuel) ; même ordre que `onPlay`. |
| 5 | `onSettings` | `onSettings(callback: (payload: OverlaySettingsPayload) => void): () => void` | `overlay:settings` | `ipcMain.on overlay:renderer-ready` (`:60-62`), ready-to-show overlay (`runtime-window-socket-service.ts:86-88`), board (`:99-101`), reboot d'appairage (`pairing-consume-service.ts:65-134`) | `OverlaySettingsPayload` (= `OverlayConfig`) → désabonnement | Abonnement aux changements de config ; installé avant `rendererReady`. |
| 6 | `reportError` | `reportError(payload: OverlayErrorPayload): void` | `overlay:error` | `ipcMain.on` → `overlay-playback-forwarding-service.handleOverlayError` (`:64-66`, service `:38-49`) | `{ jobId, code, message }` → void | Télémétrie d'erreur de lecture ; ignorée en invité et si socket absent/hors ligne. |
| 7 | `reportPlaybackState` | `reportPlaybackState(payload: OverlayPlaybackStatePayload): void` | `overlay:playback-state` | `ipcMain.on` → `handleOverlayPlaybackState` (`:68-70`, service `:51-78`) | `{ jobId, state, remainingMs }` → void | Progression de lecture ; bufferisée si socket hors ligne (`setPendingPlaybackStatePayload`), rejouée à la reconnexion (`socket-service.ts:105-124`). |
| 8 | `reportPlaybackStop` | `reportPlaybackStop(payload: OverlayPlaybackStopPayload): void` | `overlay:playback-stop` | `ipcMain.on` → `handleOverlayPlaybackStop` (`:72-74`, service `:80-96`) | `{ jobId }` → void | Fin de lecture ; `jobId` vide normalisé en `'unknown'` (`overlay-playback-forwarding-service.ts:24-31`) ; bufferisée hors ligne. |
| 9 | `consumePairing` | `consumePairing(payload: PairingConsumeRequest): Promise<PairingConsumeResponse>` | `pairing:consume` | `ipcMain.handle` → `pairing-consume-service.consumePairing` (`register-pairing-ipc-handler.ts:36-38`, service `pairing-consume-service.ts:65-134`) | `{ serverUrl, code, deviceName? }` → `{ ok: boolean }` | Appel HTTP POST `{serverUrl}/overlay/pair/consume`, persiste la config, ouvre l'overlay, ferme la fenêtre d'appairage. |
| 10 | `getMemeBindings` | `getMemeBindings(): Promise<MemeBoardBindingsResponse>` | `meme-board:get-bindings` | `ipcMain.handle` → `meme-board-ipc-service.getBindings` (`:46-48`, service `:29-34`) | `undefined` → `{ bindings }` | Bindings raccourci → itemId, normalisés. |
| 11 | `setMemeBindings` | `setMemeBindings(payload: MemeBoardSetBindingsRequest): Promise<MemeBoardSetBindingsResponse>` | `meme-board:set-bindings` | `ipcMain.handle` → `meme-board-ipc-service.setBindings` (`:50-52`, service `:36-53`) | `{ bindings }` → `{ ok, bindings, failedAccelerators }` | Normalise, enregistre les accélérateurs globaux en mode strict (rollback si échec), persiste. |
| 12 | `triggerMeme` | `triggerMeme(payload: MemeBoardTriggerRequest): Promise<MemeBoardActionResponse>` | `meme-board:trigger` | `ipcMain.handle` → `meme-board-ipc-service.trigger` (`:54-56`, service `:55-58`) | `{ itemId?, trigger?: 'ui' \| 'shortcut' }` → `{ ok, reason? }` | Émet `overlay:meme-trigger` sur le socket (défaut `trigger: 'shortcut'`). |
| 13 | `stopMemePlayback` | `stopMemePlayback(): Promise<MemeBoardActionResponse>` | `meme-board:stop` | `ipcMain.handle` → `meme-board-ipc-service.stop` (`:58`, service `:60-62`) | `undefined` → `{ ok, reason? }` | Émet `overlay:stop { jobId: 'manual-stop' }` sur le socket (stop manuel). |

Remarques transverses :

- Les 6 méthodes `invoke` acceptent/renvoient des objets JSON dont les clés sont en **camelCase** (mêmes payloads qu'aujourd'hui).
- Les 4 méthodes `send` sont fire-and-forget : le renderer legacy **ignore leur retour**. Le bridge renvoie malgré tout une
  `Promise` toujours résolue (voir §7, risque « rejets non gérés »).
- Deux méthodes ne sont utilisées que par certaines pages (voir §3) mais les 13 sont toujours exposées.
- `overlay/src/preload/bridge-api.ts` est une variante mortes des mêmes helpers (`invokeTyped` l.14, `sendTyped` l.21,
  `onTyped` l.28, `preloadChannels` l.43) : **non utilisée** par `index.ts`, aucun canal propre, aucune action requise.

### 2.1 Contrat de type des payloads

Types exacts (à porter en `serde` avec `rename_all = "camelCase"`) :

- `OverlayConfig` — `overlay/src/shared/overlay-config.ts:3-12` : `{ serverUrl, clientToken, guildId, clientId: string|null; guestMode, showText: boolean; volume: number; memeBindings: Record<string,string> }`.
  Projection renvoyée au renderer (8 champs) : `overlay-config-ipc-service.ts:25-47`.
- `OverlaySettingsPayload = OverlayConfig` — `overlay-config.ts:14`.
- `OverlayPlaybackStatePayload` — `:16-20` : `{ jobId: string; state: string; remainingMs: number|null }`.
- `OverlayPlaybackStopPayload` — `:22-24` : `{ jobId: string }`.
- `OverlayErrorPayload` — `:26-30` : `{ jobId: string; code: string; message: string }`.
- `OverlayAuthorPayload` — `:32-36` ; `OverlayTextPayload` — `:38-41`.
- `OverlayMediaPayload` — `:43-50` : `{ kind: 'image'|'audio'|'video'; url?; sourceUrl?; mime?; startOffsetSec?; isVertical? }`.
- `OverlayPlayPayload` — `:52-59` : `{ jobId?; durationSec?; media?; author?; text? }` + index signature (champs libres tolérés).
- `OverlayStopPayload` — `:61-63` : `{ jobId? }`.
- `MemeBoardBindingsResponse` — `overlay/src/shared/meme-board.ts:3-5` ; `MemeBoardSetBindingsRequest` — `:7-9` ;
  `MemeBoardSetBindingsResponse` — `:11-15` ; `MemeBoardTriggerRequest` — `:17-20` ; `MemeBoardActionResponse` — `:22-25`.
- `PairingConsumeRequest` — `overlay/src/shared/pairing.ts:1-5` ; `PairingConsumeResponse` — `:7-9`.

### 2.2 Ordre d'appel réel (bootstrap des pages)

**Overlay** (`overlay/src/renderer/overlay/legacy-bootstrap-utils.ts:56-73`, page `overlay/renderer/overlay.js:104-136`) :

1. `await getConfig()` (`legacy-bootstrap-utils.ts:57`, appel `overlay.js:106`) ;
2. `onPlay(...)` (`:63`, `overlay.js:112`) ;
3. `onStop(...)` (`:64`, `overlay.js:115`) ;
4. `onSettings(...)` (`:65`, `overlay.js:118`) ;
5. `ensureTwitterWidgets(...)` (`:70`, échec avalé) ;
6. `rendererReady()` (`:72`, `overlay.js:129`) — dernier, par contrat.

**Board** (`overlay/src/renderer/board/legacy-bootstrap-utils.ts:54-74`, page `overlay/renderer/board.js:182-225`) :

1. `await getConfig()` (`:56`, `board.js:184`) ;
2. si non appairé (`serverUrl`/`clientToken`/`guildId` absents), message + désactivation + `return` (`:59-63`) ;
3. `await getMemeBindings()` (`:65`, `board.js:185`) ;
4. `onSettings(...)` (`:68`, `board.js:186-188`) ;
5. observer + premier chargement des items (`:72-73`).
   ⚠️ Le board **n'appelle pas `rendererReady`** : ses settings arrivent via `onBoardReadyToShow`
   (`runtime-window-socket-service.ts:99-101`). Conséquence (décision §8.1) : **aucun buffer d'événements n'est
   nécessaire côté board**, il n'a pas de handshake à attendre.

**Pairing** (`overlay/src/renderer/pairing/pairing-controller.ts`, page `overlay/renderer/pairing.js` **non chargée** —
fichier mort, voir §4 et décision §8.7) :

1. `await getConfig()` (`pairing-controller.ts:17`) ;
2. `await consumePairing({ serverUrl, code })` au submit (`:25-28`, trim l.26-27) ;
3. statuts UI busy/succès/erreur (`:31-46`, statuts l.36/40/42, réactivation du bouton l.43), erreur affichée via
   `error.message` (l.44).
   Le pairing n'utilise **que** `getConfig` + `consumePairing` : 2 méthodes sur 13.

## 3. Inventaire exhaustif des canaux IPC

Source unique : `overlay/src/shared/ipc.ts:19-33`. Aucun `ipcRenderer` n'existe en dehors de
`overlay/src/preload/index.ts` (grep exhaustif : seuls `index.ts` et la variante morte `bridge-api.ts` importent
`ipcRenderer`). Aucun `window.require`, aucun Node dans le renderer.

### 3.1 renderer → main : `invoke` (6 canaux)

| Constante `ipc.ts` | Chaîne | Payload requête | Réponse | Handler | Utilisé par |
| --- | --- | --- | --- | --- | --- |
| `overlayGetConfig` | `overlay:get-config` | `undefined` | `OverlayConfig` | `register-overlay-ipc-handlers.ts:42-44` | overlay, board, pairing |
| `pairingConsume` | `pairing:consume` | `PairingConsumeRequest` | `PairingConsumeResponse` | `register-pairing-ipc-handler.ts:36-38` | pairing |
| `memeBoardGetBindings` | `meme-board:get-bindings` | `undefined` | `MemeBoardBindingsResponse` | `register-overlay-ipc-handlers.ts:46-48` | board |
| `memeBoardSetBindings` | `meme-board:set-bindings` | `MemeBoardSetBindingsRequest` | `MemeBoardSetBindingsResponse` | `:50-52` | board |
| `memeBoardTrigger` | `meme-board:trigger` | `MemeBoardTriggerRequest` | `MemeBoardActionResponse` | `:54-56` | board |
| `memeBoardStop` | `meme-board:stop` | `undefined` | `MemeBoardActionResponse` | `:58` | board |

Types de maps : `IpcInvokeRequestMap` `ipc.ts:35-42`, `IpcInvokeResponseMap` `:44-51`.

### 3.2 renderer → main : `send` (4 canaux)

| Constante | Chaîne | Payload | Handler | Utilisé par |
| --- | --- | --- | --- | --- |
| `overlayRendererReady` | `overlay:renderer-ready` | `undefined` | `register-overlay-ipc-handlers.ts:60-62` | overlay |
| `overlayError` | `overlay:error` | `OverlayErrorPayload` | `:64-66` | overlay |
| `overlayPlaybackState` | `overlay:playback-state` | `OverlayPlaybackStatePayload` | `:68-70` | overlay |
| `overlayPlaybackStop` | `overlay:playback-stop` | `OverlayPlaybackStopPayload` | `:72-74` | overlay |

Map : `IpcSendPayloadMap` `ipc.ts:53-58`.

### 3.3 main → renderer : `webContents.send` (3 canaux, exhaustif)

| Constante | Chaîne | Payload | Émis par (preuve) | Fenêtre(s) cible(s) |
| --- | --- | --- | --- | --- |
| `overlayPlay` | `overlay:play` | `OverlayPlayPayload` | `runtime-window-socket-service.ts:130-137` (envoi l.136) | overlay uniquement |
| `overlayStop` | `overlay:stop` | `OverlayStopPayload` | `runtime-window-socket-service.ts:138-145` (envoi l.144) | overlay uniquement |
| `overlaySettings` | `overlay:settings` | `OverlaySettingsPayload` | `renderer-settings-service.ts:52-56` (envoi l.33), cibles l.54 (overlay) et l.55 (board) | overlay **et** board, jamais pairing |

Déclencheurs de `overlay:settings` : `renderer-ready` (`register-overlay-ipc-handlers.ts:60-62`), `ready-to-show` de
l'overlay (`runtime-window-socket-service.ts:86-88`), `ready-to-show` du board (`:99-101`), et fin d'appairage
(`pairing-consume-service.ts:65-134`). Map : `IpcEventPayloadMap` `ipc.ts:60-64`.

Grep exhaustif : ce sont les **seuls** `webContents.send` du main.

### 3.4 Événements socket (hors IPC Electron, à ne pas confondre)

`overlay/src/shared/socket-events.ts:1-9` définit 7 événements de transport socket.io, qui ne passent **pas** par IPC :
`overlay:play`, `overlay:stop`, `overlay:heartbeat`, `overlay:error`, `overlay:playback-state`,
`overlay:meme-trigger`, `overlay:peers`. Ils sont émis/consommés par `socket-service.ts` (heartbeat 15 s `:83-89`,
flush des buffers `:105-124`, connexion `:126-166`), `overlay-playback-forwarding-service.ts`, `meme-bindings-service.ts`
et `overlay-peers-service.ts` (tray). Le bridge ne les voit jamais.

### 3.5 Surface publique du preload

`overlay/src/preload/index.ts` : helpers `invokeTyped` l.14-19, `sendTyped` l.21-23, `onTyped` l.25-38 (retourne une
fonction de désabonnement), objet `api` l.40-54, `contextBridge.exposeInMainWorld('livechatOverlay', api)` l.56.
L'ordre d'exposition et les noms sont testés par `overlay/tests/main/preload-runtime.test.js:30,69`
(`exposedName === 'livechatOverlay'`, ordre des invoke/send, comptage des listeners).

### 3.6 Matrice page ↔ méthode

| Méthode | overlay | board | pairing |
| --- | :-: | :-: | :-: |
| `getConfig` | ✅ | ✅ | ✅ |
| `rendererReady` | ✅ | — | — |
| `onPlay` / `onStop` | ✅ | — | — |
| `onSettings` | ✅ | ✅ | — |
| `reportError` / `reportPlaybackState` / `reportPlaybackStop` | ✅ | — | — |
| `consumePairing` | — | — | ✅ |
| `getMemeBindings` / `setMemeBindings` / `triggerMeme` / `stopMemePlayback` | — | ✅ | — |

## 4. Assets et fichiers statiques des 3 pages

Le renderer est conservé **tel quel** ; les chemins relatifs ci-dessous devront être préservés dans l'arborescence servie
par Tauri (sinon les `<link>`/`<script>`/`@import` cassent).

| Page | HTML | CSS | Module ESM chargé | Script legacy | Particularités |
| --- | --- | --- | --- | --- | --- |
| Overlay | `overlay/renderer/overlay.html` (18 l.) | `<link href="./overlay.css">` l.7 ; `overlay.css` importe `styles/overlay/{base,tweet-card,meta}.css` | `../dist/renderer/overlay/index.js` l.16 | `overlay/renderer/overlay.js` (137 l.) via `legacy-loader.ts:1-8` | DOM requis : `#overlay-root` l.10, `#media-layer` l.11, `#countdown-layer` l.12, `#text-layer` l.13. Transparent, click-through, toujours au-dessus. |
| Board | `overlay/renderer/board.html` (135 l.) | `<link href="./board.css">` l.7 ; `board.css` importe `styles/board/{base,list,preview,dialogs,responsive}.css` | `../dist/renderer/board/index.js` l.133 | `overlay/renderer/board.js` (230 l.) via `legacy-loader.ts:1-8` | IDs l.17-128 (search-form l.17, search-input l.18, stop-playback-button l.21, refresh-button l.22, add-link-button l.23, status l.27, count-pill l.33, items-list l.35, selected-title l.42, selected-meta l.44, preview-stage l.46, capture-overlay l.53, capture-current l.57, capture-cancel l.59, capture-save l.60, add-overlay l.65, add-form l.66, add-url-input l.69, add-title-input l.71, add-message-input l.79, add-refresh-input l.86, add-cancel l.90, add-submit l.91, rename-overlay l.96, rename-form l.97, rename-input l.101, rename-message-input l.109, rename-cancel l.116, rename-submit l.117, delete-overlay l.122, delete-message l.125, delete-cancel l.127, delete-confirm l.128). |
| Pairing | `overlay/renderer/pairing.html` (125 l.) | **Aucun CSS externe** : `<style>` inline l.7-97 | `../dist/renderer/pairing/index.js` l.123 | **Aucun** : `pairing/index.ts` (5 l.) appelle `createPairingDom(document)` l.4 + `initializePairingController(dom)` l.5 ; `overlay/renderer/pairing.js` (52 l.) est un artefact **mort**, embarqué par `build.files` mais jamais chargé — **ne pas le câbler ni le copier dans `apps/lc-overlay/web/`** (décision §8.7). | Form `#pairing-form` l.100, `#server-url` l.109, `#pairing-code` l.114, `#submit-button` l.118, `#status` l.120. |

Fichiers statiques requis et leur résolution :

- **Modules ESM** : `overlay/src/renderer/overlay/index.ts` (71 l., installe les globals `window.__overlayLegacy*`
  l.26-69 puis charge `./overlay.js` l.71) ; `board/index.ts` (102 l., globals l.52-100 puis `./board.js` l.102) ;
  `pairing/index.ts` (5 l.). Les modules sont compilés en `dist/renderer/{overlay,board,pairing}/index.js` par
  `overlay/tsconfig.renderer.json` (module ES2022, outDir `dist`) et vérifiés ESM par
  `overlay/scripts/assert-renderer-esm.mjs` (pas de `require`/`module.exports` dans les entrées renderer ; `dist/shared/ipc.js`
  reste CJS).
- **Scripts legacy** : chargés par `overlay/src/renderer/legacy-loader.ts:1-8` avec
  `new URL(relativeScriptPath, window.location.href)` : le chemin est résolu **relativement à l'URL de la page**. Si les
  HTML sont à la racine web et les JS dans un sous-dossier différent de l'actuel, le loader casse.
- **CSS** : aucun fichier image/police local dans `overlay/renderer/` ; les seuls styles sont les 3+5 CSS ci-dessus.
- **Icônes** (racine `overlay/`, copiées dans le cache runtime) : `icon.png` (1 363 390 o), `icon.ico` (211 141 o),
  `icon-tray.png` (1 327 203 o) — voir §5.
- **Dépendance externe (supprimée en P1-11)** : le renderer overlay legacy injectait le script
  `https://platform.twitter.com/widgets.js` (`overlay/src/renderer/overlay/legacy-twitter-widgets-utils.ts:54`, appelé au
  bootstrap l.70). Ce chargement est **désactivé** et la CSP n'autorise plus de script tiers (cf. §12.2) ; le rendu de carte
  tweet est statique.
- **Réseau renderer** (hors bridge, conservé en Phase 0) : le board fait des `fetch()` REST vers
  `/overlay/meme-board/items` (`legacy-item-utils.ts:65,71,114,115`), `/items/:itemId` (`:95,96,136,137`), média
  `/overlay/meme-board/media/:assetId` (`legacy-preview-render-state-utils.ts:80`) ; l'overlay ajoute des URLs média
  (`overlay/src/renderer/overlay/legacy-media-dom-utils.ts:115-116`, diagnostic
  `overlay/src/renderer/overlay/legacy-media-diagnostics.ts:72`) ; `buildAuthedUrl`
  (`overlay/src/renderer/board/legacy-preview-media-utils.ts:44-53`) ajoutait `?token=<clientToken>` ; le portage
  P1-11 l'a supprimé et passe l'authentification par en-tête `Authorization: Bearer` via
  `window.livechatOverlay.authorizedFetch` (cf. §12.4). Ces appels ne passent pas par IPC mais dépendent de l'origine
  WebView et de la CSP (§7 risques #3/#4 et §8.2/§8.3).

## 5. Chemins runtime et devenir en Tauri

Source : `overlay/src/main/app/runtime-paths.ts` (164 l.) et `overlay/src/main/services/config-service.ts`.

### 5.1 Résolution actuelle (Electron)

| Élément | Chemin actuel | Preuve |
| --- | --- | --- |
| `config.json` | `app.getPath('userData')/config.json`, soit `%APPDATA%\LiveChat\config.json` (`userData` dérivé de `productName: LiveChat`, `build.appId: com.livechat`, `overlay/package.json`) | `runtime-paths.ts:117` (cache) et `:160-163` (toujours `userDataPath/config.json`) |
| Cache `runtime-assets/` | `userData/runtime-assets/` avec `renderer/`, `dist/renderer/`, `dist/preload/`, `icon.png`, `icon.ico`, `icon-tray.png` | `copyRuntimeAssetsToStableCache` `runtime-paths.ts:71-137` (cacheRoot l.72, copies l.84-107, chemins retournés l.117-123) |
| HTML | `runtime-assets/renderer/{overlay,pairing,board}.html` (copiés depuis `overlay/renderer/`) | `runtime-paths.ts:55-57`, copie l.84-88 |
| Modules renderer | `runtime-assets/dist/renderer/**` (copiés depuis `overlay/dist/renderer/`) | copie l.89-93 |
| Preload | `runtime-assets/dist/preload/index.js` (copié depuis `overlay/dist/preload/`) | copie l.94-98 ; assertion de test `overlay/tests/main/preload-runtime.test.js` |
| Icône fenêtre | `icon.ico` si Windows et présent, sinon `icon.png` | `resolvePreferredIconPath` `:30-36` |
| Icône tray | `icon-tray.png` sinon `icon.png` | `resolvePreferredTrayIconPath` `:38-44` |
| Racine source | `appRoot = path.join(baseDir, '../../..')` (dossier `overlay/`) | `resolveSourcePaths` `:46-59` (l.47) |
| Fallback | si les sources sont incomplètes (`allRequiredSourceFilesExist` `:61-69` exige `icon.png` + preload + 3 HTML, pas les `.ico`/tray), le main utilise les chemins sources | `resolveRuntimePaths` `:139-164` (fallback l.141-149 et l.151-153, cache l.155-158) |

Le cache existe uniquement pour stabiliser les chemins (mise à jour applicative, verrous de fichier Windows) ; il n'y a
aucune logique métier dans `runtime-assets/`.

### 5.2 Configuration persistée

`OverlayRuntimeConfig` (`config-service.ts:4-21`), 16 champs, avec `DEFAULT_CONFIG` (`:42-59`) :
`displayId/displayIndex/displayKey/displayLabel` `null`, `volume: 1`, `enabled: true`, `autoStart: false`,
`guestMode: false`, `showText: true`, `serverUrl/clientToken/clientId/guildId/authorName/deviceName: null`,
`memeBindings: {}`. Normalisations : volume clampé `[0,1]` (`:68-74`), autoStart booléen (`:76-78`), guestMode booléen
(`:80-82`), `memeBindings` trim/drop des vides (`:84-103`), URLs trim + sans `/` final (`:105-107`). Écriture
`JSON.stringify(config, null, 2)` (`:134-157`), lecture avec défauts si fichier absent/inexploitable (`:109-132`).
`hasPairingConfig` = `serverUrl && clientToken && guildId` (`:159-161`).

Le renderer ne reçoit qu'une **projection de 8 champs** (`overlay-config-ipc-service.ts:25-47`) :
`serverUrl`, `clientToken`, `guildId`, `clientId`, `guestMode`, `showText`, `volume`, `memeBindings` — champs de fenêtre
(`display*`, `enabled`, `autoStart`) exclus.

### 5.3 Devenir en Tauri 2

| Élément actuel | Devenir Tauri 2 | Action / remarque |
| --- | --- | --- |
| `runtime-assets/` + double copie | **Supprimé** : le frontend est embarqué dans le binaire (`frontendDist` → `tauri://localhost` / `http://tauri.localhost`) ou dans les ressources | Ne pas porter `copyRuntimeAssetsToStableCache` ; garder le renderer sous `apps/lc-overlay/web/` et le déclarer dans `tauri.conf.json`. |
| `renderer/{overlay,pairing,board}.html` + `dist/renderer/**` + CSS | `apps/lc-overlay/web/` (HTML/CSS/JS copiés à l'identique) ; URL de fenêtre relative à `frontendDist` (`overlay.html`, `board.html`, `pairing.html`) | Préserver les chemins relatifs : `./overlay.css`, `../dist/renderer/...`, `./overlay.js` (loader relatif à la page). |
| `dist/preload/index.js` | **Remplacé par `apps/lc-overlay/web/bridge.js`** chargé par les pages | Le main Rust n'injecte rien : pas de `preload` en Tauri 2. |
| `config.json` (`%APPDATA%\LiveChat\config.json`) | **Chemin forcé, conservé** : `%APPDATA%\LiveChat\config.json`, calculé manuellement depuis la variable `APPDATA` ; **ne pas** utiliser `app_config_dir()` (décision Lead §8.6) | Migration one-shot `.bak` si nécessaire ; le même fichier sert au **rollback Electron**. Identifier `com.livechat`, productName `LiveChat` (cohérents avec `build.appId` Electron) ; champs et format JSON inchangés. |
| `icon.png` / `icon.ico` | Bundle Windows (`bundle.icon`) + icône de fenêtre ; `.ico` requis pour l'installateur NSIS, PNG accepté pour les fenêtres | Fournir les deux, plus `icon-tray.png` pour le tray (`TrayIconBuilder::icon(Image::from_path(...))`). |
| `icon-tray.png` | Icône tray Rust | Même asset. |
| `appRoot = ../../../` | **Disparaît** : plus de résolution de chemins source | — |
| `%APPDATA%` userData | `app_data_dir()`/`app_config_dir()` selon identifiant | Voir ligne config. |
| Chemins Windows `.ico` conditionnels | Options de bundle, pas de code conditionnel | — |

Tranché par le Lead le 2026-11-09 (détail §8) : identifiant Tauri `com.livechat`, `productName` `LiveChat`, config
**forcée** à `%APPDATA%\LiveChat\config.json` (migration one-shot `.bak`, rollback Electron possible) et CSP cible
(§8.2). Reste ouvert : mode de chargement du frontend (embarqué via `frontendDist` vs ressources) — sans impact sur le
bridge.

## 6. Mapping « nom actuel → nom Tauri »

Convention retenue et implémentée dans `apps/lc-overlay/web/bridge.js` (`COMMANDS` l.39-50, `EVENTS` l.52-58) :
**commandes snake_case**, **événements `overlay://...`**, payload passé sous la clé `payload`
(`invoke('overlay_get_config', {})`, `invoke('overlay_report_error', { payload })`).

### 6.1 Commandes (`invoke`)

| Méthode JS | Canal IPC actuel | Commande Tauri | Arguments JS | Réponse Rust |
| --- | --- | --- | --- | --- |
| `getConfig` | `overlay:get-config` | `overlay_get_config` | `{}` (aucun payload) | `OverlayConfig` |
| `rendererReady` | `overlay:renderer-ready` | `overlay_renderer_ready` | `{}` | `()` (fire-and-forget) |
| `reportError` | `overlay:error` | `overlay_report_error` | `{ payload: OverlayErrorPayload }` | `()` |
| `reportPlaybackState` | `overlay:playback-state` | `overlay_report_playback_state` | `{ payload: OverlayPlaybackStatePayload }` | `()` |
| `reportPlaybackStop` | `overlay:playback-stop` | `overlay_report_playback_stop` | `{ payload: OverlayPlaybackStopPayload }` | `()` |
| `consumePairing` | `pairing:consume` | `pairing_consume` | `{ payload: PairingConsumeRequest }` | `PairingConsumeResponse` |
| `getMemeBindings` | `meme-board:get-bindings` | `meme_board_get_bindings` | `{}` | `MemeBoardBindingsResponse` |
| `setMemeBindings` | `meme-board:set-bindings` | `meme_board_set_bindings` | `{ payload: MemeBoardSetBindingsRequest }` | `MemeBoardSetBindingsResponse` |
| `triggerMeme` | `meme-board:trigger` | `meme_board_trigger` | `{ payload: MemeBoardTriggerRequest }` | `MemeBoardActionResponse` |
| `stopMemePlayback` | `meme-board:stop` | `meme_board_stop` | `{}` | `MemeBoardActionResponse` |

Signatures Rust recommandées (payloads `serde` avec `rename_all = "camelCase"`, erreurs en `Result<_, String>` pour rester
compatible avec `error.message` du renderer) :

```rust
#[tauri::command]
async fn overlay_get_config(state: State<'_, OverlayState>) -> Result<OverlayConfig, String>;

#[tauri::command]
async fn overlay_renderer_ready(app: AppHandle, state: State<'_, OverlayState>) -> Result<(), String>;

#[tauri::command]
async fn overlay_report_error(payload: OverlayErrorPayload, state: State<'_, OverlayState>) -> Result<(), String>;

#[tauri::command]
async fn overlay_report_playback_state(payload: OverlayPlaybackStatePayload, state: State<'_, OverlayState>) -> Result<(), String>;

#[tauri::command]
async fn overlay_report_playback_stop(payload: OverlayPlaybackStopPayload, state: State<'_, OverlayState>) -> Result<(), String>;

#[tauri::command]
async fn pairing_consume(payload: PairingConsumeRequest, app: AppHandle, state: State<'_, OverlayState>) -> Result<PairingConsumeResponse, String>;

#[tauri::command]
async fn meme_board_get_bindings(state: State<'_, OverlayState>) -> Result<MemeBoardBindingsResponse, String>;
// idem meme_board_set_bindings(payload), meme_board_trigger(payload), meme_board_stop()
```

Notes de contrat :

- Les commandes sans payload reçoivent `{}` : ne pas déclarer d'argument `payload` optionnel ambigu.
- Les clés d'arguments multi-mots suivent la conversion Tauri (`snake_case` Rust ↔ `camelCase` JS) ; ici tout tient dans
  `payload`, donc stable.
- Tauri rejette les commandes en renvoyant une valeur sérialisable ; une `String` d'erreur devient `error.message` côté
  bridge (normalisée en `Error`, `.cause` conservée).
- `renderer_ready` ne doit pas bloquer : le main renvoie les settings via l'événement, pas via le retour de commande.

### 6.2 Événements (`listen`)

| Méthode JS | Canal IPC actuel | Événement Tauri | Payload | Émis par (Rust) vers |
| --- | --- | --- | --- | --- |
| `onPlay` | `overlay:play` | `overlay://play` | `OverlayPlayPayload` | fenêtre `overlay` uniquement |
| `onStop` | `overlay:stop` | `overlay://stop` | `OverlayStopPayload` | fenêtre `overlay` uniquement |
| `onSettings` | `overlay:settings` | `overlay://settings` | `OverlaySettingsPayload` | fenêtres `overlay` **et** `board`, jamais `pairing` |

Recommandations Rust :

- Préférer un emit **par fenêtre** (`WebviewWindow::emit`) pour conserver les cibles actuelles ; `app.emit(...)` diffuse à
  toutes les webviews (le pairing n'écoutant pas, ce serait inoffensif mais moins fidèle).
- Les noms `overlay://play|stop|settings` sont **conservés** (décision Lead §8.5) : Tauri 2 accepte `a-zA-Z0-9`, `-`, `_`,
  `:`, `/`. Émission/réception réelle **vérifiée par le POC Tauri en cours** ; repli `overlay:play`/`overlay:stop`/
  `overlay:settings` **uniquement si le POC l'exige**, avec le même mapping applicatif (la table `EVENTS` de
  `bridge.js:52-58` est le point unique de changement, le renderer ne voit pas la différence).
- Le bridge normalise l'objet reçu `{ event, id, payload }` en **payload brut** avant d'appeler le callback : le renderer
  ne voit jamais l'enveloppe Tauri.
- Le bridge s'abonne avec `target: { kind: 'Any' }` (défaut Tauri 2), ce qui reçoit les emits ciblés webview.

### 6.3 Exemples JSON

```js
// getConfig
await window.livechatOverlay.getConfig();
// → invoke('overlay_get_config', {})

// rapport d'état
window.livechatOverlay.reportPlaybackState({ jobId: 'j1', state: 'playing', remainingMs: 4200 });
// → invoke('overlay_report_playback_state', { payload: { jobId: 'j1', state: 'playing', remainingMs: 4200 } })

// appairage
await window.livechatOverlay.consumePairing({ serverUrl: 'https://livechat.example.com', code: 'AB12CD' });
// → invoke('pairing_consume', { payload: { serverUrl: '…', code: 'AB12CD' } })

// événement (côté Rust)
// window.emit("overlay://play", OverlayPlayPayload { .. })
```

## 7. Risques de compatibilité

| # | Risque | Symptôme | Mitigation / décision |
| --- | --- | --- | --- |
| 1 | **Bridge absent au premier paint** | `TypeError: window.livechatOverlay is undefined` au bootstrap ; écran vide ou overlay muet. | `bridge.js` doit être un `<script>` **classique dans le `<head>`**, avant le module de page (`README.md`). Vérifier l'ordre à l'intégration des pages ; le bridge est idempotent. |
| 2 | **Événement play/stop perdu avant les listeners** | Un job lancé juste après l'ouverture de la fenêtre n'est jamais joué. | **Décidé (§8.1) : bufferiser côté Rust.** File bornée par fenêtre ; conserver le **dernier play** et le **dernier stop**, les rejouer **dans l'ordre** dès réception de `overlay_renderer_ready` ; timeout **5 s** puis drop + `warn`. C'est la correction d'une **course de la v1 Electron** (le main Electron n'a pas de file ; listeners posés seulement après `await getConfig()`, `legacy-bootstrap-utils.ts:56-73`), pas une mesure de compatibilité. `overlay://settings` reste envoyé après le handshake. Board : aucun buffer nécessaire (§8.7). |
| 3 | **CSP** | Scripts bloqués (`bridge.js`, modules, twitter), styles inline ignorés, médias bloqués. | **Décidé (§8.2)** : `script-src 'self'` strict ; `style-src 'self' 'unsafe-inline'` (seule justification : `<style>` inline de `pairing.html:7-97`, contenu local et statique) ; `img-src 'self' asset: http://asset.localhost data:` ; `media-src 'self' blob: {API_URL}` ; `connect-src 'self' {API_URL} {WS_API_URL}` ; `frame-src https://platform.twitter.com` ; `object-src 'none'`. **Pas de hash à maintenir** pour l'instant. À valider au POC (le widget Twitter peut exiger des domaines supplémentaires, ex. `syndication.twitter.com`). `migration-plan.md:29` signalait le `unsafe-inline` du legacy : la décision le circonscrit aux styles. |
| 4 | **Origine WebView et contenu mixte** | `useHttpsScheme` false ⇒ origine `http://tauri.localhost` : contexte non sécurisé, requêtes http↔https mélangées, `fetch` média bloqué. | **Décidé (§8.3)** : garder le schéma par défaut (**pas** de `useHttpsScheme`) ⇒ origine `http://tauri.localhost` sous Windows. Conséquences : CORS serveur à adapter (§8.3, ligne 5) et `{API_URL}` en HTTPS recommandée pour limiter le contenu mixte. Les URLs média sont utilisées **verbatim** (§8.4). |
| 5 | **CORS / Origin** | Le board fait du REST direct (`/overlay/meme-board/items`, média) : en Electron l'origine était `file://` (souvent `null`) ; en Tauri elle devient `tauri.localhost` ⇒ préflight/CORS différents. | **Décidé (§8.3)** : ajouter **`http://tauri.localhost` et `https://tauri.localhost`** à la liste CORS des routes `/overlay` (et `/admin` si nécessaire) côté `apps/lc-server` — **action Phase 3**, hors bridge (routeur via commandes Rust écarté pour l'instant). |
| 6 | **CSP et `window.require` / Node** | Le renderer n'utilise pas `require` (grep exhaustif) ; toute dépendance résiduelle à `process`/`require` casserait. | Aucun changement nécessaire, mais l'acceptation doit vérifier `nodeIntegration` absent (déjà `false` en Electron, `window.require` absent en WebView2). |
| 7 | **Modules ES vs scripts classiques** | `dist/renderer/*/index.js` doit rester chargé en `type="module"` ; `bridge.js` et `overlay.js`/`board.js` en classique. | Conserver les types de scripts actuels ; ne pas convertir le bridge en module (il doit s'exécuter avant les modules). `legacy-loader.ts:1-8` reste en ESM (import/export). |
| 8 | **Chemins relatifs / racine web** | CSS/JS 404 après copie dans `web/`. | Préserver l'arborescence : HTML à la racine de `frontendDist`, `dist/renderer/**` et `renderer/*.js` sous la même racine ; conserver `./overlay.css`, `../dist/renderer/...` et le loader relatif à `window.location.href`. |
| 9 | **`withGlobalTauri` / API Tauri indisponible** | Mode mock activé par erreur en production ; ou `__TAURI__` absent alors que `withGlobalTauri: false`. | Le bridge détecte d'abord `__TAURI_INTERNALS__.invoke` (toujours présent) puis `__TAURI__.core.invoke` ; pour les événements il utilise `__TAURI__.event.listen` si disponible, sinon `plugin:event|listen` + `transformCallback` (fonctionne sans `withGlobalTauri`). Ajouter en prod une assertion `mode === 'tauri'` (recommandé). |
| 10 | **Mock silencieux** | Page ouverte hors Tauri : appels « réussis » sans backend, données factices. | Le bridge loggué `[lc-overlay-bridge]` + `window.__livechatOverlayBridge.mode`; gate prod recommandé (console.error/fail-fast si `mock`). |
| 11 | **Rejets non gérés** | Les 4 méthodes `send` du renderer ignorent le retour : un rejet Tauri produirait `unhandledrejection`. | Le bridge renvoie une `Promise` toujours résolue pour ces 4 méthodes (échec en `console.warn`) ; les méthodes `invoke` propagent l'erreur normalisée. |
| 12 | **Erreurs non-`Error`** | Tauri peut rejeter une `String` ; `error.message` vaut alors `undefined` (pairing affiche `undefined`). | Le bridge normalise en `Error` avec `.message` d'origine et `.cause` (`bridge.js`). |
| 13 | **Permissions / ACL** | `listen` refusé si la permission événement manque. | `core:event:default` (allow-listen/unlisten/emit) est active par défaut : aucune capability supplémentaire. Les commandes applicatives ne sont pas ACL-gated. Vérifier la capability de fenêtre (labels `overlay`, `board`, `pairing`). |
| 14 | **Comportements WebView2 de l'overlay** | Transparence, click-through, always-on-top, focusabilité, topmost 2 s, autoplay média. | `set_ignore_cursor_events(true)` + `WS_EX_NOACTIVATE`, topmost réaffirmé toutes les 2 s (`overlay-window-service.ts:25,55`), `focusable: false` (`window-factory.ts:32-53`). Autoplay : Electron utilise `autoplay-policy=no-user-gesture-required` (`runtime-legacy-entry.ts:20-27`) ; en Tauri/WebView2 passer des args navigateur équivalents. POC go/no-go déjà prévu (`migration-plan.md:82-88`). |
| 15 | **`ignore-certificate-errors`** | Le main Electron ignore les erreurs TLS (`runtime-legacy-entry.ts:20-27`), ce qui masque des problèmes de sécurité et n'a pas d'équivalent WebView2 propre. | **Ne pas porter** ce contournement ; garder la validation TLS stricte (l'appairage fait déjà un repli `isLikelyTlsError`, `pairing-consume-service.ts:85-97`). Risque de régression si un serveur de dev a un certificat auto-signé : documenter. |
| 16 | **Protocole média v2 vs renderer legacy** | Le renderer legacy ajoute `?token=<clientToken>` (`legacy-preview-media-utils.ts:44-53`) alors que le protocole v2 supprime le token et signe les URLs (`migration-plan.md:63-72`). | **Décidé (§8.4)** : bridge et renderer utilisent `payload.media.url` **verbatim**, aucune réécriture ni paramètre ajouté. Toute logique legacy qui ajoute encore le token est **supprimée en Phase 8** : `overlay/src/renderer/board/legacy-preview-media-utils.ts:44-53` et `overlay/src/renderer/overlay/legacy-media-dom-utils.ts:115-116`. Seule exception tolérée : fragment `#t=` ajouté par le lecteur vidéo côté client pour le seek, à condition que la signature **ne couvre pas le fragment** (déjà le cas). Garder `clientToken` dans `OverlayConfig` pour le reste. |
| 17 | **Double chargement / HMR** | Deux bridges, fuite de listeners, désabonnements croisés. | Garde d'idempotence dans `bridge.js` (si `window.livechatOverlay` existe, réinstallation contrôlée) ; un seul `<script>` par page. |
| 18 | **Perte des listeners au reload du renderer** | `overlayWindow.reload()` (raccourci manuel `shortcut-service.ts:31-39`) : les abonnements Tauri (`eventId`) restent-ils côté Rust ? | En Tauri, `listen` enregistre un callback JS ; un reload de page détruit le contexte ⇒ les listeners JS disparaissent, le bridge doit se réabonner au prochain bootstrap (comportement identique à Electron). Vérifier qu'aucun état Rust ne dépend d'un abonnement. |

## 8. Décisions Lead (2026-11-09)

Arbitrages du Lead sur les points ouverts de la Phase 0. Ils font foi pour la Phase 8 et **remplacent les mitigations
provisoires** du §7 pour les risques concernés. Aucun changement de `apps/lc-overlay/web/bridge.js` n'est requis.

### 8.1 Bufferisation play/stop (risque #2)

**Décision : bufferiser côté Rust.** L'adaptateur Tauri garde une **file bornée par fenêtre** ; si `overlay://play` ou
`overlay://stop` arrive avant que la commande `overlay_renderer_ready` soit reçue pour la fenêtre `overlay` :

- conserver le **dernier play** et le **dernier stop** (pas d'accumulation illimitée) ;
- les rejouer **dans l'ordre d'arrivée** dès `overlay_renderer_ready` ;
- si le handshake n'arrive pas dans les **5 s**, vider la file avec un `warn` et ne pas rejouer.

C'est la **correction d'une course de la v1 Electron**, pas une mesure de compatibilité : le main Electron n'a jamais eu
de file et le renderer ne pose ses listeners qu'après `await getConfig()` (`legacy-bootstrap-utils.ts:56-73`).
`overlay://settings` reste envoyé après le handshake uniquement. Le board n'appelle pas `rendererReady` (§2.2, §8.7) :
aucun buffer côté board.

### 8.2 CSP (risque #3)

**Décision : `script-src 'self'` strict ; `'unsafe-inline'` pour les styles uniquement**, justifié par le `<style>` inline
de `pairing.html:7-97` (contenu local et statique) ; **pas de hash à maintenir** pour le moment. CSP cible :

```text
default-src 'self';
script-src 'self';
style-src 'self' 'unsafe-inline';
img-src 'self' data: blob: asset: http://asset.localhost http: https:;
media-src 'self' blob: http: https:;
connect-src 'self' ipc: http://ipc.localhost http: https: ws: wss:;
frame-src 'none';
object-src 'none';
```

`{API_URL}` / `{WS_API_URL}` ne peuvent pas être interpolées statiquement dans `tauri.conf.json` (config dynamique) : le
portage P1-11 élargit donc `img-src`/`media-src`/`connect-src` à `http:`/`https:` (et `ws:`/`wss:`) tout en gardant
`script-src 'self'` strict. **Aucun script tiers** : la dépendance Twitter a été supprimée (cf. §12.2).

### 8.3 Origine WebView et CORS (risques #4, #5)

**Décision : garder le schéma par défaut (pas de `useHttpsScheme`) ⇒ origine `http://tauri.localhost` sous Windows.**
Le serveur Rust (`apps/lc-server`) devra ajouter **`http://tauri.localhost` et `https://tauri.localhost`** à la liste CORS
des routes `/overlay` (et `/admin` si nécessaire) — **action Phase 3**. Une `{API_URL}` en HTTPS reste recommandée pour
limiter le contenu mixte.

### 8.4 URLs média protocole v2 (risque #16)

**Décision : bridge et renderer utilisent `payload.media.url` VERBATIM.** Aucune réécriture, aucun ajout de `?token=` ni
d'aucun autre paramètre. Toute logique legacy qui ajoute encore le token est **supprimée en Phase 8** :
`overlay/src/renderer/board/legacy-preview-media-utils.ts:44-53` (`buildAuthedUrl`) et
`overlay/src/renderer/overlay/legacy-media-dom-utils.ts:115-116` (URL média overlay). Seule exception tolérée : ajout du
fragment `#t=` par le lecteur vidéo côté client pour le seek, à condition que la signature **ne couvre pas le fragment**
(déjà le cas).

### 8.5 Noms d'événements Tauri (§6.2)

**Décision : conserver `overlay://play`, `overlay://stop`, `overlay://settings`.** Les noms Tauri 2 valides acceptent
`a-zA-Z0-9`, `-`, `_`, `:`, `/` ; l'émission/réception réelle est **vérifiée par le POC Tauri en cours**. Repli
`overlay:play` / `overlay:stop` / `overlay:settings` **uniquement si le POC l'exige**, avec le même mapping applicatif
(point unique de changement : table `EVENTS`, `bridge.js:52-58` ; le renderer ne voit pas la différence).

### 8.6 Chemin de config (§5.3)

**Décision : ne PAS dépendre de `app_config_dir()`.** La config reste à **`%APPDATA%\LiveChat\config.json`**, calculé
manuellement depuis la variable d'environnement `APPDATA`, avec migration one-shot `.bak` ; le **même fichier** est
conservé pour permettre le **rollback Electron**. L'identifier Tauri est **`com.livechat`** (cohérent avec le `build.appId`
Electron) et le `productName` **`LiveChat`** ; le chemin de config est forcé indépendamment de l'identifier. Conséquence :
un upgrade in-place ou side-by-side **conserve les réglages existants**.

### 8.7 Rappels d'intégration

- Le renderer **board n'appelle pas `rendererReady`** (`legacy-bootstrap-utils.ts:54-74`) : **aucun buffer d'événements
  n'est nécessaire côté board** ; ses settings arrivent via `onBoardReadyToShow`
  (`runtime-window-socket-service.ts:99-101`).
- `overlay/renderer/pairing.js` (52 l.) est **mort** : la page pairing charge `pairing-controller.ts` via
  `pairing/index.ts:4-5`. **Ne pas le copier dans `apps/lc-overlay/web/`**, ne pas le câbler (cf. §4).
- `bridge.js` n'a besoin d'aucune modification pour ces décisions (URLs média verbatim, noms d'événements conservés,
  handshake inchangé).

---

## 9. Contrat d'implémentation Phase 8 (côté Rust)

Checklist minimale pour que le renderer conservé fonctionne sans modification :

1. **10 commandes** Tauri avec les noms §6.1, enregistrées via `.invoke_handler(tauri::generate_handler![...])`.
2. **3 événements** `overlay://play|stop|settings` émis vers les bonnes fenêtres (§6.2), payloads sérialisés **camelCase**.
3. **Handshake** : `overlay_renderer_ready` déclenche l'envoi `overlay://settings` à la fenêtre overlay (équivalent
   `register-overlay-ipc-handlers.ts:60-62`) ; `overlay://settings` aussi sur `ready-to-show` (équivalent
   `runtime-window-socket-service.ts:86-88,99-101`).
4. **Buffering** : file bornée par fenêtre overlay (décision §8.1) — dernier play + dernier stop conservés, rejoués dans
   l'ordre dès `overlay_renderer_ready`, timeout 5 s puis drop avec `warn` ; aucun buffer côté board (§8.7).
5. **Sémantique métier identique** aux services Electron : projection 8 champs (`overlay-config-ipc-service.ts:25-47`),
   normalisations de config (`config-service.ts:68-107`), appairage HTTP (`pairing-consume-service.ts:65-134`), mémoires
   (`meme-bindings-service.ts:51-204`), télémétrie avec guest/offline (`overlay-playback-forwarding-service.ts:38-96`).
6. **Erreurs** : `Result<_, String>` avec messages exploitables par `error.message` ; pas de panic (lints workspace
   `deny(unwrap_used, expect_used, panic)`, `forbid(unsafe_code)`).
7. **Async / I/O** : commandes `async`, jamais de blocage du thread UI (fs via `spawn_blocking`, HTTP via client async).
8. **Sécurité** : TLS strict (pas d'`ignore-certificate-errors`), pas de secrets dans les logs, pas de token dans les URLs
   exposées au DOM (cible protocole v2).
9. **Fenêtres** : labels `overlay`/`board`/`pairing` ; overlay transparent + click-through + topmost 2 s ; pairing
   460×580 ; board 1180×780 min 960×640 (plan l.82-88, `window-factory.ts:32-96`).
10. **Config** : chemin **forcé** `%APPDATA%\LiveChat\config.json` (calcul manuel via `APPDATA`, décision §8.6), schéma
    JSON inchangé, migration one-shot `.bak`, **fichier partagé avec Electron** pour permettre le rollback.

## 10. Livrable bridge et vérifications

`apps/lc-overlay/web/bridge.js` (719 lignes, script classique IIFE, zéro dépendance) :

| Élément | Détail |
| --- | --- |
| Détection | `__TAURI_INTERNALS__.invoke` → transport `internals-invoke` ; sinon `__TAURI__.core.invoke` → `global-invoke` ; sinon `mock` (`bridge.js:62-94`). |
| Commandes | Table `COMMANDS` l.39-50 (10 commandes §6.1). |
| Événements | Table `EVENTS` l.52-58 + `EVENT_LIST` l.58 (`overlay://play|stop|settings`). |
| Émission | `invoke(commande, { payload })` ou `invoke(commande, {})`, journalisation, erreurs normalisées. |
| Abonnement | `nativeListen` via `__TAURI__.event.listen` sinon `plugin:event|listen` + `transformCallback` ; désabonnement via `plugin:event|unlisten` / `dispose()` (l.299-…, l.349-…) ; liste locale pour le mock. |
| Journal | `pushCall` l.196 ; max 200 entrées ; masquage `token|code|sig|secret|password` ; troncature >300 caractères. |
| Mock | valeurs neutres ; `rendererReady` → `overlay://settings` différé ; aucun réseau. |
| API | objet `api` l.625-681, exactement 13 clés ; exposition `window.livechatOverlay` l.683 + `window.__livechatOverlayBridge` l.688-… |
| Test | `window.__bridgeMockEmit` l.705 (alias `play`/`overlay:play`/`overlay://play`, retour nb listeners) ; `window.__bridgeCalls`, `window.__bridgeLastEvent`. |

Vérifications :

- `node --check apps/lc-overlay/web/bridge.js` — **OK** (vérifié).
- `apps/lc-overlay/dev/bridge-test.html` — harnais manuel : 10 commandes, 3 abonnements, émission simulée, journal des
  appels, bouton « Tout tester (13 méthodes) » (score attendu 13/13 en mock). Hors bundle depuis P1-11 (cf. §12.6).
- Reste à valider en Phase 8 : comportement réel `invoke`/`listen` sous WebView2, ordre des events, CSP, CORS, origines,
  click-through/transparence, autoplay — ces points sont hors de portée du mock.

## 11. Fichiers de référence

| Fichier | Rôle |
| --- | --- |
| `overlay/src/shared/ipc.ts:19-64` | Source unique des 13 canaux IPC (strings, maps de payloads). |
| `overlay/src/preload/index.ts:14-56` | API `window.livechatOverlay` (13 méthodes) + exposition. |
| `overlay/src/types/livechat-overlay.d.ts:19-39` | Signatures TypeScript exactes de l'API. |
| `overlay/src/main/ipc/register-overlay-ipc-handlers.ts:42-74` | Handlers main des 9 canaux overlay/mémoires. |
| `overlay/src/main/ipc/register-pairing-ipc-handler.ts:36-38` | Handler `pairing:consume`. |
| `overlay/src/main/services/*.ts` | Sémantique métier (config, appairage, télémétrie, mémoires, socket). |
| `overlay/src/main/app/runtime-paths.ts` | Résolution config/HTML/preload/icônes + cache `runtime-assets`. |
| `overlay/src/main/windows/window-factory.ts` | Options des 3 fenêtres. |
| `overlay/src/renderer/**` | Modules ESM + bootstrap legacy (ordre d'appel). |
| `overlay/renderer/*.html|css|js` | Pages conservées telles quelles. |
| `overlay/docs/architecture.md:152-170` | Politique de compatibilité renderer/main. |
| `docs/rust/migration-plan.md` | Plan approuvé (§4 protocole v2, §6 overlay Tauri, risques). |
| `apps/lc-overlay/web/bridge.js` | Bridge Tauri 2 livré (Phase 0, durci P1-11). |
| `apps/lc-overlay/dev/bridge-test.html` | Harnais de test navigateur (hors bundle). |
| `apps/lc-overlay/dev/README.md` | Mode d'emploi du bridge et du mock (hors bundle). |

## 12. Durcissement P1-11 (audit A6)

Cette section remplace les points de la matrice ci-dessus devenus obsolètes
après les correctifs `AUD-A6-002/003/004/005/008/015` (task-43).

### 12.1 Appairage et WebSocket : HTTPS obligatoire par défaut (A6-003/004)

- `pairing_consume` refuse une `serverUrl` `http://` par défaut : le message
  d'erreur indique explicitement qu'il faut `https://`.
- `net::ws::build_session_config` refuse de construire une URL `ws://` ; seul
  `wss://` est accepté. La dérogation dev est centralisée dans
  `apps/lc-overlay/src/net/tls.rs` : `LIVECHAT_INSECURE_TLS=1` n'est honoré
  **qu'en build debug** (`debug_assertions`) et émet un `warn!`.
- Un `apiBaseUrl` renvoyé par `POST /overlay/pair/consume` ne peut jamais
  rétrograder un appairage HTTPS vers HTTP ; l'upgrade HTTP→HTTPS reste accepté.
- Le repli `?token=` du board côté serveur reste transitoire (contrat serveur
  inchangé) ; le renderer ne l'utilise plus (voir §12.4).

### 12.2 Carte tweet : widgets tiers désactivés, rendu statique (A6-002)

- `legacy-twitter-widgets-utils.js::ensureTwitterWidgets` lève
  `tweet_widgets_disabled` sauf si `window.__LIVECHAT_TWEET_WIDGETS_ENABLED === true`
  (opt-in explicite, jamais activé en production).
- `legacy-tweet-card-renderer.js` n'utilise plus `innerHTML` : le HTML distant
  est parsé dans un document inerte et seul son `textContent` est affiché
  (aucun script ne peut s'exécuter, même si le serveur renvoyait `tweet_card`).
- CSP : `script-src 'self'` et `frame-src 'none'` (plus de `platform.twitter.com`).
  Le serveur v2 n'expose pas `tweet_card` (atténuation) ; le renderer est
  désormais sûr même en cas de régression serveur.

### 12.3 Capabilities et autorisation IPC par fenêtre (A6-008)

- `capabilities/overlay.json`, `board.json`, `pairing.json` : une capability par
  fenêtre (`windows: ["overlay"]`, etc.), permissions `core:default` minimales.
  `capabilities/default.json` est supprimé.
- La matrice `commands::COMMAND_MATRIX` limite les 12 commandes par label :
  overlay = handshake/télémétrie ; board = meme-board + lecture config +
  check_updates ; pairing = appairage + lecture config. Toute autre combinaison
  est refusée avec un message explicite (testé par
  `commands::tests::command_matrix_is_window_scoped`).

### 12.4 Board : Authorization Bearer via le bridge, plus de token en query (A6-001/004)

- `bridge.js` expose `window.livechatOverlay.authorizedFetch(url, options)` :
  il lit la configuration via `getConfig()`, ajoute
  `Authorization: Bearer <clientToken>` et appelle `fetch`.
- Les shims board (`legacy-item-utils.js`, `legacy-preview-media-utils.js`)
  n'ajoutent plus `?token=` : `buildAuthedUrl` ne pose plus le token, les 4
  appels REST passent par `authorizedFetch`, et les médias d'aperçu sont
  récupérés en binaire puis exposés en `blob:` object URL.
- Le token ne doit plus apparaître dans les URLs ni dans les logs HTTP.

### 12.5 Updater désactivé pour 3.0.0 (A6-005)

- `updater.rs` expose `UPDATER_ENABLED_FOR_RELEASE = false` : les commandes
  renvoient l'état `disabled` **sans** appeler le plugin (pas de requête réseau,
  pas d'erreur de clé placeholder). L'activation exige la clé minisign réelle et
  des artefacts signés (Phase 9, voir `docs/rust/overlay-release.md`).

### 12.6 Harnais de développement hors bundle (A6-015)

- `bridge-test.html` et `README.md` ont été déplacés de `web/` vers
  `apps/lc-overlay/dev/` : `frontendDist` reste `web/`, donc le harnais n'est
  plus embarqué dans le binaire ni dans l'installeur.

