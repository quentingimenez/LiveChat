# LiveChat Extension (Chromium)

Extension MV3 pour envoyer rapidement des liens média vers `POST /ingest` du bot LiveChat.

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
  - appairage via code Discord (`/overlay-code`)
  - `API_URL`
  - `INGEST_API_TOKEN`
  - `guildId`
  - `authorName`
  - `authorImage`
- Permissions domaine API dynamiques (`chrome.permissions`).

## Pré-requis

- Bot LiveChat lancé avec `/ingest` activé (`INGEST_API_TOKEN` non vide).
- `API_URL` joignable en `https://` (ou `http://` en local).

## Développement

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
2. Saisir `API_URL`, puis un code généré par `/overlay-code`.
3. Cliquer `Récupérer la configuration`.
4. Vérifier les champs récupérés (`INGEST_API_TOKEN`, `guildId`, `authorName`, `authorImage`) puis sauvegarder si besoin.
5. Accepter la permission domaine demandée pour l’URL API.
6. (Optionnel) Cliquer `Tester la config`.

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

Workflow: `.github/workflows/extension-release.yml`

- Sur `pull_request` et `push` sur `main`: checks + build + artifact zip.
- Sur tag `v*` (ex: `v0.1.0`): création automatique d’une GitHub Release avec le zip.

Exemple publication:

```bash
git tag v0.1.0
git push origin v0.1.0
```
