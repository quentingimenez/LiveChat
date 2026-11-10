# Audit A5 — Base de données, concurrence et migrateur

| Champ | Valeur |
|---|---|
| Date | 2026-11-10 |
| Branche / commit | `feat/rust-rebuild` @ `e0a381f` |
| Auditeur | `http-core` (lane A5, audit indépendant — n'a pas écrit `lc-db` ni `xtask`) |
| Périmètre | `crates/lc-db/**`, `crates/lc-domain` (temps/ids/clock), `crates/lc-playback` (store/claim/watchdog), `xtask/src/migrate/**`, `docs/rust/{schema-v3.sql,lc-db.md,migrator.md}` |
| Méthode | Lecture statique critique + reproduction SQLite/Python sur bases de travail + exécution des binaires déjà construits (`xtask`, `lc-server`) ; aucun build, aucune modification de code de production |
| Artefacts | Scripts et DB sous `.phase0/audit/A5/` (répertoire ignoré par git) |
| Code modifié | **Aucun** (`git status --short` vide au début et à la fin de l'audit) |

> Objet de l'audit : vérifier les garanties réelles de la persistance SQLite, de la
> concurrence (claims, busy lock, multi-instances), de la reprise après crash, des
> migrations sqlx et du migrateur legacy → v3, avant la phase de mise en production.

---

## 1. Résumé exécutif

La couche `lc-db` est **globalement saine et bien outillée** : les primitives de claim
et de consommation de code d'appairage sont réellement atomiques, le schéma est en
`STRICT` avec FK/CHECK, le DDL est identique octet pour octet à la copie documentaire,
et le migrateur fait une lecture réellement read-only avec une vérification sérieuse
(comptes, sommes, échantillon, FK, `integrity_check`).

Cependant, l'audit a mis en évidence **un défaut bloquant de bout en bout** dans la
chaîne migrateur → serveur, ainsi que plusieurs failles de cohérence dans le cycle de
vie des jobs et la robustesse aux incidents transitoires :

1. **`AUD-A5-001` (critique, prouvé end-to-end)** — la base produite par le migrateur
   officiel ne contient pas le ledger `_sqlx_migrations` ; au démarrage, `Db::open`
   tente d'appliquer la migration `0001` sur un schéma déjà créé et **le serveur refuse
   de démarrer** (`table app_metadata already exists`). Le runbook du migrateur
   (`docs/rust/migrator.md:277`) demande pourtant explicitement de démarrer le serveur
   sur cette base.
2. **`AUD-A5-002` / `AUD-A5-003` (hautes)** — le migrateur n'a ni snapshot de lecture
   ni détection fiable d'une source qui bouge (hash du seul fichier principal, alerte
   *après* installation), et l'installation par double `rename` possède une fenêtre de
   crash qui laisse la cible absente avec un `.old` orphelin non récupéré.
3. **`AUD-A5-004` / `AUD-A5-005` (hautes)** — l'invariant « un seul `PLAYING` par
   guilde » n'est garanti que par la requête de claim (aucune contrainte DB) et
   `claim_next` peut renvoyer `SQLITE_BUSY` au lieu de `None`, ce qui **tue la boucle
   du moteur** jusqu'au redémarrage du processus.

**Compteurs** : 1 critique, 4 hautes, 7 moyennes, 8 basses, 1 info (**21 findings**).
**Top 3 risque** : `AUD-A5-001` (boot impossible), `AUD-A5-003` (fenêtre de crash
migrateur), `AUD-A5-004` (invariant `PLAYING` non contraint). `AUD-A5-002` et
`AUD-A5-005` complètent le top 5.

| ID | Sévérité | Confiance | Résumé |
|---|---|---|---|
| `AUD-A5-001` | **Critique** | Haute | Cible `xtask` sans `_sqlx_migrations` → le serveur ne démarre pas |
| `AUD-A5-002` | Haute | Haute | Migrateur : pas de snapshot, hash partiel, source mouvante seulement alertée |
| `AUD-A5-003` | Haute | Haute | Fenêtre de crash entre les deux `rename` de `install` ; `.old` non récupéré |
| `AUD-A5-004` | Haute | Haute | Invariant 1 `PLAYING`/guilde non garanti par la DB (`set_status` peut le violer) |
| `AUD-A5-005` | Haute | Moyenne-haute | `claim_next` peut renvoyer `SQLITE_BUSY` ; le moteur s'arrête (`?`) |
| `AUD-A5-006` | Moyenne | Haute | Deux `Db::open` concurrents : un boot peut échouer (lock sqlx no-op SQLite) |
| `AUD-A5-007` | Moyenne | Haute | `purge_expired` supprime en cascade des items de board épinglés et laisse les fichiers |
| `AUD-A5-008` | Moyenne | Moyenne | Watchdog ignore `remaining_ms_snapshot` → `DONE` forcé et busy libéré pendant une pause |
| `AUD-A5-009` | Moyenne | Haute | Cycle de vie non atomique : claims `RETURNING id` + `get`, `release_playing` mensonger |
| `AUD-A5-010` | Moyenne | Haute | `playback.state` écrit `busy_until` avant validation, sans vérifier la guilde du job |
| `AUD-A5-011` | Moyenne | Haute | Jobs `FAILED` depuis `PENDING` sans `finished_at` → jamais purgés |
| `AUD-A5-012` | Moyenne | Haute | `Db::connect` ignore `?mode=ro` / `immutable` (ouverture RO réellement RW) |
| `AUD-A5-013` | Basse | Moyenne | Aucun CHECK sur durées/priorité/offset/remaining (valeurs négatives acceptées) |
| `AUD-A5-014` | Basse | Moyenne | Pas de checkpoint WAL explicite à la fermeture ; sauvegarde/`VACUUM INTO` non outillés |
| `AUD-A5-015` | Basse | Haute | `run_migrations=false` sur base vide → erreur Sqlx opaque au lieu de `SchemaVersion` |
| `AUD-A5-016` | Basse | Haute | `verify.rs` : échantillon comparé à la mémoire, version dupliquée, pas de test de boot |
| `AUD-A5-017` | Basse | Moyenne | `GuildId`/`PairingCode` : validation laxiste (zéros de tête, code 1 caractère) |
| `AUD-A5-018` | Basse | Haute | DDL non idempotent : toute ré-application manuelle échoue (voulu sqlx, à documenter) |
| `AUD-A5-019` | Basse | Moyenne | Upserts partiels non transactionnels (`busy_until` vs settings) — lost update |
| `AUD-A5-020` | Info | Haute | Commentaire `handle_multi_playing` (« libère les plus anciens ») contredit le code |
| `AUD-A5-021` | Basse | Moyenne | Chaîne de reprise `resumes_after_job_id` sans FK ; orphelins non vérifiés par le migrateur |

---

## 2. Détail des findings

### AUD-A5-001 — La cible du migrateur n'a pas de ledger `_sqlx_migrations` : le serveur ne peut pas démarrer

- **Sévérité** : Critique — **Confiance** : Haute (reproduit end-to-end).
- **Preuves** : `xtask/src/migrate/target.rs:40-46` crée la cible via
  `sqlx::raw_sql(SCHEMA_V3_SQL).execute(...)` (DDL brut, aucun ledger) ;
  `crates/lc-db/src/db.rs:107` puis `:120-124` : `Db::open`/`connect` appellent
  `MIGRATOR.run(&self.pool)` dès que `run_migrations=true` (défaut, cf.
  `apps/lc-server/src/app.rs:123` `DbConfig::default()`) ;
  `sqlx-core-0.8.6/src/migrate/migrator.rs:145-185` : `run_direct` crée
  `_sqlx_migrations` si absente, lit les migrations appliquées (0), puis applique
  `0001_init.sql` → `CREATE TABLE app_metadata` échoue puisque la table existe déjà.
  Le DDL n'est pas idempotent (`CREATE TABLE` sans `IF NOT EXISTS`,
  `crates/lc-db/migrations/0001_init.sql:22-166`). `docs/rust/migrator.md:277` demande
  de démarrer le serveur 3.0 sur la base cible.
