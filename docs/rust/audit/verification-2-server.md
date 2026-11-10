# Vérification V2-A — correctifs P0/P1 côté serveur (re-audit adversarial)

- Date : 2026-11-10
- Auditeur : `audit-dynamique` (lane serveur), auteur de l'audit A2 mais **sans** participation aux vagues P0/P1.
- Branche : `feat/rust-rebuild`, HEAD `63e12f5` (`fix(p1): close major audit findings (COR-05..COR-11)`), parent `ab97b1c` (`fix(p0)`).
- Périmètre : serveur Rust (`apps/lc-server`), base (`crates/lc-db`), média (`crates/lc-media`), playback (`crates/lc-playback`), migrations, déploiement.
- Méthode : lecture statique + `grep` de preuve, diff `ab97b1c..63e12f5` (80 fichiers, +4490/−663). Aucun serveur lancé, aucun fichier de code modifié, aucun commit (`git status --porcelain` vide au moment de la rédaction).
- Référentiels : `docs/rust/audit/p0-correctifs.md`, `docs/rust/audit/p1-correctifs.md`, `docs/rust/audit/backlog-corrections.md`.

## 1. Verdict global

| Vague | Verdict | Commentaire |
|---|---|---|
| **P0** (COR-01..04) | **TENU sur pièces** | Garde SSRF/LFI câblée et défense en profondeur, ledger migrateur présent, `session_mode` effectif, gate CI déclaré (non rejoué ici). |
| **COR-05** Jetons hors URL/logs | **TENU** | Bearer-only admin, query ignorée, logs chemin seul, `app.js` en `localStorage`. |
| **COR-06** Origin / plafonds WS | **TENU** (1 nit) | Allowlist effective, caps 3/16 + 4409, timeout 30 s avec exclusions ciblées. |
| **COR-07** Texte-seul / révocation / purge | **TENU** | `play` texte-seul émis sans média, `stop`+4402 sur révocation, `stop` purge via `AdminHub`. |
| **COR-08** Invariants DB / migrateur | **TENU** (1 nit) | Migration 0002 + index partiel, retry BUSY, `mode=ro`, récupération `.old`. |
| **COR-09** Confinement média & pipeline | **PARTIEL** | Fail-closed, HMAC, cap TTL, publication atomique, pipeline partagé = OK ; mais board non routé par `MediaService` (cache/quotas/FAILED) et **worker de purge** non durci (cf. §3). |
| **COR-10** Rate-limit admin / compose | **TENU** | 429 avant auth, admin plus exempté hors dev-loopback, bind `127.0.0.1`, `ADMIN_UI_LOCAL_ONLY`. |
| **Fix purge prioritaire** (`normalized_path`) | **PARTIEL** | Le cas de la régression (`\\?\` / absolu non canonicalisé) est corrigé et testé ; **relatifs, symlinks, hors-racine et fichiers partagés ne sont pas couverts**. |

**Compteurs** : 0 Bloquant · 3 Majeurs · 1 Mineur · 4 Nits. Aucune régression P0 détectée.

## 2. Points forts observés

- `crates/lc-media/src/guard.rs` est une vraie garde centrale (allowlist de schémas, refus des IP internes, épinglage d'IP anti-rebinding, redirections revalidées max 3) et elle est **propagée** : `crates/lc-media/src/http.rs:52-61`, `crates/lc-media/src/ingest.rs:151`, `crates/lc-media/src/download.rs:107`, plus pré-validation HTTP `apps/lc-server/src/http/ingest/submit.rs:93` (`source::validate_ingest_source`), qui refuse `file:`/`gtts:`/UNC/relatif (`crates/lc-media/src/source.rs:305-339`).
- Service média désormais strictement fail-closed : racine canonicalisée au démarrage (`apps/lc-server/src/lib.rs:130-138`, `apps/lc-server/src/media/confinement.rs:22+`) et refus 404 si la base ne se canonicalise pas côté handler (`apps/lc-server/src/http/overlay/media.rs:231-275`, en particulier 265-272).
- URL média signée : format `assetId|clientId|exp`, HMAC-SHA256, comparaison `subtle::ConstantTimeEq`, expiration inclusive, cap `exp ≤ now + 3600 s` (`apps/lc-server/src/media_url.rs:48-105` ; application `apps/lc-server/src/http/overlay/media.rs:126`), y compris pour les URL injectées dans `play` WS (`apps/lc-server/src/ws/hub.rs:872` via `expiry_for`).
- Publication atomique : `staged_path` + `File::open(..).sync_all()` + `fs::rename` + fsync répertoire (`crates/lc-media/src/transcode.rs:119-128/176-208/228-287`, `crates/lc-media/src/storage.rs:197-225`).
- La régression de purge signalée par le Lead est bien comprise et testée : `keeps_referenced_files_when_storage_path_is_not_canonical` (`apps/lc-server/src/media/purge.rs:544`) couvre le chemin absolu non canonicalisé comparé aux entrées `read_dir` préfixées `\\?\`.

## 3. Priorité 1 — vérification du worker de purge (`apps/lc-server/src/media/purge.rs`)

Modèle : `purge_once` (`purge.rs:71-108`) enchaîne FAILED, PROCESSING, READY expirés, puis balayage des fichiers. Le balayage (`purge_storage_files`, `purge.rs:208-257`) construit `known` avec `normalized_path()` de tous les `storage_path` (217-227), liste `list_variants()` (`crates/lc-media/src/storage.rs:240-281`) et supprime après `ORPHAN_GRACE` (1 h) tout fichier absent de `known` (250-253). `normalized_path` = `std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())` (`purge.rs:262-264`).

| Cas demandé par le Lead | Verdict | Preuve |
|---|---|---|
| Chemin relatif en DB | **NON TENU** | Le repli brut de `normalized_path` compare une chaîne relative (résolue contre le **cwd du process**) à un chemin canonique absolu ; aucun test relatif. Voir AUD-V2-S-002. |
| Préfixe `\\?\` | **TENU** | Canonicalisation des deux côtés ; test `keeps_referenced_files_when_storage_path_is_not_canonical`. |
| Symlink dans la racine | **NON TENU** | `list_variants` suit le symlink (`entry.metadata()`), `normalized_path` résout la cible hors racine, puis `remove_path` supprime la cible. Voir AUD-V2-S-001. |
| Chemin hors racine (suppressions) | **NON TENU** | `purge_stale_status` et `purge_expired_ready` suppriment `Path::new(&storage_path)` **brut** (`purge.rs:139`, `purge.rs:199`) sans confinement ; idem purge admin (`apps/lc-server/src/http/admin/guilds.rs:273`). Voir AUD-V2-S-002. |
| Deux assets sur le même fichier | **NON TENU** | `purge_expired_ready` supprime le fichier de la ligne expirée sans vérifier les autres lignes ; aucune contrainte d'unicité. Voir AUD-V2-S-004. |

## 4. Findings

### AUD-V2-S-001 — Le balayage d'orphelins peut supprimer un fichier **hors** de `MEDIA_STORAGE_DIR` via symlink

- **Sévérité** : Majeur · **Confiance** : haute (statique) · **Effort correctif** : S.
- **Preuve** : `apps/lc-server/src/media/purge.rs:250-253` (comparaison `known`), `apps/lc-server/src/media/purge.rs:262-264` (canonicalisation), `crates/lc-media/src/storage.rs:240-281` (`list_variants` suit les symlinks et garde `entry.path()`), `crates/lc-media/src/storage.rs:179-189` (`remove_path` = `fs::remove_file` sans confinement).
- **Scénario** : un lien (fichier) présent dans le répertoire média pointe vers un fichier hors racine ; après `ORPHAN_GRACE`, `variant_path` devient la cible canonique hors racine, absente de `known`, et `remove_path` supprime la cible. Uniquement des fichiers déjà présents dans le dossier média (pas d'écriture arbitraire via HTTP), mais destruction hors périmètre pour tout opérateur/outil local.
- **Correctif** : ignorer les entrées `entry.file_type()?.is_symlink()` (ou exiger `fs::symlink_metadata`), puis ne supprimer que si `canonical.starts_with(root)`. Ajouter 2 tests (symlink fichier/dossier).
- **Test de non-régression** : symlink dans `list_variants` → 0 suppression, cible intacte.

### AUD-V2-S-002 — Suppressions de fichiers non confinées et dépendantes du cwd (risque de perte de données référencées)

- **Sévérité** : Majeur · **Confiance** : moyenne-haute (statique, scénarios de déploiement) · **Effort** : M.
- **Preuve** :
  - `apps/lc-server/src/media/purge.rs:213-227` (ensemble `known`), `:262-264` (repli brut si `canonicalize` échoue) ;
  - `apps/lc-server/src/media/purge.rs:139` et `:199` : suppression de `Path::new(&storage_path)` **sans canonicalisation ni confinement** ;
  - `apps/lc-server/src/config.rs:49` et `:376-377` : `MEDIA_STORAGE_DIR` par défaut **relatif** (`./data/media`) ; `crates/lc-media/src/ingest.rs:191-196` + `apps/lc-server/src/media/settings.rs:28` écrivent ce répertoire tel quel dans `storage_path` ⇒ nouvelles lignes DB **relatives** par défaut ;
  - les `storage_path` legacy, eux, sont **absolus** (`server/src/services/media/mediaCache.ts:25-35`, `mediaTranscode.ts:315/323/338`), copiés tels quels par `xtask/src/migrate/tables.rs:87` ;
  - purge admin : `apps/lc-server/src/http/admin/guilds.rs:273`.
- **Scénarios** : (a) DB alimentée avec le défaut relatif puis serveur relancé avec un cwd différent et `MEDIA_STORAGE_DIR` absolu (cas conteneur : `/app/data/media` + WORKDIR différent) → `canonicalize("./data/media/...")` échoue, repli brut ≠ chemin canonique du fichier présent dans la racine → fichier **référencé** traité en orphelin et supprimé après 1 h ; (b) `purge_stale_status`/`purge_expired_ready` suppriment un `storage_path` absolu **hors** racine (import legacy non réaligné, DB éditée) ; (c) la purge admin de guilde fait de même.
- **Correctif** : (1) résoudre chaque `storage_path` contre la **racine média canonicalisée** (pas le cwd), refuser les chemins qui sortent de la racine, et n'utiliser `canonicalize` que pour la comparaison après vérification `starts_with` ; (2) au démarrage, canonicaliser/normaliser les `storage_path` relatifs ou refuser de démarrer si la configuration est ambiguë ; (3) confiner toutes les suppressions (`purge_*` et admin) ; (4) tester : relatif + cwd différent, absolu hors racine, migration legacy.
- **Test de non-régression** : DB avec `storage_path` relatif valide mais cwd différent → fichier non supprimé ; `storage_path` hors racine → jamais supprimé.

### AUD-V2-S-003 — COR-09 PARTIEL : le board contourne encore `MediaService` (cache, quotas, FAILED, `forceRefresh`)

- **Sévérité** : Majeur (écart au correctif annoncé, pas une faille d'accès) · **Confiance** : haute · **Effort** : M.
- **Preuve** : `apps/lc-server/src/http/overlay/meme_board.rs:177-369` appelle directement le pipeline partagé (`meme_board.rs:218-230`), puis gère lui-même l'asset ; il ne passe **pas** par `MediaService::ingest` (`apps/lc-server/src/media/service.rs:98-167`) qui, lui, applique `assets::find_reusable` (cache), `quotas::enforce_cache_budget`, `record_failure` (status FAILED) et la mise à jour des métadonnées. Sur asset existant avec `force_refresh=true`, le code fait `update_status(Ready)` + pin **sans réécrire `storage_path`/`mime`/dimensions** (`meme_board.rs:217-287`) ⇒ le fichier fraîchement téléchargé/normalisé devient orphelin et le « refresh » ne rafraîchit rien.
- **Impact** : `forceRefresh` répété sur une même URL → re-téléchargement + normalisation à chaque POST (CPU/disque), orphelins jusqu'à purge ; budget cache non appliqué au board (seul le quota board l'est) ; assets FAILED du board non enregistrés → diagnostic incomplet. Le correctif P1 dit « cache/quotas/limites appliqués au board » : **seul le pipeline partagé est tenu**.
- **Correctif** : faire du POST board un appel à `MediaService` avec option « save to board », ou factoriser cache/quotas/record_failure/métadonnées. Test : deux POST identiques → un seul download ; `forceRefresh` → `storage_path`/mtime mis à jour, un seul asset.

### AUD-V2-S-004 — Fichiers partagés entre assets et course `remove_variants`

- **Sévérité** : Mineur · **Confiance** : haute (statique) · **Effort** : S.
- **Preuve** : `apps/lc-server/src/media/purge.rs:156-205` : suppression conditionnelle de la ligne puis `storage.remove_variants(&source_hash)` (197) et `remove_path(&storage_path)` (199) **sans** vérifier si un autre `media_assets.storage_path` référence le même fichier ; la passe FAILED/PROCESSING (`:112-153`) supprime aussi le fichier immédiatement, sans grâce. `remove_variants` (`crates/lc-media/src/storage.rs:150-158`) supprime tous les variants du hash sans regarder les lignes.
- **Scénarios** : deux lignes pointant le même fichier (copie/import/édition DB, `storage_path` dupliqué) → la purge d'une ligne casse l'autre ; une ingestion du même `source_hash` démarrée entre le DELETE et `remove_variants` peut voir ses fichiers supprimés (fenêtre courte, pas de orphan grace).
- **Correctif** : avant suppression, `SELECT COUNT(*) FROM media_assets WHERE storage_path = ?` (ou normaliser les chemins dans une colonne dédiée) ; ajouter une grâce sur `remove_variants` ou le faire sous verrou d'ingestion.

### AUD-V2-S-005 — Worker de purge : arrêt non lié au shutdown, observabilité et coût

- **Sévérité** : Nit · **Confiance** : haute · **Effort** : S.
- **Preuve** : `apps/lc-server/src/lib.rs:151-157` (« handle délibérément détaché ») ; `apps/lc-server/src/media/purge.rs:271-309` (boucle purge + `sleep(PURGE_INTERVAL)`, aucun signal d'arrêt, aucun compteur) ; `known` recalculé à chaque passe (`purge.rs:217-227`).
- **Impact** : la purge peut s'exécuter pendant le drain ; pas de métrique du `PurgeReport` ; coût O(lignes + fichiers) toutes les 10 min acceptable mais non borné en nombre de fichiers.
- **Correctif** : `JoinHandle` arrêté par `Shutdown` (ou `tokio::select!`), log structuré du `PurgeReport`, éventuel plafond.

### AUD-V2-S-006 — Rate-limit 100/15 min appliqué aussi au streaming média (parité legacy, risque opérationnel)

- **Sévérité** : Nit · **Confiance** : haute · **Effort** : S.
- **Preuve** : `apps/lc-server/src/http/rate_limit.rs:40-44` (100 req / 15 min / IP), `:226-228` (seules exemptions : dev + loopback) ; les routes `/overlay/media/` et `/overlay/meme-board/media/` ne sont pas exemptées, chaque requête `Range` consomme un jeton ; `apps/lc-server/src/app.rs` (timeout exempte ces chemins, pas le rate-limit).
- **Impact** : lecture vidéo par plages derrière NAT partagé → 429 en cours de lecture ; parité legacy (même limite globale) mais le nouveau client v2 peut multiplier les plages. Recommandation : budget dédié ou exemption des GET média (le token/HMAC reste la barrière).

### AUD-V2-S-007 — Branches « unique violation » mortes (hérité, backlog AUD-COR-13)

- **Sévérité** : Nit · **Confiance** : haute · **Effort** : S.
- **Preuve** : `crates/lc-db/src/error.rs:53-56` (`is_unique_violation` ne matche que `Self::Sqlx`) vs `conflict_on_unique` (`error.rs:130-138`) utilisé par `overlay_clients::create`, `ingest_clients::create`, `meme_board::create`, `pairing_codes::create` (mapping UNIQUE → `DbError::Domain(DomainError::Conflict)`). Donc `apps/lc-server/src/http/overlay/pair.rs:166-171` (`409 overlay_client_conflict`), `apps/lc-server/src/http/overlay/meme_board.rs:296/379` et `apps/lc-server/src/media/board.rs:89` ne sont atteints que si repos renvoient Sqlx. Collisions de hash 32 octets improbables, mais les branches de course board le sont moins.
- **Correctif** : matcher aussi `DbError::Domain(DomainError::Conflict(_))` (déjà référencé backlog AUD-COR-13).

### AUD-V2-S-008 — Exemptions de timeout par préfixe de chemin

- **Sévérité** : Nit · **Confiance** : haute · **Effort** : S.
- **Preuve** : `apps/lc-server/src/app.rs:81-86` : `starts_with("/ingest")`, `starts_with("/overlay/media/")`, `starts_with("/overlay/meme-board/")`, `== "/ws"`.
- **Impact** : toute future route `/<prefixe>*` héritera silencieusement de l'exemption ; aujourd'hui le périmètre correspond aux routes attendues. Recommandation : correspondance exacte ou liste de `Method+Path` normalisés, test dédié.

## 5. Verdicts détaillés par COR

### COR-05 — TENU
- `apps/lc-server/src/http/admin/auth.rs` : ordre `503 admin_ui_disabled` → `403 admin_local_only` → `401 unauthorized` (50-80) ; `request_token` lit uniquement `Authorization: Bearer` (129-144) et comparaison à temps constant (147-156) ; `?token=` explicite­ment ignoré (commentaire 126-128, tests 184-191). Tests HTTP : `apps/lc-server/src/http/admin/tests.rs:358-376` (query refusée) ; journalisation chemin seul : `apps/lc-server/src/app.rs:269-294` (`uri.path()`), `apps/lc-server/src/http/admin/mod.rs:126-149` ; `static/admin/app.js` lit `localStorage` et ne pose plus de `?token=` (test `admin/tests.rs:1262-1281`). Les logs de révocation n'exposent que ids/labels (`admin/clients.rs:57-62`, `:100-104`).

### COR-06 — TENU (nit S-008)
- `apps/lc-server/src/ws/session.rs:138` + `origin_allowed` (394-435) : allowlist API_URL/tauri.localhost/`ADDITIONAL_CORS_ORIGINS`/`WS_ALLOWED_ORIGINS`, refus `null`/`data:`/`file:`, loopback dev seulement ; close 1008 testée (`ws/tests.rs:985-1008`).
- Caps : `apps/lc-server/src/ws/hub.rs:44/47` (3 par client, 16 par guilde), comptage sous verrou (`hub.rs:150-194`), close 4409 (`ws/session.rs:219-240`), test `ws/tests.rs` (caps).
- Timeout 30 s : `apps/lc-server/src/app.rs:78/81-86/93-105` → 408 `request_timeout` hors `/ws`, `/overlay/media/`, `/overlay/meme-board/`, `/ingest` ; ordre layers `app.rs:238-266`.

### COR-07 — TENU
- Texte-seul : `crates/lc-playback/src/engine.rs:688` ne FAIL que si `media_asset_id.is_some()` et média non prêt ; `payload::build_play_payload` accepte `Option<&MediaAsset>` et émet `media=null` (`payload.rs:35/83/103-120`) ; compté dans les destinataires (probes P1).
- Révocation : `apps/lc-server/src/ws/hub.rs:571-608` émet `stop {reason:"revoked"}` puis `Close(4402)` ; purge : `hub.rs:668-676` (`broadcast_purge`, `StopReason::Purge`) ; tests `ws/tests.rs:1181-1254`.
- Note : « aucun overlay connecté ⇒ job FAILED » (`engine.rs:727`) reste le comportement de repli, identique au legacy.

### COR-08 — TENU (nit S-007)
- `crates/lc-db/migrations/0002_playback_single_playing.sql` : normalisation des doublons PLAYING (CTE `ROW_NUMBER() OVER (PARTITION BY guild_id…)`) puis index unique partiel `WHERE status='PLAYING'`.
- `crates/lc-db/src/retry.rs` : 4 tentatives, backoffs 25/50/100 ms, uniquement sur `is_busy()` ; appliqué aux chemins de claim (`repos/playback_jobs.rs:162-215`, `crates/lc-playback/src/store.rs:73-188`) ; `set_status(Playing)` traduit la violation unique en `InvalidState` (`repos/playback_jobs.rs:226-246`).
- `mode=ro` : `crates/lc-db/src/db.rs:81-135/254-330` (pragmas lecture seule, pas de migrations) + tests `read_only.rs` ; `.old` : récupération atomique `xtask/src/migrate/mod.rs:300-350`, reprise sous `--force`.
- Note : les adresses `is_busy` ne matchent que `DbError::Sqlx`, ce qui est correct pour les claims playback (erreur sqlx brute).

### COR-09 — PARTIEL
- **TENU** : fail-closed racine (`lib.rs:130-138`, `media/confinement.rs:22+`), handler média 404 si base incanonicalisable (`http/overlay/media.rs:265-272`), signatures constant-time + cap TTL (`media_url.rs:48-105`, `http/overlay/media.rs:126`), pipeline partagé (`media/service.rs:192`, `overlay/meme_board.rs:218-230`), `session_mode` (`meme_board.rs:194-195/443/514/565-575`), publication atomique (`transcode.rs`, `storage.rs:197-225`), TTL WS (`ws/hub.rs:872`).
- **PARTIEL** : board non routé par `MediaService` (S-003) ; worker de purge présent mais non durci sur les cas relatifs/symlinks/hors-racine/fichiers partagés (S-001, S-002, S-004, S-005).

### COR-10 — TENU
- `apps/lc-server/src/http/rate_limit.rs` : 100 req/15 min par IP (`40-44`), `is_exempt` dev+loopback seulement (`226-228`, admin plus exempté), 429 + en-têtes + `Retry-After` (`181-212`), exécuté **avant** l'extracteur d'auth (couche middleware) ⇒ 429 avant 401/403/503.
- IP réelle : `ConnectInfo` alimenté par `into_make_service_with_connect_info` (`apps/lc-server/src/lib.rs:193-195`) ; `TRUST_PROXY_HEADERS` opt-in pour `X-Forwarded-For`.
- Compose : bind `127.0.0.1:3000:3000` (`deploy/docker-compose.yml:45`), `ADMIN_UI_LOCAL_ONLY: "true"` (`:36`) avec avertissement conteneur documenté (`:30-35`), healthcheck `db=ok` (`:54`).

### P0 (vérification sur pièces, gates non rejouées)
- **P0-1 SSRF/LFI : TENU** (garde centrale et propagation, cf. §2 ; pré-validation avant DB `apps/lc-server/src/http/ingest/submit.rs:93`).
- **P0-2 carnet `_sqlx_migrations` : TENU sur pièces** (`xtask/src/migrate/ledger.rs` présent, checksum SHA-384 référencé ; tests migrateur présents).
- **P0-3 `session_mode` : TENU** (`ensure_writable` → 403 `read_only_session` avant validation, `http/overlay/meme_board.rs:194-195/443/514/565-575`).
- **P0-4 gate CI/release : TENU sur pièces** (`deny.toml`, `.github/workflows/rust.yml` existants et modifiés au commit P0 ; non rejoués dans cette vérification, cf. §6).

## 6. Limites et zones non vérifiées

- Aucune gate exécutée (par contrainte de temps) : `cargo fmt`/`clippy`/`test --workspace`/`cargo deny` non rejoués ; verdicts P0-4/COR-08 « sur pièces ».
- Aucun probe runtime (consigne Lead) : les observations dynamiques P1 (`p1-correctifs.md`, 17/17) n'ont pas été reproduites ; les findings ci-dessus sont statiques et reproductibles par lecture.
- Tests ciblés exécutés pendant la vérification (log `%TEMP%\lc-audit\v2-tests.log`) : `cargo test -p lc-server --lib media::purge` → **4/4 passés** (dont `keeps_referenced_files_when_storage_path_is_not_canonical`) ; `cargo test -p lc-db` → **56 tests + 1 doctest, 0 échec** (dont `migration_0002_normalise_les_doublons_de_playing`, `consume_concurrent_un_seul_gagnant`, `claim_un_seul_playing_par_guilde`, `mode_ro_autorise_la_lecture_et_refuse_les_ecritures`, `purge_expired_protege_board_et_jobs_actifs`).
- Non couverts : overlay Tauri (COR-11), protocole WS v2 en profondeur (lane dédiée), comportement Windows réel des chemins (les scénarios relatifs/symlinks sont statiques, non rejoués sur disque).
- Le format des erreurs `408`/`416`/JSON malformé (AUD-A2-009) est inchangé et reste suivi au backlog AUD-COR-13.

## 7. Recommandations (ordre)

1. **Purge (Majeur)** : refuser/skipper les symlinks, confiner toutes les suppressions (`starts_with(racine)`), résoudre les `storage_path` relatifs contre la racine média (jamais le cwd), vérifier les références multiples avant suppression, et ajouter 4 tests (relatif+cwd différent, symlink, hors racine, fichier partagé) — à traiter avant toute release avec migration de `MEDIA_STORAGE_DIR`.
2. **Board (Majeur)** : router l'ingestion board par `MediaService` (cache/quotas/FAILED/métadonnées), ou documenter explicitement l'écart et son impact disque ; corriger le `forceRefresh` orphelin.
3. **Nits** : aligner la détection unique (Domain(Conflict)), budget rate-limit média, exemptions de timeout par liste exacte, arrêt/observabilité du worker purge.
4. Rejouer les gates complètes + probes P1 avant de conclure P1 intégralement (recommandation au Lead), le point 1 étant le seul blocage de release identifié.

— Fin du rapport V2-A (serveur).
