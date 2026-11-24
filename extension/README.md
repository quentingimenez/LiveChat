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
  - appairage via code Discord (`/pair-code`) : l’adresse du serveur et le code suffisent
  - `INGEST_API_TOKEN` et `guildId` restent saisissables dans « Configuration manuelle » (replié)
  - aucun champ auteur : le pseudo et l’avatar viennent du profil du membre lié au client côté serveur
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
2. Saisir l’**adresse du serveur** (URL racine), par exemple `https://livechat.exemple.com` — sans `/overlay` ni `/ingest`.
3. Générer un code avec `/pair-code` (ou Overlay → Mon profil → Appairer l’extension navigateur), le saisir puis cliquer `Connecter` : l’extension récupère le jeton et la guilde du serveur.
4. Accepter la permission domaine demandée pour l’adresse du serveur.
5. Si aucun code n’est disponible, ouvrir `Configuration manuelle` et saisir `INGEST_API_TOKEN` et `guildId`, puis cliquer `Enregistrer` (`Tester` vérifie la connexion).

## Contrat `/ingest` utilisé

- `POST ${API_URL}/ingest`
- Header: `Authorization: Bearer <INGEST_API_TOKEN>`
- Payload: `{ guildId, url, text?, forceRefresh?, saveToBoard?, durationSec? }`
- Aucun champ auteur : le serveur résout pseudo et avatar à chaque envoi depuis le profil du membre propriétaire du client d’ingestion (repli sur l’instantané d’appairage côté serveur).

## Limites connues

- Le bouton injecté dépend des structures DOM des plateformes (susceptibles d’évoluer).
- Navigateurs supportés : Chromium ≥ 148 (namespace `browser` avec promesses) et Firefox ≥ 140.
- Certificats TLS invalides/self-signed non supportés côté extension.

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

