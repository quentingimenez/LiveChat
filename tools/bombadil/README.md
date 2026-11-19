# `tools/bombadil/` — exploration de l'interface par propriétés

[Bombadil](https://github.com/antithesishq/bombadil) (Antithesis, MIT) pilote un
Chromium : il clique, tape, fait défiler et navigue au hasard, et vérifie à
chaque état des propriétés écrites en TypeScript. Il complète les scénarios
Playwright de `apps/lc-admin-web/e2e` : il trouve les erreurs qui n'apparaissent
qu'après une suite d'actions qu'aucun scénario n'a prévue.

| Spec | Cible | Lanceur |
| --- | --- | --- |
| `specs/admin-console.ts` | console d'admin, contre le vrai binaire `lc-server` (base neuve, deux guildes créées, session ouverte par le secret) | `npm run admin` |
| `specs/board.ts` | board de l'overlay dans un navigateur, via le mock du shell Tauri (`apps/lc-overlay/dev/board-mock.html?media`) | `npm run board` |

Propriétés vérifiées :

- par défaut (Bombadil) : aucune exception non rattrapée, aucune promesse
  rejetée sans gestion, aucune erreur console, aucune réponse HTTP ≥ 400 ;
- `noLeakedValues` : jamais de `undefined`, `NaN` ou `[object Object]` à
  l'écran ;
- `noLastingBlankPage` : une page vide redevient non vide en 5 s au plus.

Sur la console, l'explorateur ne clique jamais « Se déconnecter » (l'écran de
connexion reçoit un 401 attendu). Sur le board, il déclenche aussi des
événements du mock : fin du média, coupure du WebSocket, redémarrage du
serveur, entrée tray « Paramètres… ».

## Lancer

Bombadil ne tourne que sous Linux et macOS (pas de binaire Windows) ; il pilote
le Chrome ou Chromium trouvé sur le `PATH`. En CI :

- à chaque changement : job `test (ubuntu-latest)` de `rust.yml`, 3 min par
  cible, pas encore bloquant, traces en artefact `bombadil-traces` ;
- chaque nuit (et à la demande, *Run workflow* avec la durée en minutes) :
  `bombadil-nightly.yml`, 20 min par cible, le board dans cinq modes du mock
  (`admin`, `member`, `guest`, `empty`, `removed`), avec les specs `*-soak.ts`.
  Les traces ne sont téléversées qu'en cas d'échec.

Les specs `*-soak.ts` ajoutent `noDomNodeLeak` et `noListenerLeak`
(`specs/leaks.ts`) : sur 2 minutes, le nombre de nœuds DOM ou d'écouteurs ne
doit pas exploser. Bombadil mesure sans forcer le ramasse-miettes, et la
croissance normale atteint déjà des milliers d'écouteurs en quelques secondes ;
les seuils, à environ trois fois le maximum observé, n'attrapent donc qu'une
fuite qui s'emballe.

```sh
npm ci
npm run typecheck
# Console : construire d'abord la console puis le serveur, qui l'embarque.
(cd ../../apps/lc-admin-web && npm ci && npm run build)
cargo build -p lc-server
LC_SERVER_BIN=../../target/debug/lc-server npm run admin
npm run board
```

Variables : `BOMBADIL_TIME_LIMIT` (`3m`), `BOMBADIL_NO_SANDBOX=1` (Chromium sans
sandbox, utile en conteneur ou en CI), `BOMBADIL_CMD` (commande de Bombadil en
tableau JSON, par exemple `["wsl.exe","-d","Ubuntu","--","bombadil"]`),
`LC_BOMBADIL_ADMIN_PORT` (39417), `LC_BOMBADIL_BOARD_PORT` (39418),
`LC_BOMBADIL_BOARD_QUERY` (options du mock, `media` ; par exemple
`role=guest&media`), `BOMBADIL_SPEC` et `BOMBADIL_OUTPUT` (spec et dossier de
sortie, par défaut ceux de la cible).

Le serveur de la console tourne en `NODE_ENV=development` : c'est ce qui
exempte les clients loopback des limites de débit, que l'explorateur, bien plus
rapide qu'une personne, atteint en moins d'une minute.

## Lire un échec

Le code de sortie vaut 2 quand une propriété est violée. La sortie donne la
propriété, l'instant et la commande de rejeu ; `out/<admin|board>/` contient
`trace.jsonl` et les captures d'écran :

```sh
npx bombadil browser inspect out/board
```

Le rejeu (`--reproduce`) est approximatif : il échoue si l'application diverge.
