# `lc-media` — pipeline média (Phase 5a)

Crate Rust indépendante de la base de données : résolution de sources, téléchargement,
sonde `ffprobe`, transcodage `ffmpeg`, stockage content-addressed et politique de cache.
Elle produit un [`NormalizedMedia`] (chemin sur disque + métadonnées) que la couche de
persistance (`lc-db`, Phase 5b) enregistre ensuite comme `MediaAsset`.

- Pas d'accès SQL, pas d'état global : toutes les fonctions reçoivent leur configuration
  (`MediaConfig`) et un client HTTP (`reqwest::Client`).
- Aucun `unwrap`/`expect`/`panic`/`dbg` hors tests (lints workspace, `-D warnings`).
- Les binaires externes (`ffmpeg`, `ffprobe`, `yt-dlp`) sont configurables et jamais
  invoqués dans les tests par défaut.

## Vue d'ensemble du pipeline

```
URL brute
   │  source::resolve_source            (normalisation URL, hash SHA-256, offset YouTube)
   ▼
MediaSource
   │  resolve::resolve_media            (oEmbed / yt-dlp --dump-json / syndication X / HTML TikTok)
   ▼
ResolvedMedia  (titre, auteur, dimensions, kind, carte tweet, URL média directe)
   │  download::download_source         (HTTP streaming borné, ou yt-dlp, ou fichiers locaux)
   ▼
DownloadResult (fichier temporaire + kind/mime devinés)
   │  transcode::normalize_downloaded_media (probe ffprobe → copie ou transcodage ffmpeg)
   ▼
NormalizedMedia (chemin final sous MEDIA_STORAGE_DIR, mime, durée, dimensions, verticalité)
```

`ingest::MediaPipeline` enchaîne ces étapes avec une concurrence bornée par un
`sémaphore` (`MEDIA_MAX_CONCURRENT_INGESTIONS`, défaut 2) et un répertoire temporaire
propre à chaque ingestion.

## API publique

### Erreurs (`error`)

- `MediaErrorCode` : les 11 codes legacy
  `UNSUPPORTED_SOURCE`, `PRIVATE_OR_AUTH_REQUIRED`, `DRM_PROTECTED`, `MEDIA_NOT_FOUND`,
  `BOARD_STORAGE_LIMIT_REACHED`, `CACHE_STORAGE_LIMIT_REACHED`, `FILE_TOO_LARGE`,
  `INVALID_MEDIA`, `TRANSCODE_FAILED`, `DOWNLOAD_TIMEOUT`, `DOWNLOAD_FAILED`.
  Chaque code expose `as_str()`, `priority()` (ordre de pertinence historique),
  `http_status()` et `legacy_http_status()` (422, contrat REST v1).
- `ApiErrorLabel` : libellés JSON (`invalid_payload`, `unauthorized`,
  `media_ingestion_failed`, `ingest_api_disabled`) pour la parité des routes.
- `MediaError` : `code`, `message` (≤ 500 caractères), `raw_message` (≤ 2000) ;
  `from_http_status`, `classify(raw, fallback)`, `from_command_failure(..)`,
  `most_relevant(other)`.
- Helpers `code_from_http_status` et `classify_code` : mapping HTTP 401/403/404/408/410/
  413/504 puis signaux textuels (DRM, auth, introuvable, timeout, taille, quotas,
  média invalide, source non supportée, transcodage, téléchargement).

### Configuration (`config`)

