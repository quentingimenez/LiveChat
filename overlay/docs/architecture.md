# Architecture Overlay Client

## Objectif

Refactorisation incrémentale vers une structure TypeScript claire, sans casser le comportement existant.

Le runtime fonctionnel reste conservé via une couche de compatibilité, pendant la migration progressive des modules.

## Structure actuelle (post-refactor)

- `main.js`
  - Bootstrap Node/Electron minimal qui charge `dist/main/index.js`.
- `main.runtime.js`
  - Shim de compatibilité minimal qui délègue au runtime TypeScript compilé.
- `preload.js`
  - Bootstrap minimal qui charge `dist/preload/index.js`.
- `src/main`
  - Entrypoint TypeScript (`src/main/index.ts`) + modules main en cours de migration.
  - `app/runtime-orchestration-service.ts`: orchestration second-instance / startup / activate.
  - `app/runtime-app-events-service.ts`: enregistrement centralisé des hooks Electron app lifecycle.
  - `app/runtime-lifecycle-service.ts`: composition des handlers runtime (`ready`, `activate`, `second-instance`, `will-quit`).
  - `app/runtime-tray-service.ts`: lifecycle tray runtime paresseux (`initialize`, `create`, `update`).
  - `app/runtime-tray-bootstrap-service.ts`: bootstrap des callbacks tray (menu/actions/quit).
  - `app/runtime-core-facade-service.ts`: façade des accès config/display/peers/connection pour réduire le couplage runtime.
  - `app/runtime-core-services-factory.ts`: composition des services core runtime (config/display/peers/connection + auto-start/auto-update).
  - `app/runtime-auto-update-bootstrap-service.ts`: bootstrap auto-update (support portable/env + callbacks runtime).
  - `app/runtime-process-guard-service.ts`: guards process Electron (switches runtime + single-instance lock).
  - `app/runtime-legacy-entry.ts`: composition runtime principale (main process), appelée par le shim legacy.
  - `app/runtime-window-socket-service.ts`: composition fenêtres + renderer settings + socket overlay.
  - `app/runtime-interaction-service.ts`: composition des interactions runtime (meme bindings + actions overlay).
  - `app/runtime-bootstrap-service.ts`: bootstrap des enregistrements IPC + lifecycle applicatif.
  - `app/runtime-startup-hooks-service.ts`: composition des hooks startup (sync auto-start + enregistrement raccourcis runtime).
  - `app/runtime-window-actions-service.ts`: façade runtime pour les actions fenêtres (overlay/pairing/board) + garde-fous métier board.
  - `app/runtime-paths.ts`: résolution centralisée des chemins runtime (config/icon/preload/pages renderer).
  - `services/config-service.ts`: lecture/écriture config + normalisation.
  - `services/display-service.ts`: sélection écran cible + labels/clés display.
  - `services/overlay-peers-service.ts`: normalisation des peers connectés + construction tooltip tray.
  - `services/tray-menu-service.ts`: génération du template de menu tray (actions injectées).
  - `services/tray-volume-service.ts`: presets volume et items radio du menu tray.
  - `services/tray-context-menu-service.ts`: positionnement popup tray + ouverture/fermeture context menu.
  - `services/tray-lifecycle-service.ts`: état/lifecycle tray (create/update/events écran/clics).
  - `services/auto-start-service.ts`: gestion login-item/démarrage système.
  - `services/auto-start-runtime-service.ts`: adaptation runtime de l'auto-start (capabilities + persistance fallback).
  - `services/startup-auto-start-service.ts`: synchronisation préférence auto-start au boot app.
  - `services/shortcut-service.ts`: enregistrement des raccourcis globaux runtime.
  - `services/runtime-actions-service.ts`: actions runtime (enable/guest/display/volume/reset) orchestrées.
  - `services/renderer-settings-service.ts`: construction payload `overlay:settings` + broadcast aux renderers.
  - `services/overlay-config-ipc-service.ts`: projection typée de la config runtime vers la réponse IPC `overlay:get-config`.
  - `services/connection-state-service.ts`: état de connexion overlay + label/reason tray.
  - `services/socket-service.ts`: lifecycle socket (connect/reconnect/heartbeat/buffer flush).
  - `services/socket-listeners-service.ts`: enregistrement des listeners socket overlay (`connect/play/stop/peers/disconnect/error/reconnect`) et parsing payload.
  - `services/overlay-playback-forwarding-service.ts`: forwarding des événements renderer `overlay:error`, `overlay:playback-state`, `overlay:playback-stop` vers le socket (buffering/logging), consommé par les handlers IPC.
  - `services/meme-board-ipc-service.ts`: orchestration des actions IPC `meme-board:*` (normalisation bindings, apply strict/persist, trigger/stop).
  - `services/pairing-consume-service.ts`: orchestration métier du pairing (`pairing:consume`) avec fallback TLS, validation réponse, persistance config et transitions runtime.
  - `services/meme-bindings-service.ts`: shortcuts globaux + trigger/stop socket.
  - `services/auto-update-service.ts`: machine d'état auto-update au démarrage.
  - `infra/http-client.ts`: requêtes JSON + normalisation erreurs réseau/TLS.
  - `ipc/register-*.ts`: handlers IPC segmentés (overlay/meme-board, pairing).
  - `windows/window-factory.ts`: création des BrowserWindow isolée.
  - `windows/overlay-window-service.ts`: lifecycle fenêtre overlay (create/destroy/move/keep-on-top).
  - `windows/auxiliary-window-service.ts`: lifecycle fenêtres pairing/board.
