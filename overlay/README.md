# LiveChat Overlay (EXE)

Client desktop Electron pour recevoir et afficher les médias LiveChat.

Documentation projet: [`../README.md`](../README.md)

## Projets liés (monorepo [LiveChat](https://github.com/quentingimenez/LiveChat))

- Overlay desktop (ce dossier): [`overlay/`](.)
- Server (Discord + API): [`server/`](../server)
- Extension navigateur (MV3): [`extension/`](../extension)

## Rôle

- Appairage via code one-shot (`/pair-code`).
- Connexion socket authentifiée.
- Lecture média locale sans contrôles visibles.
- Options tray: activation overlay, lancement au démarrage, appairage overlay, écran, volume, affichage texte.

## Démarrage rapide

Depuis `overlay/`:

```bash
npm install
npm start
```

## Build

Pré-requis Windows:
- Build local `.exe` recommandé sur Windows.
- Sur Linux/macOS, `electron-builder --win` nécessite un environnement compatible (`wine`).

### Windows (.exe setup + portable)

```bash
npm run package:win
```

Validation locale des artefacts Windows (setup + portable + latest.yml):

```bash
npm run verify:win:local
```

Setup only:

```bash
npm run package:win:setup
```

Portable only:

```bash
npm run package:win:portable
```

### macOS (.dmg + .zip)

```bash
npm run dist -- --mac dmg zip
```

## CI/CD stable (auto-update)

Le workflow [`overlay-release.yml`](../.github/workflows/overlay-release.yml) (racine du monorepo) build et publie automatiquement sur les tags `overlay-vX.Y` ou `overlay-vX.Y.Z`. `package.json` reste en `X.Y.Z` (ex: tag `overlay-v2.0` <-> version `2.0.0`) :
- Windows: `setup .exe` (NSIS) + `portable .exe`
- Métadonnées update: `latest.yml` (+ `*.blockmap` si généré)

```bash
git tag overlay-v2.0.2
git push origin overlay-v2.0.2
```

L'auto-update (`electron-updater`, provider GitHub `quentingimenez/LiveChat`) lit la release marquée **Latest** du monorepo.
Seules les releases stables de l'overlay sont marquées *Latest*: les releases du serveur et de l'extension
sont toujours publiées avec `latest=false` (ou en prerelease) pour ne pas casser l'auto-update.

## CI/CD nightly

Le workflow [`overlay-nightly.yml`](../.github/workflows/overlay-nightly.yml) build et publie automatiquement (push sur `main` touchant `overlay/`), dans la prerelease `overlay-nightly`:
- Windows: `.exe portable`

Le workflow [`overlay-ci.yml`](../.github/workflows/overlay-ci.yml) exécute `npm run verify` sur les pull requests touchant `overlay/`.

## macOS: warning de sécurité

Les nightlies macOS sont distribuées sans certificat Apple (pas de secrets requis).
Le comportement attendu est un warning Gatekeeper, puis ouverture manuelle:
clic droit sur l'app -> `Ouvrir` -> confirmer.

Si macOS affiche quand même `"LiveChat est endommagé..."`, retire la quarantaine:

```bash
xattr -dr com.apple.quarantine "/Applications/LiveChat.app"
```