`MediaConfig { storage, download, video, tools, resolve }`, construite par
`MediaConfig::from_env()` (variables d'environnement), `load()` (alias) ou
`from_pairs(iter)` (tests, sans lire l'environnement global).

| Champ | Variables | Défaut |
|---|---|---|
| `storage.directory` | `MEDIA_STORAGE_DIR` | `./data/media` |
| `storage.cache_ttl` | `MEDIA_CACHE_TTL_HOURS` | 12 h |
| `storage.cache_max_total_bytes` | `MEDIA_CACHE_MAX_TOTAL_MB` | 5120 Mo |
| `storage.board_max_total_bytes` | `MEDIA_BOARD_MAX_TOTAL_MB` | 15360 Mo |
| `download.timeout` | `MEDIA_DOWNLOAD_TIMEOUT_MS` | 180 s |
| `download.max_size_bytes` | `MEDIA_MAX_SIZE_MB` | 2048 Mo |
| `download.max_concurrent_ingestions` | `MEDIA_MAX_CONCURRENT_INGESTIONS` | 2 |
| `download.user_agent` | `MEDIA_USER_AGENT` | Chrome/133 |
| `download.short_url_timeout` | — | 8 s |
| `video.max_height` | `MEDIA_VIDEO_MAX_HEIGHT` | 1080 |
| `video.encoder` | `MEDIA_VIDEO_ENCODER` (`auto`/`libx264`/`h264_nvenc`) | `auto` |
| `video.cpu_preset` / `video.nvenc_preset` | `MEDIA_VIDEO_PRESET` / `MEDIA_VIDEO_NVENC_PRESET` | `ultrafast` / `p4` |
| `video.audio_normalize` | `MEDIA_AUDIO_NORMALIZE_ENABLED` | `true` |
| `video.loudnorm` | `MEDIA_AUDIO_LOUDNORM_I`/`_LRA`/`_TP` | -18 / 11 / -1.5 |
| `video.max_duration_sec` | `MEDIA_VIDEO_MAX_DURATION_SEC` | illimité |
| `tools.ffmpeg` / `tools.ffprobe` / `tools.ytdlp` | `FFMPEG_BINARY` / `FFPROBE_BINARY` / `YTDLP_BINARY` | `ffmpeg` / `ffprobe` / `yt-dlp` |
| `tools.ytdlp_format` | `YTDLP_FORMAT` | sélecteur avc1/mp4 ≤ 1080 |
| `tools.ytdlp_concurrent_fragments` | `YTDLP_CONCURRENT_FRAGMENTS` | 1 |
| `tools.tiktok_cookie` | `TIKTOK_COOKIE` | vide |
| `resolve.twitter_oembed_endpoint` | `TWITTER_OEMBED_ENDPOINT` | `https://publish.twitter.com/oembed` |
| `resolve.twitter_video_resolver_base_url` | `TWITTER_VIDEO_RESOLVER_BASE_URL` | `https://vxtwitter.com` |
| `resolve.twitter_card_duration` | `TWITTER_CARD_DURATION_SEC` | 15 s |
| `resolve.youtube_oembed_endpoint` / `tiktok_oembed_endpoint` | — | endpoints publics |
| `resolve.language` | `I18N` (préfixe `fr`) | `fr` |

Les valeurs invalides retombent sur le défaut avec un `tracing::warn` ; les minimas sont
forcés (1 h de TTL, 1 Mo, 1 ms, 1 ingestion, 1 fragment).

### Sources (`source`)

- `SourceKind` : `YouTube`, `TikTok`, `Twitter`, `DirectMedia`, `LocalFile`, `Gtts`,
  `Unknown`.
- `MediaSource { original, canonical_url, source_hash, source_kind, start_offset_sec }`.
- `canonicalize_source_url(raw)` : trim, supprime le fragment ; YouTube normalisé en
  `https://www.youtube.com/watch?v=<id>` (y compris `youtu.be`, `shorts`, `embed`, `live`) ;
  TikTok `https://www.tiktok.com/@_/{video|photo}/<id>` ; `file:` et `gtts:` conservés.
- `source_hash(url)` : SHA-256 hex minuscule (clé content-addressed).
- `parse_youtube_start_offset(url)` : `t`/`start` en query ou fragment, formats
  `123`, `123s`, `mm:ss`, `hh:mm:ss`, `1h2m3s`.
- `kind_from_url` / `kind_from_mime` / `extension_from_mime` / `mime_from_extension`.
- `resolve_source(client, raw, config)` : suit les redirections TikTok courtes
  (`vm.tiktok.com`, `vt.tiktok.com`, …) avant de canonicaliser.

### Résolution (`resolve`)

`resolve_media(client, source, config) -> MediaResult<ResolvedMedia>`.

- YouTube : oEmbed (titre/auteur/dimensions) puis repli `yt-dlp --dump-json`.
- TikTok : oEmbed puis scraping de la page (`__UNIVERSAL_DATA_FOR_REHYDRATION__`,
  `__FRONTITY_CONNECT_STATE__`), détection des murs d'authentification, secours sur
  l'embed `/embed/v2/<id>`, extraction des carrousels photo (max 35 slides, 3 candidats
  par slide, 5 candidats audio) et des URL brutes `~tplv-photomode-image`.
- X/Twitter : carte oEmbed (HTML assaini) + variantes vidéo via
  `cdn.syndication.twimg.com/tweet-result`, repli sur les balises `og:video:*` et
  `video.twimg.com` ; `TweetCard { url, html, author_name, author_url }`.

### Téléchargement (`download`)

- `download_source(client, source, resolved, config, temp_dir)` : dispatch par type de
  source (YouTube, TikTok, Twitter, fichier local, HTTP direct).
- HTTP streaming : `reqwest` rustls, timeout global, plafond 2 Gio compté au fil de l'eau,
  sniffing des 32 premiers octets, rejet des réponses HTML (`UNSUPPORTED_SOURCE`).
- `yt-dlp` : format avc1/mp4 borné à 1080p, `--max-filesize`, fusion mp4, cookies TikTok
  générés au format Netscape ; sur YouTube, un échec auth/bot rejoue avec
  `youtube:player_client=android` et un échec de format rejoue avec un sélecteur relâché.
- `download_tiktok` : vidéo d'abord, puis carrousel photo (assemblage ffmpeg en 9:16),
  remontée prioritaire des erreurs `FILE_TOO_LARGE` / `PRIVATE_OR_AUTH_REQUIRED`.
- `DownloadResult { path, kind, mime, size_bytes, source_url }`.

### Sonde (`probe`)

`probe(ffprobe, path, timeout)` → `ProbeInfo { duration_sec, width, height, format_name,
video_codec, audio_codec, pixel_format, has_video, has_audio }`, plus
`probe_args`, `parse_probe_json`, `duration_ceil_sec()`, `is_vertical()`.

### Transcodage (`transcode`)

`normalize_downloaded_media(input, content_type, output_base, config)` :

- détection du type (mime + sonde) ; image = copie avec extension normalisée ;
  audio = copie mp3 ou réencodage `libmp3lame -b:a 192k` (+ loudnorm) ;
  vidéo = copie si H.264/AAC/yuv420p déjà conforme, sinon transcodage
  `-c:v libx264|h264_nvenc -pix_fmt yuv420p -movflags +faststart -c:a aac -b:a 192k`
  avec `scale=-2:1080` et `loudnorm=I=-18:LRA=11:TP=-1.5` optionnels.
- NVENC choisi automatiquement si `ffmpeg -encoders` l'expose ; échec NVENC → nouvel
  essai `libx264`.
- `finalize` vérifie la taille (> 1024 o), la durée (vidéo > 0), la présence d'une piste
  vidéo et calcule `is_vertical` (hauteur > largeur).
- `build_carousel_video_args` / `build_carousel_video` assemblent un carrousel TikTok
  (`-loop 1 -t 3` par image, `fps=30`, `pad` 9:16, `concat`, audio aac `-shortest`).

### Stockage et cache (`storage`, `cache`)

- `MediaStorage` : chemins content-addressed `<root>/<source_hash>` + variantes
  `<root>/<source_hash>.<ext>`, `sanitize_hash`, `remove_variants`, `list_variants`.
- `cache` : fonctions **pures** (sans I/O ni DB) pour que les quotas soient testables.
  `CacheEntry { key, size_bytes, last_accessed_ms, created_at_ms, pinned, active, stale }`,
  `plan_cache_eviction(entries, required, max, exclude)` (LRU, épinglés hors budget,
  actifs jamais évincés), `plan_board_eviction` (épinglés inactifs seulement),
  `purge_candidates` (TTL 12 h + FAILED/PROCESSING après 2 h), `cache_total_bytes`,
  `board_total_bytes`.

### Ingestion (`ingest`)

- `MediaPipeline::new(config)` / `from_parts(config, client)` puis
  `ingest_url(raw)` ou `ingest_source(&MediaSource)` → `IngestedMedia { source, resolved,
  download, normalized }`.
- `IngestLimiter` (sémaphore) borne les ingestions simultanées.
- En cas d'échec, l'erreur `MediaError` remonte avec le code legacy ; Phase 5b la
  persiste en `[CODE] message` sur l'asset FAILED.

## Parité legacy et écarts assumés

- Codes, priorités, mapping HTTP et libellés JSON reproduits depuis
  `server/src/services/media/mediaErrors.ts` ; un échec d'ingestion doit être exposé en
  HTTP **422** `{ error: "media_ingestion_failed", code, message }` par la couche REST
  (Phase 3).
- `classify_code` conserve l'ordre historique : un texte contenant à la fois
  `not found` et un signal de corruption est classé `MEDIA_NOT_FOUND` (comportement de
  l'ancien serveur, y compris pour `moov atom not found`).
- Les messages d'erreur sont en français ; les codes et libellés restent en anglais.
- Différences assumées : pas de base de données ni de téléchargement différé (le pipeline
  rend un fichier, la persistance arrive en Phase 5b) ; l'i18n des messages n'est pas
  portée ; la recherche de vignettes/couvertures TikTok couvre les structures rencontrées
  dans le legacy mais peut manquer de nouvelles clés.
- Détection de statut HTTP insensible à la casse (corrige un écart legacy où
  `HTTP/1.1 503` n'était pas reconnu quand la casse différait).

## Tests

- 105 tests unitaires dans la crate (URL/hash, résolution offline sur fixtures, parsing
  HTML/JSON, arguments ffmpeg, décisions de copie, éviction LRU, configuration, erreurs).
- `tests/offline.rs` : 7 tests sur fixtures (`tiktok_video.html`, `tiktok_photo.html`,
  `twitter_syndication.json`, `yt_dlp_dump.json`, `youtube_oembed.json`,
  `tiktok_oembed.json`).
- `tests/network.rs` : 4 tests marqués `#[ignore]` (YouTube e2e, TikTok en ligne, carte
  tweet, échantillon `testsrc` ffmpeg) ; ils nécessitent réseau + `ffmpeg`/`yt-dlp`.

Commandes :

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$env:CARGO_BUILD_JOBS = "4"
cargo fmt -p lc-media -- --check
cargo clippy -p lc-media --all-targets -- -D warnings
cargo test -p lc-media
cargo test -p lc-media --test network -- --ignored   # réseau + binaires requis
```

## Points ouverts / TODO Phase 5b

1. Persistance des assets : insertion `MediaAsset` PROCESSING → READY/FAILED, erreur
   `[CODE] message` (≤ 500), TTL et `last_accessed_at` ; worker de purge.
2. Application des plans de cache : lecture des `CacheEntry` depuis la DB, suppression des
   fichiers + lignes, propagation des erreurs `CACHE_STORAGE_LIMIT_REACHED` /
   `BOARD_STORAGE_LIMIT_REACHED`.
3. Annulation et arrêt propre : le `sémaphore` borne la concurrence mais il reste à
   propager un `CancellationToken` et à tuer les groupes de processus (`ffmpeg`, `yt-dlp`)
   sur arrêt du serveur (exigence audit S10).
4. `MediaPipeline` : brancher la file d'attente Phase 5b (workers, reprise après crash,
   quotas globaux) et exposer des métriques `tracing`.
5. URL signées HMAC (`MEDIA_URL_SECRET`, TTL) : implémentation côté serveur Phase 3,
   hors périmètre `lc-media`.
6. Tests réseau : ajouter un environnement de CI optionnel avec `ffmpeg`/`yt-dlp` et des
   fixtures HTTP servies localement pour tester le streaming sans accès Internet.
