# Migrateur de données legacy (Phase 2)

Migration one-shot de la base SQLite du serveur legacy (Prisma 5.22) vers le
schéma 3.0 Rust (`docs/rust/schema-v3.sql`). Le migrateur est la sous-commande
`migrate-legacy` du binaire `xtask`.

- **Contrat de schéma** : [`docs/rust/schema-v3.sql`](schema-v3.sql) (gelé en
  Phase 0).
- **Format des dates mesuré** : [`docs/rust/prisma-dates.txt`](prisma-dates.txt).
- **Matrice sécurité liée** : [`docs/rust/audit-dynamique.md`](audit-dynamique.md)
  (§6-7 : la migration ne modifie jamais la base legacy).

## 1. Garanties

| Garantie | Mise en œuvre |
| --- | --- |
| La base legacy n'est **jamais modifiée** | connexion `read_only(true)` + `PRAGMA query_only=ON`, pool à 1 connexion, aucun `ATTACH`/`VACUUM`/`PRAGMA` d'écriture |
| Rejouable | aucune donnée legacy n'est modifiée ; la cible est entièrement reconstruite à chaque exécution |
| Pas de fichier final partiel | écriture dans `<to>.tmp`, vérification complète, puis renommage atomique |
| Échec propre | sur erreur, le `.tmp` est supprimé et `<to>` reste tel quel ; aucun `.old` ne subsiste |
| Traçabilité | empreinte SHA256 de la source, chemin, horodatage et version de l'outil enregistrés dans `app_metadata` |

La migration lit **toutes** les lignes en mémoire avant d'écrire la cible
(validation précoce, rapport `--dry-run` complet). Volumétrie attendue pour
LiveChat : quelques milliers à quelques dizaines de milliers de lignes (voir
§9).

## 2. Prérequis et compilation

```powershell
# toolchain validée en Phase 0 (rustc/cargo 1.99.1, MSVC Build Tools 2022)
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build -p xtask
```

Remarque : `clap` n'appartient pas aux dépendances du workspace (une seule
sous-commande) ; la CLI est analysée à la main dans `xtask/src/main.rs`. Si une
deuxième sous-commande apparaît, proposer `clap` au Lead plutôt que de dupliquer
l'analyseur.

## 3. Usage

```powershell
# simulation complète : lit, normalise, affiche le rapport, n'écrit rien
cargo run -p xtask -- migrate-legacy --from .\server\prisma\dev.sqlite --to .\data\livechat-v3.sqlite --dry-run

# migration réelle (refuse une cible existante sans --force)
cargo run -p xtask -- migrate-legacy --from .\server\prisma\dev.sqlite --to .\data\livechat-v3.sqlite

# remplacement d'une cible existante (ancienne cible conservée puis supprimée)
cargo run -p xtask -- migrate-legacy --from .\server\prisma\dev.sqlite --to .\data\livechat-v3.sqlite --force
```

| Option | Sens |
| --- | --- |
| `--from <PATH>` | base legacy (obligatoire, ouverture strictement en lecture seule) |
| `--to <PATH>` | base cible 3.0 à installer (obligatoire) ; seul `<to>.tmp` est écrit avant le renommage |
| `--dry-run` | valide la lecture et la normalisation, affiche le rapport, n'écrit rien |
| `--force` | autorise le remplacement d'une cible existante **et** la suppression d'un `<to>.tmp` résiduel |
| `-h`, `--help` | aide |

Codes de sortie : `0` succès, `1` échec de migration/vérification, `2` arguments
invalides.

Les chemins peuvent aussi être passés en ligne : `--from=legacy.sqlite`.

## 4. Déroulé

1. **Validation des chemins** : source existante et de type fichier ; cible non
   dossier, dossier parent existant ; source ≠ cible (canonicalisation, y compris
   chemins relatifs) ; refus si la cible existe sans `--force`.
