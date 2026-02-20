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

## macOS: warning de sécurité

Les nightlies macOS sont distribuées sans certificat Apple (pas de secrets requis).
Le comportement attendu est un warning Gatekeeper, puis ouverture manuelle:
clic droit sur l'app -> `Ouvrir` -> confirmer.

Si macOS affiche quand même `"Overlay-Client est endommagé..."`, retire la quarantaine:

```bash
xattr -dr com.apple.quarantine "/Applications/Overlay-Client.app"
```
