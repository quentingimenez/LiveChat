# `lc-media` — pipeline média (Phase 5a)

Crate Rust indépendante de la base de données : résolution de sources, téléchargement,
sonde `ffprobe`, transcodage `ffmpeg`, stockage content-addressed et politique de cache.
Elle produit un [`NormalizedMedia`] (chemin sur disque + métadonnées) que la couche de
persistance (`lc-db`, Phase 5b) enregistre ensuite comme `MediaAsset`.

- Pas d'accès SQL, pas d'état global : toutes les fonctions reçoivent leur configuration
  (`MediaConfig`) ; tout accès réseau passe par la garde SSRF/LFI (`guard`) décrite plus bas.
- Aucun `unwrap`/`expect`/`panic`/`dbg` hors tests (lints workspace, `-D warnings`).
- Les binaires externes (`ffmpeg`, `ffprobe`, `yt-dlp`) sont configurables et jamais
  invoqués dans les tests par défaut.

## Vue d'ensemble du pipeline

```
URL brute
   │  source::resolve_source            (normalisation URL, hash SHA-256, offset YouTube)
   │  source::validate_ingest_source    (allowlist http/https + refus IP internes)
   ▼
MediaSource
   │  resolve::resolve_media            (oEmbed / yt-dlp --dump-json / syndication X / HTML TikTok)
   ▼
ResolvedMedia  (titre, auteur, dimensions, kind, carte tweet, URL média directe)
   │  download::download_source         (HTTP streaming borné + garde SSRF, ou yt-dlp)
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
forcés (1 h de TTL, 1 Mo, 1 ms, 1 ingestion, 1 fragment). `TIKTOK_COOKIE` est
normalisé par `sanitize_tiktok_cookie` (task-49, AUD-A4-013) : trim, préfixe
`cookie:` retiré, refus des valeurs vides ou contenant un caractère de contrôle
(`\r`, `\n`, `\t`…), plafond 4096 caractères ; la valeur n'est **jamais**
journalisée (remplacement par défaut + `warn` sans le contenu).

### Garde SSRF/LFI (`guard`)

Garde centrale (P0-1, audits AUD-A2/A4/A7/V) appliquée à **toute** requête réseau
(résolutions oEmbed, téléchargements, yt-dlp, redirections) et à la pré-validation REST :

- `parse_media_url(raw)` : allowlist stricte `http`/`https` ; refus de `file:`, `gtts:`,
  `ftp:`, `data:`, `javascript:`, des chemins UNC (`\\host\share`, `//host/share`) et des
  URL avec identifiants intégrés (`user:pass@`). Erreur `UNSUPPORTED_SOURCE`.
- `ip_block_reason(ip)` : refuse loopback, non spécifiée, RFC1918, lien-local
  (169.254/16, métadonnées cloud), multicast, broadcast, 0.0.0.0/8, CGNAT
  (100.64/10), documentation, benchmarking (198.18/15), réservé 240/4 ; côté IPv6 : `::1`,
  `::`, multicast, ULA `fc00::/7`, lien-local `fe80::/10`, documentation `2001:db8::/32`
  et adresses IPv4-mapped reclassées.
- `validate_target(raw) -> ValidatedTarget { url, host, port, addrs }` : résout l'hôte
  (`tokio::net::lookup_host`) **avant** connexion, refuse si **au moins une** adresse
  résolue est interne, déduplique les `SocketAddr`. Erreur `UNSUPPORTED_SOURCE` (cible
  refusée) ou `DOWNLOAD_FAILED` (résolution impossible).
- `build_pinned_client(config, target)` : client `reqwest` avec redirections désactivées,
  `no_proxy()` et `resolve(host, ip)` par adresse validée → la connexion se fait sur
  **l'IP validée** (anti-rebinding DNS).
- `resolve_redirect(current, location)` : jointure relative, allowlist de schéma ;
  `http::send_guarded` suit au maximum `MAX_REDIRECTS = 3` sauts et revalide
  schéma + hôte + IP à chaque saut. Les en-têtes ne sont propagés que vers le même hôte,
  sauf allowlist (`accept`, `accept-language`, `range`, `referer`).