2. **Empreinte de la source** : taille + SHA256 (blocs de 1 Mio), état des
   fichiers annexes `-wal`/`-shm`.
3. **Ouverture en lecture seule** : `PRAGMA integrity_check` doit valoir `ok`,
   mode de journalisation affiché, présence des 7 tables Prisma vérifiée.
4. **Lecture et normalisation** : 7 tables lues via `SELECT rowid, typeof(...), *
   ... ORDER BY rowid` ; dates converties en `i64` ms UTC, enums normalisées,
   booléens validés, clés étrangères orphelines refusées avant écriture.
5. **Écriture de `<to>.tmp`** : DDL 3.0 (constante `lc_db::MIGRATION_SQL`,
   migration `crates/lc-db/migrations/0001_init.sql` embarquée via
   `include_str!`), métadonnées d'import, puis une transaction par table dans
   l'ordre des FK (`media_assets`, `meme_board_items`, `playback_jobs`, `guilds`,
   `overlay_clients`, `ingest_clients`, `pairing_codes`). Journal `DELETE` +
   `synchronous=FULL` : le fichier temporaire est autonome (pas de `-wal` qui
   serait perdu au renommage).
6. **Vérification du temporaire** (voir §6) : comptes, sommes numériques,
   échantillon champ à champ, FK, intégrité, `schema_version`. Échec ⇒ suppression
   du `.tmp`, aucun renommage.
7. **Installation atomique** : `rename(<to>.tmp, <to>)`. Si `<to>` existe
   (`--force`), ancienne cible renommée en `<to>.old`, bascule, suppression du
   `.old` ; restauration de l'ancienne cible si la bascule échoue.
8. **Contrôle post-installation** : nouvelle empreinte de la source, avertissement
   si elle a changé (serveur legacy encore actif), taille de la cible affichée.

## 5. Format du rapport

Exemple (mode réel, base de 20 013 lignes) :

```text
migrate-legacy : C:\LiveChat\legacy.sqlite -> C:\LiveChat\v3.sqlite
  source  : C:\LiveChat\legacy.sqlite (3186688 octet(s), sha256 19c788c0...ecec)
  WAL     : absent
  intégrité : ok ; journal legacy : delete
  lecture  : 20013 ligne(s) ; dates : 240156 valeur(s) : 239973 INTEGER ms, 183 TEXT « AAAA-MM-JJ HH:MM:SS » UTC, 0 RFC 3339, 0 NULL
  cible    : C:\LiveChat\v3.sqlite.tmp (4476928 octet(s)) vérifiée
      comptes par table (legacy -> cible) :
        - media_assets            2 ->        2  OK
        - meme_board_items        2 ->        2  OK
        ...
      sommes numériques : 7 contrôlées
        - media_assets.size_bytes          1050624 = 1050624  OK
        ...
      échantillon : 13 ligne(s) comparée(s) champ à champ (20 par table max)
      clés étrangères : 0 violation(s) ; integrity_check = ok ; schema_version = 3.0.0
      métadonnées d'import :
        - legacy_import_at = 2026-11-09T23:41:02.123Z
        - legacy_source_path = C:\LiveChat\legacy.sqlite
        - legacy_source_sha256 = 19c788c0...
        - legacy_import_tool = xtask 3.0.0 migrate-legacy
  installé : C:\LiveChat\v3.sqlite (4476928 octet(s))
migration terminée en 1.98 s
```

En `--dry-run`, le rapport s'arrête aux comptes par table, aux statistiques de
dates et aux avertissements, avec une ligne « aucune écriture ».

Les **avertissements** non bloquants sont comptés par message, par exemple :
`playback_jobs.resume_offset_sec : NULL legacy remplacé par 0 (×3)`.

## 6. Vérifications post-migration

Exécutées sur `<to>.tmp` **avant** le renommage ; la moindre anomalie annule
l'installation.

