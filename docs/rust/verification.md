# Pipeline de vérification

> [`audit/verification.md`](audit/verification.md) est l'audit passé (novembre 2026,
> lane V, revue adversariale du rebuild Rust) — ce document décrit autre chose : le
> pipeline de vérification construit par le plan
> [`docs/superpowers/plans/2026-11-18-verification-pipeline.md`](../superpowers/plans/2026-11-18-verification-pipeline.md)
> pour rendre automatiquement vérifiables les changements écrits par des agents.
> Spécification approuvée :
> [`docs/superpowers/specs/2026-11-18-verification-pipeline-design.md`](../superpowers/specs/2026-11-18-verification-pipeline-design.md).

## 1. Table phase → technique → commande → gate

| Phase | Technique | Commande (locale, PowerShell 5.1) | Gate |
|---|---|---|---|
| Pré-prod | Unité / intégration / doctests | `cargo xtask verify --offline` | exit 0 exigé ; CI `rust.yml` bloquante |
| Pré-prod | Contrats HTTP + WS | idem — `apps/lc-server/tests/contracts.rs` (clés `/health` exactes, 401 admin anonyme), matrice HELLO des versions dans `apps/lc-server/src/ws/tests.rs` | incluse dans la batterie |
| Pré-prod | Propriétés (proptest) | idem — `crates/{lc-protocol,lc-media,lc-db}/tests/properties.rs` | incluse dans la batterie |
| Pré-prod | Fuzz | `cargo test --manifest-path fuzz/Cargo.toml --no-default-features --lib --locked --offline` (seeds + golden sur stable) ; nightly : `cargo fuzz run` 45 s par cible | seeds + golden verts ; nightly advisory |
| Pré-prod | Mutation | `cargo xtask verify --offline --mutants` (lignes du diff, kill-or-justify) ; nightly : 6 scopes mesurés notés contre [`scripts/mutants-baseline.json`](../../scripts/mutants-baseline.json) | local : 0 manqué **non justifié** en `crates/` et `apps/` (le manqué justifié du scope nightly `lc-db-queue`, `playback_jobs.rs:211`, est hors diff local) ; nightly : score ≥ baseline et total ≥ `min_mutants` ; un scope encore provisoire échoue explicitement au lieu de passer sans mesure (garde-fou inutilisé aujourd'hui) |
| Test | Charge | `cargo xtask loadtest --clients 20 --duration-secs 20` (budgets : p95 HELLO ≤ 250 ms, 0 erreur WS/ingest, 0 déconnexion, accueils = clients, pas de fuite fd sous Linux, p95 board signé ≤ 250 ms à 200 items, p95 média signé ≤ 250 ms, p95 ack `media.trigger` ≤ 250 ms à 10/s) ; nightly : 50 clients / 60 s avec les mêmes budgets | exit 0 local ; nightly advisory |
| Test | Chaos | idem batterie — `apps/lc-server/tests/kill_recover.rs` (kill brutal en pleine ingestion → WAL → reprise : chaque ligne confirmée retrouvée, `PRAGMA integrity_check` ok) | incluse dans la batterie |
| Déploiement | Canary | `powershell -NoProfile -File scripts/release.ps1 deploy-server X.Y.Z -Canary` — conteneur `LiveChat-canary` sur `127.0.0.1:3001`, copie de la sauvegarde vérifiée, bot Discord de TEST (env file hôte, jamais lu ni affiché), santé + Discord avant de toucher au principal ; harnais `scripts/release/tests/Canary.Tests.ps1` | GO explicite de l'utilisateur, jamais automatique |
| Transverse | Revue adversariale | hors commande : second agent indépendant relit le diff (tests invincibles, sabotages harnais vs produit, périmètre, générateurs aveugles) | cadence du plan : Review A (mi-pipeline), Review B (fin) |

## 2. Adaptations assumées (diagramme source → LiveChat)

| Famille d'origine | Adaptation LiveChat |
|---|---|
| Formal verification | invariants en **property tests** + protocole v2 **gelé** ([`protocol-v2.md`](protocol-v2.md), 38 fixtures intouchables) ; pas de Kani (coût > bénéfice ici) |
| Shadowing / traffic mirroring | harnais de charge + canary sur une **copie** de la base réelle (homelab : pas de trafic prod à mirorer) |
| Monitoring | `/health` + console admin existantes + **sonde externe** côté utilisateur (p. ex. Uptime Kuma sur Unraid) ; `/metrics` abandonné (pas de Prometheus) |
| Feature flagging | réglages runtime existants (`/admin/api/runtime-settings`, journalisés) ; pas de système de flags env |
| Real user monitoring | déjà stocké au HELLO : `appVersion` + `platform` de chaque overlay |
| Logs JSON / OTLP / benches criterion | hors périmètre (pas d'agrégateur de logs ; microseconde de décodage, pas de comparaison CI) |

## 3. Nightly ([`verification-nightly.yml`](../../.github/workflows/verification-nightly.yml))

- **`fuzz`** : matrix des 3 cibles (`protocol_frame`, `media_url`, `html_parsers`), corpus semé depuis les fixtures, `-max_total_time=45` par cible, seeds+golden (jamais un merge gate) ; artefacts uploadés si échec.
- **`mutants`** : une jambe de matrix par scope, six scopes **mesurés** — `lc-protocol`, `lc-media --file guard.rs`, `lc-db --file playback_jobs.rs`, et les trois ajoutés par Q-T4 (`lc-playback --file payload.rs`, `lc-db --file db.rs`, `lc-media --file storage.rs`), mesurés sur `ubuntu-latest` avec la commande ci-dessous (run [45234358272](https://github.com/quentingimenez/LiveChat/actions/runs/45234358272), 23/11 : 57, 81 et 79). Score `floor(100·caught/(caught+missed+timeout))` comparé à la baseline ; garde d'entiers jq, échec sur toute outcome `Failure`, rc admis 0/2, `rm -rf target/mutants` (rust-cache peut restaurer de vieux outcomes), outils épinglés `cargo-mutants@27.1.0` / `nextest@0.9.151`. Chaque jambe garde `timeout-minutes: 60` et `fail-fast: false` ; une matrix par scope évite de dépasser le budget de 55 min de `ci-gate` sur une PR.
- Aucun scope n'est provisoire aujourd'hui : les six `caught_pct` sont numériques et comparés à la baseline. Le garde-fou provisoire reste dans le workflow et dans la baseline pour un scope ajouté avant sa première mesure (`caught_pct: null`, `status: "provisional"`) : sa jambe est sautée sur `pull_request` (`if: github.event_name != 'pull_request' || matrix.provisional != true`) pour ne pas rendre `ci-gate` rouge sans mesure ; le nightly planifié l'exécute, uploade `outcomes.json` (artefact `mutants-report-<scope>`), puis échoue explicitement en indiquant le score observé et les deux champs à passer à `measured` / `provisional: false`. Le garde [`MutantsNightly.Tests.ps1`](../../scripts/release/tests/MutantsNightly.Tests.ps1) vérifie cette cohérence.
- **`load`** : ubuntu-latest, build release (`LC_ADMIN_WEB_BUILD=0`), 50 clients / 60 s,
  budgets §4.1 explicites (`--board-items 200 --max-board-p95-ms 250
  --max-media-p95-ms 250 --media-trigger-rate 10 --max-media-trigger-p95-ms 250`),
  artefact uploadé dans tous les cas. Les budgets board/média/trigger ont été ajoutés
  après la dernière mesure de mutation : le run nightly `load` est leur seule preuve,
  pas le stage `--mutants` du 20/11 (§5).
- **`ignored`** : ubuntu-latest, `cargo nextest run -p lc-media -p lc-server -p xtask
  --all-features --locked --run-ignored only --no-fail-fast -E "$NEXTEST_FILTER"`
  (FFmpeg et yt-dlp installés par apt). `NEXTEST_FILTER` est posé par le step `Select`
  depuis [`scripts/release/ignored-nightly-tests.json`](../../scripts/release/ignored-nightly-tests.json),
  qui inventorie et classe les 52 tests ignorés : 33 déterministes exécutés chaque nuit,
  19 qui exigent un service externe ou des credentials (tikwm/TikTok, CDN TikTok, X,
  Tenor, Giphy, Imgur, YouTube, Dailymotion, images Brave, gateway Discord). Le run
  automatique applique `not (...)` construit depuis `network_filter` ; les 19 tests
  réseau ne s'exécutent qu'à la demande, par `workflow_dispatch` avec
  `network_tests = true`, ou localement (voir §4). Le test Discord, seul test à
  credentials, échoue explicitement sans `DISCORD_TOKEN`/`DISCORD_CLIENT_ID` et
  vérifie le jeton auprès de l'API, au lieu de passer vide. La garde
  [`IgnoredNightly.Tests.ps1`](../../scripts/release/tests/IgnoredNightly.Tests.ps1)
  confronte le manifeste aux `#[ignore]` réels et épingle les catégories : ajouter,
  retirer ou reclasser un test sans mettre le manifeste à jour fait échouer
  `release-check`.
- Déclencheurs : `cron '30 3 * * *'`, `workflow_dispatch`, `pull_request` limité à ses propres fichiers + `fuzz/**` + la baseline. **Jamais un merge gate** — un nightly rouge est un signal, pas un blocus.

## 4. Usage local

```powershell
cargo xtask verify --offline                                              # fmt, clippy, nextest, doctests, cargo deny
cargo xtask verify --offline --mutants                                    # + mutation du diff (kill-or-justify, §5)
cargo xtask loadtest --clients 20 --duration-secs 20                      # harnais de charge + budgets
powershell -NoProfile -File scripts/release/tests/run-tests.ps1            # harnais PowerShell de release (Canary inclus)
powershell -NoProfile -File scripts/release/tests/run-tests.ps1 -Filter ignored  # garde de l'inventaire du nightly `ignored`
cargo test --manifest-path fuzz/Cargo.toml --no-default-features --lib --locked --offline  # seeds + golden fuzz
# tests ignorés réseau (nightly manuel) — Internet + DISCORD_TOKEN + yt-dlp/ffmpeg requis
cargo nextest run -p lc-media -p lc-server -p xtask --all-features --run-ignored only -E 'all()'
```

## 4.1 Budgets de charge du harnais

Le harnais `cargo xtask loadtest` crée une guilde de sonde séparée des guildes de
charge, y sème `--board-items` assets `READY` (200 par défaut, un fichier de 32 Kio
sous `MEDIA_STORAGE_DIR`) et autant d'items de board, puis mesure trois budgets
supplémentaires en plus des budgets historiques (HELLO, erreurs WS/ingest,
déconnexions, fd). Tous sont des lignes `PASS`/`FAIL` ; le processus sort en 1 dès
qu'une ligne échoue.

| Métrique | Budget | Comment c'est mesuré | Statut |
|---|---|---|---|
| TTFB liste board (B-03) | p95 ≤ 250 ms | 1 GET/s de `/overlay/media-board/items?limit=200` avec le Bearer de sonde, temps jusqu'aux en-têtes | **exécutable** (`--board-items 200 --max-board-p95-ms 250`) |
| TTFB média signé | p95 ≤ 250 ms | GET de `items[0].mediaUrl` (URL HMAC renvoyée par la liste), temps jusqu'aux en-têtes, corps vidé | **exécutable** (`--max-media-p95-ms 250`) |
| Coût `MediaTrigger` (B-02) | p95 ack ≤ 250 ms à 10/s, et tous les triggers envoyés acquittés `ok` | un client WS de sonde envoie `media.trigger` à 10/s (max 19, plafond serveur 20/s) et attend l'ack correspondant par `payload.ref` ; nécessite `--board-items > 0` | **exécutable** (`--media-trigger-rate 10 --max-media-trigger-p95-ms 250`) — latence de bout en bout, pas le nombre de SQL |
| Profondeur de la file WS sortante (B-01) | p95 ≤ 64 frames, max ≤ 256 frames | file bornée `OUTBOUND_QUEUE_CAPACITY = 256` **trames** par connexion (pas une borne d'octets : ~16 Mio de texte au pire par socket) ; profondeur exposée par `Hub::outbound_queue_depth` et testée (unitaire + intégration avec client qui cesse de lire → close `4408 slow_client`, close explicite prioritaire) | **partiellement exécutable** — borne max 256 testée, p95 sous charge non mesuré (pas encore exposé par `cargo xtask loadtest`) |
| Attente du pool SQLite | proposition : p95 ≤ 25 ms, max ≤ 100 ms | aucune instrumentation sqlx (`before_acquire`) ; le TTFB board/média n'est qu'un proxy de bout en bout | **non exécutable** — gap assumé |
| SSE admin | proposition : p95 de rafraîchissement ≤ 250 ms avec 10 consoles, 0 déconnexion sur 60 s | `GET /admin/api/events` exige une session admin que le harnais ne sait pas créer | **non exécutable** — gap assumé |
| CPU extension | proposition : moyenne ≤ 5 % d'un cœur et p95 frame ≤ 16,7 ms sur 60 s de scan | l'extension tourne dans le navigateur, hors du processus `lc-server` | **non exécutable** — gap assumé |

Justification des seuils exécutables : 250 ms est le budget HELLO déjà accepté et
la recommandation de l'audit perf pour la liste board à 200 items ; les budgets
média et trigger s'alignent dessus pour rester sous la même enveloppe perceptible.
10/s est la moitié du plafond serveur de 20 messages client/s, ce qui laisse la
marge de heartbeats et rend le budget sensible à une régression du handler sans le
pousser dans la zone de rate-limit. Les trois budgets de gap restants (pool
SQLite, SSE admin, CPU extension) sont des **propositions chiffrées à valider
après instrumentation**, pas des mesures : ils ne doivent pas être présentés
comme verts tant que le compteur n'existe pas. Pour B-01, la borne 256 trames,
la fermeture `4408 slow_client` et la priorité du close explicite sont désormais
testées, mais le p95 sous charge demande encore un compteur dans le rapport de
charge.

Commandes :

```powershell
# budgets historiques + board/média/trigger (nightly et local)
cargo xtask loadtest --clients 20 --duration-secs 20 `
  --board-items 200 --max-board-p95-ms 250 `
  --max-media-p95-ms 250 `
  --media-trigger-rate 10 --max-media-trigger-p95-ms 250

# désactiver les nouvelles sondes (comparaison avec l'ancien harnais uniquement)
cargo xtask loadtest --clients 20 --duration-secs 20 --board-items 0 --media-trigger-rate 0
```

Le rapport JSON expose `board_p95_ms`, `board_samples`, `board_errors`,
`media_p95_ms`, `media_samples`, `media_errors`, `media_trigger_p95_ms`,
`media_trigger_sent`, `media_trigger_acked`, `media_trigger_errors` : une ligne
`PASS` n'est jamais rendue sans échantillon (`n/a` et `FAIL` si `samples == 0`).

### Plan des gaps

- **B-01 file WS** : implémenté dans `apps/lc-server/src/ws` — canal borné à
  **256 trames par connexion** (`OUTBOUND_QUEUE_CAPACITY`, borne en messages et
  non en octets), production `try_send` non bloquante, close `4408 slow_client`
  de la seule connexion saturée, close explicite (`4402`/admin) toujours
  prioritaire même quand la file est pleine, timeout de `sink.send` à 5 s
  (`WRITER_SEND_TIMEOUT`) ; profondeur exposée en test par
  `Hub::outbound_queue_depth`. En saturation, le close explicite reste prioritaire
  sauf si la session a déjà lu un `4408` publié avant son annonce ; le `stop` qui
  le précède est best-effort. Reste pour la charge : exposer
  p95/max dans le rapport de `cargo xtask loadtest` et y scénariser un client qui
  cesse de lire. Le harnais actuel ne peut pas distinguer une file applicative
  d'un tampon noyau : ne pas déduire un PASS de l'absence de déconnexion.
- **Pool SQLite** : instrumenter `before_acquire`/`after_acquire` dans
  `crates/lc-db`, agréger p95/max par run ; le TTFB board actuel inclut les
  requêtes et la signature, pas l'attente de connexion seule.
- **SSE admin** : créer une session admin de test (ou un jeton de charge dédié)
  puis mesurer 10 connexions `GET /admin/api/events` et la latence de diffusion ;
  aujourd'hui l'authentification admin n'est pas dans le harnais.
- **CPU extension** : profil Playwright/Chrome sur l'extension réelle (scan board
  + scroll) avec `Performance.getMetrics` ; hors périmètre du binaire serveur.

## 5. Le stage mutants : kill-or-justify, zéro survivant

`cargo xtask verify --offline --mutants` termine en `FAIL mutants (exit 1)` dès
qu'il reste un mutant manqué : l'exigence n'est pas « vert », elle est
**kill-or-justify** — chaque mutant manqué tué par un test, ou justifié avec
`file:line` + catégorie + risque résiduel. Aucun survivant toléré sans raison
tracée. Depuis le round du 20/11, **`xtask/` n'a plus aucun survivant** : le
stage local est vert (`59 mutants tested in 8m: 52 caught, 7 unviable`,
**0 manqué, 0 timeout** → `PASS mutants`, `verify: 6 passed, 0 failed`).

- Stage local du 20/11 (cargo-mutants 27.1.0) :
  **`59 mutants tested in 8m: 52 caught, 7 unviable`**, **0 manqué, 0 timeout**
  → `PASS mutants`, `verify: 6 passed, 0 failed`. Le `--in-diff` du stage ne
  mute que les lignes du diff de branche : 59 et non 141.
- Historique : le stage du 19/11 finissait rouge sur **62 survivants `xtask/`**
  (`144 outcomes = 1 baseline + 65 caught + 62 missed + 16 unviable`,
  « 143 mutants tested in 18m » ; `loadtest.rs` 60, `verify.rs` 2) — **0 manqué
  en `crates/` et `apps/`** dans le stage local : le seul manqué produit connu,
  `playback_jobs.rs:211` (justifié, §6), vit dans le scope nightly
  `lc-db-queue`, hors diff local. Critère décisif, respecté dans tous les runs
  de la branche. Les compteurs bruts d'`xtask/` bougeaient d'un run à l'autre
  (reclassements caught/unviable/missed des mêmes lignes : 86 puis 77 missed
  sur les runs C4/DoD#5) : seul le critère « 0 manqué » fait foi.
- Ces 62 survivants sont **éteints** depuis le round du 20/11 :
  - **30 éliminés par refactor** (structural-first) : `execute()` dissous des
    deux côtés (`main.rs` appelle `verify::drive(&verify::default_stages(..))`
    et `loadtest::run(..)`), agrégation des 8 budgets remplacée par
    `budget_lines`/`count_failures`/`print_budgets` (typée, testable), et la
    construction faillible `reqwest::Client::builder().build()` remplacée par
    `Client::new()` (infaillible ; le timeout de 5 s par requête passe dans
    `post_ingest`) — la branche `Err` inatteignable disparaît au lieu d'être
    justifiée.
  - **32 tués par 25 tests** via des seams injectables : `check_server_binary`,
    `window_has_ended`, `health_deadline`, `straggler_timeout`,
    `wait_healthy_with`, `is_healthy_body`, `Tally`, `ingest_tick_interval`,
    `classify_ingest_status`, `post_ingest`, `IngestStats::record`,
    `budget_lines`, `print_budgets`, `exit_code` (+ test process-level de
    `cli_exit_codes.rs`).
- Mesures par scope en copy mode hors repo après le round
  (`--cargo-arg=--offline`) : `loadtest.rs` **94 outcomes = 1 baseline +
  93 mutants (86 caught + 0 manqué + 7 unviable)** ; `verify.rs`
  **37 outcomes = 1 baseline + 36 mutants (27 caught + 0 manqué + 9 unviable)**
  (36 et non 38 : les 2 mutants `execute` ont disparu avec la fonction) ;
  `main.rs` **13 outcomes = 1 baseline + 12 mutants (7 caught + 0 manqué +
  5 unviable)** ; `download.rs` 3/3 CaughtMutant. Total : **141 mutants,
  120 caught, 21 unviable, 0 manqué, 0 timeout**. Les 21 « unviable » sont des
  mutations qui ne compilent pas (types sans `Default`, `+`→`*` sur des
  `Duration`/`Instant`) : éliminées par le compilateur, pas à justifier.

- Seuls **4 fichiers du diff produisent des mutants** (loadtest 93, verify 36,
  main 12, download 3) sur les `.rs` du diff de branche : les fichiers de test
  ne sont pas mutés (`kill_recover.rs`, `contracts.rs`,
  `xtask/tests/cli_exit_codes.rs` → 0 mutant).
- Preuve produit : les 3 mutants du **retour de `build_ytdlp_args`**
  (`crates/lc-media/src/download.rs:413` : `vec![]`, `vec![String::new()]`,
  `vec!["xyzzy".into()]`) sont CaughtMutant — la fonction production modifiée par
  le pipeline (insertion de `--` avant l'URL) est exercée et contrainte.
- Preuve à joindre : le `target/verify/mutants.out/missed.txt` du stage du 20/11
  est **vide** (idem `timeout.txt`). Si un futur diff réintroduit un survivant,
  le stage repasse rouge et `missed.txt` en liste les positions : le tuer, ou le
  justifier ici même avec `file:line` + catégorie + risque résiduel — jamais le
  tolérer en silence.
- **Risque résiduel assumé (hors garantie mutation)** : les corps des bras
  `tokio::select!` de `run` (armement du timer fd, comparaisons hello/welcomes
  dans la macro) ne sont pas visités par cargo-mutants — une invocation de macro
  n'est pas mutée. Ces branches restent couvertes uniquement par le run de charge
  **nightly `load`**, pas par le kill-or-justify.

## 6. Nightly mutants : baselines et fragilité

- [`scripts/mutants-baseline.json`](../../scripts/mutants-baseline.json), six scopes, tous mesurés :
  - `lc-protocol {caught_pct: 99, min_mutants: 47}`,
    `lc-media-guard {caught_pct: 99, min_mutants: 86}`,
    `lc-db-queue {caught_pct: 93, min_mutants: 92}` — mesuré 100/100/94, baseline = mesure − 1 ;
  - Q-T4, remesurés après kill-or-justify le 24/11 en local (Windows
    `x86_64-pc-windows-msvc`, cargo-mutants 27.1.0, nextest 0.9.151, commande du
    nightly) :
    `lc-playback-payload {caught_pct: 97, min_mutants: 44}`
    (`crates/lc-playback/src/payload.rs`, 39 caught / 1 manqué justifié / 3
    unviable), `lc-db-core {caught_pct: 100, min_mutants: 77}`
    (`crates/lc-db/src/db.rs`, 16 / 0 / 60), `lc-media-storage {caught_pct: 100,
    min_mutants: 125}` (`crates/lc-media/src/storage.rs`, 49 / 0 / 75). Les 30
    survivants du run
    [45274080193](https://github.com/quentingimenez/LiveChat/actions/runs/45274080193)
    sont tués par les tests du même changement, sauf l'équivalent ci-dessous. Les
    scores restent enregistrés **bruts**
    (`floor(100·caught/(caught+missed+timeout))` = 39/40, 16/16 et 49/49), pas en
    mesure − 1 : le dénominateur viable est petit et un seul mutant reclassé vaut
    2 à 6 points. `min_mutants` = total `--list` + 1 outcome de baseline :
    43 + 1, 76 + 1 et 124 + 1 (les 7 mutants de la fonction extraite
    `classify_removal`, tous unviable, expliquent le passage de 117 à 124).
- Les `min_mutants` Q-T4 viennent de `cargo mutants --list -p <crate> --file <fichier> --test-tool nextest -j 2` au commit de base
  (`32686b6`) : 43, 76 et 117 mutants, **+ 1** pour l'outcome `Success` de la baseline, comme les scopes existants (85 → 86,
  91 → 92) ; après l'extraction de `classify_removal`, `lc-media-storage` liste 124 mutants et les runs de mesure ont confirmé
  les totaux 44/77/125. Ce sont des **planchers** : un total observé inférieur échoue ; un total supérieur reste accepté.
- Le provisoire reste un **garde-fou inutilisé** pour un futur scope ajouté avant sa première mesure (`caught_pct: null`,
  `status: "provisional"`). Pour lever ce provisoire :
  1. lire `outcomes.json` de l'artefact `mutants-report-<scope>` et calculer
     `floor(100·caught/(caught+missed+timeout))` ;
  2. écrire ce score dans `caught_pct` et passer `status` à `"measured"` dans la baseline ;
  3. passer `provisional: false` pour ce scope dans la matrix de `verification-nightly.yml` ;
  4. mettre à jour les valeurs épinglées de `scripts/release/tests/MutantsNightly.Tests.ps1` dans le même changement.
- Survivants Q-T4 du run 45274080193, tués par les tests ajoutés (tous dans le
  `#[cfg(test)] mod tests` du fichier muté, donc exécutés par le filtre des
  jambes) :
  - `crates/lc-playback/src/payload.rs` (17) : `45:38`, `55:59`, `61:28` par les
    trois `dispatch_timing_*` de bornes ; `66:80` (`-`→`+` et `-`→`/`), `70:48`
    par `dispatch_timing_applies_the_resume_offset_to_the_remaining_duration` ;
    `114:46` par `build_play_payload_omits_a_non_positive_media_duration` ;
    `116:43`, `116:70` (trois) par
    `build_play_payload_emits_the_offset_only_for_seekable_media` et
    `build_play_payload_pins_the_protocol_fields` ; `149:5` (deux) et `151:19`
    (trois) par `build_play_payload_pins_the_protocol_fields` et
    `..._emits_the_offset_only_for_seekable_media` (URL exacte, offset 0 et 5,
    audio vs image).
  - `crates/lc-db/src/db.rs` (3) : `198:9` par `close_closes_the_shared_pool`
    (pool cloné, `acquire` en erreur après `close`) ; `374:5` par
    `sqlite_url_without_mode_ro_is_writable` et
    `requests_read_only_requires_both_mode_and_ro` ; `380:46` par le second
    (`?advice=ro` ne doit pas activer la lecture seule, `?mode=ro` si).
  - `crates/lc-media/src/storage.rs` (10) : `119:27` et `326:27` par
    `storage_root_that_is_a_file_surfaces_the_error` ; `180:27` par
    `confined_path_surfaces_invalid_os_paths` ; `265:27` (deux) et `265:40` par
    `removal_classifies_missing_files_and_surfaces_other_errors` — la
    classification de `remove_path` est extraite dans `classify_removal`
    (`storage.rs:439-448`, mutants devenus `442`), testée sur `Ok`, `NotFound` et
    `PermissionDenied` au lieu d'être justifiée comme race-only ; `349:39` par
    `list_variants_ignores_directories` ; `443:5` (trois) par
    `now_ms_is_a_current_epoch_timestamp` (fenêtre autour de `SystemTime::now`).
- Le seul survivant restant est **équivalent** : `payload.rs:141:9`
  (`tweet_card: None` supprimé de `build_play_payload`). `PlayPayload::default()`
  affecte déjà `tweet_card: None` (`crates/lc-protocol/src/lib.rs:953-972`, ligne
  967) et le littéral finit par `..Default::default()` : la suppression ne change
  pas la valeur construite, aucun test ne peut distinguer le mutant. Le champ
  explicite est conservé et `build_play_payload_pins_the_protocol_fields` y épingle
  `tweet_card.is_none()`.
- Limites de mesure Q-T4 : la remesure ci-dessus est locale sur Windows, pas sur
  `ubuntu-latest` ; le prochain nightly (ou un run manuel) doit confirmer les trois
  scores. Un premier run local de `lc-playback-payload` a marqué `116:70`
  (`>`→`<`) unviable sur un `LNK1104` transitoire du linker Windows ; relancé, il
  donne 39/1/3, cohérent avec les 3 unviable du CI. Le total `lc-media-storage`
  passe de 117 à 124 à cause de `classify_removal` (7 mutants de remplacement de
  fonction, tous unviable), d'où `min_mutants: 125`.
- Mutants manqués connus : `crates/lc-db/src/repos/playback_jobs.rs:211` et
  `crates/lc-playback/src/payload.rs:141:9` sont les deux manqués **justifiés**
  (le premier race-only, bras unique-violation, sqlite mémoire mono-connexion,
  justification aussi en doc du test
  `raw_db_errors_are_not_silenced_nor_mistranslated` ; le second équivalent,
  preuve ci-dessus).
- `min_mutants` (total exact d'outcomes, incl. baseline + unviable) est **fragile** :
  un changement de version cargo-mutants ou d'échantillonnage peut décaler le total —
  recaler baselines et min_mutants ensemble si cela arrive.
- Addendum Task 5 : le 3ᵉ scope `lc-db-queue` et les `min_mutants` étendent les
  2 scopes du plan initial, ajoutés dans `db74f49` (correctifs Review A) sur
  directive utilisateur du 19/11.
- Addendum Q-T4 : les trois scopes ajoutent les fichiers critiques de `lc-playback`, `lc-db` et `lc-media` au-delà de `guard.rs` ; chaque
  jambe est un runner séparé pour rester sous 60 min. Ils sont remesurés après kill-or-justify le 24/11 en local (97/100/100) ; le run
  45274080193 (PR #187) reste la référence des survivants. Le garde `MutantsNightly.Tests.ps1` fait échouer toute incohérence
  matrix/baseline et conserve le scénario provisoire comme garde-fou.

## 7. Pièges locaux connus (Windows, PowerShell 5.1)

- **cargo-mutants `--in-place` restaure les fichiers en LF** → statut git fantômes
  ` M` à diff vide : normaliser en CRLF byte-level (working tree autocrlf) ; « git
  checkout -- » écraserait les modifications locales et n'est pas la procédure
  validée ici — utiliser la restauration byte-pure ci-dessous. **Plus grave :
  un run `--in-place` tué en plein vol (crash, reboot) laisse le mutant EN COURS
  appliqué dans l'arbre** — le restart DSH du 19/11 a laissé un `p % 100.0` dans
  le calcul de percentile de `xtask/src/loadtest.rs`, fmt/tests/mutants rouges au
  run suivant. Après toute
  interruption : `git diff` (pas seulement `git status`) sur les fichiers du scope
  AVANT de relancer ; restaurer via `git --no-pager show HEAD:<fichier>` redirigé
  par `cmd /c` (byte-pure, pas de couche d'encodage PowerShell) + conversion
  LF→CRLF byte-level + `git update-index --refresh` pour purger le marqueur stat.
- proptest écrit ses graines d'échec dans `<crate>/tests/*.proptest-regressions*` :
  purger après un sabotage, sinon elles traînent dans le statut.
- PS 5.1 uniquement en local : pas de `&&` ; `.Count` sur un objet unique piège les
  compteurs (préférer `Group-Object`).
- Le job `load` nightly n'a **jamais tourné sur Linux** : le premier nightly sera la
  première exécution du budget fd (risque accepté, surveiller le premier run).
- **`cargo xtask` tourne en `--release`** (alias dans [.cargo/config.toml](../../.cargo/config.toml)) :
  dès que xtask a un test d'intégration (`tests/cli_exit_codes.rs`, binaire
  `CARGO_BIN_EXE_xtask`), le stage tests de la batterie
  (`cargo test --no-run --workspace --all-features`) relink `target\debug\xtask.exe`
  **pendant que le processus batterie l'exécute** → Windows refuse de remplacer l'image
  en cours (« failed to remove xtask.exe ») si l'alias lance le debug. L'alias release
  disjoint les chemins (batterie = `target\release\`, stage tests = `target\debug\`),
  collision impossible quel que soit l'état des artefacts ; coût : un build release de
  xtask (~2-4 min la première fois, incrémental ensuite). Ne pas repasser l'alias en
  debug sans avoir lu ce paragraphe.
- **Un kill probabiliste n'est pas un kill** : une propriété dont la stratégie ne
  tire un voisin frontière qu'avec une faible probabilité par run n'est pas une
  garantie — cas réel : `guard.rs:153:45` (`&&`→`||` interne de la plage de
  documentation 198.51.100) a survécu un run nightly complet (score 98 < 99) :
  ses 256 cas aléatoires ont manqué le tirage que chaque run local avait
  touché. Épingler les littéraux exacts, un par bras de mutant, comme
  `documentation_range_neighbors_pin_the_boundary_literals`
  (`crates/lc-media/tests/properties.rs`).
- Nits acceptées en revue B : TOCTOU de `reserve_port` (`loadtest.rs:807`, flake rare),
  alphabet du pin UNC sans `:` ni `\` (`properties.rs`), `-Canary` ignoré en silence par
  les autres sous-commandes de `release.ps1`, borne `uptimeSec >= 0` faible
  (`contracts.rs`).

## 8. Statut final du plan

T0 (spec) · T0b (deps) · T1 (xtask verify) · T2/T2b (propriétés protocole) ·
T3 (propriétés garde/queue) · T4 (fuzz) · T5 (nightly mutants) ✅ ·
**Review A** ✅ (correctifs commités : kill-or-justify des mutants garde, jq durci,
stage_argv épinglé sur `rust.yml`, golden fuzz) · T6 (contrats HTTP + matrice WS) ·
T7 (harnais de charge) · T8 (kill-and-recover) · T9 (canary + bot de test) ✅ ·
**Review B** ✅ (S-a documenté dans le skill release, S-b matrice complétée) ·
T10 = ce document. La table d'état du fichier de plan était un instantané de suivi ;
la présente section fait foi.

**Round kill-or-justify xtask** (directive du 19/11, post-T10) : gates de
`verify.rs` couverts (execute → `drive` testé octet par octet, `require_tool`
sans `!`, `probe_installed` déterministe), helpers `loadtest.rs`
dissous/épinglés, `main.rs` entièrement couvert, tests process-level
(`tests/cli_exit_codes.rs`), alias `cargo xtask` passé en release pour la
collision `xtask.exe` (§7) ; restaient 62 survivants justifiés (§5), éteints au
round suivant.

**Round « tuer les mutants xtask »** (20/11, post-merge) : les 62 survivants du
round précédent sont éteints — 30 par refactor (executes dissous, budgets typés,
branche `reqwest` infaillible supprimée), 32 par 25 tests via des seams
injectables (§5) — mesures copy-mode `loadtest.rs` 93 / `verify.rs` 36 /
`main.rs` 12 = **0 manqué, 0 timeout**, et le stage local `--mutants` passe
**vert** pour la première fois (`59 mutants, 52 caught, 7 unviable`,
`verify: 6 passed, 0 failed`) ; `scripts/mutants-baseline.json` inchangé (`xtask`
n'est pas un scope nightly).

**Q-T4** (élargissement mutation, lot L1) : le job nightly `mutants` passe d'un run
monolithique de 3 scopes à une matrix de 6 jambes indépendantes ; les trois fichiers
critiques ajoutés (`lc-playback/src/payload.rs`, `lc-db/src/db.rs`,
`lc-media/src/storage.rs`) ont été mesurés sur `ubuntu-latest` avec la commande du
workflow (run [45234358272](https://github.com/quentingimenez/LiveChat/actions/runs/45234358272),
23/11) : **57, 81 et 79** de `caught_pct`, `min_mutants` 44/77/118. Ils sont
`status: "measured"` et `provisional: false` ; les six jambes tournent sur
`pull_request` comme sur le nightly. Le garde
[`MutantsNightly.Tests.ps1`](../../scripts/release/tests/MutantsNightly.Tests.ps1)
exige la cohérence matrix/baseline et couvre encore le scénario provisoire comme
garde-fou pour un futur scope.

## 9. Skill agent

Les agents suivent [`.claude/skills/verify/SKILL.md`](../../.claude/skills/verify/SKILL.md) :
spec → tests défaillants d'abord → implémentation → `verify --offline` → `--mutants`
kill-or-justify → sabotage → revue adversariale → canary avant tout déploiement.