- `src/preload`
  - API `contextBridge` typée (`window.livechatOverlay`).
- `src/renderer`
  - Entrypoints TypeScript par page (`overlay`, `board`, `pairing`).
  - `pairing` est migré en TS natif.
  - `board/legacy-utils.ts`: utilitaires purs extraits du legacy board.
  - `board/legacy-shortcut-utils.ts`: utilitaires purs de capture/rendu des raccourcis clavier board.
  - `board/legacy-bindings-utils.ts`: utilitaires purs des mappings raccourcis <-> items board.
  - `board/legacy-bindings-action-utils.ts`: orchestration métier des bindings board (sync avec items valides + clear raccourci).
  - `board/legacy-bootstrap-utils.ts`: bootstrap board (config appairage, bindings init, settings live, observer preview).
  - `board/legacy-bootstrap-orchestration-utils.ts`: orchestration runtime du bootstrap board (merge settings, disable controls, observer preview).
  - `board/legacy-capture-utils.ts`: transitions de capture clavier (pending/detected/cancelled) côté board.
  - `board/legacy-capture-flow-utils.ts`: orchestration UI de capture raccourci (état capture, commit, messages statut).
  - `board/legacy-capture-orchestration-utils.ts`: composition des handlers capture board (refresh/end/commit/begin) via `legacy-capture-flow-utils`.
  - `board/legacy-keyboard-utils.ts`: résolution des actions clavier dialogs (add/rename/delete) côté board.
  - `board/legacy-dialog-flow-utils.ts`: orchestration d’ouverture des dialogs board (fallback prompt/confirm + focus/reset champs).
  - `board/legacy-dialog-state-utils.ts`: gestion d'état générique des dialogs board (hide/close + resolver Promise).
  - `board/legacy-dialog-orchestration-utils.ts`: composition des handlers dialogs board (isReady/open/close add/rename/delete) via les utils dialog existants.
  - `board/legacy-search-utils.ts`: normalisation de recherche + orchestration debounce côté board.
  - `board/legacy-event-bindings-utils.ts`: câblage centralisé des listeners UI board (dialogs, recherche, boutons).
  - `board/legacy-dialog-utils.ts`: vérifications d'état UI dialog/capture + fermeture backdrop.
  - `board/legacy-item-action-utils.ts`: orchestration des actions UI item board (ajout lien, trigger, stop playback).
  - `board/legacy-item-actions-orchestration-utils.ts`: composition des handlers actions board (bindings + item actions + rename/delete).
  - `board/legacy-item-metadata-actions-utils.ts`: orchestration des mutations item board (rename + delete + cleanup raccourcis).
  - `board/legacy-item-utils.ts`: requêtes items board (fetch/create/patch/delete) + mise à jour locale des métadonnées.
  - `board/legacy-list-action-utils.ts`: callbacks d'actions liste board (sélection, trigger, bind capture, rename, delete).
  - `board/legacy-list-orchestration-utils.ts`: composition du rendu liste board (actions + rendu cards + refresh preview).
  - `board/legacy-load-search-utils.ts`: orchestration chargement items/recherche board (request race guard + debounce).
  - `board/legacy-load-search-orchestration-utils.ts`: composition des handlers chargement/recherche board (fetch/patch/load/search instantané).
  - `board/legacy-lifecycle-event-utils.ts`: binding des listeners globaux board (clavier capture/dialogs + hooks lifecycle renderer).
  - `board/legacy-list-render-utils.ts`: rendu de la liste des items board (empty state + cards + callbacks d'actions).
  - `board/legacy-render-utils.ts`: primitives DOM board (empty states, media preview, card item, meta preview).
  - `board/legacy-preview-controls-utils.ts`: construction DOM de l'éditeur message preview et des actions preview.
  - `board/legacy-preview-action-utils.ts`: callbacks d'actions preview (play/rename/bind/clear/delete) avec garde de sélection.
  - `board/legacy-preview-layout-utils.ts`: adaptation du layout preview (fit média + refresh des cibles `ResizeObserver`).
  - `board/legacy-preview-media-utils.ts`: utilitaires média preview (URL auth + application du volume perceptuel sur media nodes).
  - `board/legacy-preview-mount-utils.ts`: montage conditionnel du bloc preview (media + editor + controls) avec gestion du media key.
  - `board/legacy-preview-message-utils.ts`: orchestration autosave/flush des messages preview (anti-race + gestion in-flight).
  - `board/legacy-preview-message-orchestration-utils.ts`: composition des handlers preview message côté board (autosave/save/flush/lifecycle).
  - `board/legacy-preview-render-orchestration-utils.ts`: orchestration du rendu preview board (empty state, reset editor, montage media/controls, meta).
  - `board/legacy-preview-render-state-utils.ts`: états de rendu preview (empty state, reset editor, calcul media key/url).
  - `board/legacy-runtime-context-utils.ts`: helpers de contexte runtime board (URL auth, volume, sélection active, meta preview, checks capture UI).
  - `board/legacy-runtime-types.ts`: types runtime partagés board (`state`, `nodes`) pour la composition legacy.
  - `board/legacy-runtime-inputs-utils.ts`: initialisation centralisée des références DOM + état runtime board.
  - `board/legacy-runtime-foundation-utils.ts`: composition status/layout/context runtime board.
  - `board/legacy-runtime-dialog-handlers-utils.ts`: composition runtime des handlers dialogs board + backdrop close.
  - `board/legacy-runtime-preview-list-handlers-utils.ts`: composition runtime preview/list board (autosave message, rendu preview, rendu liste).
  - `board/legacy-runtime-item-load-handlers-utils.ts`: composition runtime load/search + item actions board.
  - `board/legacy-runtime-capture-handlers-utils.ts`: composition runtime capture raccourcis + helpers clavier board.
  - `board/legacy-runtime-lifecycle-utils.ts`: composition runtime wireup + bootstrap board.
  - `board/legacy-selection-utils.ts`: résolution de sélection active (preview) et réconciliation après refresh des items.
  - `board/legacy-status-utils.ts`: gestion centralisée du toast statut board (timers + variants + reset visuel).
  - `board/legacy-wireup-registration-utils.ts`: composition du registre d’interactions board (clavier, dialogs, capture, recherche) vers `wireBoardInteractions`.
  - `board/legacy-wireup-runtime-utils.ts`: composition runtime du câblage board (assemblage des dépendances vers `wireup-registration`).
  - `board/legacy-wireup-utils.ts`: câblage centralisé des listeners UI board (keyboard/lifecycle/dialogs/recherche/capture).
  - `overlay/legacy-utils.ts`: utilitaires purs extraits du legacy overlay.
  - `overlay/legacy-media-dom-utils.ts`: primitives DOM média overlay (create/attach element, volume, URL tokenisée).
  - `overlay/legacy-media-frame-utils.ts`: composition frame média overlay (header/content/footer, countdown parent, calcul shift header).
  - `overlay/legacy-media-frame-layout-utils.ts`: orchestration état/layout du media frame overlay (sync/schedule/ensure/reset).
  - `overlay/legacy-media-render-utils.ts`: orchestration du rendu média standalone overlay (create/attach/playback + événements média).
  - `overlay/legacy-play-flow-utils.ts`: orchestration du flow `onPlay` overlay (reset, rendu, erreurs, countdown, auto-clear).
  - `overlay/legacy-countdown-utils.ts`: orchestration countdown overlay (start/tick/pause/resume/clear + rendu du timer).
  - `overlay/legacy-playback-utils.ts`: logique playback/countdown pure (normalisation, dérivation state, tick, auto-clear).
  - `overlay/legacy-playback-session-utils.ts`: orchestration session playback overlay (report state/stop, sync timer, dérivation pause/play).
  - `overlay/legacy-bootstrap-utils.ts`: bootstrap overlay + application settings (merge config, listeners, ready signal).
  - `overlay/legacy-overlay-reset-utils.ts`: reset des couches overlay (media/text/countdown) + libération object URL.
  - `overlay/legacy-media-diagnostics.ts`: utilitaires diagnostics média (probe HTTP + logs d'erreurs/stall) extraits du legacy overlay.
  - `overlay/legacy-inline-video-utils.ts`: logique inline-video (autoplay retry, loop policy, fallback audio) extraite du legacy overlay.
  - `overlay/legacy-overlay-text-utils.ts`: rendu DOM des métadonnées auteur/texte (header/footer/layer) extrait du legacy overlay.
  - `overlay/legacy-media-offset-utils.ts`: application des offsets de lecture média (seek sécurisé après metadata).
  - `overlay/legacy-tweet-card-renderer.ts`: rendu DOM des tweet cards (widget + inline videos) extrait du legacy overlay.
  - `overlay/legacy-tweet-card-playback-utils.ts`: orchestration runtime des tweet cards (attach DOM, widgets, autoplay inline videos).
  - `overlay/legacy-tweet-card-utils.ts`: normalisation/étiquetage des données tweet card (inline videos + contexte réponse).
  - `overlay/legacy-twitter-widgets-utils.ts`: chargement/caching des widgets Twitter (script + disponibilité `twttr.widgets.load`).
  - `overlay/legacy-runtime-types.ts`: types runtime partagés overlay (`state`, `nodes`, `constants`).
  - `overlay/legacy-runtime-inputs-utils.ts`: initialisation centralisée DOM + état runtime overlay.
  - `overlay/legacy-runtime-media-frame-handlers-utils.ts`: composition runtime du frame média overlay (layout/schedule/ensure/reset).
  - `overlay/legacy-runtime-playback-countdown-handlers-utils.ts`: composition runtime playback + countdown + clear overlay.
  - `overlay/legacy-runtime-render-handlers-utils.ts`: composition runtime rendu média/tweet card + flow `onPlay`/`onSettings`.
  - `overlay` et `board` utilisent encore un chargeur legacy pendant la migration.
- `src/types/legacy-renderer-globals.d.ts`
  - Typage des helpers globaux injectés avant chargement legacy pour `board` et `overlay` (utils historiques + orchestrateurs runtime extraits: `runtime-inputs`, `runtime-foundation`, `runtime-dialog-handlers`, `runtime-preview-list-handlers`, `runtime-item-load-handlers`, `runtime-capture-handlers`, `runtime-lifecycle`).
- `src/shared`
  - Contrats IPC centralisés, types partagés, événements socket.
- `tests/main`
  - Tests unitaires et d'intégration légère sur services/main IPC (`display`, `overlay-peers`, `auto-start-runtime`, `startup-auto-start`, `socket-listeners`, `overlay-playback-forwarding`, `overlay-config-ipc`, `meme-board-ipc`, `pairing-consume`, `register-overlay-ipc-handlers`, `register-pairing-ipc-handler`, `runtime-paths`).
- `renderer/`
  - HTML/CSS + scripts legacy conservés pendant la migration.
  - `board.css` et `overlay.css`: feuilles d'index (`@import`) vers des modules découpés dans `renderer/styles/board/*` et `renderer/styles/overlay/*` pour garder des fichiers plus courts.

## Flux IPC

Source unique des canaux: `src/shared/ipc.ts`.

Flux principal:

1. Main expose des handlers/listeners IPC (`overlay:*`, `pairing:*`, `meme-board:*`).
2. Preload (`src/preload/index.ts`) mappe une API sûre via `contextBridge`.
3. Renderer consomme `window.livechatOverlay` sans accès Node direct.

Détail:
- `register-overlay-ipc-handlers.ts`: `overlay:get-config`, `overlay:*`, `meme-board:*`
- `register-pairing-ipc-handler.ts`: `pairing:consume`

Compatibilité:

- Noms des channels inchangés.
- Formats de payload inchangés.
- API publique preload inchangée.

## Note Packaging

- `package.json` déclare `packageManager: traversal@1.0.0` pour forcer `electron-builder` à utiliser son collecteur interne de dépendances (pas `npm list`), contournant un bug `npm@10` + `electron-builder@26` qui peut casser `npm run package -- --dir` avec `No JSON content found in output`.

## Règles de code

- Types stricts sur tout nouveau code TypeScript.
- Pas de `any` dans le nouveau code.
- Canaux IPC ajoutés/édités uniquement dans `src/shared/ipc.ts`.
- Logique métier hors preload; preload ne fait que bridge.
- Logging via `src/main/infra/logger.ts` injecté dans les services runtime (pas de `console.*` dispersé dans ces modules).

## Exceptions temporaires (legacy)

- Aucune exception active sur la taille des fichiers renderer legacy (`renderer/board.js` et `renderer/overlay.js` sont <=250 lignes).
- La migration reste incrémentale pour limiter le risque de régression comportementale.

## Ajouter une feature

1. Définir le contrat partagé dans `src/shared`:
   - channel IPC
   - types request/response/payload
2. Implémenter le handler côté main (service dédié, sans logique UI).
3. Exposer la méthode dans `src/preload/index.ts`.
4. Consommer dans renderer via `window.livechatOverlay`.
5. Vérifier:
   - `npm run lint`
   - `npm run check:lines`
   - `npm run typecheck`
   - `npm run build`
   - `npm run test:unit` (ou `npm run test:unit:build` si build main non fait)
   - `npm run package -- --dir`
   - Windows release artifacts (setup + portable): `npm run verify:win:local` (sur Windows, ou Linux avec `wine`)
