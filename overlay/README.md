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

## macOS: blocage "application endommagée"

Si tu télécharges une nightly non signée/non notarized, Gatekeeper peut afficher
`"Overlay-Client est endommagé..."`.

Déblocage local (temporaire):

```bash
xattr -dr com.apple.quarantine "/Applications/Overlay-Client.app"
```

Pour un build qui s'ouvre sans cette manipulation, configure les secrets GitHub:
`CSC_LINK`, `CSC_KEY_PASSWORD`, `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID`.
