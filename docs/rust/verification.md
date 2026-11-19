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
**kill-or-justify** (chaque mutant manqué tué par un test, ou justifié équivalent).

- Mesures (HEAD post-rebase) : **139 outcomes = 1 baseline + 27-28 caught +
  77-82 missed + 28-34 unviable** (deux runs : DoD#5 et batterie finale T10) ; missed
  **tous dans xtask** (`loadtest.rs` 63-68, `verify.rs` 12, `main.rs` 2) —
  **0 manqué non justifié en `crates/` et `apps/`** dans le stage local (diff de
  branche) : le seul manqué produit connu, `playback_jobs.rs:211` (justifié, §6),
  vit dans le scope nightly `lc-db-queue`, hors diff local — critère décisif,
  respecté dans tous les runs de la branche.
- Les compteurs xtask bougent de quelques unités d'un run à l'autre (reclassements
  caught/unviable/missed des **mêmes 138 mutants** ; le run C4 pré-rebase avait
  compté 86 missed) : seul le critère « 0 missed produit non justifié » est stable,
  c'est lui qui fait foi.
- Justification des missed xtask : ~60 sur le **chemin d'exécution** (run, wait_healthy,
  run_ingest, run_client, shutdown/Drop) qu'aucun test unitaire n'exerce sans serveur
  réel ; ~8-11 **helpers purs** selon le run, testables mais non encore contraints
  (`loadtest.rs:533` report_socket_end, `:616` budget, `:833` ServerGuard::pid,
  `:637` reserve_port, `:646-726` seed_database, `:820` count_server_fds,
  `main.rs:341` default_server_bin). Piste optionnelle : des tests unitaires sur ces
  helpers tueraient ~8-10 mutants.
- Seuls **4 fichiers du diff produisent des mutants** (loadtest 92, verify 34, main 9,
  download 3) sur les 15 fichiers `.rs` du diff de branche `1a787ac..HEAD` : les
  fichiers de test ne sont pas mutés (`kill_recover.rs`, `contracts.rs` → 0 mutant).
- Preuve produit : les 3 mutants du **retour de `build_ytdlp_args`**
  (`crates/lc-media/src/download.rs:413` : `vec![]`, `vec![String::new()]`,
  `vec!["xyzzy".into()]`) sont CaughtMutant — la fonction production modifiée par le pipeline
  (insertion de `--` avant l'URL) est exercée et contrainte.

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

## 9. Skill agent

Les agents suivent [`.claude/skills/verify/SKILL.md`](../../.claude/skills/verify/SKILL.md) :
spec → tests défaillants d'abord → implémentation → `verify --offline` → `--mutants`
kill-or-justify → sabotage → revue adversariale → canary avant tout déploiement.
