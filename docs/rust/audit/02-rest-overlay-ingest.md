# Audit A2 — REST `/overlay` et `/ingest`

- **Date** : 2026-11-10 (Europe/Paris)
- **Auditeur** : `audit-dynamique` (audit indépendant, lane A2)
- **Périmètre** : `apps/lc-server/src/http/overlay/**`, `apps/lc-server/src/http/ingest/**`, `apps/lc-server/src/media_url.rs`, surface HTTP de `crates/lc-media`, middlewares `http/error.rs` + `http/rate_limit.rs`, `app.rs` ; comparaison avec le legacy TS (`server/src/components/{overlay,ingest}`, services média, `mediaIngestRoutes`).
- **Hors périmètre** : WebSocket overlay/`protocol-v2`, admin, playback engine, base de données (audités par d'autres lanes).
- **Mode** : revue statique exhaustive + comparaison legacy + **probes dynamiques sur le binaire `target/debug/lc-server.exe`** (build debug du 2026-11-10 01:43:04, sources overlay plus récentes : 01:41:56) + tests ciblés.
- **Aucun code modifié**, aucun commit. Artefacts de probe isolés dans `.phase0/audit/A2/` (scripts, JSON de résultats, logs, DB jetable).

---

## 1. Synthèse

**17 findings** : 1 haute, 7 moyennes, 8 basses, 1 informative.

| ID | Sévérité | Confiance | Titre | Statut |
|----|----------|-----------|-------|--------|
| AUD-A2-001 | **Haute** | Élevée | Lecture de fichiers locaux arbitraires via les URL `file:` (ingest + meme board) | Confirmé (probe) |
| AUD-A2-002 | Moyenne | Élevée | SSRF / relais réseau via les URL d'ingestion (http/https sans allowlist) | Confirmé (probe) |
| AUD-A2-003 | Moyenne | Élevée | Confinement média *fail-open* si `MEDIA_STORAGE_DIR` n'est pas canonicalisable | Confirmé (probe B) |
| AUD-A2-004 | Moyenne | Élevée | URL média signée = capability non liée à la session ; plus de contrôle job/guilde | Parité legacy |
| AUD-A2-005 | Moyenne | Élevée | TTL d'asset prolongé à chaque lecture (`touch_asset` → `now+12 h`) : assets « immortels » | Confirmé (DB) |
| AUD-A2-006 | Moyenne | Élevée | `POST /overlay/meme-board/items` reconstruit un `MediaPipeline` par requête (sémaphore/cache/quotas contournés) | Statique |
| AUD-A2-007 | Moyenne | Élevée | `forceRefresh` inopérant sur asset READY (téléchargement orphelin, métadonnées non rafraîchies) | Statique |
| AUD-A2-008 | Moyenne | Élevée | Le rate limit 100/15 min/IP s'applique au streaming média (lecture + DoS de bucket) | Confirmé (probe) |
| AUD-A2-009 | Basse | Élevée | JSON malformé → 400 `request_error` au lieu de `invalid_payload` (label/contrat divergent) | Confirmé (probe) |
| AUD-A2-010 | Basse | Élevée | Branche 409 `overlay_client_conflict` morte (`is_unique_violation` ≠ `Domain(Conflict)`) → 500 | Statique |
| AUD-A2-011 | Basse | Élevée | Repli `?token=` : doc contredit le code, accepté sur le média board, y compris après Bearer malformé | Confirmé (probe) |
| AUD-A2-012 | Basse | Élevée | Troncature `deviceName` incohérente : overlay 200 chars, ingest illimité | Confirmé (probe) |
| AUD-A2-013 | Basse | Élevée | Coûts O(N) : listing board en mémoire + N+1, quota N+1, DELETE scanne toutes les guildes | Statique |
| AUD-A2-014 | Basse | Moyenne | `coerce_flag` board ignore `"on"` (legacy `toBooleanFlag` l'acceptait) | Statique |
| AUD-A2-015 | Basse | Moyenne | 416 avec corps JSON `request_error` (legacy : corps vide) ; compression média non reproduite (doc obsolète) | Confirmé |
| AUD-A2-016 | Info | Élevée | `/overlay/config` supprimé (breaking pour clients legacy, assumé protocol v2) | Confirmé |
| AUD-A2-017 | Basse | Moyenne | Auth ingest : `list()` complet des clients à chaque requête non authentifiée (O(n)) | Statique |

### Points forts observés

- **Consommation des codes d'appairage atomique** : `consume(code, now)` = UPDATE conditionnel ; probe concurrente `[200, 404]`, une seule session créée (corrige le TOCTOU legacy). 15 tests overlay + 18 tests ingest verts (`cargo test -p lc-server --lib http::overlay` → 15 passed ; `http::ingest` → 18 passed).
- **HMAC média robuste** : signature `assetId|clientId|exp` vérifiée en temps constant (`subtle::ConstantTimeEq`, `media_url.rs:59-80`), refus des signatures malformées, expiration inclusive, TTL signé borné 300–3600 s côté serveur (`lc_protocol::media_url_ttl_sec`).
- **Confinement média effectif quand le répertoire existe** (asset hors `MEDIA_STORAGE_DIR` → 404 `media_not_found_on_disk`, probe A).
- **Range/HEAD/206/416 conformes** : 206 `content-range`/`content-length`, 416 `content-range: bytes */size`, `accept-ranges: bytes`, `cache-control: no-store`, `content-type: asset.mime` (fallback octet-stream).
- **Meme board** : dédoublonnage `(guilde, asset)`, quota board 422 `BOARD_STORAGE_LIMIT_REACHED`, épinglage +100 ans, `guildId` query ≠ Bearer → 403, PATCH partiel conservant les champs absents, DELETE idempotent, pagination bornée 1..=200 (tests).
- **Ingestion** : auth avant corps, body limit global **1 MiB** (`413` confirmé), statuts 201/400/401/403/422/503 conformes, `saveToBoard` sans source refusé (400), erreurs média normalisées `{error: media_ingestion_failed, code, message}`, conversion de flags legacy (`boolean_flag` accepte `on`), `showText` booléen strict, durée > 0 arrondie au supérieur.
- **Erreurs non-JSON normalisées** (`normalize_errors`) : pas de fuite de stack/HTML, status conservés, `Internal` → `{"error":"internal_error"}`.
- **Pas d'injection de commande** via URL : `run_command` passe des `args` vectorisés (pas de shell) ; yt-dlp/ffprobe reçoivent l'URL en argument.

---

## 2. Findings détaillés

### AUD-A2-001 — Lecture de fichiers locaux arbitraires via `file:` (Haute)

**Preuve statique**
- `crates/lc-media/src/source.rs:253-277` : `classify_source("file:///…")` → `SourceKind::LocalFile` ; `from_raw` (l.101-120) accepte toute chaîne non vide ; `canonicalize_source_url` (l.136-157) laisse passer `file:` et `gtts:`.
- `crates/lc-media/src/download.rs:62-82` : `SourceKind::LocalFile → download_local` ; `download_local` (l.104-143) lit **n'importe quel fichier** du système (chemin brut, `file:` relatif/absolu), sans aucun confinement sous `MEDIA_STORAGE_DIR`.
- Chemins d'entrée HTTP : `POST /ingest` (`apps/lc-server/src/http/ingest/submit.rs:89-130` → `MediaService::ingest` → `pipeline.ingest_url`) et `POST /overlay/meme-board/items` (`apps/lc-server/src/http/overlay/meme_board.rs:202-206`, pipeline construit localement).

**Preuve dynamique** (serveur A, `.phase0/audit/A2/probe.py`)
```
POST /ingest  {"url":"file:C:\\...\\pixel.png"}
  → 422 {"error":"media_ingestion_failed","code":"INVALID_MEDIA",
         "message":"média normalisé vide ou trop petit"}      # fichier lu, normalization tentée
POST /ingest  {"url":"file:C:\\...\\pixel.png.missing"}
  → 422 {"code":"MEDIA_NOT_FOUND","message":"fichier local introuvable"}  # oracle d'existence
POST /overlay/meme-board/items  {"url":"file:C:\\...\\pixel.png"}
  → 422 INVALID_MEDIA (même différentiel)
```
Le même différentiel (`MEDIA_NOT_FOUND` vs `INVALID_MEDIA` vs `DOWNLOAD_FAILED`) permet aussi un **oracle d'existence de fichiers** et, si le fichier ciblé est un média valide (image/audio/vidéo présent sur la machine), son **exfiltration complète** via l'URL média signée / le média board.

**Comparaison legacy** : aucune branche `file:` dans le téléchargement legacy (`mediaIngestion.downloadSourceToTempFile` → yt-dlp puis `downloadWithHttp` node-fetch, http(s) uniquement) ; le seul chemin local était `ingestMediaFromLocalFile` réservé aux commandes TTS (`talkCommand.ts:35`, `hidetalkCommand.ts:37`). ⇒ **régression de surface d'attaque** introduite par le portage.

**Impact** : tout client authentifié (token ingest, ou token overlay via `/pair-code` — accessible à tout membre de guilde, sans admin) peut faire lire au serveur des fichiers locaux de l'hôte, en exfiltrer les médias, provoquer des lectures lourdes (DoS) et sonder l'existence de fichiers.

**Correctif proposé** : dans la couche HTTP (ou `MediaService`), refuser toute source dont le schéma n'est pas `http`/`https` (`MediaSource::from_raw` + allowlist), et n'autoriser `LocalFile`/`Gtts` qu'en interne (TTS). Ajouter un test de non-régression `file:` en 400 `invalid_payload`.
**Effort** : S (1/2 j).

---

### AUD-A2-002 — SSRF / relais réseau via les URL d'ingestion (Moyenne, hérité legacy)

**Preuve dynamique**
- `POST /ingest {"url":"http://127.0.0.1:8123/x.png"}` → 422 `INVALID_MEDIA` (le serveur a bien fetché le serveur python témoin, log `GET /x.png 200`) ;
- `http://127.0.0.1:8123/missing.png` → 422 `MEDIA_NOT_FOUND` « la source a répondu HTTP 404 » ;
- `http://127.0.0.1:9/...` → 422 `DOWNLOAD_FAILED` ⇒ différentiel de codes = **oracle port/scan** interne.
- Même comportement via `POST /overlay/meme-board/items`.

**Statique** : aucune validation d'hôte/IP dans `lc-media` (`source.rs`, `download.rs`) ; client reqwest `redirect(Policy::limited(10))` (`crates/lc-media/src/http.rs:14-19`) → redirections vers des cibles internes possibles. Les branches YouTube/TikTok sont limitées aux hôtes connus, mais `DirectMedia`/`Unknown` utilise `reqwest` sur l'URL fournie.

**Comparaison legacy** : le legacy utilisait aussi node-fetch/yt-dlp sur l'URL fournie (même risque) ⇒ **risque hérité**, mais il est aggravé par AUD-A2-001 (schémas non http) et reste pertinent pour un serveur de bureau où `127.0.0.1`/LAN exposent d'autres services.

**Correctif** : allowlist de schémas + résolution DNS contrôlée (refuser loopback, link-local `169.254.0.0/16`, RFC1918, IPv6 ULA/link-local), re-validation après redirection ; option de config pour autoriser un proxy sortant. Tests unitaires du matcher d'IP.
**Effort** : M (1–2 j).

---

### AUD-A2-003 — Confinement média *fail-open* (Moyenne)

**Preuve statique** : `apps/lc-server/src/http/overlay/media.rs:242-260` — si `tokio::fs::canonicalize(&state.config.media_storage_dir)` échoue (répertoire absent, chemin relatif résolu depuis un autre cwd, volume non monté), le handler log un `warn!` puis **retourne `Ok(canonical)` sans vérifier que `canonical.starts_with(base)`**. Le même `resolve_media_file` sert `GET /overlay/media/{assetId}` et `GET /overlay/meme-board/media/{assetId}`.

**Preuve dynamique (serveur B)** : `MEDIA_STORAGE_DIR=./missing-media`, asset seedé avec `storage_path=C:\Users\SuperKnacky\code\LiveChat\docs\rust\migration-plan.md`, URL signée valide → **HTTP 200, `content-type: text/markdown`, 28 611 octets** → fichier hors du répertoire autorisé servi.
À l'inverse, quand le répertoire existe (serveur A), le même asset est bien refusé (404 `media_not_found_on_disk`).

**Impact** : si le répertoire média est mal configuré/absent (première exécution, déplacement du dossier, migration legacy avec chemins absolus en DB), tout asset dont `storage_path` pointe hors du répertoire est servi ; chaîne d'attaque possible via import de base legacy ou écriture DB (défense en profondeur). Aucun attaquant HTTP ne peut créer un `storage_path` arbitraire (le pipeline écrit sous `MEDIA_STORAGE_DIR`).

**Correctif** : fail-closed — si `canonicalize(base)` échoue, répondre 404/503 et logguer une erreur (ne jamais servir hors confinement vérifié) ; test unitaire avec répertoire base inexistant.
**Effort** : XS.

---

### AUD-A2-004 — URL média signée = capability sans liaison de session ni contrôle job/guilde (Moyenne)

**Statique** : `GET /overlay/media/{assetId}` (`media.rs:86-159`) valide uniquement `sig = HMAC(assetId|clientId|exp)`, l'expiration, le **client actif** (par id) et l'asset READY/non expiré. Aucune vérification de :
- possession du token overlay (Bearer/`?token=`), contrairement au legacy qui exigeait `resolveOverlayClientFromRequest` ;
- `playback_jobs` lié à `(mediaAssetId, guildId)` : legacy `overlayRoutes.ts:317-374` renvoyait **403 `forbidden`** sans job de la guilde (grep `playback_jobs` vide dans `http/overlay/**`) ;
- l'URL est transmise par l'overlay (éventuellement dans les logs/referer/historique) : quiconque l'obtient peut lire le média jusqu'à `exp` (≤ 1 h).

**Nuance** : le protocole v2 remplace volontairement le token par une capability signée (documenté dans `docs/rust/server-overlay.md`), et `clientId` borne la fuite à un client actif. La régression réelle est la **perte du binding job/guilde** et l'absence de preuve de possession ; à arbitrer avec la lane protocol-v2. Si l'arbitrage est « capability assumée », documenter explicitement le risque (pas de contrôle d'accès par job) et raccourcir le TTL par défaut.

**Correctif** : soit vérifier l'existence d'un job de la guilde du client (parité legacy, coût DB négligeable), soit signer aussi un nonce de session et l'exiger en header ; à défaut, TTL 300 s max et `Referrer-Policy: no-referrer` sur les pages overlay.
**Effort** : M.

---

### AUD-A2-005 — TTL d'asset prolongé à chaque lecture (Moyenne)

**Statique** : `media.rs:302-319` `touch_asset` best-effort → `expires_at = max(expires_at, now + media_cache_ttl_hours)` ; `crates/lc-db/src/repos/media_assets.rs:138-148` `touch_last_accessed` met à jour `last_accessed_at` **et** `expires_at`. Appelé avant `serve_file`, donc même une lecture avortée prolonge l'asset.

**Preuve dynamique** : asset vidéo seedé avec `expires_at = now+1 h` ; après **un seul** GET média signé, la DB affiche `expires_at ≈ now+11,98 h` (12 h = `MEDIA_CACHE_TTL_HOURS`).

**Impact** : la purge de cache ne s'applique jamais aux médias rejoués régulièrement (même anciens) → croissance disque au-delà du budget `MEDIA_CACHE_MAX_TOTAL_MB` (le quota n'est contrôlé qu'à l'ingestion, `quotas::enforce_cache_budget`), et « épinglage » implicite non voulu ; un client peut maintenir indéfiniment un asset en le lisant. Incohérence aussi avec le pin board (l'asset est épinglé pour 100 ans, mais le simple fait de le lire modifie son expiration).

**Correctif** : sur lecture, ne mettre à jour que `last_accessed_at` (pas `expires_at`) ; ou prolonger borné par l'expiration d'origine/le TTL config, et purger selon la dernière utilisation. Test : deux lectures → expiration inchangée.
**Effort** : S.

---

### AUD-A2-006 — `MediaPipeline` reconstruit par requête sur la meme board (Moyenne)

**Statique** : `apps/lc-server/src/http/overlay/meme_board.rs:202-205` — `MediaPipeline::new(media_config(&state))` est instancié **à chaque POST**, puis `ingest_url` (l.206). Contourne :
- le pipeline partagé `MediaService::shared_pipeline` (`media/service.rs:187-199`, `OnceLock`) ;
- le sémaphore global `MEDIA_MAX_CONCURRENT_INGESTIONS=2` (par-pipeline → chaque requête a son propre limiteur, donc concurrence illimitée) ;
- le budget cache (`quotas::enforce_cache_budget`), l'enregistrement `FAILED`, le cache `assets::find_reusable`, le client HTTP partagé.

**Impact** : un client authentifié peut lancer un nombre illimité d'ingestions concurrentes (yt-dlp/ffmpeg/requêtes) → épuisement CPU/disque/réseau, amplification DoS ; duplication de logique avec `MediaService` (risque de divergence).

**Correctif** : réutiliser `MediaService::for_state(&state)` (ou exposer le pipeline partagé dans `AppState`) pour POST board ; supprimer le `MediaPipeline::new` local ; test de concurrence (N requêtes simultanées bornées).
**Effort** : S.

---

### AUD-A2-007 — `forceRefresh` inopérant sur asset READY (Moyenne)

**Statique** : `meme_board.rs:217-246`. Cas `Ok(Some(existing))` avec `force_refresh == true` (ou asset non READY) : le code **a déjà téléchargé/normalisé** le média (l.206), puis :
- `update_status(existing.id, Ready, None)` et épinglage,
- **ne met à jour ni `storage_path`, ni `mime`, ni dimensions, ni `source_url`** → le nouvel objet `ingested` est jeté ; le fichier téléchargé reste orphelin dans le storage ;
- un asset non-READY (échec précédent) est repassé READY **sans re-validation** du fichier.

**Comparaison legacy** : `ingestMediaFromSource({forceRefresh:true})` ré-ingérait et mettait l'asset à jour (upsert pipeline) ; le board legacy n'avait pas cette branche « marquer READY sans rafraîchir ».

**Impact** : `forceRefresh` ne rafraîchit rien (attente fonctionnelle cassée), fuite d'objets storage, états READY mensongers.

**Correctif** : en cas de forceRefresh, remplacer les métadonnées de l'asset (nouveau `storage_path`/mime/etc.), supprimer l'ancien fichier si plus référencé ; ne jamais repasser READY sans fichier présent.
**Effort** : M.

---

### AUD-A2-008 — Le rate limit s'applique au streaming média (Moyenne)

**Statique** : `http/rate_limit.rs:40-43` : 100 requêtes / 15 min, fenêtre fixe, par IP (`ConnectInfo`), middleware appliqué au routeur global (`app.rs:182-217`) ; exemptions : `/admin*` et loopback **si `NODE_ENV=development`** ; `TRUST_PROXY_HEADERS` opt-in.

**Preuve dynamique** : 130 × `GET /overlay/config` → 78 × 429, premier 429 à l'index 52 (quota déjà entamé), `Retry-After: 844`, `x-ratelimit-limit: 100`. Les GET média signés et média board consomment le même bucket.

**Impact** : lecture vidéo via Range = dizaines de requêtes (et HEAD) → l'overlay peut être throttlé (429) en cours de lecture ; plusieurs overlays/NAT partagés peuvent épuiser le quota d'une IP ; un client malveillant peut épuiser le bucket d'une IP et bloquer les overlays. Note : le legacy avait aussi un rate limit global 100/15 min (parité), mais le passage à des URL média par requête rend le problème plus visible.

**Correctif** : exempter (ou bucket séparé) les routes média signées (HMAC = déjà authentifiées) et le média board ; ou quota par token/client plutôt que par IP ; documenter le comportement derrière proxy (`TRUST_PROXY_HEADERS`).
**Effort** : S.

---

### AUD-A2-009 — JSON malformé → 400 `request_error` au lieu de `invalid_payload` (Basse)

**Preuve dynamique (re-mesurée 02:25)** : corps JSON invalide (`{"url": `, `{bad`) sur :
- `POST /overlay/pair/consume` → **400** `{"error":"request_error"}` ;
- `POST /overlay/meme-board/items` → **400** `{"error":"request_error"}` ;
- `POST /ingest` (body vide ou syntaxe invalide) → **400** `{"error":"request_error"}`.

Cause : rejet de l'extracteur `Json` de serde (400 `JsonDataError`/syntaxe) réécrit par `normalize_errors` (`http/error.rs:53-105`) qui conserve le statut mais remplace le corps et le label. Le legacy Fastify renvoyait son propre format 400 (`statusCode`/`error`/`message`), et la doc Rust annonce `invalid_payload` pour les corps invalides.

**Impact** : contrat d'erreur divergent (les clients qui branchent sur `invalid_payload` ne reconnaissent pas l'erreur ; les tests de non-régression API doivent tolérer ce label), pas de fuite d'information. Sévérité basse.

**Correctif** : soit mapper explicitement les rejets d'extracteurs (`Json`/`Query`/`Path`) vers `400 invalid_payload` dans les handlers (ou via un `JsonRejection` handler), soit documenter que les erreurs de parsing restent `request_error` ; aligner `server-overlay.md`/`server-ingest.md`. Tests de non-régression.
**Effort** : S.

---

### AUD-A2-010 — Branche 409 morte sur conflit d'unicité (Basse)

**Statique** :
- `crates/lc-db/src/error.rs:53-56` : `DbError::is_unique_violation()` ne matche **que** `Self::Sqlx` ;
- `crates/lc-db/src/error.rs:102-108` : `map_sqlx_error` convertit UNIQUE/PRIMARY KEY en `DbError::Domain(DomainError::Conflict(...))`.
- `apps/lc-server/src/http/overlay/pair.rs:166-171` utilise `is_unique_violation()` pour répondre **409** `overlay_client_conflict` ⇒ branche jamais atteinte ; un conflit (collision de token, très improbable) produirait 500.
Même motif à vérifier côté `ingest/pair.rs:131-133` (renvoie directement `media_service_error` → 500).
Même piège dans `meme_board.rs:272` (`insert` asset) : la relecture de secours sur conflit unique ne se déclenche pas → `db_failure` 500 au lieu de relire.

**Correctif** : helper `DbError::is_conflict()` matchant `Domain(Conflict(_))` (et `is_unique_violation`), l'utiliser dans les trois sites ; test unitaire de collision (injecter un doublon en DB).
**Effort** : XS.

---

### AUD-A2-011 — Repli `?token=` : doc contredite et surface élargie (Basse)

**Statique/dynamique** : `overlay/auth.rs:109-120` accepte `?token=` uniquement si `path.starts_with("/overlay/meme-board/")` — ce qui inclut `/overlay/meme-board/media/{assetId}` ; `docs/rust/server-overlay.md` §10 #2 affirme « ?token= supprimé, Bearer obligatoire » (contradiction). Le repli s'applique aussi quand le header `Authorization` est présent mais mal formé : probe `Authorization: Bearer` (sans token) + `?token=token_ok` → **200** (comportement legacy identique : `bearer || query`).

**Impact** : token 64 hex en query string → fuite via logs/referer/historique, surface élargie au média board (même si le média board est déjà protégé par le token, l'URL le divulgue). Ce n'est pas une élévation de privilège.

**Correctif** : décision d'arbitrage : soit interdire complètement `?token=` (et corriger les tests), soit restreindre le repli au seul `GET /overlay/meme-board/items` + documenter ; mettre la doc en cohérence. Refuser le repli si un header Authorization est présent (même malformé) pour éviter l'ambiguïté.
**Effort** : S.

---

### AUD-A2-012 — Troncature `deviceName` incohérente (Basse)

**Statique/dynamique** : overlay pair tronque à 200 caractères (`pair.rs:125-135,196-203` ; probe : 300 chars → 200) ; ingest pair ne tronque pas (probe : `device_len = 300`, DB label 300). Legacy ne tronquait ni l'un ni l'autre.
**Impact** : labels incohérents entre les deux flux ; collisions de label côté overlay (deux appareils au même préfixe de 200 chars) → la révocation « même label » peut révoquer la mauvaise session ; DB/API acceptent des labels ingest très longs (limite body 1 MiB).
**Correctif** : une seule politique (borne 200 pour les deux + validation) ; test comparatif.
**Effort** : XS.

---

### AUD-A2-013 — Coûts O(N) sur la meme board (Basse)

**Statique** :
- `meme_board.rs:107-174` listing : `list_by_guild` charge **tous** les items, puis un `media_assets().get` par item (N+1), filtre `q` en Rust, pagine après filtre ⇒ mémoire/latence O(items) par requête (legacy : `count` + `findMany` paginé avec JOIN, une requête).
- `meme_board.rs:305-330` quota : `board_total_bytes` (N+1 sur les items, `meme_board.rs:593-601`) à chaque POST.
- `meme_board.rs:604-614` `board_reference_count` : boucle **toutes les guildes** puis tous leurs items à chaque DELETE (O(G·I)).
- `board_reference_count` non transactionnel avec l'ajout → course possible : un item ajouté entre le comptage et la libération du pin peut référencer un asset dont l'expiration est raccourcie (board item pointant vers un média expiré ensuite).
**Impact** : dégradation progressive (board jusqu'à des milliers d'items ; quota MB n'empêche pas des items minuscules), amplification par requêtes authentifiées.
**Correctif** : requêtes SQL agrégées (`SUM(size_bytes) JOIN`), filtre/pagination en SQL, `COUNT(*)` pour les références ; transaction pour DELETE.
**Effort** : M.

---

### AUD-A2-014 — `coerce_flag` ignore `"on"` (Basse)

**Statique** : `meme_board.rs:665-677` `coerce_flag` accepte bool, `1`, `"1"`, `"true"`, `"yes"` mais **pas `"on"`** ; legacy `server/src/services/stringUtils.ts:13` `toBooleanFlag` accepte `'on'`/`'On'` (tests `stringUtils.test.ts:74-98`), et la doc `server-overlay.md` liste `on`. ⇒ `forceRefresh:"on"` sur POST board est interprété `false` (divergence de coercition, silencieuse).
**Correctif** : ajouter `"on"` (et casse/trim) dans `coerce_flag`, test comparatif avec `boolean_flag` d'ingest.
**Effort** : XS.

---

### AUD-A2-015 — 416 avec corps JSON ; compression média non reproduite (Basse)

- Probe : `Range: bytes=900000-` sur média 788 493 o → **416** avec `content-range: bytes */788493` **et corps `{"error":"request_error"}`** ; legacy renvoyait un corps vide. Cause : `ServeFile` + `normalize_errors`. Cosmétique/parité.
- Compression : `Accept-Encoding: gzip` sur média image 68 o et sur asset 788 493 o → 200 **sans `content-encoding`** (ni média signé, ni média board), alors que `server-overlay.md` §12.1 prétend que la compression globale peut compresser les réponses média 200 (`app.rs::compression_predicate` n'exclut pas `image/*`). Non reproduit sur le binaire audité ⇒ **doc obsolète** (ou binaire antérieur au prédicat) ; à re-vérifier après rebuild et à corriger dans la doc.
**Effort** : XS (doc) / S (test).

---

### AUD-A2-016 — `/overlay/config` supprimé (Info)

Probe : `GET /overlay/config` → 404 `request_error`. Documenté comme écart (`server-overlay.md` §10) mais **breaking** : tout overlay client legacy (protocol v1) appelant cette route échoue. À traiter dans la matrice de compatibilité/migration overlay (protocol v2 = nouvel overlay).
**Effort** : XS.

---

### AUD-A2-017 — Auth ingest : liste complète des clients à chaque requête non authentifiée (Basse)

`apps/lc-server/src/http/ingest/auth.rs:85-88` : `client_is_enabled` = `ingest_clients().list()` + `any(is_active)` pour décider 401 vs 503, à chaque requête sans token. Table petite en pratique (clients appairés), mais O(n) DB par requête anonyme (amplification DoS si table gonflée). Legacy : `findFirst` (coût O(1)).
**Correctif** : requête `findFirst(revoked_at IS NULL)` ou colonne/flag.
**Effort** : XS.

---

## 3. Couverture endpoint par endpoint

| Endpoint | Vérifications | Résultat |
|---|---|---|
| `GET /health` | statut, DB, Discord | 200 OK ; `{status:ok, version:3.0.0, protocolVersion:2, db:ok, discord:disabled}` |
| `POST /overlay/pair/consume` | code normal/invite/absent/inconnu/expiré/concurrent, deviceName 300, JSON malformé, DB absente | OK (consume atomique, mode DB, TTL, révocation même label, 400/404) sauf JSON→400 `request_error` (009) et 409 mort (010) |
| `GET /overlay/media/{assetId}` | sig valide/invalide, exp passée, client révoqué/inconnu, asset READY/expiré, fichier absent/hors dir, Range/HEAD/416, compression, content-type/no-store | OK sauf fail-open (003), absence de binding job/guilde (004), TTL glissant (005), 416/compression (015) |
| `GET /overlay/meme-board/items` | Bearer/`?token=`, origine, pagination/filtre/q, guildId étranger | OK (tests) ; `?token=` (011), coûts O(N) (013) |
| `POST /overlay/meme-board/items` | auth, url manquante, JSON malformé, file:/http:/port fermé, dédoublonnage, quota, forceRefresh | OK sauf 001/002/006/007/009/014 |
| `GET /overlay/meme-board/media/{assetId}` | auth, binding (guilde,asset), asset READY, Range/HEAD | OK ; mêmes réserves 003/005/011 |
| `PATCH /overlay/meme-board/items/{id}` | champs absents/nulls, item autre guilde, JSON invalide | OK (parité `{updated:true,item:{id,title,message}}`) |
| `DELETE /overlay/meme-board/items/{id}` | item autre guilde, libération pin, `{deleted}` | OK ; O(G·I) + course (013) |
| `GET /overlay/config` | route absente | 404 `request_error` (016) |
| `POST /ingest/pair/consume` | code absent/inconnu/expiré/invite/normal, deviceName 300, auth | OK (403 scope invite, 400 missing, 404) ; 010/012 |
| `POST /ingest` | sans token (503/401), token inconnu/révoqué, guild mismatch, authorName, contenu manquant, saveToBoard sans source, url http/file, port fermé, body 2 MiB | OK sauf 001/002/008/009 |
| Limites de corps | 2 MiB → 413 `request_error` ; JSON 1 MiB global | OK |
| Rate limit | 130 requêtes → 429 après ~100, headers, Retry-After | OK mais média inclus (008) |
| Erreurs normalisées | non-JSON 4xx/5xx → `request_error`/`internal_error` | OK (effet de bord 009/015) |

**Tests ciblés exécutés** : `cargo test -p lc-server --lib http::overlay` → **15 passed** ; `http::ingest` → **18 passed** (aucun réseau requis, DB mémoire). Les probes dynamiques couvrent le reste (voir §5).

---

## 4. Parité legacy — écarts relevés

| Sujet | Legacy TS | Rust v2 | Verdict |
|---|---|---|---|
| Consommation code appairage | `findFirst` puis `update` (TOCTOU) | `consume()` UPDATE atomique | **Amélioration** |
| Code invite (overlay) | mode DB, sans expiration | idem + `deviceName` tronqué 200 | Écart mineur (012) |
| Code invite côté ingest | 403 `pairing_code_scope_mismatch` | idem (probe) | Parité |
| `?token=` | accepté sur **toutes** les routes overlay | limité à `/overlay/meme-board/*` | Restriction (doc à corriger, 011) |
| Média overlay | token + `playbackJob` guilde obligatoire (403) | HMAC `assetId\|clientId\|exp` ; **aucun contrôle job/guilde** | **Régression fonctionnelle/sécurité (004)** |
| TTL média | asset expires inchangé à la lecture (`touchMediaAsset` best-effort) | `expires_at = max(..., now+12 h)` | **Écart (005)** |
| Range/416 | 416 corps vide + Content-Range | 416 + corps JSON | Cosmétique (015) |
| Compression média | non applicable | doc §12.1 annonce compression possible ; non reproduite | Doc obsolète (015) |
| Pair réponse | pas de `protocolVersion` | + `protocolVersion:2` | Additif acceptable |
| `/overlay/config` | existe | supprimé | Breaking documenté (016) |
| Meme board listing | SQL paginé + JOIN asset | chargement total + N+1 + filtre Rust | Écart perf (013) |
| Meme board création | `ingestMediaFromSource` partagé (cache, sémaphore) | pipeline neuf par requête | **Écart sécurité/perf (006)** |
| `forceRefresh` board | ré-ingestion + mise à jour asset | asset READY marqué READY sans mise à jour | **Écart fonctionnel (007)** |
| Coercition flags | `toBooleanFlag` (bool, 1, `"1"`,`"true"`,`"yes"`,`"on"`, trim/casse) | ingest `boolean_flag` OK ; board `coerce_flag` sans `"on"` | Écart mineur (014) |
| deviceName | jamais tronqué | overlay 200 / ingest illimité | Incohérence (012) |
| `file:` | refusé (pas de chemin local hors TTS) | accepté par `lc-media` via HTTP | **Régression (001)** |
| SSRF | URL arbitraire via node-fetch/yt-dlp | URL arbitraire via reqwest | Risque hérité (002) |
| Auth ingest | Bearer uniquement ; `isIngestApiEnabled` = findFirst | idem + `client_is_enabled` = list() | Parité fonctionnelle ; perf (017) |
| Tweets `/ingest` | branche rich tweet (réseau) | non portée (écart déclaré) | Écart assumé (Phase 6b) |
| Erreurs inattendues | 500 `internal_error` | 500 `internal_error` | Parité |
| JSON malformé | 400, format Fastify (`statusCode`/`error`/`message`) | 400 `request_error` (label différent) | Écart mineur de contrat (009) |
| Conflit unique → 409 | n/a | branche morte (500) | Bug (010) |

---

## 5. Méthode et reproductibilité

### 5.1 Environnement de probe (isolé)
- `.phase0/audit/A2/server` — serveur A : `PORT=3411 NODE_ENV=production API_URL=http://127.0.0.1:3411 DATABASE_URL=file:./livechat.sqlite MEDIA_STORAGE_DIR=./media MEDIA_URL_SECRET=probe-secret I18N=fr` ; DB migrée automatiquement (schema 3.0.0) ; `media/mov_bbb.mp4` (788 493 o) + `media/pixel.png` (68 o).
- `.phase0/audit/A2/serverB` — serveur B : `PORT=3412 MEDIA_STORAGE_DIR=./missing-media MEDIA_URL_SECRET=probe-secret-b`.
- Témoin SSRF : `python -m http.server 8123 --bind 127.0.0.1` servant une image.
- Seeds SQLite (guildes `111111111111111111`/`222222222222222222`, codes `PROBE1/2/3/PROBEXP`, `ING1..ING4`, clients `token_ok`/`token_revoked`, `tok_ingest`, assets vidéo/image/hors-dir).
- Scripts : `.phase0/audit/A2/probe.py` → `probe-results.json` ; `probe2.py` → `probe-results-2.json` ; logs `server.log`/`serverB.log`.
- **Note opérationnelle** : les deux jobs serveur se sont arrêtés en fin de commande de probe (comportement du harness) ; pour rejouer, démarrer serveur + probes **dans la même commande** pwsh.

### 5.2 Commandes clés de reproduction
```powershell
# serveur A
$env:PORT='3411'; $env:NODE_ENV='production'; $env:API_URL='http://127.0.0.1:3411'
$env:DATABASE_URL='file:./livechat.sqlite'; $env:MEDIA_STORAGE_DIR='./media'
$env:MEDIA_URL_SECRET='probe-secret'; $env:I18N='fr'
.\target\debug\lc-server.exe
# pair normal
curl.exe -s -X POST http://127.0.0.1:3411/overlay/pair/consume -H 'content-type: application/json' -d '{"code":"PROBE1","deviceName":"ProbeOverlay"}'
# asset local (finding 001)
curl.exe -s -X POST http://127.0.0.1:3411/ingest -H 'authorization: Bearer tok_ingest' -H 'content-type: application/json' -d '{"url":"file:C:\\...\\pixel.png"}'
# fail-open (finding 003, serveur B)
curl.exe -s -o out.bin -w '%{http_code} %{content_type}\n' 'http://127.0.0.1:3412/overlay/media/<assetId>?clientId=...&exp=...&sig=...'
# rate limit
1..130 | % { curl.exe -s -o NUL -w "%{http_code}\n" http://127.0.0.1:3411/overlay/config }
```

### 5.3 Tests ciblés
- `cargo test -p lc-server --lib http::overlay` → 15/15 verts (pair, média signé, board, config absent).
- `cargo test -p lc-server --lib http::ingest` → 18/18 verts (auth, body, pair, submit).

---

## 6. Limites de l'audit / zones non couvertes

1. **Pas de test réseau réel** : YouTube/TikTok/Twitter et tweets non exercés (le réseau et ces branches sont hors périmètre Phase 5b) ; les conclusions SSRF reposent sur les URL http(s) directes, suffisantes pour l'oracle interne.
2. **Branches `saveToBoard` réussies, `POST /ingest` 201 avec vrai média** non testées dynamiquement (nécessitent un média réel fixe) ; couvertes par tests unitaires (`text_only_submission_creates_a_pending_job`, `save_to_board_requires_a_media_source`) mais pas de bout en bout HTTP avec DB réelle.
3. **PATCH/DELETE board, list/filtre** vérifiés par tests unitaires (pas de probe HTTP exhaustif).
4. **Compression** : non reproduite sur le binaire audité ; à re-tester après rebuild (peut dépendre d'une version antérieure du `compression_predicate`).
5. **Charge/perf** : pas de benchmark ; les findings O(N) sont statiques.
6. **TLS/reverse-proxy** : pas de test HTTPS, ni de comportement `X-Forwarded-For` en production (`TRUST_PROXY_HEADERS` opt-in) ; rate limit par IP avec NAT/partage non mesuré à l'échelle.
7. **Concurrence DB** : seule la course sur la consommation de code a été testée ; races board/delete non reproduites.
8. **WS/overlay protocol v2** : hors périmètre (autres lanes) — les interactions `sessionMode` (invite read-only) et les autorisations d'écriture board pour ce mode n'ont pas été auditées ici.

---

## 7. Recommandations priorisées

1. **Bloquer les sources non HTTP(S)** dans les chemins HTTP (AUD-A2-001) — XS/S, à faire avant toute exposition.
2. **Fail-closed** du confinement média (AUD-A2-003) + test répertoire absent — XS.
3. **SSRF** : allowlist schémas + bloqueurs d'IP internes/re-validation après redirect (AUD-A2-002) — M.
4. **Média signé** : arbitrer et implémenter le binding job/guilde **ou** documenter la capability + TTL court + `Referrer-Policy` (AUD-A2-004) — M.
5. **Ne plus prolonger `expires_at` à la lecture** ; purger sur `last_accessed_at` (AUD-A2-005) — S.
6. **Router la création board via `MediaService`** partagé (sémaphore/cache/quotas) (AUD-A2-006) — S.
7. **Corriger `forceRefresh`** + suppression de l'ancien fichier (AUD-A2-007) — M.
8. **Rate limit** : bucket séparé/exemption HMAC pour le streaming (AUD-A2-008) — S.
9. **Erreurs de parsing** : mapper vers 400 `invalid_payload` ou documenter `request_error` (AUD-A2-009) — S.
10. **Helper `is_conflict`** et branches 409/relire (AUD-A2-010) — XS.
11. Cohérence `?token=` doc/code + refus du repli si header present (AUD-A2-011) — S.
12. Politique unique `deviceName` (AUD-A2-012), `"on"` dans `coerce_flag` (AUD-A2-014) — XS.
13. Requêtes SQL agrégées board (AUD-A2-013) — M.

**Décision de release** : AUD-A2-001 + AUD-A2-003 sont à corriger avant exposition (même locale) ; AUD-A2-002/004/005/006/007 à couvrir en Phase 6b/7 ; le reste en dette documentée.

---

## 8. Annexes

### 8.1 Findings → tests de non-régression proposés

| ID | Test proposé |
|---|---|
| 001 | `ingest_rejects_file_source` et `board_rejects_file_source` (400 `invalid_payload`, aucun accès disque) |
| 002 | `ingest_rejects_loopback_and_private_ips` (unit test du matcher + handler) |
| 003 | `resolve_media_file_fails_closed_when_base_missing` |
| 004 | `signed_media_requires_playback_job_for_guild` (ou test documentant la capability) |
| 005 | `reading_media_does_not_extend_expiry` |
| 006 | `board_create_uses_shared_pipeline` (concurrence accompagnée) |
| 007 | `board_force_refresh_updates_asset_metadata` |
| 008 | `media_routes_are_exempt_from_ip_rate_limit` (ou bucket dédié) |
| 009 | `malformed_json_returns_400_invalid_payload` (pair + board + ingest) |
| 010 | `unique_conflict_returns_409_or_reloads_asset` |
| 011 | `query_token_rejected_when_authorization_header_present` |
| 012 | `device_name_is_truncated_consistently` |
| 013 | `board_listing_uses_sql_pagination` (benchmark) |
| 014 | `coerce_flag_accepts_on` |
| 015 | doc mise à jour compression/416 |

### 8.2 Fichiers lus (sources principales)

- `apps/lc-server/src/http/overlay/{mod,auth,pair,media,meme_board,tests}.rs`
- `apps/lc-server/src/http/ingest/{mod,auth,body,pair,submit}.rs`
- `apps/lc-server/src/http/{error.rs,rate_limit.rs}`, `apps/lc-server/src/media_url.rs`, `apps/lc-server/src/media/{service.rs,board.rs,jobs.rs}`, `apps/lc-server/src/app.rs`
- `crates/lc-media/src/{download.rs,source.rs,ingest.rs,process.rs,http.rs}`
- `crates/lc-protocol/src/lib.rs` (signature/TTL), `crates/lc-db/src/{error.rs,repos/media_assets.rs}`
- Legacy : `server/src/components/overlay/overlayRoutes.ts`, `server/src/components/ingest/ingestRoutes.ts`, `server/src/services/{overlayAuth.ts,ingestAuth.ts,memeBoard.ts,stringUtils.ts}`, `server/src/services/media/{mediaSourceResolver.ts,mediaIngestion.ts}`, `server/src/utils/overlay/mediaStreaming` (logique Range legacy)
- Docs : `docs/rust/server-overlay.md`, `docs/rust/server-ingest.md`
