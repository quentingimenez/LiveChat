# LiveChatCaCaBox

Backend Discord/API pour l'architecture LiveChat **EXE-only**.

Documentation projet: `../README.md`
PRD: `../PRD.md`

## Rôle

- Reçoit les commandes Discord.
- Télécharge et normalise les médias (yt-dlp + ffmpeg).
- Met en cache les assets (TTL 12h).
- Expose les routes `/overlay/*`.
- Diffuse les événements socket vers les overlays appairés.

## Démarrage rapide

```bash
cp .env.example .env
pnpm install
pnpm dev
```
