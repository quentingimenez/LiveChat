# LiveChat Extension (Chromium + Firefox)

Extension MV3 pour envoyer rapidement des liens média vers `POST /ingest` du serveur LiveChat.
Le code source est commun ; `scripts/build.mjs` produit deux paquets, `dist/chromium/` et
`dist/firefox/`, à partir de `manifests/base.json` et d'une surcouche par cible.

Documentation projet: [`../README.md`](../README.md)

## Projets liés (monorepo [LiveChat](https://github.com/quentingimenez/LiveChat))

- Extension navigateur (ce dossier): [`extension/`](.)
- Server (Discord + API): [`server/`](../server)
- Overlay desktop (Electron): [`overlay/`](../overlay)

## Fonctionnalités V1

- Bouton in-page d’envoi rapide:
  - YouTube (`watch`, `shorts`)
  - TikTok (page + feed)
  - X/Twitter (bouton sur chaque tweet détecté)
- Menu contextuel du navigateur:
  - `Envoyer rapidement vers LiveChat`
  - `Envoyer vers LiveChat avec texte`
- Popup d’envoi avec texte (`url`, `text`, `forceRefresh`)
- Page d’options:
  - appairage via code Discord (`/pair-code`)
  - `API_URL`
  - `INGEST_API_TOKEN`
  - `guildId`
  - `authorName`
  - `authorImage`
- Permissions domaine API dynamiques (`browser.permissions`).

## Pré-requis

