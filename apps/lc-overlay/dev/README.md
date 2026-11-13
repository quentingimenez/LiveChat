# `apps/lc-overlay/dev/` — harnais de développement du bridge (hors bundle)

> **AUD-A6-015** : ce dossier n'est **pas** inclus dans `frontendDist` (`web/`).
> Le harnais `bridge-test.html` et ce README servent au développement local et ne
> sont plus embarqués dans l'application ni dans l'installeur release.

Ce dossier contient les outils de développement du bridge Tauri 2 ; les pages
HTML, le CSS et le JS du renderer Electron existant sont dans
`apps/lc-overlay/web/` et un fichier **`bridge.js`** y réinstalle l'API publique
`window.livechatOverlay` que ces pages attendent.

L'app Rust Tauri 2 (`apps/lc-overlay/src/`) est complète depuis la Phase 8 : les
pages réelles, les styles et les shims du renderer Electron ont été portés dans
`apps/lc-overlay/web/` (cf. `docs/rust/lc-overlay-shell.md` et
`docs/rust/overlay-bridge.md`). Ce dossier `dev/` ne contient que le harnais de
test manuel et sa documentation.

## Contenu actuel

| Fichier | Rôle |
| --- | --- |
| `bridge.js` | Script classique autonome (aucune dépendance, aucun bundler) qui installe `window.livechatOverlay` (13 méthodes legacy + `authorizedFetch`, soit 14 au total) au-dessus des commandes/événements Tauri 2, avec un mode **mock** si Tauri est absent. |
| `dev/bridge-test.html` | Harnais navigateur autonome : charge `web/bridge.js`, exerce les 13 méthodes legacy, simule les 3 événements, et affiche le journal des appels. |
| `README.md` | Ce document. |

Contenu actuel de `apps/lc-overlay/web/` (embarqué dans le binaire) :

```
web/
├── bridge.js                 <- API window.livechatOverlay (hors de ce dossier dev)
├── overlay.html  overlay.css  overlay.js   <- rendu des médias à l'écran
├── board.html                <- board (guildes, médias, file, admin, appairage)
├── app/                      <- application de la board, modules ES sans build
│   ├── main.js  data.js  app.css
│   ├── core/                 <- ipc, api, état, DOM, notifications
│   ├── views/                <- une vue par fonctionnalité
│   └── styles/               <- une feuille par fonctionnalité
├── renderer-errors.js        <- télémétrie d'erreurs du renderer
├── favicon.ico
├── styles/overlay/*.css
└── dist/renderer/overlay/index.js   <- shim ESM porté
    └── legacy-loader.js + utils (dont twitter-widgets neutralisé)
```

## Intégration dans une page

`bridge.js` doit être chargé comme **script classique**, avant le module de page, de préférence dans le `<head>` :

```html
<script src="./bridge.js"></script>
...
<script type="module" src="../dist/renderer/overlay/index.js"></script>
```

Règles à respecter :

1. **Ne pas** ajouter `type="module"` à `bridge.js` : il doit s'exécuter pendant l'analyse du document, avant les modules
   différés. C'est ce qui garantit que `window.livechatOverlay` existe quand le bootstrap de la page démarre.
2. Ne pas utiliser `defer`/`async` : l'ordre de chargement importe.
3. Le bridge est **idempotent** : si `window.livechatOverlay` existe déjà, il se réinstalle par-dessus (utile en dev avec
   rechargement de HMR, et sans risque en production). Il ne doit y avoir qu'un seul `bridge.js` chargé par page.
4. Aucun `require`, aucun import : le renderer n'a besoin ni de Node ni de bundler, comme aujourd'hui en Electron.

## Modes de fonctionnement

Le bridge choisit son transport au chargement :

