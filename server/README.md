# LiveChatCaCaBox

Backend Discord/API pour l'architecture LiveChat **EXE-only**.

Documentation projet: `../README.md`
PRD: `../PRD.md`

## Rôle

- Reçoit les commandes Discord.
- Télécharge et normalise les médias (yt-dlp + ffmpeg).
- Met en cache les assets (TTL 12h).
- Expose les routes `/overlay/*`.
- Expose la route `/ingest` (API simple pour raccourcis iOS).
- Diffuse les événements socket vers les overlays appairés.

## Démarrage rapide

```bash
cp .env.example .env
pnpm install
pnpm dev
```

## API iOS (`/ingest`)

Configurer `INGEST_API_TOKEN` dans `.env`, puis appeler:

```bash
curl -X POST "$API_URL/ingest" \
  -H "Authorization: Bearer $INGEST_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "guildId": "123456789012345678",
    "url": "https://vm.tiktok.com/ZNRfPWMaF/",
    "text": "hello depuis iOS"
  }'
```

Payload minimal:
- `guildId` obligatoire
- au moins un de `url`, `media`, `text`

## Normalisation du volume

La normalisation loudness est activée par défaut pendant la transcode:
- `MEDIA_AUDIO_NORMALIZE_ENABLED=true`
- `MEDIA_AUDIO_LOUDNORM_I=-16`
- `MEDIA_AUDIO_LOUDNORM_LRA=11`
- `MEDIA_AUDIO_LOUDNORM_TP=-1.5`

## Endpoints publics

- `GET /` -> statut service minimal
- `GET /health` -> healthcheck

Les erreurs HTTP sont volontairement normalisées (`not_found`, `request_error`, `internal_error`) sans stack trace exposée.

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