- `http::get_text` / `get_json` bornent les corps texte (4 Mio) et les corps d'erreur
  (8 Kio) ; `resolve_short_url` (URLs courtes TikTok) passe aussi par la garde.
- `html::sanitize_embed_html` (durci task-49, AUD-A4-015) : liste **fermée** de
  tags (`a,p,span,blockquote,br,strong,em,b,i,code,pre,ul,ol,li`) et d'attributs
  (`href,src,alt,title,class,dir,lang,rel,target,width,height`) ; blocs
  `script/style/iframe/object/embed/form` supprimés avec leur contenu,
  commentaires retirés, `href`/`src` limités à `http(s)`, `rel`/`target` forcés
  sur `<a>`, attributs ré-échappés. Aucune heuristique de retrait seule : tout
  ce qui n'est pas autorisé disparaît. Le pendant « carte tweet » côté serveur
  est `apps/lc-server/src/media/html.rs::sanitize_tweet_embed` (Phase 6b).

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
- `validate_ingest_source(raw) -> MediaResult<MediaSource>` : validation complète d'une
  source d'ingestion — canonicalisation, classification, refus de `file:`/`gtts:`/inconnu
  et `guard::validate_target` sur l'URL canonique. **Aucune connexion** n'est ouverte pour
  une cible interne.
- `resolve_source(config, raw)` : suit les redirections TikTok courtes
  (`vm.tiktok.com`, `vt.tiktok.com`, …) via `http::resolve_short_url`, puis passe par
  `validate_ingest_source`.

### Résolution (`resolve`)

`resolve_media(config, source) -> MediaResult<ResolvedMedia>`.

- YouTube : oEmbed (titre/auteur/dimensions) puis repli `yt-dlp --dump-json`.
- TikTok : oEmbed puis scraping de la page (`__UNIVERSAL_DATA_FOR_REHYDRATION__`,
  `__FRONTITY_CONNECT_STATE__`), détection des murs d'authentification, secours sur
  l'embed `/embed/v2/<id>`, extraction des carrousels photo (max 35 slides, 3 candidats
  par slide, 5 candidats audio) et des URL brutes `~tplv-photomode-image`.
- X/Twitter : carte oEmbed (HTML assaini) + variantes vidéo via
  `cdn.syndication.twimg.com/tweet-result`, repli sur les balises `og:video:*` et
  `video.twimg.com` ; `TweetCard { url, html, author_name, author_url }`.

### Téléchargement (`download`)

- `download_source(source, resolved, config, temp_dir)` : dispatch par type de source
  (YouTube, TikTok, Twitter, HTTP direct) après validation de la cible ; les sources
  `file:`/`gtts:` sont **refusées** (`UNSUPPORTED_SOURCE`) — l'ancien `download_local`
  a été supprimé avec la garde LFI.
- HTTP streaming : `reqwest` rustls via `http::send_guarded` (garde SSRF, redirections
  manuelles max 3, corps borné), timeout global, plafond 2 Gio compté au fil de l'eau,
  sniffing des 32 premiers octets, rejet des réponses HTML (`UNSUPPORTED_SOURCE`).
- `yt-dlp` : la cible est validée (schéma + hôte + IP) avant tout lancement ; format
  avc1/mp4 borné à 1080p, `--max-filesize`, fusion mp4, cookies TikTok générés au format
  Netscape ; sur YouTube, un échec auth/bot rejoue avec `youtube:player_client=android` et
  un échec de format rejoue avec un sélecteur relâché.
- `download_tiktok` : vidéo d'abord, puis carrousel photo (assemblage ffmpeg en 9:16),
  remontée prioritaire des erreurs `FILE_TOO_LARGE` / `PRIVATE_OR_AUTH_REQUIRED`.
- `DownloadResult { path, kind, mime, size_bytes, source_url }`.