| Contrôle | Détail |
| --- | --- |
| `PRAGMA integrity_check` | doit valoir `ok` |
| `PRAGMA foreign_key_check` | 0 violation |
| `app_metadata.schema_version` | `3.0.0` |
| Comptes par table | identiques legacy ↔ cible (7 tables) |
| Sommes numériques | `media_assets.size_bytes`, `media_assets.duration_sec`, `playback_jobs.duration_sec`, `playback_jobs.priority`, `playback_jobs.resume_offset_sec` (NULL legacy comptés 0), `guilds.default_media_time`, `guilds.max_media_time` |
| Échantillon | 20 lignes maximum par table relues par clé primaire et comparées **champ à champ** (structs identiques) ; 13 lignes sur la fixture de test |
| Métadonnées | `legacy_import_at`, `legacy_source_path`, `legacy_source_sha256`, `legacy_import_tool` relues |

## 7. Mapping legacy → schéma 3.0

Règle générale : `PascalCase`/`camelCase` → `snake_case` ; `DateTime` → `INTEGER`
millisecondes epoch UTC ; `Boolean` Prisma (0/1) → `INTEGER` 0/1 ; enums
`TEXT` en majuscules ; `NULL` conservé sauf `resumeOffsetSec` (voir plus bas).

### Tables

| Prisma | Table 3.0 |
| --- | --- |
| `MediaAsset` | `media_assets` |
| `MemeBoardItem` | `meme_board_items` |
| `PlaybackJob` | `playback_jobs` |
| `Guild` | `guilds` |
| `OverlayClient` | `overlay_clients` |
| `IngestClient` | `ingest_clients` |
| `PairingCode` | `pairing_codes` |

### Colonnes

| Legacy | 3.0 | Transformation |
| --- | --- | --- |
| `sourceHash` | `source_hash` | texte |
| `sourceUrl` | `source_url` | texte |
| `durationSec`, `width`, `height` | `duration_sec`, `width`, `height` | INTEGER nullable |
| `isVertical`, `showText`, `displayMediaFull` | `is_vertical`, `show_text`, `display_media_full` | booléen validé ∈ {0,1} |
| `storagePath` | `storage_path` | texte |
| `sizeBytes` | `size_bytes` | INTEGER |
| `lastAccessedAt`, `expiresAt`, `createdAt` | `last_accessed_at`, `expires_at`, `created_at` | date → ms UTC |
| `guildId` | `guild_id` | texte |
| `mediaAssetId` | `media_asset_id` | texte nullable (PlaybackJob) / NOT NULL (MemeBoardItem) |
| `createdByDiscordUserId`, `createdByName` | `created_by_discord_user_id`, `created_by_name` | texte nullable |
| `tokenHash` | `token_hash` | texte |
| `sessionMode` | `session_mode` | enum normalisée (cf. §8) |
| `defaultAuthorName`, `defaultAuthorImage` | `default_author_name`, `default_author_image` | texte nullable (sauf IngestClient `default_author_name` NOT NULL) |
| `lastSeenAt`, `revokedAt` | `last_seen_at`, `revoked_at` | date nullable → ms UTC |
| `busyUntil` | `busy_until` | date nullable → ms UTC |
| `defaultMediaTime`, `maxMediaTime` | `default_media_time`, `max_media_time` | INTEGER nullable |
| `submissionDate`, `executionDate`, `scheduledAt` | `submission_date`, `execution_date`, `scheduled_at` | date → ms UTC |
| `startedAt`, `finishedAt` | `started_at`, `finished_at` | date nullable → ms UTC |
| `lastPlaybackStateAt` | `last_playback_state_at` | date nullable → ms UTC |
| `remainingMsSnapshot` | `remaining_ms_snapshot` | INTEGER nullable |
| `resumesAfterJobId` | `resumes_after_job_id` | texte nullable |
| `resumeOffsetSec` | `resume_offset_sec` | `NULL` legacy → `0` + avertissement (colonne NOT NULL en 3.0) |
| `code` | `code` | clé primaire texte |
| `mode` | `mode` | enum normalisée |
| `usedAt` | `used_at` | date nullable → ms UTC |