| Mode | Détection | Comportement |
| --- | --- | --- |
| `tauri` / transport `internals-invoke` | `window.__TAURI_INTERNALS__?.invoke` présent (toujours le cas en Tauri 2) | Chaque méthode appelle `invoke(commande, { payload })` ou écoute un événement Tauri. |
| `tauri` / transport `global-invoke` | `window.__TAURI_INTERNALS__` absent mais `window.__TAURI__.core.invoke` présent (`app.withGlobalTauri: true`) | Même chose via l'API globale `__TAURI__`. |
| `mock` | Aucun des deux | Aucun réseau, aucun backend : valeurs neutres, événements simulables, journal console. |

L'état est exposé **uniquement en mode mock** (navigateur / ce harnais) ou si la
page pose `window.__LIVECHAT_BRIDGE_DEBUG = true` **avant** de charger
`bridge.js`. En mode Tauri release, ces globals n'existent pas (AUD-A6-010 /
O-007) :

```js
window.__livechatOverlayBridge.mode;          // 'tauri' | 'mock'
window.__livechatOverlayBridge.transport;     // 'internals-invoke' | 'global-invoke' | 'mock'
window.__livechatOverlayBridge.hasNativeEvents; // true si listen/unlisten natifs disponibles
window.__bridgeCalls;                          // journal des appels (max 200)
window.__bridgeLastEvent;                      // dernier événement reçu { event, payload, at }
window.__bridgeMockEmit(event, payload);       // émet un événement vers les listeners locaux
```

### Configuration Tauri actuelle

- `app.withGlobalTauri: false` (P2-E) : le bridge utilise
  `window.__TAURI_INTERNALS__.invoke` + le repli `plugin:event|listen` /
  `transformCallback`. Aucun contenu de page n'a accès à `window.__TAURI__`.
- La permission `core:event:default` (listen/unlisten/emit) est activée par défaut : aucune capability supplémentaire n'est
  nécessaire pour les événements. Les commandes applicatives ne sont pas soumises à l'ACL Tauri.

## Tester dans un navigateur (mode mock)

1. Ouvrir `apps/lc-overlay/dev/bridge-test.html` directement (double-clic, `file://`) ou via un petit serveur statique
   (`python -m http.server`, `npx serve`, etc. — utile pour tester le comportement en contexte `http`).