### Sonde (`probe`)

`probe(ffprobe, path, timeout)` → `ProbeInfo { duration_sec, width, height, format_name,
video_codec, audio_codec, pixel_format, has_video, has_audio }`, plus
`probe_args`, `parse_probe_json`, `duration_ceil_sec()`, `is_vertical()`.

Depuis task-49 (AUD-A4-012), `probe_args` insère `-i` **avant** le chemin :
un nom de fichier local commençant par `-` (ex. `-version`) n'est plus
interprété comme une option ffprobe. Test dédié
(`probe_args_protect_option_like_paths`).

### Transcodage (`transcode`)

`normalize_downloaded_media(input, content_type, output_base, config)` :

- détection du type (mime + sonde) ; **images** : `image/svg+xml` est refusé avant
  toute copie (Content-Type, extension `.svg`, ou signature `<?xml`/`<svg` sur
  les 1024 premiers octets) → `INVALID_MEDIA` « image SVG non supportée… » —
  jamais de SVG stocké ni servi tel quel (AUD-A4-004) ; pour les autres images,
  le MIME est dérivé du probe codec d'abord (whitelist png/jpeg/webp/gif/bmp/
  avif), puis du Content-Type distant whitelisté, puis de l'extension, et la
  copie garde une extension normalisée (AUD-A4-010) ;
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

### Processus externes (`process`)

`run_command(program, args, timeout)` (ffmpeg/ffprobe/yt-dlp) lance le processus
avec `kill_on_drop` et un timeout ; depuis task-49 (AUD-A4-005) stdout/stderr
sont drainés avec un plafond `MAX_COMMAND_OUTPUT_BYTES = 8 Mio` — au-delà,
l'enfant est tué et l'erreur remonte en `INVALID_MEDIA` (« sortie de … trop
volumineuse »), au lieu d'un buffer mémoire illimité. La limite est
paramétrable en interne (`run_command_with_limit`, privé) pour les tests.

### Stockage et cache (`storage`, `cache`)

- `MediaStorage` : chemins content-addressed `<root>/<source_hash>` + variantes
  `<root>/<source_hash>.<ext>`, `sanitize_hash`, `remove_variants`,
  `remove_other_variants(hash, keep)` (supprime les variantes du même hash sauf le
  fichier publié), `list_variants`, `is_staged_file_name`.
- **Publication atomique (P1-09, task-42)** : toutes les écritures passent par un
  fichier de transit `<stem>.tmp-<8 hex>.<ext>` (`staged_path`) puis
  `MediaStorage::commit_file(staged, final)` = `create_dir_all` du parent,
  `sync_all` best-effort du transit, `rename` atomique, `sync_all` best-effort du
  parent et suppression du transit en cas d'échec. Une annulation, un crash ou un
  `ffmpeg` en échec ne laisse donc jamais un média partiellement publié : l'ancien
  fichier reste intact jusqu'au `rename`, et un reliquat `.tmp-*` est reconnaissable
  et purgé par le serveur (`media::purge`).
- **Racine canonicalisée** : `ensure_canonical()` / `canonical_root()` créent puis
  canonicalisent la racine de stockage et échouent en `DOWNLOAD_FAILED` avec un
  message explicite si ce n'est pas possible (fail-closed : le pipeline ne publie
  jamais dans un chemin non vérifiable).
- **Confinement des suppressions (V2-S, task-46)** : `remove_path` résout d'abord
  la référence (`resolve_reference` : un `storage_path` relatif importé du legacy est
  lu **sous la racine média**, jamais relatif au cwd), refuse les liens symboliques
  (`symlink_metadata`, no-follow) et tout chemin dont la cible canonique sort de la
  racine (`starts_with` sur racine canonicalisée) : `Ok(false)` + `warn`, jamais de
  suppression hors confinement (AUD-V2-S-001/002). `list_variants` ignore les liens
  symboliques et les non-fichiers, et `remove_variants`/`remove_other_variants`
  passent par ce même garde.
