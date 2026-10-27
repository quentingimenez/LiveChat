# LiveChat-server

Backend Discord/API pour l'architecture LiveChat **EXE-only**.

Documentation projet: [`../README.md`](../README.md)

## Projets liés (monorepo [LiveChat](https://github.com/quentingimenez/LiveChat))

- Server (ce dossier): [`server/`](.)
- Overlay desktop (Electron): [`overlay/`](../overlay)
- Extension navigateur (MV3): [`extension/`](../extension)

## Rôle

- Reçoit les commandes Discord.
- Télécharge et normalise les médias (yt-dlp + ffmpeg).
- Met en cache les assets (TTL 12h).
- Expose les routes `/overlay/*`.
- Expose la route `/ingest` (API simple pour raccourcis iOS).
- Diffuse les événements socket vers les overlays appairés.

## Démarrage rapide

Depuis `server/`:

```bash
cp .env.example .env
pnpm install
pnpm dev
```

Option TikTok restreint (login requis):
- `TIKTOK_COOKIE` peut contenir un header cookie TikTok (`name=value; name2=value2`) pour tenter l'extraction avec un compte autorisé.

## Quotas stockage média

- `MEDIA_CACHE_MAX_TOTAL_MB=5120` (5 Go): limite totale du cache **non persistant**.
- `MEDIA_BOARD_MAX_TOTAL_MB=15360` (15 Go): limite par serveur Discord pour la meme board persistante.

## API iOS (`/ingest`)

Créer d'abord un **ingest client token** dans `/admin` (section `Create Ingest Client`), puis appeler:

```bash
curl -X POST "$API_URL/ingest" \
  -H "Authorization: Bearer $INGEST_CLIENT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://vm.tiktok.com/ZNRfPWMaF/",
    "text": "hello depuis iOS",
    "forceRefresh": true
  }'
```

Payload minimal:
- au moins un de `url`, `media`, `text`
- `forceRefresh` optionnel (`true` pour ignorer le cache média et forcer re-download/re-transcode)
- `guildId`, `authorName`, `authorImage` sont liés au token ingest client et ne doivent pas être envoyés.

## Normalisation du volume

La normalisation loudness est activée par défaut pendant la transcode:
- `MEDIA_AUDIO_NORMALIZE_ENABLED=true`
- `MEDIA_AUDIO_LOUDNORM_I=-18`
- `MEDIA_AUDIO_LOUDNORM_LRA=11`
- `MEDIA_AUDIO_LOUDNORM_TP=-1.5`

## Endpoints publics

- `GET /` -> statut service minimal
- `GET /health` -> healthcheck
- `GET /admin?token=<ADMIN_UI_TOKEN>` -> panel admin local (token requis)

## Admin panel local (`/admin`)

- Activer avec `ADMIN_UI_TOKEN` dans `.env`.
- `ADMIN_UI_LOCAL_ONLY=true` (défaut): accès limité loopback (`127.0.0.1` / `::1`).
- `ADMIN_UI_LOCAL_ONLY=false`: accès LAN autorisé (token toujours requis).
- Le panel expose:
  - guilds connues + nom Discord (si disponible)
  - overlays connectés/hors ligne
  - usage cache global et meme board par guild
  - actions admin: réglages guild, stop playback, création/révocation clients ingest, révocation clients overlay, gestion pairing codes

Les erreurs HTTP sont volontairement normalisées (`not_found`, `request_error`, `internal_error`) sans stack trace exposée.

## Image Docker (GHCR)

Le workflow [`server.yml`](../.github/workflows/server.yml) (racine du monorepo) teste le serveur puis build l'image:
- sur pull request touchant `server/`: tests + build de l'image (sans push)
- sur push vers `main` touchant `server/`: tests + build + push sur GHCR
- sur tag `server-v*` (ex: `server-v1.2.0`): image versionnée `1.2.0`
- en exécution manuelle (`workflow_dispatch`)

Aucun secret requis: l'authentification utilise le `GITHUB_TOKEN` du workflow (permission `packages: write`).

Images publiées:
- `ghcr.io/quentingimenez/livechat-server:latest` (dernier build de `main`)
- `ghcr.io/quentingimenez/livechat-server:sha-<commit>`
- `ghcr.io/quentingimenez/livechat-server:<version>` (depuis un tag `server-v<version>`)

### Lancer l'image

Avec [`docker-compose.yml`](docker-compose.yml), depuis `server/`:

```bash
cp .env.example .env   # renseigner au minimum DISCORD_TOKEN et DISCORD_CLIENT_ID
docker compose up -d
```

La base SQLite et les médias sont persistés dans `server/data/` (monté sur `/app/data`).