## 8. Normalisation des énumérations

Les CHECK du schéma 3.0 imposent des majuscules. Le migrateur applique
`trim()` puis ASCII majuscules et valide contre le contrat (erreur
`InvalidEnum` avec la valeur fautive et les valeurs autorisées) :

| Colonne | Valeurs acceptées |
| --- | --- |
| `media_assets.kind` | `IMAGE`, `AUDIO`, `VIDEO` |
| `media_assets.status` | `PROCESSING`, `READY`, `FAILED` |
| `playback_jobs.status` | `PENDING`, `PLAYING`, `DONE`, `FAILED` |
| `overlay_clients.session_mode` | `NORMAL`, `INVITE_READ_ONLY` |
| `pairing_codes.mode` | `NORMAL`, `INVITE_READ_ONLY` |

Le serveur JS stockait `sessionMode` en `NORMAL`/`INVITE_READ_ONLY` en base mais
en minuscules (`normal`, `invite_read_only`) dans le protocole : la normalisation
absorbe les deux formes.

## 9. Performances et extrapolation

Mesure reproductible (`#[ignore]`, build **debug**) :

```powershell
cargo test -p xtask -- --ignored benchmark_volume --nocapture
# benchmark : 20013 lignes en 1.98 s (10112 lignes/s, 0.099 ms/ligne)
```

Cible de 4,5 Mo produite (dont 20 000 `PlaybackJob`). Le coût est linéaire :
lecture, normalisation, une insertion par ligne, vérification. Extrapolation
conservatrice en debug (les tests unitaires tournent en debug) :

| Lignes legacy | Durée attendue (debug) | Cible |
| --- | --- | --- |
| 1 000 | ≈ 0,1 s | instantané |
| 10 000 | ≈ 1 s | instantané |
| 100 000 | ≈ 10 s | acceptable avant démarrage |
| 1 000 000 | ≈ 100 s | lancer hors ligne, vérifier l'espace disque |

Un build `--release` réduit sensiblement ce temps (non mesuré ici). La migration
n'est pas incrémentale : relancer sur une base plus grosse repart de zéro.

## 10. Métadonnées ajoutées à la cible

| Clé `app_metadata` | Valeur |
| --- | --- |
| `schema_version` | `3.0.0` (écrit par le DDL) |
| `legacy_import_at` | horodatage de migration, epoch ms UTC |
| `legacy_source_path` | chemin de la base legacy au moment de la migration |
| `legacy_source_sha256` | empreinte SHA256 de la source |
| `legacy_import_tool` | `xtask <version> migrate-legacy` |

Ces clés permettent de vérifier qu'une cible correspond bien à une source donnée
avant de démarrer le serveur 3.0.

## 11. Procédure de rollback

La base legacy n'étant jamais modifiée, le rollback consiste à revenir au
serveur legacy et à sa base d'origine :

1. Arrêter le serveur 3.0.
2. Conserver la cible 3.0 (`<to>`) pour analyse, par exemple
   `Move-Item .\data\livechat-v3.sqlite .\data\livechat-v3.failed.sqlite`.
3. Redémarrer le serveur legacy (TS) sur la base d'origine `--from`, inchangée.
4. Optionnellement rejouer la migration avec le migrateur pour diagnostiquer.

Si la migration a été interrompue avant le renommage : `<to>` n'existe pas ou
reste l'ancienne cible, et le `<to>.tmp` éventuel peut être supprimé sans risque
ou réutilisé avec `--force`.

## 12. Runbook de production

1. **Arrêter le serveur legacy** (sinon la migration lit un état en cours
   d'écriture et avertit si l'empreinte change). Les fichiers `-wal` et `-shm`
   présents sont lus sans corruption, mais un arrêt propre est préférable :
   il garantit un checkpoint complet.
