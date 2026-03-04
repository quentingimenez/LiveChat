# LiveChat Overlay (EXE)

Client desktop Electron pour recevoir et afficher les médias LiveChat.

Documentation projet: `../README.md`
PRD: `../PRD.md`

## Projets liés (sources)

- Overlay desktop (ce dépôt): https://github.com/SuperKn4cky/LiveChat-Overlay
- Bot/API: https://github.com/WardenPro/LiveChat-Bot
- Extension navigateur (MV3): https://github.com/SuperKn4cky/LiveChat-Extension

## Rôle

- Appairage via code one-shot (`/pair-code`).
- Connexion socket authentifiée.
- Lecture média locale sans contrôles visibles.
- Options tray: activation overlay, lancement au démarrage, appairage overlay, écran, volume, affichage texte.

## Démarrage rapide

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

Le workflow GitHub `release.yml` build et publie automatiquement sur les tags de version (`X.Y` ou `X.Y.Z`, sans préfixe `v`) qui correspondent exactement à `package.json`:
- Windows: `setup .exe` (NSIS) + `portable .exe`
- Métadonnées update: `latest.yml` (+ `*.blockmap` si généré)

## CI/CD nightly

Le workflow GitHub `nightly.yml` build et publie automatiquement:
- Windows: `.exe portable`

## macOS: warning de sécurité

Les nightlies macOS sont distribuées sans certificat Apple (pas de secrets requis).
Le comportement attendu est un warning Gatekeeper, puis ouverture manuelle:
clic droit sur l'app -> `Ouvrir` -> confirmer.

Si macOS affiche quand même `"LiveChat est endommagé..."`, retire la quarantaine:

```bash
xattr -dr com.apple.quarantine "/Applications/LiveChat.app"
```
