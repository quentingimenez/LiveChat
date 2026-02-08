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

## Docker Hub (GitHub Actions)

Le workflow `/.github/workflows/docker.yml` build et pousse l'image sur Docker Hub:
- sur push vers `main`
- sur tag `v*`
- en exécution manuelle (`workflow_dispatch`)

Secrets GitHub requis:
- `DOCKERHUB_USERNAME`
- `DOCKERHUB_TOKEN` (token d'accès Docker Hub)

Image publiée:
- `${DOCKERHUB_USERNAME}/livechat-bot:latest`
- `${DOCKERHUB_USERNAME}/livechat-bot:sha-<commit>`