- `transcode::normalize_*` produisent dans un transit puis publient via
  `commit_file`, et `ingest` n'appelle `remove_other_variants` qu'**après** succès
  de la publication.
- `cache` : fonctions **pures** (sans I/O ni DB) pour que les quotas soient testables.
  `CacheEntry { key, size_bytes, last_accessed_ms, created_at_ms, pinned, active, stale }`,
  `plan_cache_eviction(entries, required, max, exclude)` (LRU, épinglés hors budget,
  actifs jamais évincés), `plan_board_eviction` (épinglés inactifs seulement),
  `purge_candidates` (TTL 12 h + FAILED/PROCESSING après 2 h), `cache_total_bytes`,
  `board_total_bytes`.

### Ingestion (`ingest`)

- `MediaPipeline::new(config)` puis `ingest_url(raw)` ou `ingest_source(&MediaSource)` →
  `IngestedMedia { source, resolved, download, normalized }`.
- `IngestLimiter` (sémaphore) borne les ingestions simultanées.
- **Single-flight par `source_hash` (P1-09)** : `lock_source_hash(&source.source_hash)`
  sérialise les ingestions concurrentes de la même source (deux requêtes simultanées
  réutilisent/complètent le même fichier au lieu de se supprimer mutuellement) ; les
  verrous sans référence sont élagués. La fonction est publique : le worker de purge du
  serveur prend le même verrou avant d'évincer un asset `READY` expiré, pour ne pas
  supprimer des fichiers pendant une ingestion concurrente du même média
  (AUD-V2-S-004).
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
- **Durcissement sécurité assumé (P0-1, task-35)** : le legacy acceptait `file:` (lecture de
  fichiers locaux), les URL `http(s)` vers loopback/RFC1918/métadonnées cloud et suivait les
  redirections sans revalidation. La crate refuse désormais ces cibles
  (`UNSUPPORTED_SOURCE`), résout le DNS avant connexion, se connecte à l'IP validée
  (anti-rebinding) et revalide chaque saut de redirection (max 3). Le serveur pré-valide la
  source dans `POST /ingest` **avant** toute création d'asset (`media_ingestion_failed`
  422, même format que le pipeline) ; le board média est protégé indirectement via
  `MediaPipeline`. Ce refus est un changement de comportement voulu, pas une régression
  de parité.