2. **Sauvegarder** la base legacy et ses annexes :
   `Copy-Item .\data\livechat.sqlite* X:\sauvegardes\` (le `-wal` fait partie de
   la sauvegarde).
3. **Mode `--dry-run`** : vérifier les comptes par table, les statistiques de
   dates et l'absence d'avertissement inattendu (notamment
   `resume_offset_sec`).
4. **Migration réelle** vers un chemin neuf (pas de `--force` nécessaire).
5. **Contrôler** le rapport : `integrity_check = ok`, 0 violation FK,
   `schema_version = 3.0.0`, échantillon sans anomalie.
6. **Démarrer le serveur 3.0** sur la base cible et vérifier le démarrage
   (< 500 ms attendu) et l'authentification overlay/ingest.
7. **Conserver** la base legacy et la cible précédente au moins jusqu'à la fin
   de la phase de validation (rollback possible à tout moment).

## 13. Limites connues

- Migration **one-shot et non incrémentale** ; pas de reprise après interruption
  (relancer entièrement).
- Toutes les lignes sont chargées en mémoire : prévoir ~100 Mo de RAM pour
  1 000 000 de lignes (estimation large).
- Le chemin cible doit être sur un volume où le renommage atomique est possible
  (même volume que `<to>.tmp`, toujours le cas puisque le `.tmp` est dérivé du
  chemin cible).
- Sous Windows, source et cible sont comparées après canonicalisation ; les
  chemins courts `8.3` (`SUPERK~1`) sont résolus par `canonicalize`.
- DDL **embarqué** via `lc_db::MIGRATION_SQL` (migration
  `crates/lc-db/migrations/0001_init.sql`) : source unique consommée par
  `lc-db` et `xtask`, et un test de `lc-db` vérifie l'égalité octet pour octet
  avec `docs/rust/schema-v3.sql` (copie humaine du contrat). Toute évolution du
  schéma se fait dans la migration puis dans la copie de doc.
- `clap` absent : CLI maison, à remplacer si le nombre de sous-commandes croît.
- Pas de migration des tokens/sessions applicatives : seuls les hash de tokens
  (`tokenHash`, `token_hash`) sont transférés ; les secrets en clair n'existent
  pas en base.

## 14. Dépannage

| Message | Cause probable | Action |
| --- | --- | --- |
| `base legacy introuvable` | mauvais chemin, base pas encore copiée | vérifier `--from` |
| `ouverture de la base legacy ... piste : un fichier -wal est présent` | serveur legacy actif ou `-wal` orphelin | arrêter le serveur ; sinon copier base + `-wal` + `-shm` |
| `PRAGMA integrity_check ... ` | fichier corrompu | restaurer une sauvegarde avant de migrer |
| `table legacy absente` | base non initialisée par Prisma | lancer `prisma migrate deploy` sur la base legacy |
| `date invalide dans Table.colonne (ligne N, clé "...")` | format hors contrat (`T` sans fuseau, REAL, fraction) | corriger la valeur en base legacy ou demander un assouplissement |
| `valeur énumérée invalide` | statut/mode inattendu | vérifier `server/src/services/prisma/prismaEnums.ts` et `overlayAuth.ts` |
| `clés étrangères orphelines` | `MemeBoardItem`/`PlaybackJob` pointant vers un média absent | réparer la base legacy (supprimer ou recréer la ligne média) |
| `cible déjà existante` | `--to` pointe sur une base existante | `--force` ou nouveau chemin |
| `fichier temporaire déjà existant` | migration précédente interrompue | supprimer `<to>.tmp` ou `--force` |
| `la source et la cible désignent le même fichier` | `--from` = `--to` | utiliser un chemin cible distinct |
| `échantillon incohérent` / `somme incohérente` | bug de mapping ou base modifiée pendant la migration | ne pas ouvrir la cible ; relancer sur une source au repos et signaler |