- Serveur LiveChat lancé avec `/ingest` activé (`INGEST_API_TOKEN` non vide).
- `API_URL` joignable en `https://` (le jeton n'est jamais envoyé en clair).

## Développement

Depuis `extension/`:

```bash
npm install
npm run typecheck
npm run test
npm run build          # les deux cibles
npm run build:firefox  # dist/firefox seulement
```

Les content scripts sont bundlés chacun en IIFE autonome (`scripts/build.mjs`): Chrome les exécute comme des scripts classiques, donc un `import`/`export` en tête de fichier les fait rejeter (« Cannot use import statement outside a module ») et plus aucun bouton n'est injecté. `npm run verify:dist` — inclus dans `ci:check` et `release:build` — vérifie les deux manifestes et échoue si un content script du build redevient un module ES.

Le build Firefox n'est pas minifié : AMO exige sinon une archive de sources séparée. Chromium garde la minification par défaut de Vite.

## Build propre et package

```bash
npm run release:build
```

Ce script:
- supprime `dist` et `release`,
- reconstruit l’extension pour Chromium et Firefox,
- crée `release/livechat-extension-chromium-v<version>.zip` et
  `release/livechat-extension-firefox-v<version>.zip`.

Note: les zips sont générés via `python3` (`scripts/package.py`).

Commandes utiles:
- `npm run clean`
- `npm run build:clean`
- `npm run build:chromium` / `npm run build:firefox`
- `npm run dev` (rebuild en continu des deux cibles)
- `npm run verify:dist`
- `npm run lint:amo` (web-ext lint sur `dist/firefox`)
- `npm run package`
- `npm run ci:check`

## Charger dans Chrome/Chromium

1. Exécuter `npm run build:chromium`.
2. Ouvrir `chrome://extensions`.
3. Activer `Mode développeur`.
4. Cliquer `Charger l’extension non empaquetée`.
5. Sélectionner le dossier `dist/chromium`.

## Charger dans Firefox

1. Exécuter `npm run build:firefox`.
2. Ouvrir `about:debugging#/runtime/this-firefox`.
3. Cliquer `Charger un module temporaire`.
4. Sélectionner `dist/firefox/manifest.json`.

Firefox exige un add-on signé par Mozilla pour une installation permanente : le
chargement temporaire sert au développement, la version publiée passe par AMO
(voir [`docs/store/amo.md`](../docs/store/amo.md)).

## Configuration

1. Ouvrir la page d’options de l’extension.
2. Saisir **obligatoirement** `API_URL` (URL racine du serveur), par exemple `https://livechat.exemple.com`.
3. Ne pas mettre `/overlay` ni `/ingest` dans `API_URL`.
4. Générer un code avec `/pair-code`, puis cliquer `Récupérer la configuration`.
5. Vérifier les champs récupérés (`INGEST_API_TOKEN`, `guildId`, `authorName`, `authorImage`) puis sauvegarder si besoin.
6. Accepter la permission domaine demandée pour l’URL API.
7. (Optionnel) Cliquer `Tester la config`.

## Contrat `/ingest` utilisé

- `POST ${API_URL}/ingest`
- Header: `Authorization: Bearer <INGEST_API_TOKEN>`
- Payload: `{ guildId, url, text?, forceRefresh?, saveToBoard?, durationSec? }`
- `authorName`/`authorImage` restent dans `browser.storage.local` et ne sont pas envoyés par l’extension.

## Limites connues

- Le bouton injecté dépend des structures DOM des plateformes (susceptibles d’évoluer).
- Navigateurs supportés : Chromium ≥ 148 (namespace `browser` avec promesses) et Firefox ≥ 140.
- Certificats TLS invalides/self-signed non supportés côté extension.

### Jeton d'ingestion et stockage local — décision C-03

`INGEST_API_TOKEN` est sauvegardé en clair dans `browser.storage.local` avec la
configuration. Au démarrage du service worker, l'extension appelle
`setAccessLevel({ accessLevel: 'TRUSTED_CONTEXTS' })` sur cette area (Chrome ≥ 102)
pour la rendre illisible depuis les content scripts injectés sur les pages tierces.
L'API est absente sur Firefox et sur Chrome < 102 : l'appel est alors un no-op
documenté, sans erreur. Les content scripts n'accèdent plus au stockage : la
préférence de thème leur est fournie par messages runtime.

**Limite technique assumée, pas une acceptation de confort :** ce n'est pas du
chiffrement. `storage.local` reste en clair dans le profil du navigateur
(sauvegarde de profil, synchro, autre processus local). Chiffrer le jeton avec
une clé conservée dans la même extension ne protégerait de rien : en MV3, sans
keystore OS ni native messaging dans cette extension, toute clé stockée à côté
du chiffré est lisible par le même adversaire. Le backlog autorise explicitement
« chiffrer **ou** documenter l'acceptation » ; la réduction de surface above
est le correctif réel, la limitation de stockage est documentée avec sa preuve :
la [documentation Chrome de `StorageArea.setAccessLevel`](https://developer.chrome.com/docs/extensions/reference/api/storage/StorageArea#method-setAccessLevel)
indique que `local` autorise par défaut les contextes fiables *et* non fiables,
et que `TRUSTED_CONTEXTS` restreint l'accès aux contextes de l'extension.

Le jeton n'est jamais journalisé : sur le chemin d'erreur d'`ingestClient`, les
messages d'erreur réseau sont filtrés (`lib/redact.ts`) avant d'être exposés,
et le champ « Tester la config » des options applique le même masquage.

**Suivi :** la protection dépend du niveau d'accès fourni par le navigateur ;
si un moteur expose un stockage sécurisé (keystore OS ou native messaging), le
chiffrement redevient envisageable et cette note devra être révisée.

### Durée de vie du jeton d'ingestion

Comme pour le raccourci iOS, le serveur refuse le jeton d'ingestion de
l'extension après **180 jours sans activité** : la requête échoue alors avec
`ingest_token_expired`. Tant que l'extension envoie régulièrement du contenu,
chaque envoi remet le compteur à zéro (aucune date d'expiration n'est stockée).
Un jeton déjà expiré ne peut pas être réactivé : génère un nouveau code
d'appairage (`/pair-code`) depuis la board, puis récupère la configuration dans
les options de l'extension pour obtenir un nouveau jeton. Le jeton reste stocké
en clair dans le profil comme décrit ci-dessus ; cette limite d'inactivité
s'applique côté serveur et ne modifie pas ce mode de stockage.

## CI / Release GitHub

Workflow: [`extension.yml`](../.github/workflows/extension.yml) (racine du monorepo)

- Sur `pull_request` et `push` sur `main` touchant `extension/`: checks + build des deux cibles + artifact zip.
- Sur `push` sur `main`: mise à jour de la prerelease `extension-latest` avec les zips.
- Sur tag `extension-v*` (ex: `extension-v0.1.0`): création automatique d’une GitHub Release avec les zips, puis envoi du paquet Chromium au Chrome Web Store (`publish-webstore`) et du paquet Firefox à AMO (`publish-amo`).
- Manuellement (`workflow_dispatch`): relancer le build, et cocher `publish_to_webstore` et/ou `publish_to_amo` pour envoyer la version du dépôt sans poser de nouveau tag.

Les releases de l’extension ne sont jamais marquées *Latest* (réservé à l’overlay, dont l’auto-update lit la release *Latest* du monorepo).

Exemple publication:

```bash
git tag extension-v0.1.0
git push origin extension-v0.1.0
```

## Auto-update

- **Chromium** : le Chrome Web Store met à jour l’extension installée après
  approbation ; la CI envoie le paquet et soumet la version (API v2,
  `media:upload` puis `publishers.items:publish`).
- **Firefox** : AMO met à jour les installations listées après approbation ; il
  n’y a pas d’`update_url` dans le manifeste, AMO l’interdit pour un add-on
  hébergé par Mozilla. Le job `publish-amo` signe et soumet à chaque tag.

### Secrets GitHub requis (Chromium)

Configurer ces secrets dans `Settings > Secrets and variables > Actions`:

- `CWS_PUBLISHER_ID`: ID éditeur (premier segment de l’URL de la console, `devconsole/<publisherId>/...`).
- `CWS_EXTENSION_ID`: ID de l’article Chrome Web Store (32 caractères, dans l’URL de l’article).
- `CWS_CLIENT_ID`: OAuth Client ID Google Cloud.
- `CWS_CLIENT_SECRET`: OAuth Client Secret Google Cloud.
- `CWS_REFRESH_TOKEN`: refresh token OAuth avec scope Chrome Web Store.

### Secrets GitHub requis (Firefox / AMO)

- `AMO_JWT_ISSUER`: issuer JWT de la clé API AMO.
- `AMO_JWT_SECRET`: secret JWT de la clé API AMO.

Les générer sur <https://addons.mozilla.org/developers/addon/api/key/>. La
procédure complète (première soumission, métadonnées, licence, revue) est dans
[`docs/store/amo.md`](../docs/store/amo.md).

### Préparation Google Cloud / Web Store

1. Créer un projet Google Cloud.
2. Activer l’API Chrome Web Store.
3. Créer des credentials OAuth 2.0 (type Desktop ou Web).
4. Générer un refresh token OAuth avec le scope `https://www.googleapis.com/auth/chromewebstore` (le plus simple : OAuth Playground avec ses propres identifiants, ou `gcloud auth application-default` puis l’échange `grant_type=refresh_token`).
5. Renseigner les 5 secrets dans GitHub.

### Mise à jour de l’article (fiche store)

L’API ne touche que le paquet : logo, captures, description, catégorie et champs de
confidentialité se modifient à la main dans la
[console développeur](https://chrome.google.com/webstore/devconsole). Les textes et
visuels prêts à copier sont dans [`docs/store/`](../docs/store/).