- **Reproduction** (résultats exacts, `.phase0/audit/A5/`) :
  1. `python .phase0/audit/A5/mk_legacy.py` puis
     `target\debug\xtask.exe migrate-legacy --from .phase0/audit/A5/legacy.sqlite --to .phase0/audit/A5/xtask-target.sqlite`
     → succès 0,27 s, rapport 7 comptes / 7 sommes / 10 lignes échantillon / FK 0 /
     `integrity ok` / `schema_version=3.0.0` ; `journal_mode=delete`.
  2. `python .phase0/audit/A5/inspect_target.py` → tables présentes :
     `app_metadata, guilds, ingest_clients, media_assets, meme_board_items,
     overlay_clients, pairing_codes, playback_jobs` — **`_sqlx_migrations` absente**.
  3. `lc-server.exe` avec `DATABASE_URL=file:.../xtask-target.sqlite`, `API_URL`,
     `PORT=3999` → **exit 1**, stderr :
     `erreur fatale : ouverture de la base SQLite impossible (file:...): échec des
     migrations : while executing migration 1: error returned from database: (code: 1)
     table app_metadata already exists: ...`.
  4. Effet de bord : après l'échec, la cible contient une table `_sqlx_migrations`
     **vide** et est passée en `journal_mode=wal` (le serveur a muté la cible avant de
     planter).
- **Impact** : la procédure de bascule 3.0 documentée produit une base inutilisable par
  le serveur ; en prod cela se traduit par un refus de démarrage immédiat après une
  migration annoncée comme vérifiée.