- **Durcissement stockage (P1-09, task-42)** : publication atomique (transit `.tmp-*` +
  `rename`, jamais de fichier partiel visible), racine canonicalisée fail-closed,
  single-flight par `source_hash`, remplacement des variantes uniquement après succès
  de la publication. Le legacy écrivait directement puis supprimait les anciens fichiers
  avant transcodage (perte de média possible en cas d'échec) : écart voulu.
- **Durcissement confinement purge (V2-S, task-46)** : toute suppression de fichier est
  confinée à la racine média canonicalisée — un `storage_path` relatif (legacy) est résolu
  sous la racine et **jamais** relativement au cwd, les liens symboliques sont refusés
  (`symlink_metadata`, no-follow) et une cible canonique hors racine est ignorée (`warn`,
  pas de suppression). `list_variants` ignore les liens et la purge du serveur déduplique
  les références (deux assets vers le même fichier : le fichier n'est supprimé qu'à la
  dernière référence) sous le verrou `lock_source_hash` avant d'évincer un `READY` expiré,
  et la purge admin (`POST /admin/api/guilds/:id/purge`) passe par le même garde. Écart
  legacy assumé : l'ancien serveur supprimait sans vérifier le confinement
  (AUD-V2-S-001/002/004).
- **Dette P2 résolue (task-49, AUD-COR-13 / AUD-V2-S-003)** : `POST
  /overlay/media-board/items` passe désormais par `MediaService::ingest` — cache
  `READY`/`source_hash`, sémaphore, budget cache, `record_failure` ;
  l'asset existant est rafraîchi par le pipeline puis épinglé via `assets::pin`,
  et un échec `forceRefresh` supprime les variantes partielles
  (`discard_partial_files`). Les variantes périmées restent nettoyées lors d'une
  publication réussie (`remove_other_variants`) ; le worker de purge serveur
  traite le reste.
- **Durcissement sécurité COR-15 (task-49)** : SVG refusé et jamais servi, MIME
  image dérivé en priorité du probe, `ffprobe` protégé par `-i`, sorties de
  commandes bornées à 8 Mio (kill + `INVALID_MEDIA`), cookie TikTok validé sans
  log, sanitizers HTML allowlist (oEmbed + carte tweet). Voir
  `docs/rust/server-ingest.md` §5 « Sécurités pipeline ».

## Tests

- 130 tests unitaires dans la crate (URL/hash, résolution offline sur fixtures, parsing
  HTML/JSON, arguments ffmpeg, décisions de copie, éviction LRU, configuration, erreurs,
  garde SSRF : schémas/UNC, plages IPv4/IPv6, redirections ; stockage : racine
  canonicalisée fail-closed, chemins de transit, publication atomique, purge ciblée des
  variantes, confinement des suppressions — chemins hors racine refusés, liens
  symboliques ignorés, références relatives résolues sous la racine). Task-49 ajoute :
  refus SVG par MIME et par signature + résolution MIME probe-first (`transcode`),
  `probe_args` protégeant un chemin en `-…`, sortie de commande plafonnée
  (`oversized_output_is_rejected`), validation du cookie TikTok (contrôle de
  caractères, taille), et sanitizer HTML allowlist (XSS, blocs actifs, `rel`/`target`).
- `tests/offline.rs` : 7 tests sur fixtures (`tiktok_video.html`, `tiktok_photo.html`,
  `twitter_syndication.json`, `yt_dlp_dump.json`, `youtube_oembed.json`,
  `tiktok_oembed.json`).
- `tests/ssrf_guard.rs` : 5 tests d'intégration sans réseau — refus loopback **avant toute
  connexion** (sentinelle `TcpListener` + `accept()`/`WouldBlock`), schémas/UNC refusés,
  redirection interne refusée, IP publique littérale acceptée sans DNS, plages
  privées/métadonnées refusées.
- `tests/network.rs` : 4 tests marqués `#[ignore]` (YouTube e2e, TikTok en ligne, carte
  tweet, échantillon `testsrc` ffmpeg) ; ils nécessitent réseau + `ffmpeg`/`yt-dlp`.
- Doctest de la crate : 1.

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
   `[CODE] message` (≤ 500), TTL et `last_accessed_at` — fait côté serveur (Phase 5b) ;
   worker de purge des lignes/fichiers orphelins — fait côté serveur
   (`apps/lc-server/src/media/purge.rs`, task-42).
2. Application des plans de cache : lecture des `CacheEntry` depuis la DB, suppression des
   fichiers + lignes, propagation des erreurs `CACHE_STORAGE_LIMIT_REACHED` /
   `BOARD_STORAGE_LIMIT_REACHED` — fait côté serveur (`media/quotas.rs`).
3. Annulation et arrêt propre : le `sémaphore` borne la concurrence ; les
   processus externes sont lancés avec `kill_on_drop`, donc le drop de la future
   d'ingestion (shutdown axum/abandon de handler) tue `ffmpeg`/`yt-dlp`. Un
   `CancellationToken` explicite n'est pas encore propagé (ligne `PROCESSING`
   nettoyée par le worker de purge après 2 h) (exigence audit S10, partiellement
   couverte).
4. `MediaPipeline` : brancher la file d'attente Phase 5b (workers, reprise après crash,
   quotas globaux) et exposer des métriques `tracing`.
5. URL signées HMAC (`MEDIA_URL_SECRET`, TTL) : implémentation côté serveur Phase 3,
   hors périmètre `lc-media`.
6. Tests réseau : ajouter un environnement de CI optionnel avec `ffmpeg`/`yt-dlp` et des
   fixtures HTTP servies localement pour tester le streaming sans accès Internet.
