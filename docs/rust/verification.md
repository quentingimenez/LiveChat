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
| Test | Charge | `cargo xtask loadtest --clients 20 --duration-secs 20` (budgets : p95 HELLO ≤ 250 ms, 0 erreur WS/ingest, 0 déconnexion, accueils = clients, pas de fuite fd sous Linux) ; nightly : 50 clients / 60 s | exit 0 local ; nightly advisory |
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
- **`load`** : ubuntu-latest, build release (`LC_ADMIN_WEB_BUILD=0`), 50 clients / 60 s, artefact uploadé dans tous les cas.
- Déclencheurs : `cron '30 3 * * *'`, `workflow_dispatch`, `pull_request` limité à ses propres fichiers + `fuzz/**` + la baseline. **Jamais un merge gate** — un nightly rouge est un signal, pas un blocus.

## 4. Usage local

```powershell
cargo xtask verify --offline                                              # fmt, clippy, nextest, doctests, cargo deny
cargo xtask verify --offline --mutants                                    # + mutation du diff ; FAIL/exit 1 attendu (§5)
cargo xtask loadtest --clients 20 --duration-secs 20                      # harnais de charge + budgets
powershell -NoProfile -File scripts/release/tests/run-tests.ps1            # harnais PowerShell de release (Canary inclus)
cargo test --manifest-path fuzz/Cargo.toml --no-default-features --lib --locked --offline  # seeds + golden fuzz
```

## 5. Le stage mutants échoue localement — c'est voulu

`cargo xtask verify --offline --mutants` termine en `FAIL mutants (exit 1)` dès
qu'il reste un mutant manqué : l'exigence n'est pas « vert », elle est
**kill-or-justify** — chaque mutant manqué tué par un test, ou justifié avec
`file:line` + catégorie + risque résiduel. Aucun survivant toléré sans raison
tracée.

- Stage local (round kill-or-justify xtask du 19/11, cargo-mutants 27.1.0) :
  **144 outcomes = 1 baseline + 65 caught + 62 missed + 16 unviable**
  (« 143 mutants tested in 18m ») ; missed **tous dans xtask**
  (`loadtest.rs` 60, `verify.rs` 2) — **0 manqué en `crates/` et `apps/`**
  dans le stage local (diff de branche) : le seul manqué produit connu,
  `playback_jobs.rs:211` (justifié, §6), vit dans le scope nightly
  `lc-db-queue`, hors diff local — critère décisif, respecté dans tous les
  runs de la branche.
- Mesures par scope (copy mode hors repo, `--cargo-arg=--offline`) :
  `verify.rs` 39 = 1 + 26 caught + 2 missed + 10 unviable ; `loadtest.rs`
  94 = 1 + 30 + 60 + 3 ; `main.rs` 13 = 1 + 7 + **0 missed** + 5 (main
  entièrement couvert) ; `download.rs` 3/3 CaughtMutant.
- Les compteurs xtask bougent de quelques unités d'un run à l'autre
  (reclassements caught/unviable/missed des mêmes lignes ; les runs C4/DoD#5
  pré-round avaient compté 86 puis 77 missed) : seul le critère « 0 missed
  produit non justifié » est stable, c'est lui qui fait foi.

**Liste exacte des 62 survivants justifiés** (tous `xtask/`) — répartition
exhaustive par catégorie ci-dessous ; les positions ligne par ligne vivent dans
`target/verify/mutants.out/missed.txt` du run de stage :

| Catégorie | Mutants | Pourquoi aucun test rapide ne les tue | Risque résiduel |
|---|---|---|---|
| Call-site (4) | `verify.rs:77` `execute->0/1`, `loadtest.rs:127` `execute->0/1` | `execute` ne fait que déléguer : verify → `drive` (verdicts, notes et résumé épinglés octet par octet), loadtest → `run` (budgets end-to-end). Tuer la valeur de retour exigerait de muter le point d'entrée process lui-même. | nul — le délégué est intégralement épinglé |
| End-to-end, boucle d'événement (45) | `loadtest.rs` `run` : tally `+=/-=/*=`, comparaisons de budgets, `delete !` | colle de synchronisation exercée par le vrai serveur : sabotage 300 ms avant welcome → `hello_p95_ms` FAIL (~321-346 ms vs 42) ; sabotage stall → `welcomed_clients` FAIL (0 == 20). Un test unitaire du timing exigerait de réintégrer le harnais. | un budget inversé dans `run` ne serait vu qu'au prochain run de charge (nightly `load`) |
| End-to-end, ingest (6) | `loadtest.rs` `run_ingest`, dont `-> Default::default()` | compteurs contraints par les budgets réels (ingest envoyés/acceptés, `ingest_errors`, `ingest_rate_limited`) ; la première impulsion du ticker est ignorée par construction (le premier tick est consommé avant la boucle, `loadtest.rs:576`). | idem — visible au prochain nightly |
| End-to-end, readiness (6) | `loadtest.rs` `wait_healthy`, dont `-> Ok(())` | le contrat « le serveur répond /health » reste asserté par le budget `server_alive` ; la boucle de polling n'est que du timing, un kill ciblé exigerait un serveur volontairement malade, hors périmètre du harnais de charge. | une régression de la gate de readiness passerait verte tant que le serveur est sain |
| Process-entry (1) | `loadtest.rs:446` `run_client -> ()` | démarrer le client WS est la raison d'être du harnais ; couvert par les budgets `welcomed_clients`/`disconnects` sur le vrai serveur. | idem end-to-end |

- Seuls **4 fichiers du diff produisent des mutants** (loadtest 94, verify 39,
  main 13, download 3) sur les `.rs` du diff de branche : les fichiers de test
  ne sont pas mutés (`kill_recover.rs`, `contracts.rs`,
  `xtask/tests/cli_exit_codes.rs` → 0 mutant).
- Preuve produit : les 3 mutants du **retour de `build_ytdlp_args`**
  (`crates/lc-media/src/download.rs:413` : `vec![]`, `vec![String::new()]`,
  `vec!["xyzzy".into()]`) sont CaughtMutant — la fonction production modifiée par
  le pipeline (insertion de `--` avant l'URL) est exercée et contrainte.

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
  `xtask/src/loadtest.rs:627`, fmt/tests/mutants rouges au run suivant. Après toute
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
- Nits acceptées en revue B : TOCTOU de `reserve_port` (`loadtest.rs:637`, flake rare),
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
sans `!`, `probe_installed` déterministe, 2 survivants call-site) ; helpers
`loadtest.rs` dissous/épinglés (report_socket_end, budget_line,
read_fd_count, reserve_port, seed_database, ServerGuard) ; `main.rs`
entièrement couvert (0 missed) ; tests process-level
(`tests/cli_exit_codes.rs`) ; 62 survivants justifiés par catégorie
(§5) ; alias `cargo xtask` passé en release pour la collision
`xtask.exe` (§7).

## 9. Skill agent

Les agents suivent [`.claude/skills/verify/SKILL.md`](../../.claude/skills/verify/SKILL.md) :
spec → tests défaillants d'abord → implémentation → `verify --offline` → `--mutants`
kill-or-justify → sabotage → revue adversariale → canary avant tout déploiement.