2. Vérifier les badges : `mode: mock`, `transport: mock`, `API: 13/13 méthodes` (les 13 méthodes legacy ;
   `authorizedFetch` n'est pas exercée par le harnais car elle nécessite un backend).
3. Exécuter les 10 commandes, s'abonner aux 3 événements, cliquer sur « Simuler `overlay://play|stop|settings` »,
   puis sur **« Tout tester (13 méthodes) »** : le rapport doit afficher `13/13 méthodes exercées — tout est OK`.
4. Le journal du bas montre chaque appel (méthode, statut, argument masqué, résultat, horodatage) ; il se rafraîchit tout
   seul et peut être vidé.

Le harnais n'est pas un runner de tests automatisé : c'est un outil manuel de smoke-test du bridge, utilisable sans
backend ni WebView2. `node --check apps/lc-overlay/web/bridge.js` valide la syntaxe du script.

### Utiliser `__bridgeMockEmit` dans les pages

En mode mock (ou pour tester un listener sans backend), les noms d'événements acceptés sont indifféremment
`play`/`stop`/`settings`, `overlay:play`/… ou `overlay://play`/… ; le second argument est le payload transmis tel quel :

```js
const off = window.livechatOverlay.onPlay((payload) => console.log(payload));
window.__bridgeMockEmit('overlay://play', { jobId: 'demo', durationSec: 5 });
window.__bridgeMockEmit('play', { jobId: 'demo' }); // même effet
off();
```

`__bridgeMockEmit` renvoie le nombre de listeners locaux notifiés (0 si aucun abonnement).

## Mode mock en détail

| Méthode | Valeur simulée |
| --- | --- |
| `getConfig()` | `{ serverUrl: null, clientToken: null, guildId: null, clientId: null, guestMode: false, showText: true, volume: 1, mediaBindings: {} }` |
| `rendererReady()` | No-op ; planifie l'émission `overlay://settings` (comme le fait le main après le handshake). |
| `reportError` / `reportPlaybackState` / `reportPlaybackStop` | No-op, journalisés dans le journal des appels. |
| `consumePairing()` | `{ ok: true }` — **aucune requête réseau** n'est faite. |
| `getMediaBindings()` | `{ bindings: {} }` |
| `setMediaBindings(payload)` | `{ ok: true, bindings: <payload normalisé>, failedAccelerators: [] }` |
| `triggerMedia()` / `stopMediaPlayback()` | `{ ok: true }` |

Rien n'est persisté : le mock ne lit ni n'écrit de configuration, ne contacte pas le serveur et ne crée pas de fenêtre.

## API exposée (contrat inchangé)

Les 13 méthodes legacy, signatures et payloads sont identiques à `overlay/src/preload/index.ts` ; seul le transport change.
Depuis P1-11, une 14<sup>e</sup> méthode additive, `authorizedFetch(url, options)`, est exposée pour l'authentification
Bearer du board (voir `docs/rust/overlay-bridge.md` §12.4). Tableau complet, canaux, handlers et sémantique :
**`docs/rust/overlay-bridge.md`**.

| Méthode | Forme |
| --- | --- |
| `getConfig()` | `Promise<OverlayConfig>` |
| `rendererReady()` | fire-and-forget |
| `onPlay(cb)` / `onStop(cb)` / `onSettings(cb)` | retourne une fonction de désabonnement |
| `reportError(p)` / `reportPlaybackState(p)` / `reportPlaybackStop(p)` | fire-and-forget |
| `consumePairing(p)` | `Promise<{ ok: boolean }>` |
| `getMediaBindings()` / `setMediaBindings(p)` | `Promise<…>` |
| `triggerMedia(p)` / `stopMediaPlayback()` | `Promise<{ ok, reason? }>` |

Les méthodes fire-and-forget renvoient une `Promise` **toujours résolue** (échec journalisé en `console.warn`) afin de ne
jamais produire de rejet non géré dans le renderer legacy, qui ignore leur retour. Les méthodes de type `invoke`
propagent l'erreur normalisée en `Error` (avec `.message` exploité par les pages, ex. appairage).

## Sécurité et hygiène

- Le journal des appels (`window.__bridgeCalls`) masque récursivement les clés sensibles (`token`, `code`, `sig`,
  `secret`, `password`, insensible à la casse) et tronque les chaînes de plus de 300 caractères.
- Le mode mock ne doit jamais être considéré comme une validation d'intégration : il ne teste pas les commandes Rust.
- Aucun secret n'est inclus dans ce dossier ni dans le harnais ; les payloads d'exemple sont fictifs.

## Limites connues

- Le harnais est un outil manuel de smoke-test : il n'exerce pas le backend Rust (commandes, WebSocket, appairage) et ne
  remplace pas les tests `cargo test -p lc-overlay`.
- `authorizedFetch` et les appels réseau du board ne sont pas testables en mode mock (pas de backend) : utiliser l'app
  réelle avec un serveur local pour ces parcours.
- Les diagnostics du bridge (`__bridgeCalls`, `__bridgeMockEmit`, etc.) ne sont
  exposés qu'en mode mock ou via `window.__LIVECHAT_BRIDGE_DEBUG = true` posé
  avant `bridge.js` : un installeur release n'expose rien (P2-E / O-007).
- `bridge-test.html` n'est pas exécuté en CI : les tests Rust couvrent la matrice IPC et le bridge est vérifié par
  `node --check`.

## Vérifications

```powershell
node --check apps/lc-overlay/web/bridge.js
```

Le harnais se vérifie à l'œil dans un navigateur, ou via le bouton « Tout tester (13 méthodes) » (méthodes legacy).