- **Correctif proposé** (le plus propre d'abord) :
  1. Faire construire la cible par le **migrateur de `lc-db`** (exécuter
     `MIGRATOR.run` sur une base vide) au lieu de `raw_sql(SCHEMA_V3_SQL)` — le DDL
     reste unique via `lc_db::MIGRATION_SQL`, mais le ledger est créé dans la même
     transaction ; ou exposer dans `lc-db` un helper `create_schema_file(path)` utilisé
     par `xtask` **et** par les tests.
  2. À défaut (correctif court), seed du ledger après le DDL : table
     `_sqlx_migrations` au DDL sqlx-sqlite 0.8.6, ligne
     `version=1, description='init', success=1, execution_time=0`,
     `checksum = SHA-384(migration SQL)` (cf.
     `sqlx-core-0.8.6/src/migrate/migration.rs:25`). **Vérifié** :
     `python .phase0/audit/A5/seed_ledger.py` puis démarrage serveur →
     `{"status":"ok","db":"ok",...}` et ledger préservé.
  3. Ne pas « corriger » côté serveur par `run_migrations=false` sans mécanisme de
     vérification/upgrade explicite (cf. `AUD-A5-006`/`AUD-A5-015`).
- **Test de non-régression** : test `xtask` qui rouvre la cible via `Db::open` (pool +
  migrations no-op) **et** smoke test du binaire serveur sur la cible (ou test
  d'intégration `lc-db` : `Db::open(target)` après `xtask migrate`). Ce test aurait
  attrapé le défaut (cf. `AUD-A5-016`).
- **Effort** : S (seed + test ≈ 1-2 h) ; M (refactor « migrator-first » + tests).

### AUD-A5-002 — La source peut changer pendant la migration sans provoquer d'échec

- **Sévérité** : Haute — **Confiance** : Haute (statique ; comportement reproductible en
  forçant un écrivain entre deux lectures).
- **Preuves** : lecture en 7 requêtes séquentielles, sans transaction de snapshot :
  `xtask/src/migrate/legacy.rs:173-246` (`read_tables` → `read_table` par table) ; hash
  de la source limité au fichier principal **avant** l'ouverture :
  `legacy.rs:88-96` (`source_meta`) + `xtask/src/migrate/mod.rs:168-202` ; les fichiers
  `-wal`/`-shm` ne sont jamais hachés (`wal_info`, `legacy.rs:74-85`, seulement
  affichés) ; après installation, un hash différent ne produit qu'un **avertissement**,
  sans rollback ni abandon (`mod.rs:261-270`).
- **Impact** : si un serveur legacy écrit pendant la migration (oubli d'arrêt, service
  qui redémarre), la cible peut contenir un mélange d'instants différents, **installée
  et rapportée comme vérifiée** (les sommes sont relues en direct mais l'échantillon est
  comparé à la copie mémoire, cf. `AUD-A5-016`). Risque de perte/incohérence silencieuse
  sur les jobs et le board.
- **Reproduction** : `mk_legacy.py` puis lancer un writer Python qui insère une ligne
  `PlaybackJob` pendant `xtask migrate-legacy` ; le migrateur termine avec code 0 (au
  pire un WARNING post-install). Procédure détaillée : voir annexe B.
- **Correctif proposé** :
  - Lire dans **une seule transaction read-only** (`BEGIN` + `query_only`) et exiger un
    snapshot SQLite unique (WAL : le snapshot de la transaction est stable).
  - Hacher `db` **et** `-wal`/`-shm` (octets + tailles) avant et après lecture ; si
    l'un change → abandon **avant** écriture/installation (pas d'avertissement
    post-hoc).
  - Refuser par défaut une source dont le `-wal` est non vide (exiger
    `--allow-dirty-source` explicite), ou faire un `sqlite3 backup` cohérent dans un
    fichier de travail avant lecture.
- **Test** : test d'intégration avec un writer concurrent (barrière entre les lectures) :
  le migrateur doit échouer et laisser source et cible intactes.
- **Effort** : M (≈ 0,5-1 j).

### AUD-A5-003 — Fenêtre de crash dans `install` ; `.old` orphelin non récupéré

- **Sévérité** : Haute — **Confiance** : Haute (reproduit par simulation).
- **Preuves** : `xtask/src/migrate/target.rs:113-137` : si la cible existe et `--force`,
  `rename(target, target.old)` puis `rename(tmp, target)` ; en cas d'échec du second
  rename, `drop(rename(.old, target))` **sans vérifier** le résultat ; le `.old` est
  supprimé avec `drop(...)` (échec avalé) ; pas de `fsync` du répertoire après rename.
  Aucun test ne couvre ce chemin (`xtask/src/migrate/tests.rs`).
- **Reproduction** : simulation du crash entre les deux renames : cible renommée en
  `.old`, puis relance du migrateur sur la cible absente → le migrateur **recrée** une
  nouvelle cible et **laisse le `.old` de 159 744 octets** (aucune détection ni
  récupération) ; sortie : `installé : ... (151552 octet(s))`. Script :
  voir annexe B (`install_repro`).
- **Impact** : après un crash (OOM, coupure, Ctrl-C) la base courante peut disparaître
  du chemin attendu ; l'opérateur doit restaurer manuellement `.old` ; risque de
  démarrer le serveur sur une cible plus ancienne/nouvelle incomprise.
- **Correctif proposé** : ordre de bascule robuste (écrire `target.tmp` → `fsync` →
  `rename(tmp, target)` atomique si le FS le permet ; pour remplacer une cible
  existante, conserver `.old` jusqu'à un `fsync` réussi puis le supprimer, et au
  démarrage suivant **détecter `.old` sans cible** et proposer/restaurer) ; vérifier
  chaque `rename`/`fsync` et faire remonter l'échec de restauration.
- **Test** : test de crash simulé (injection d'erreur au second `rename`) : cible
  restaurée ou `.old` détecté au run suivant ; test du chemin `--force` nominal.
- **Effort** : S-M (≈ 0,5 j + tests).

### AUD-A5-004 — L'invariant « un seul `PLAYING` par guilde » n'est pas garanti par la DB

- **Sévérité** : Haute — **Confiance** : Haute (violation reproduite).
- **Preuves** : `crates/lc-db/src/repos/playback_jobs.rs:199-206` `set_status` =
  `UPDATE playback_jobs SET status = ? WHERE id = ?` sans garde ; `finish`
  (`:211-230`) n'impose que « statut terminal », pas de transition source ; aucune
  contrainte/index partiel dans `crates/lc-db/migrations/0001_init.sql` (seule FK
  média, pas de trigger) ; `claim_next` (`:155-194`) protège uniquement le claim.
  `docs/rust/lc-db.md` §6.1 admet que la garantie est applicative.
- **Reproduction** : `purge_repro.py` : deux `UPDATE playback_jobs SET
  status='PLAYING'` sur deux jobs de la même guilde → 2 `PLAYING` simultanés
  (l'UPDATE passe, aucune contrainte).
- **Impact** : entre deux ticks, deux jobs peuvent être considérés actifs ; le watchdog
  prend `playing.first()` (le plus ancien) et peut terminer le mauvais job ; les
  clients reçoivent un état incohérent ; toute évolution future qui utiliserait
  `set_status(Playing)` (replay manuel, admin) peut casser le scheduler.
- **Correctif proposé** : migration `0002` avec index unique partiel
  `CREATE UNIQUE INDEX playback_jobs_single_playing ON playback_jobs(guild_id)
  WHERE status = 'PLAYING';` (SQLite supporte les index partiels), et rendre les
  transitions de statut explicites (`claim`, `finish`, `suspend`, `requeue`) sans
  `set_status` libre (ou mapper la violation de contrainte en `Conflict`).
  Faire valider le DDL par `schema_contract`/`schema-v3.sql`.
- **Test** : tentative de second `Playing` → erreur ; test de concurrence
  claim vs `set_status` concurrent ; test existant
  `playback_jobs.rs:230-285` complété par « un seul PLAYING même après transitions
  manuelles ».
- **Effort** : S (index) + M (refactor transitions).

### AUD-A5-005 — `claim_next` peut renvoyer `SQLITE_BUSY` au lieu de `None`, et le moteur s'arrête

- **Sévérité** : Haute — **Confiance** : Moyenne-haute (erreur SQLite reproduite ; chemin
  moteur déduit du code).
- **Preuves** : `crates/lc-db/src/repos/playback_jobs.rs:152-154` documente « le perdant
  voit `None` » ; en pratique une écriture concurrente (writer `BEGIN IMMEDIATE`,
  checkpoint, migrateur) au-delà de `busy_timeout` (5 s par défaut,
  `crates/lc-db/src/config.rs`) produit `SQLITE_BUSY`/`database is locked` ;
  `crates/lc-playback/src/engine.rs:509-545` : `run`/`run_until_shutdown` font
  `self.tick().await?` dans la boucle → toute erreur DB remonte et **arrête la
  boucle** ; `apps/lc-server/src/ws/hub.rs:437-443` lance le moteur avec
  `tokio::spawn` et se contente de `tracing::error!("moteur de lecture arrêté sur
  erreur")` — pas de redémarrage.
- **Reproduction** : `claim_busy_repro.py` : writer `BEGIN IMMEDIATE`, puis requête de
  claim avec `busy_timeout=5000` → `OperationalError('database is locked')` après
  **5,43 s** (au lieu de `None`). Le harness sqlx transforme cela en
  `DbError::Sqlx` ; le moteur retourne l'erreur.
- **Impact** : un verrou transitoire (migration, purge lourde, backup, autre process)
  tue le scheduler de lecture **globalement** jusqu'au redémarrage du processus ; les
  parties déjà en cours continuent mais la file ne progresse plus ; les `busy_until`
  peuvent rester posés et bloquer les guildes.
- **Correctif proposé** : dans la boucle moteur, distinguer les erreurs transitoires
  (`SQLITE_BUSY`/`SQLITE_LOCKED`, code sqlx) et faire `warn!` + backoff + `continue` ;
  ou wrapper les claims avec un retry borné (3 tentatives, backoff exponentiel) dans
  `store`/`engine`. Optionnel : `busy_timeout` configurable par l'engine.
- **Test** : test d'intégration tenant un `BEGIN IMMEDIATE` sur une seconde connexion,
  exécuter un tick, relâcher, vérifier que le moteur continue (pas d'erreur fatale) et
  que le job est claimé ensuite.
- **Effort** : S (≈ 2-3 h avec test).

### AUD-A5-006 — Deux `Db::open` concurrents peuvent faire échouer un démarrage

- **Sévérité** : Moyenne — **Confiance** : Haute (mécanisme + simulation ; fenêtre
  étroite en pratique).
- **Preuves** : `sqlx-core-0.8.6/src/migrate/migrator.rs:145-147` appelle
  `conn.lock()` ; pour SQLite ce lock est un **no-op**
  (`sqlx-sqlite-0.8.6/src/migrate.rs:123-129`) ; `ensure_migrations_table` puis
  `apply` ne sont donc pas mutuellement exclus entre process ; côté `lc-db`,
  `Db::open`/`connect` lancent `MIGRATOR.run` sans verrou applicatif
  (`crates/lc-db/src/db.rs:94-124`).
- **Reproduction** : `migration_race_repro.py` simule la séquence sqlx exacte
  (`ensure table`, `list`, barrière, `apply`) avec le découpage `sqlite3` :
  résultats `['OK', 'ECHEC: table app_metadata already exists']`, ledger `[(1,)]`.
  En réel, 4 essais de double démarrage simultané de deux serveurs : les deux ont
  démarré (fenêtre étroite), mais le mécanisme est prouvé.
- **Impact** : un redémarrage concurrent (process manager + lancement manuel, CI,
  double service systemd) peut produire un `exit 1` avec un message trompeur alors que
  la base est saine ; en boucle de crash, l'instance peut rester bloquée.
- **Correctif proposé** : verrou fichier OS (`.lock` adjacent, `flock`/`LockFileEx`)
  autour du `MIGRATOR.run` + retry borné sur « table already exists » ; ou verrou SQL
  explicite (`BEGIN EXCLUSIVE`) avant migration ; documenter le contrat mono-instance
  (`docs/rust/packaging.md:335`) et ajouter une garde de démarrage.
- **Test** : deux process/threads exécutant `Db::open` sur une base neuve avec barrière
  → un seul migre, l'autre attend/réussit ; test déterministe basé sur la simulation.
- **Effort** : S-M.

### AUD-A5-007 — `purge_expired` supprime en cascade des items de board épinglés et laisse les fichiers

- **Sévérité** : Moyenne — **Confiance** : Haute (reproduit).
- **Preuves** : `crates/lc-db/src/repos/media_assets.rs:173-179` :
  `DELETE FROM media_assets WHERE expires_at <= ?`, sans exclusion des assets
  référencés par `meme_board_items` ni des jobs `PENDING/PLAYING` ; FK
  `meme_board_items.media_asset_id ... ON DELETE CASCADE`
  (`migrations/0001_init.sql`) ; `playback_jobs.media_asset_id ... ON DELETE SET NULL`.
  Aucun nettoyage de fichier dans le repo (la DB ne connaît pas le storage) ; aucun
  worker ne l'appelle dans `lc-server` (`grep purge_expired` → absent hors repo).
- **Reproduction** : `purge_repro.py` : asset expiré + item de board + job PENDING →
  `DELETE ... expires_at <= ?` supprime 1 asset, `meme_board_items` passe de 1 à 0
  (**item épinglé perdu**), `playback_jobs.media_asset_id` devient NULL, et le fichier
  média existe toujours (orphelin).
- **Impact** : perte silencieuse d'entrées du meme board si un asset épinglé a une
  `expires_at` incohérente (bug de service, horloge, import) ; fuite disque si la purge
  est branchée sans suppression des fichiers. Parité legacy partielle : le legacy
  supprime aussi fichiers + lignes (donc à terme il y a régression fonctionnelle).
  Impact conditionnel mais l'API ne protège pas l'appelant.
- **Correctif proposé** : requête de purge excluant les assets référencés :
  `AND NOT EXISTS (SELECT 1 FROM meme_board_items m WHERE m.media_asset_id =
  media_assets.id) AND NOT EXISTS (SELECT 1 FROM playback_jobs j WHERE
  j.media_asset_id = media_assets.id AND j.status IN ('PENDING','PLAYING'))` ;
  orchestrer fichier + ligne au niveau service (`MediaStorage::remove_variants` puis
  suppression) avec idempotence ; documenter la sémantique « purge cache uniquement ».
- **Test** : test repo `purge_expired_ignore_les_assets_epingles_et_actifs` + test
  service d'orphelins (fichier supprimé avec la ligne).
- **Effort** : S (SQL) + S (service).

### AUD-A5-008 — Le watchdog ignore `remaining_ms_snapshot` : pause/buffer > durée → `DONE` forcé

- **Sévérité** : Moyenne — **Confiance** : Moyenne (limite documentée ; impact déduit).
- **Preuves** : `crates/lc-playback/src/engine.rs:584-603` : `expected_end_ms =
  started_at + duration_sec.max(1) * 1000 + lock_padding_ms` ; si
  `now >= expected_end_ms + stale_release_grace_ms` → `finish(Done)` + `clear_busy`,
  sans consulter `remaining_ms_snapshot` ni `last_playback_state_at` ; le snapshot est
  pourtant persisté par `handle_playback_state` (`engine.rs:263-329`) et utilisé par
  `estimated_remaining_ms` (`engine.rs:818-823`, âge borné par
  `state_snapshot_max_age_ms = 15 s`) pour le payload uniquement.
  `docs/rust/lc-playback.md:158-161,178` documente le watchdog et la limite « pause
  prolongée non couverte ».
- **Reproduction** : scénario test possible avec `FixedClock` : job 10 s, état
  `remaining_ms=8000` reçu à t+2 s, puis aucune fin ; à t+~20 s le watchdog marque
  `DONE` alors que l'overlay joue toujours (le busy est libéré).
- **Impact** : une vidéo mise en pause/buffer > durée + 10 s est terminée côté serveur
  et la guilde peut lancer un autre média : **double lecture** (l'ancien média continue
  localement, le nouveau démarre) et reprise perdue. Acceptable si l'overlay ne peut pas
  rester en pause, à documenter explicitement côté protocole/service.
- **Correctif proposé** : intégrer le snapshot frais : si
  `last_playback_state_at` < `state_snapshot_max_age_ms` et
  `remaining_ms_snapshot > 0`, repousser l'échéance à `now + remaining_ms +
  padding` ; sinon watchdog actuel. Alternative : exiger un `playback.ended` et ne
  libérer qu'après X s de silence total.
- **Test** : test `engine_scenarios` avec `FixedClock` (pause > durée, snapshot
  frais) : pas de `DONE` ; snapshot périmé : watchdog s'applique.
- **Effort** : M.

### AUD-A5-009 — Opérations du cycle de vie non atomiques / résultats partiels

- **Sévérité** : Moyenne — **Confiance** : Haute (statique ; scénarios déterministes).
- **Preuves** :
  - `store.rs:68-108` (`claim_specific`) et `:111-150` (`claim_next_for_guild`) :
    `UPDATE ... RETURNING id` puis **second** `get(id)` ; entre les deux, le job peut
    être terminé/requeued par un autre chemin → l'engine agit sur un état périmé.
  - `store.rs:209-256` (`release_playing`) : SELECT puis UPDATE **par job**, sans
    transaction, ignore `rows_affected`, et retourne les ids comme « released » même
    si 0 ligne modifiée ; en cas d'erreur au milieu, libération partielle.
  - `store.rs:47-64` (`playing_jobs`) : SELECT ids + N+1 `get`, sans instant unique.
  - `engine.rs:646-661` : `mark_dispatched` retourne un booléen **ignoré**.
- **Impact** : `stop_with_reason` peut émettre `on_stop` et rapporter des ids qui
  n'ont pas été libérés ; les claims peuvent cacher un job déjà terminal (le scheduler
  saute une itération) ; la lecture `playing_jobs` peut voir un état mixte entre deux
  ticks. Ce sont des fenêtres courtes, mais elles compliquent l'audit et les reprises.
- **Correctif proposé** : faire les claims en une instruction `RETURNING *` et
  désérialiser directement (pas de second `get`) ; `release_playing` en transaction
  avec vérification de `rows_affected` ; `playing_jobs` en `SELECT *` ; vérifier le
  booléen `mark_dispatched` (log/erreur si false).
- **Test** : tests concurrents `finish` entre claim et get ; test `release_playing`
  avec 0 ligne modifiée ; test de transaction partielle (injection d'erreur).
- **Effort** : M (≈ 0,5 j).

### AUD-A5-010 — `playback.state` écrit le verrou avant validation et sans contrôle de guilde

- **Sévérité** : Moyenne — **Confiance** : Haute (statique).
- **Preuves** : `engine.rs:263-329` : `set_busy_until` est exécuté aux lignes
  `279-285`, **avant** de charger/vérifier le job (`313-316` : job absent → `continue`,
  statut ≠ `Playing` → `continue`) ; aucune comparaison
  `job.guild_id == event.guild_id` ; un `job_id` d'une autre guilde est accepté (seul
  le hub/rate-limit filtre l'appelant). `handle_playback_ended` (`engine.rs:335-367`)
  fait un read-then-finish non atomique, sans vérifier la guilde non plus.
- **Impact** : un client overlay authentifié (mais malveillant/buggé) peut
  prolonger/écraser `busy_until` d'une guilde avec n'importe quel `job_id`, y compris
  une autre guilde (selon le filtrage amont), retardant les lectures ; un
  `playback.ended` sur un job d'une autre guilde peut terminer le job d'autrui si les
  ids fuient. Fenêtre de double lecture quand le busy est libéré par un état
  incohérent.
- **Correctif proposé** : charger le job d'abord, vérifier
  `job.guild_id == event.guild_id` et `status == Playing`, puis mettre à jour
  `busy_until` et le snapshot dans la même transaction ; faire `handle_playback_ended`
  en une instruction conditionnelle (`UPDATE ... WHERE id=? AND guild_id=? AND
  status='PLAYING' RETURNING *`).
- **Test** : test contractuel « state/ended avec job d'une autre guilde → ignoré + warn,
  busy inchangé » ; test job inexistant → busy inchangé.
- **Effort** : S-M.

### AUD-A5-011 — Les jobs `FAILED` depuis `PENDING` n'ont pas `finished_at` et ne sont jamais purgés

- **Sévérité** : Moyenne — **Confiance** : Haute (statique ; preuve SQL).
- **Preuves** : `engine.rs:373-404` : branche `JobStatus::Pending` →
  `playback_jobs().set_status(job_id, JobStatus::Failed)` (pas de `finished_at`) ;
  `crates/lc-db/src/repos/playback_jobs.rs:259-270` :
  `purge_done_before` exige `status IN ('DONE','FAILED') AND finished_at IS NOT NULL`
  → ces lignes ne seront **jamais** supprimées par la purge de rétention (24 h).
- **Impact** : croissance illimitée de `playback_jobs` (fuite rétention), statistiques
  admin faussées, sauvegardes qui grossissent ; à long terme dégradation des scans.
- **Correctif proposé** : utiliser `finish(Failed, now)` dans `handle_error` (branche
  Pending) ; ou assouplir la purge (`finished_at IS NULL` accepté pour les jobs
  terminaux, avec date de repli `last_playback_state_at`/`submission_date`) ;
  éventuellement `CHECK (status IN ('PENDING','PLAYING') OR finished_at IS NOT NULL)`
  en migration.
- **Test** : test `handle_error` Pending → statut FAILED + `finished_at` non NULL ;
  test purge avec un job FAILED sans `finished_at` (attendu supprimé après correctif).
- **Effort** : S.

### AUD-A5-012 — `Db::connect` ignore silencieusement `?mode=ro` / `immutable`

- **Sévérité** : Moyenne — **Confiance** : Haute (reproduit end-to-end).
- **Preuves** : `crates/lc-db/src/db.rs:244-284` (`parse_connection_url`) : pour les URL
  `file:`, `mode∈{rwc,rw,ro}` est **accepté puis ignoré** (aucun
  `read_only(true)`/`immutable(true)`) ; les autres paramètres inconnus sont loggués et
  ignorés ; seule la branche `mode=memory` est traitée.
- **Reproduction** : copie de la cible seedée, puis
  `DATABASE_URL=file:.../mode-ro.sqlite?mode=ro` + `lc-server.exe` → le serveur
  **démarre** (`/health` ok) ; après arrêt : `journal_mode=wal` et `-wal` créé
  (écriture réelle). Preuve que l'ouverture est RW malgré `mode=ro`.
- **Impact** : un outil/script qui croit ouvrir une base en lecture seule (audit,
  sauvegarde, inspection, futur `xtask`) peut écrire (journal mode, checkpoint,
  **migrations appliquées**) ; risque de corruption si deux writers sont supposés
  exclus ; fausse sécurité.
- **Correctif proposé** : honorer `mode=ro` → `SqliteConnectOptions::read_only(true)`,
  `immutable=1` → `immutable(true)`, et **rejeter** les paramètres non supportés
  (`DbError::InvalidUrl`) plutôt que les ignorer ; ajouter un booléen
  `read_only` explicite dans `DbConfig` ; tester l'ouverture RO + échec d'écriture.
- **Test** : `db::connect("file:...?mode=ro")` puis tentative d'écriture → erreur
  `readonly database` ; test `immutable` sur fichier absent après ouverture.
- **Effort** : S.

### AUD-A5-013 — Aucun CHECK sur durées/priorité/offsets/remaining (valeurs négatives acceptées)

- **Sévérité** : Basse — **Confiance** : Moyenne.
- **Preuves** : `crates/lc-db/migrations/0001_init.sql` : `playback_jobs` n'a de CHECK
  que pour `status` et `show_text` ; `duration_sec`, `priority`, `resume_offset_sec`,
  `remaining_ms_snapshot` sont des INTEGER libres (seuls NOT NULL par défaut pour
  certains) ; `media_assets.duration_sec` nullable sans borne non plus.
- **Impact** : données invalides persistables (offset négatif, remaining négatif) ;
  `dispatch_timing` (`crates/lc-playback/src/payload.rs:33-44`) retombe sur la durée du
  média/`default` si `duration_sec <= 0`, et le watchdog fait `.max(1)`, donc l'impact
  immédiat est limité à la qualité des données et aux payloads (`resume_offset_sec`
  négatif transmis). Un `remaining_ms` négatif est clampé par le busy lock mais reste
  persisté.
- **Correctif proposé** : CHECK `duration_sec > 0`, `priority >= 0`,
  `resume_offset_sec >= 0`, `remaining_ms_snapshot IS NULL OR >= 0` en migration 0002,
  + validation domaine à l'insertion (`PlaybackJob::validate`), tests.
- **Test** : insertion de valeurs négatives → `CheckViolation`/`DomainError` selon
  couche ; test migrateur sur legacy contenant des valeurs négatives (mapping actuel
  les importe telles quelles).
- **Effort** : S-M.

### AUD-A5-014 — Pas de checkpoint WAL explicite / outillage sauvegarde

- **Sévérité** : Basse — **Confiance** : Moyenne.
- **Preuves** : `crates/lc-db/src/db.rs:166-168` `Db::close` = `pool.close().await`,
  pas de `PRAGMA wal_checkpoint(...)` explicite ; `apply_pragmas` ne fixe pas
  `journal_size_limit` (`db.rs:212-218`) ; `docs/rust/lc-db.md` §6.3 le reconnaît comme
  point ouvert. SQLite fait un checkpoint à la fermeture de la dernière connexion,
  mais un crash laisse `-wal`/`-shm`.
- **Impact** : sauvegarde « copie du .sqlite » sans WAL = perte des transactions
  récentes ; après crash, le migrateur peut voir une source WAL non checkpointée
  (cf. `AUD-A5-002`) ; disque WAL non borné entre checkpoints.
- **Correctif proposé** : `PRAGMA wal_checkpoint(TRUNCATE)` (ou PASSIVE) avant
  fermeture, `journal_size_limit` configurable, et documenter la sauvegarde via
  `VACUUM INTO`/backup API plutôt qu'une copie naïve ; test crash/kill -9 laissant
  `-wal` puis `Db::open` + intégrité.
- **Test** : test tuant le process pendant écriture, réouverture, `integrity_check` ok ;
  test `VACUUM INTO` sur base en WAL.
- **Effort** : S.

### AUD-A5-015 — `run_migrations=false` sur base vide → erreur opaque au lieu de `SchemaVersion`

- **Sévérité** : Basse — **Confiance** : Haute.
- **Preuves** : `crates/lc-db/src/db.rs:94-114` exécute `migrate()` puis
  `ensure_schema_version()` ; si `run_migrations=false` et
  `check_schema_version=true`, `ensure_schema_version` (`:127-140`) interroge
  `app_metadata` sur une base vide → `DbError::Sqlx("no such table: app_metadata")`
  au lieu de `DbError::SchemaVersion{expected, found:"<absente>"}`.
- **Impact** : message d'erreur trompeur pour l'opérateur (diagnostic de config
  incorrect) ; faible.
- **Correctif proposé** : pré-vérifier l'existence de `app_metadata` via
  `sqlite_master` et retourner `SchemaVersion{found:"<absente>"}` ; ou exécuter
  `ensure_schema_version` avant `migrate` quand `run_migrations=false`.
- **Test** : base vide + `DbConfig::new().without_migrations()` → erreur
  `SchemaVersion` (et non Sqlx brute).
- **Effort** : S.

### AUD-A5-016 — `verify.rs` : échantillon non relu en direct, version dupliquée, pas de contrôle de boot

- **Sévérité** : Basse — **Confiance** : Haute.
- **Preuves** : `xtask/src/migrate/verify.rs` : `SAMPLE_SIZE=20` (`:21`),
  `SCHEMA_VERSION="3.0.0"` local (`:24`, au lieu de `lc_db::SCHEMA_VERSION`) ;
  les sommes sont relues en direct (`legacy.scalar_sum`, `:211-222`) mais
  l'échantillon compare la cible aux structures **en mémoire** (`expected`, cf.
  `sample_mismatch`) ; aucune vérification de `_sqlx_migrations` ni d'ouverture de la
  cible par `Db::open`/serveur ; le test `xtask/src/migrate/tests.rs` ne fait pas ce
  boot non plus.
- **Impact** : une source modifiée pendant la migration peut produire une cible mixte
  « vérifiée » (cf. `AUD-A5-002`) ; le défaut critique `AUD-A5-001` n'a pas été détecté
  par la vérification existante ; dérive possible entre la version attendue par `lc-db`
  et celle du migrateur.
- **Correctif proposé** : réutiliser `lc_db::SCHEMA_VERSION` ; relire l'échantillon en
  direct depuis la source (comme les sommes) ou comparer à la source après coup ; ajouter
  un test « la cible s'ouvre avec `Db::open`/le serveur et la version correspond » ;
  vérifier la présence/cohérence de `_sqlx_migrations` (une fois `AUD-A5-001` corrigé).
- **Test** : test migrateur qui ouvre la cible via `Db::open` (attrapait `AUD-A5-001`).
- **Effort** : S.

### AUD-A5-017 — Validation laxiste des identifiants

- **Sévérité** : Basse — **Confiance** : Moyenne.
- **Preuves** : `crates/lc-domain/src/ids.rs:133-160` `GuildId::parse` : trim, non vide,
  chiffres ASCII, longueur ≤ 20 — accepte `"0"`, `"000123"` (représentations multiples
  de la même guilde), et des entiers > 20 chiffres non rejetés en soi (longueur seule) ;
  `:205-240` `PairingCode::parse` : trim + uppercase + alphanumérique 1..=32 → un code
  d'un caractère est valide.
- **Impact** : si un code de guilde vient d'une source externe mal normalisée (admin,
  import, URL), deux lignes différentes peuvent désigner la même guilde ; un code
  d'appairage très court réduit l'entropie (mais les codes sont générés côté Discord ;
  l'API pair/consume ne les crée pas — impact limité).
- **Correctif proposé** : rejeter les zéros de tête / exiger 17-20 chiffres + non-nul
  pour `GuildId` (snowflake) et longueur minimale 6 pour `PairingCode` ; tests aux
  bornes.
- **Test** : jeux de valeurs invalides (`"0"`, `"0001"`, `"A"`, 21 chiffres).
- **Effort** : S.

### AUD-A5-018 — DDL non idempotent (comportement sqlx) — à documenter/outiller

- **Sévérité** : Basse — **Confiance** : Haute.
- **Preuves** : `crates/lc-db/migrations/0001_init.sql` : `CREATE TABLE`/`CREATE INDEX`
  sans `IF NOT EXISTS` ; voulu pour le ledger sqlx, mais toute application manuelle
  (`sqlite3 < schema-v3.sql`) ou partielle échoue ; combiné à `AUD-A5-001`, un ledger
  manquant transforme un schéma correct en échec de boot.
- **Impact** : opérations manuelles/restaurations fragiles ; piège pour les nouveaux
  outils (cf. `AUD-A5-001`).
- **Correctif proposé** : conserver le DDL strict pour sqlx et **imposer** le passage
  par le migrateur/ledger (documentation + helper `lc-db`), plus un test de
  « ré-application = erreur attendue » pour figer le comportement.
- **Effort** : S (documentation/tests).

### AUD-A5-019 — Upserts partiels non transactionnels (lost update)

- **Sévérité** : Basse — **Confiance** : Moyenne.
- **Preuves** : `crates/lc-db/src/repos/guilds.rs` : `set_busy_until` (`:92-106`) fait
  `INSERT OR IGNORE` puis `UPDATE busy_until` ; `update_settings` (`:112-134`) met à
  jour chaque colonne en statements séparés sans transaction ; `upsert` (`:55-76`)
  remplace **toute** la ligne (y compris settings) à partir d'un état lu ailleurs.
- **Impact** : entre deux writers (busy `playback.state` et settings admin/`guild
  upsert`), une mise à jour peut écraser l'autre (last-write-wins par champ ou par
  ligne) ; sur SQLite mono-writer l'impact est limité à une fenêtre courte, mais
  l'API n'est pas atomique.
- **Correctif proposé** : `UPDATE` ciblés uniquement (pas d'upsert global pour un
  changement partiel) ; utiliser `COALESCE`/expressions conditionnelles pour les
  champs optionnels ; ou encapsuler lecture+écriture dans une transaction.
- **Test** : test concurrent `set_busy_until` + `update_settings` → les deux effets
  survivent ; test `upsert` ne doit pas écraser des settings non fournis (API dédiée).
- **Effort** : S-M.

### AUD-A5-020 — `handle_multi_playing` : commentaire contredit le code

- **Sévérité** : Info — **Confiance** : Haute.
- **Preuves** : `crates/lc-playback/src/engine.rs:695-720`
  `handle_multi_playing` : warn « libération des plus anciens » mais
  `playing.iter().skip(1)` termine les **plus récents** et conserve `playing[0]`
  (le plus ancien, car `playing_jobs` trie `started_at ASC`) ; `current_job`
  (`engine.rs:461-490`) fait `playing.pop()` → expose le plus récent. Les libérations
  ne passent pas par `on_stop`/`clear_busy` et ne nettoient pas `state_persisted_at`.
- **Impact** : pas de bug fonctionnel immédiat (nettoyage d'anomalie), mais logique
  contre-intuitive pour la maintenance et le debug ; risque d'incohérence si un futur
  code se fie au commentaire.
- **Correctif proposé** : aligner commentaire et code (choisir la sémantique : garder le
  plus ancien et terminer les autres, ou l'inverse) ; émettre `on_stop`/nettoyer l'état
  lors des libérations ; test unitaire de la sélection.
- **Effort** : S.

### AUD-A5-021 — Chaîne de reprise `resumes_after_job_id` sans FK ; orphelins non vérifiés

- **Sévérité** : Basse — **Confiance** : Moyenne.
- **Preuves** : `crates/lc-db/migrations/0001_init.sql` : `resumes_after_job_id` est un
  TEXT sans FK (la table `playback_jobs` n'a pas de FK auto-référentielle) ;
  `claim_next` (`crates/lc-db/src/repos/playback_jobs.rs:160-184`) traite un parent
  **absent** comme éligible (`NOT EXISTS ... p.id = j.resumes_after_job_id AND
  p.status IN (...)`) ; le migrateur ne vérifie que les FK média
  (`xtask/src/migrate/legacy.rs:277-303`) et n'inspecte pas les orphelins
  `resumesAfterJobId` ; l'entité/les repos n'exposent aucune validation de chaîne.
- **Impact** : après suppression/purge d'un job parent (ou import legacy incohérent),
  le job enfant devient immédiatement jouable au lieu d'être orphelin ignoré/nettoyé ;
  pas d'erreur, comportement silencieux ; l'audit ne peut pas s'appuyer sur la DB pour
  la cohérence de reprise. Impact faible (les purges ne suppriment que DONE/FAILED et
  le scheduler legacy tolérait déjà ce cas), mais la vérification migrateur revendiquée
  « FK » est incomplète.
- **Correctif proposé** : FK auto-référentielle
  `resumes_after_job_id REFERENCES playback_jobs(id) ON DELETE SET NULL` en migration
  `0002` (avec `PRAGMA foreign_keys=ON`, déjà actif) ; étendre
  `check_orphan_foreign_keys` au legacy ; test de purge d'un parent avec enfant.
- **Effort** : S.

---

## 3. Couverture

### 3.1 Tables (8, dont `app_metadata`)

| Table | Vérifié | Findings liés |
|---|---|---|
| `app_metadata` | `get`/`set`, version exacte, contract test, `ensure_schema_version` | 001, 015 |
| `media_assets` | insert unique `source_hash`, get, `update_status`, `touch_last_accessed`, `list_expired`, `purge_expired`, `list_by_status` ; purge repro | 007, 013, 014 |
| `meme_board_items` | create unique `(guild_id, media_asset_id)`, get, update, delete ; cascade FK | 004, 007 |
| `playback_jobs` | `claim_next` atomique (repro concurrence 50/50), `set_status`, `finish`, `purge_done_before`, `set_playback_state`, `requeue` | 004, 005, 009, 011, 013, 020 |
| `guilds` | `upsert`, `get`, `set_busy_until`, `update_settings` ; busy utilisé par le claim | 005, 019 |
| `overlay_clients` | create unique token, lookup non révoqué, `touch_last_seen`, `revoke`, `update_mode` | — |
| `ingest_clients` | idem + création par appairage | — |
| `pairing_codes` | `consume` atomique (`UPDATE ... RETURNING *`), `delete_expired`, unicité `code` | — |

### 3.2 Migrations

- Une seule migration (`0001_init.sql`, 7 669 octets) ; pas de test d'**évolution**
  (0002+), pas de test de checksum/ledger, pas de test de downgrade (hors périmètre
  sqlx).
- DDL `migrations/0001_init.sql` **identique octet pour octet** à
  `docs/rust/schema-v3.sql` :
  SHA-256 `5AF42FAA52FEA04B6545A77C81D67A53B263B6DCBD5371C75FFC3518A5A01CD3`
  (vérifié `Get-FileHash` + test `crates/lc-db/tests/schema_contract.rs`) — point fort.
- Tests migration : base fraîche + ré-`migrate()` idempotent, 8 tables, pragmas,
  version inattendue/absente, FK/CHECK (`crates/lc-db/tests/migration.rs`).
- Aucun test de migration concurrente (`AUD-A5-006`) ni de boot serveur post-migration
  (`AUD-A5-001`).

### 3.3 Migrateur legacy

- Couverture de test : 12 tests dans `xtask/src/migrate/tests.rs` (migration complète,
  dry-run sans écriture et source intacte, dates TEXT sans fuseau/fraction/REAL
  rejetées, enums/booléens invalides rejetés, cible existante sans/avec `--force`,
  `.tmp` résiduel, source=cible, source absente, base laissée en WAL lue sans
  corruption, benchmark volume).
- Vérification runtime : 7 comptes, 7 sommes, échantillon 20/table, FK, `integrity_check`,
  version, 4 métadonnées d'import — exhaustif sur le contenu (sous réserve de
  `AUD-A5-016`).
- **Non couvert** : `Db::open` sur la cible (001), source mouvante (002), crash entre
  renames (003), concurrence d'ouvertures (006), rollback automatisé après échec
  d'installation, WAL de la cible.

### 3.4 Moteur de lecture / store

- 16 tests : `crates/lc-playback/tests/engine_scenarios.rs` (13) couvrant claim,
  dispatch, busy, reprise, préemption, erreurs et watchdog ; `db_integration.rs` (3).
- **Non couvert** : `SQLITE_BUSY` (005), multi-processus (contrat mono-instance
  documenté), watchdog vs snapshot (008), purge des `FAILED` sans `finished_at` (011).

### 3.5 Points forts confirmés

- `claim_next` est **réellement atomique** : repro 8 threads / 50 jobs → 50 claims
  uniques, 0 double claim, 0 guilde avec >1 `PLAYING`, 0 `SQLITE_BUSY` en autocommit.
- `pairing_codes.consume` est atomique (`UPDATE ... RETURNING *` conditionnel) et
  correctement utilisé implicitement par l'API.
- Migrateur **réellement read-only** : `read_only(true)`, `create_if_missing(false)`,
  `query_only=ON`, pool max 1 ; aucune écriture constatée côté source pendant nos
  essais.
- Pragmas cohérents à l'ouverture applicative : WAL, `synchronous=NORMAL`,
  `foreign_keys=ON`, `busy_timeout` configuré (`db.rs:212-218`).
- DDL/vérification sérieux et docs honnêtes (§6.1, §6.3, limitation watchdog §178).
- Repro `claim_concurrency_repro.py` documenté et reproductible.

### 3.6 Vérifications transverses demandées par la task

- **Convertisseurs de dates ms** : `crates/lc-domain/src/time.rs:13-27` —
  `datetime_to_ms` = `timestamp_millis()` (i64 signé, pré-1970 négatif accepté) ;
  `ms_to_datetime` = `DateTime::from_timestamp_millis` → `None` hors plage chrono
  (≈ ±262 000 ans) ; `now_ms` = horloge système. Côté runtime, seules des dates
  INTEGER sont lues : `crates/lc-db/src/convert.rs:32-43` mappe toute valeur non ms
  (`from_ms` → `None`) en `DbError::Corrupt{table,detail}` — **aucun fallback TEXT**
  au runtime, donc une date TEXT laissée en base casse la lecture de la ligne/liste.
  Côté migrateur, bornes strictes `MIN_EPOCH_MS = -62_135_596_800_000` (0001-01-01) et
  `MAX_EPOCH_MS = 253_402_300_799_999` (9999-12-31 23:59:59.999)
  (`xtask/src/migrate/dates.rs:27-30`) ; formats TEXT acceptés : `AAAA-MM-JJ HH:MM:SS`
  interprété UTC (fraction refusée) ou RFC 3339 avec fuseau explicite
  (`dates.rs:99-148`) ; REAL, booléen, date illisible et `T` sans fuseau sont rejetés
  avec contexte table/rowid (`xtask/src/migrate/read.rs:197-252`). **Incohérence
  documentaire** : la plage runtime (chrono) est plus large que la plage migrateur
  (année 9999) ; aucun impact fonctionnel, à noter dans les docs.
- **Transactions / isolation** : SQLite mono-écrivain, requêtes individuelles en
  autocommit ; les repos `lc-db` n'ouvrent pas de transaction multi-statements
  (cf. `AUD-A5-009`/`019`) ; le migrateur applique chaque migration dans une
  transaction (ledger + DDL ensemble) et le migrateur legacy ouvre une lecture
  read-only mais **sans transaction snapshot globale** (cf. `AUD-A5-002`).
- **Injection SQL / SQL dynamique** : aucun `format!` avec entrée utilisateur trouvé ;
  les SQL « dynamiques » du migrateur (`select_sql`, `fetch_row`, `count_rows` dans
  `legacy.rs:308-315` et `verify.rs`) sont construits sur des **constantes** de tables/
  colonnes/PK ; les repos utilisent `sqlx` avec binds `?`. Pas d'injection identifiée.
- **Migrations / checksums / downgrade** : une seule migration (`0001`) ; aucun fichier
  `.down.sql` → pas de downgrade supporté (à documenter comme choix : restauration par
  sauvegarde/migrateur legacy, pas de rollback sqlx) ; les checksums ne sont comparés
  que si le ledger existe (`_sqlx_migrations`), d'où `AUD-A5-001`/`018` ; idempotence
  applicative assurée par le ledger sqlx, pas par le DDL.
- **busy_timeout / backoff** : `busy_timeout` configuré (5 s par défaut applicatif,
  10 s dans le migrateur) mais **aucun backoff/retry** dans le moteur ni dans les
  repos (cf. `AUD-A5-005`) ; les autres writers (migrateur, purge, pool) ne sont pas
  coordonnés.
- **Invariants de reprise** : `resumes_after_job_id` sans FK (`AUD-A5-021`) ;
  `resume_offset_sec`/`remaining_ms_snapshot` sans CHECK (`AUD-A5-013`) ; le busy lock
  (`guilds.busy_until`) est appliqué avant validation de l'événement (`AUD-A5-010`).

---

## 4. Zones non couvertes / limites de l'audit

- Pas de **build** ni de tests `cargo` exécutés (contrainte d'audit) : les preuves
  d'exécution viennent de Python/SQLite et des binaires déjà construits
  (`target/debug/xtask.exe`, `target/debug/lc-server.exe`, à la date du commit
  `e0a381f`) ; un binaire plus récent pourrait différer.
- Pas de test multi-processus réel sur le claim (contrat mono-instance,
  `docs/rust/packaging.md:335`) ; le double démarrage a été simulé pour la migration
  (`AUD-A5-006`).
- Pas de crash-test réel (kill -9) sur le serveur pendant écriture WAL
  (`AUD-A5-014`) ; uniquement des scénarios simulés.
- Purge des médias : aucun worker ne l'appelle aujourd'hui (`grep purge_expired`),
  donc les effets de `AUD-A5-007` sont latents tant que la purge n'est pas branchée ;
  l'API reste piégeuse.
- Performance : benchmark migrateur 100 000 lignes ≈ 10 s revendiqué par
  `docs/rust/migrator.md:228`, non mesuré ici (test « benchmark volume » existant, non
  réexécuté).
- CLI `xtask` (parsing `--force`/`--dry-run`) non auditée au-delà des chemins
  install/vérification exercés.

---

## 5. Annexe A — Commandes et scripts de reproduction

Tous les scripts sont sous `.phase0/audit/A5/` (ignoré par git) ; `python` = Python 3
du poste ; binaires `target\debug\xtask.exe` et `target\debug\lc-server.exe` du
workspace.

| Script | But |
|---|---|
| `mk_legacy.py` | Crée `legacy.sqlite` : 7 tables Prisma, 10 lignes, dates INTEGER + TEXT + RFC 3339, FK cohérentes |
| `inspect_target.py` | Liste les tables/métadonnées de la cible migrée |
| `seed_ledger.py` | Preuve de correctif : seed `_sqlx_migrations` (checksum SHA-384) sur une copie de cible |
| `purge_repro.py` | `purge_expired` → cascade board + fichier orphelin ; double `PLAYING` via `set_status` |
| `claim_busy_repro.py` | Writer `BEGIN IMMEDIATE` + claim → `database is locked` après 5,43 s |
| `claim_concurrency_repro.py` | 50 jobs / 8 threads → 50 claims uniques, 0 double claim |
| `migration_race_repro.py` | Simulation séquence sqlx avec barrière → un des deux `apply` échoue |
| `server-seeded.*`, `server-ro.*` | Logs des essais serveur (cible seedée / `?mode=ro`) |

Commandes clés (PowerShell, depuis la racine du dépôt) :

```powershell
# 1) Migration de la fixture legacy
python .phase0/audit/A5/mk_legacy.py
target\debug\xtask.exe migrate-legacy --from .phase0/audit/A5/legacy.sqlite --to .phase0/audit/A5/xtask-target.sqlite

# 2) Confirmer l'absence de _sqlx_migrations
python .phase0/audit/A5/inspect_target.py

# 3) Reproduire l'échec de boot serveur
$env:DATABASE_URL='file:C:/Users/SuperKnacky/code/LiveChat/.phase0/audit/A5/xtask-target.sqlite'
$env:API_URL='http://localhost:3000'; $env:PORT='3999'
target\debug\lc-server.exe   # exit 1 : "table app_metadata already exists"

# 4) Preuve du correctif (seed ledger) puis boot OK
python .phase0/audit/A5/seed_ledger.py
$env:DATABASE_URL='file:C:/Users/SuperKnacky/code/LiveChat/.phase0/audit/A5/target-seeded.sqlite'
$env:PORT='3997'; target\debug\lc-server.exe   # /health => {"status":"ok","db":"ok",...}

# 5) mode=ro ignoré
Copy-Item .phase0/audit/A5/target-seeded.sqlite .phase0/audit/A5/mode-ro.sqlite -Force
$env:DATABASE_URL='file:C:/Users/SuperKnacky/code/LiveChat/.phase0/audit/A5/mode-ro.sqlite?mode=ro'
$env:PORT='3996'; target\debug\lc-server.exe   # démarre ; journal_mode passe à wal
```

## 6. Annexe B — Résultats de reproduction détaillés

- **Cible migrée** : `xtask` exit 0 en 0,27 s ; cible 151 552 octets ; 7 comptes OK,
  7 sommes OK, 10 lignes échantillonnées, FK 0, `integrity_check=ok`,
  `schema_version=3.0.0`, `journal_mode=delete` ; `_sqlx_migrations` absente.
- **Boot serveur sur cible migrée** : exit 1 ; stderr
  `... échec des migrations : while executing migration 1 ... table app_metadata
  already exists ...` ; après l'échec : `_sqlx_migrations` créée vide, `journal_mode=wal`.
- **Cible seedée** : `/health` →
  `{"status":"ok","version":"3.0.0","protocolVersion":2,"db":"ok","discord":"disabled","uptimeSec":1,"activeJobs":0}` ;
  ledger `[(1,'init',1,48)]` préservé après arrêt.
- **`?mode=ro`** : `/health` OK ; après arrêt `journal_mode=wal`, `-wal` présent.
- **crash `install`** : cible déplacée en `.old` puis relance → nouvelle cible créée,
  `.old` (159 744 octets) orphelin conservé, aucun avertissement.
- **`purge_expired`** : `media_assets` 1→0, `meme_board_items` 1→0,
  `playback_jobs.media_asset_id` → NULL, fichier média toujours présent.
- **`set_status('PLAYING')` ×2 même guilde** : 2 lignes `PLAYING` (aucune contrainte).
- **`claim_next` sous verrou** : `OperationalError('database is locked')` après 5,43 s.
- **Claim concurrent** : 50/50 claims uniques, 0 double claim, 0 `SQLITE_BUSY`.
- **Race migration** : `['OK', 'ECHEC: table app_metadata already exists']`,
  ledger `[(1,)]` ; en réel 4 essais de double serveur ont démarré tous deux
  (fenêtre étroite, mécanisme confirmé par simulation).

## 7. Annexe C — Index des preuves par finding

| Finding | Preuves principales |
|---|---|
| 001 | `xtask/src/migrate/target.rs:40-46` ; `crates/lc-db/src/db.rs:107,120-124` ; `migrations/0001_init.sql` ; `docs/rust/migrator.md:277` |
| 002 | `xtask/src/migrate/legacy.rs:74-96,173-246` ; `xtask/src/migrate/mod.rs:168-202,261-270` |
| 003 | `xtask/src/migrate/target.rs:113-137` |
| 004 | `crates/lc-db/src/repos/playback_jobs.rs:199-230` ; `migrations/0001_init.sql` ; `docs/rust/lc-db.md` §6.1 |
| 005 | `crates/lc-db/src/repos/playback_jobs.rs:152-154` ; `crates/lc-playback/src/engine.rs:509-545` ; `apps/lc-server/src/ws/hub.rs:437-443` |
| 006 | `sqlx-core-0.8.6/src/migrate/migrator.rs:145-147` ; `sqlx-sqlite-0.8.6/src/migrate.rs:123-129` ; `crates/lc-db/src/db.rs:94-124` |
| 007 | `crates/lc-db/src/repos/media_assets.rs:173-179` ; `migrations/0001_init.sql` (FK CASCADE/SET NULL) |
| 008 | `crates/lc-playback/src/engine.rs:584-603,263-329,818-823` ; `docs/rust/lc-playback.md:158-161,178` |
| 009 | `crates/lc-playback/src/store.rs:47-64,68-150,209-256` ; `engine.rs:646-661` |
| 010 | `crates/lc-playback/src/engine.rs:263-329,335-367` |
| 011 | `crates/lc-playback/src/engine.rs:373-404` ; `crates/lc-db/src/repos/playback_jobs.rs:259-270` |
| 012 | `crates/lc-db/src/db.rs:244-284` |
| 013 | `crates/lc-db/migrations/0001_init.sql` ; `crates/lc-playback/src/payload.rs:33-44` |
| 014 | `crates/lc-db/src/db.rs:166-168,212-218` ; `docs/rust/lc-db.md` §6.3 |
| 015 | `crates/lc-db/src/db.rs:94-114,127-140` |
| 016 | `xtask/src/migrate/verify.rs:21-32,162-260` ; `xtask/src/migrate/tests.rs` |
| 017 | `crates/lc-domain/src/ids.rs:133-160,205-240` |
| 018 | `crates/lc-db/migrations/0001_init.sql` |
| 019 | `crates/lc-db/src/repos/guilds.rs:55-134` |
| 020 | `crates/lc-playback/src/engine.rs:461-490,695-720` |
| 021 | `crates/lc-db/migrations/0001_init.sql` ; `crates/lc-db/src/repos/playback_jobs.rs:160-184` ; `xtask/src/migrate/legacy.rs:277-303` |
