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
| Pré-prod | Mutation | `cargo xtask verify --offline --mutants` (lignes du diff, kill-or-justify) ; nightly : 3 scopes notés contre [`scripts/mutants-baseline.json`](../../scripts/mutants-baseline.json) | local : 0 manqué **non justifié** en `crates/` et `apps/` (le manqué justifié du scope nightly `lc-db-queue`, `playback_jobs.rs:211`, est hors diff local) ; nightly : score ≥ baseline |
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
- **`mutants`** : 3 scopes (`lc-protocol`, `lc-media --file guard.rs`, `lc-db --file playback_jobs.rs`), score `floor(100·caught/(caught+missed+timeout))` comparé à la baseline ; garde d'entiers jq, échec sur toute outcome `Failure`, rc admis 0/2, `rm -rf target/mutants` (rust-cache peut restaurer de vieux outcomes), outils épinglés `cargo-mutants@27.1.0` / `nextest@0.9.151`.
- **`load`** : ubuntu-latest, build release (`LC_ADMIN_WEB_BUILD=0`), 50 clients / 60 s,
  budgets §4.1 explicites (`--board-items 200 --max-board-p95-ms 250
  --max-media-p95-ms 250 --media-trigger-rate 10 --max-media-trigger-p95-ms 250`),
  artefact uploadé dans tous les cas. Les budgets board/média/trigger ont été ajoutés
  après la dernière mesure de mutation : le run nightly `load` est leur seule preuve,
  pas le stage `--mutants` du 20/11 (§5).
- Déclencheurs : `cron '30 3 * * *'`, `workflow_dispatch`, `pull_request` limité à ses propres fichiers + `fuzz/**` + la baseline. **Jamais un merge gate** — un nightly rouge est un signal, pas un blocus.

## 4. Usage local

```powershell
cargo xtask verify --offline                                              # fmt, clippy, nextest, doctests, cargo deny
cargo xtask verify --offline --mutants                                    # + mutation du diff (kill-or-justify, §5)
cargo xtask loadtest --clients 20 --duration-secs 20                      # harnais de charge + budgets
powershell -NoProfile -File scripts/release/tests/run-tests.ps1            # harnais PowerShell de release (Canary inclus)
cargo test --manifest-path fuzz/Cargo.toml --no-default-features --lib --locked --offline  # seeds + golden fuzz
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
  `Hub::outbound_queue_depth`. En saturation, le close explicite est garanti mais
  le `stop` qui le précède est best-effort. Reste pour la charge : exposer
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

- [`scripts/mutants-baseline.json`](../../scripts/mutants-baseline.json) :
  `lc-protocol {caught_pct: 99, min_mutants: 47}`,
  `lc-media-guard {caught_pct: 99, min_mutants: 86}`,
  `lc-db-queue {caught_pct: 93, min_mutants: 92}` — mesuré 100/100/94, baseline = mesure − 1.
- Seul mutant manqué restant, **justifié** : `crates/lc-db/src/repos/playback_jobs.rs:211`
  (bras unique-violation, race-only, sqlite mémoire mono-connexion ; justification aussi
  en doc du test `raw_db_errors_are_not_silenced_nor_mistranslated`).
- `min_mutants` (total exact d'outcomes, incl. baseline + unviable) est **fragile** :
  un changement de version cargo-mutants ou d'échantillonnage peut décaler le total —
  recaler baselines et min_mutants ensemble si cela arrive.
- Addendum Task 5 : le 3ᵉ scope `lc-db-queue` et les `min_mutants` étendent les
  2 scopes du plan initial, ajoutés dans `db74f49` (correctifs Review A) sur
  directive utilisateur du 19/11.

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

## 9. Skill agent

Les agents suivent [`.claude/skills/verify/SKILL.md`](../../.claude/skills/verify/SKILL.md) :
spec → tests défaillants d'abord → implémentation → `verify --offline` → `--mutants`
kill-or-justify → sabotage → revue adversariale → canary avant tout déploiement.
