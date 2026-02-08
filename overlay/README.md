# LiveChat Overlay (EXE)

Client desktop Electron pour recevoir et afficher les médias LiveChat.

Documentation projet: `../README.md`
PRD: `../PRD.md`

## Rôle

- Appairage via code one-shot (`/overlay-code`).
- Connexion socket authentifiée.
- Lecture média locale sans contrôles visibles.
- Options tray: écran, volume, affichage texte, reset pairing.

## Démarrage rapide

```bash
npm install
npm start
```

## Build

### Windows (.exe portable)

```bash
npm run dist -- --win portable
```

### macOS (.dmg + .zip)

```bash
npm run dist -- --mac dmg zip
```

## CI/CD nightly

Le workflow GitHub `nightly.yml` build et publie automatiquement:
- Windows: `.exe`
- macOS: `.dmg` et `.zip`
