# LiveChat Extension (Chromium)

Extension MV3 pour envoyer rapidement des liens média vers `POST /ingest` du serveur LiveChat.

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
- Menu contextuel Chromium:
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
- Permissions domaine API dynamiques (`chrome.permissions`).

## Pré-requis

- Serveur LiveChat lancé avec `/ingest` activé (`INGEST_API_TOKEN` non vide).
- `API_URL` joignable en `https://` (ou `http://` en local).

## Développement

Depuis `extension/`:

```bash
npm install
npm run typecheck
npm run test
npm run build
```

## Build propre et package

```bash
npm run release:build
```

Ce script:
- supprime `dist` et `release`,
- reconstruit l’extension,
- crée `release/livechat-extension-v<version>.zip`.

Note: le zip est généré via `python3` (`scripts/package.py`).

Commandes utiles:
- `npm run clean`
- `npm run build:clean`
- `npm run package`
- `npm run ci:check`

## Charger dans Chrome/Chromium

1. Exécuter `npm run build`.
2. Ouvrir `chrome://extensions`.
3. Activer `Mode développeur`.
4. Cliquer `Charger l’extension non empaquetée`.
5. Sélectionner le dossier `dist`.

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
- Payload rapide: `{ guildId, url, authorName, authorImage? }`
- Payload compose: `{ guildId, url, text, forceRefresh, authorName, authorImage? }`

## Limites connues

- Le bouton injecté dépend des structures DOM des plateformes (susceptibles d’évoluer).
- Pas de support Firefox dans cette V1.
- Certificats TLS invalides/self-signed non supportés côté extension.

## CI / Release GitHub

Workflow: [`extension.yml`](../.github/workflows/extension.yml) (racine du monorepo)

- Sur `pull_request` et `push` sur `main` touchant `extension/`: checks + build + artifact zip.
- Sur `push` sur `main`: mise à jour de la prerelease `extension-latest` avec le zip.
- Sur tag `extension-v*` (ex: `extension-v0.1.0`): création automatique d’une GitHub Release avec le zip.
- Sur tag `extension-v*` (ex: `extension-v0.1.0`): publication automatique sur Chrome Web Store (upload + publish) si les secrets sont configurés.

Les releases de l’extension ne sont jamais marquées *Latest* (réservé à l’overlay, dont l’auto-update lit la release *Latest* du monorepo).

Exemple publication:

```bash
git tag extension-v0.1.0
git push origin extension-v0.1.0
```

## Auto-update via Chrome Web Store (basé sur GitHub)

L’auto-update des utilisateurs finaux passe par le Chrome Web Store.
Le monorepo publie automatiquement l’extension lors d’un push de tag `extension-v*`.

### Secrets GitHub requis

Configurer ces secrets dans `Settings > Secrets and variables > Actions`:

- `CWS_EXTENSION_ID`: ID de l’extension Chrome Web Store.
- `CWS_CLIENT_ID`: OAuth Client ID Google Cloud.
- `CWS_CLIENT_SECRET`: OAuth Client Secret Google Cloud.
- `CWS_REFRESH_TOKEN`: refresh token OAuth avec scope Chrome Web Store.

### Préparation Google Cloud / Web Store

1. Créer un projet Google Cloud.
2. Activer l’API Chrome Web Store.
3. Créer des credentials OAuth 2.0 (type Desktop ou Web).
4. Générer un refresh token OAuth avec le scope:
   `https://www.googleapis.com/auth/chromewebstore`
5. Renseigner les secrets dans GitHub.

Ensuite, chaque tag `extension-v*` poussé sur GitHub déclenche:
- build + zip,
- upload GitHub Release,
- upload vers Chrome Web Store,
- publish de la nouvelle version.
