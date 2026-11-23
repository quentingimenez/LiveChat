# Instructions agents — LiveChat

Dépôt personnel de l'utilisateur (`quentingimenez/LiveChat`). Ces consignes
s'appliquent à tout agent qui travaille ici — session DSH, sous-agent, worktree —
pour toute correction, ajout ou modification demandée par l'utilisateur. Une
instruction explicite de la demande en cours l'emporte (voir « Cas particuliers »).

`main` est protégée par GitHub : `ci-gate` et `versions & release tooling` sont
requis, en mode strict. Le respect de ce flux reste la barrière avant un merge
direct.

## Règle par défaut : branche → PR → CI verte → merge

Aucune tâche ne se termine par un commit ou un merge direct sur `main`. Le flux
attendu :

1. **Branche dédiée depuis `origin/main` à jour.**
   `git fetch origin` puis `git switch -c <préfixe>/<slug>`.
   Préfixes de l'historique : `feat/`, `fix/`, `docs/`, `chore/`, `test/`,
   `release/<composant>-<version>`.
   Une branche par demande utilisateur (pas une par sous-tâche) ; les sous-agents
   qui contribuent à la même demande travaillent sur la même branche. Si une PR
   est déjà ouverte pour cette demande, continuer sa branche au lieu d'en créer
   une autre.
2. **Commits conventionnels en anglais** : `type(scope): sujet impératif en
   minuscules`, une ligne, sans `Co-Authored-By` (commitlint, `.husky/commit-msg`).
   Committer seulement les changements de la tâche ; ne jamais committer sur
   `main`, ne jamais forcer une branche partagée.
3. **Montrer le résultat et attendre la validation, avant toute PR.** Quand le
   code et les tests locaux sont prêts, présenter le résultat dans la session :
   fichiers touchés et diff (ou ses points clés), tests ajoutés, commandes
   lancées avec leur résultat, et pour un changement visuel une capture
   avant/après. La PR n'est ouverte qu'après le « go » explicite de
   l'utilisateur ; les corrections demandées se font sur la même branche, puis
   le résultat est représenté. **Aucune PR ne s'ouvre sans cette validation.**
4. **PR**, vers `main` :

   ```bash
   git push -u origin <branche>
   gh pr create --base main --title "<titre conventional>" --body-file <fichier>
   ```

   Corps en français avec `## Problème`, `## Correctif`, `## Tests`, reprenant
   ce qui a été présenté et validé (voir les PR précédentes). Ne pas ouvrir de
   PR brouillon sans demande : une PR signifie « prêt à être vérifié et mergé ».
5. **Vérifier la CI de la PR avant tout merge** :

   ```bash
   gh pr checks --watch
   ```

   `release-check` tourne sur toute PR ; `rust`, `extension` et `overlay-tauri`
   s'ajoutent selon les chemins touchés (`.github/workflows/`). Check rouge :
   corriger sur la même branche, pousser, revérifier — jamais merger rouge.
   CI bloquée par l'infrastructure (runner indisponible, file d'attente) :
   rapporter et attendre, ne pas merger.
6. **Merger soi-même** quand tout est vert :

   ```bash
   gh pr merge <numéro> --merge
   ```

   Merge commit, comme l'historique (pas de squash ni de rebase). Le dépôt a
   l'option GitHub « Automatically delete head branches » activée
   (`delete_branch_on_merge: true`) : GitHub supprime la branche distante au
   merge, inutile de passer `--delete-branch` ou de lancer
   `git push origin --delete <branche>`. Jamais `--admin`, jamais de merge sans
   checks verts.
7. **Après le merge** : mettre `main` à jour en fast-forward, supprimer la
   branche locale, vérifier que les fenêtres de navigateur ouvertes par l'agent
   sont bien fermées (elles le sont normalement dès le « go » donné et la PR
   créée — voir « Fermer les fenêtres ouvertes avec browse »), et rapporter à
   l'utilisateur : branche, URL de la PR, checks, commit de merge.

## Fermer les fenêtres ouvertes avec browse

Les agents ouvrent le navigateur (OpenCode Browse — outil `browse`, sur Brave)
pour vérifier un changement : captures avant/après, parcours de la board, de la
console admin ou de la boutique. Ces onglets ne sont pas ceux de l'utilisateur :
ils ne doivent pas lui rester ouverts indéfiniment, mais ils restent ouverts le
temps que l'utilisateur vérifie lui-même la page.

- Quand l'agent présente le résultat et attend la validation avant la PR
  (étape 3 du flux), il **garde ouverts** les onglets et fenêtres ouverts pour
  la vérification et donne leurs URL dans le message de présentation :
  l'utilisateur regarde la page de ses propres yeux avant de donner son « go ».
- Il ne les ferme qu'**après le « go » explicite** de l'utilisateur pour ouvrir
  la PR, une fois la PR créée — et, de fait, après le merge — avec `tabs` action
  `close` ou `page.close()`, y compris ceux ouverts par un sous-agent qu'il a
  lancé. Il ne ferme que ses propres onglets : jamais ceux que l'utilisateur a
  ouverts ou prêtés.
- Corrections demandées après la vérification : l'agent corrige sur la même
  branche, rouvre ce qu'il faut, représente le résultat et laisse de nouveau
  les onglets ouverts jusqu'au « go » suivant.
- Tâche sans PR à venir (diagnostic, investigation) : les onglets sont fermés
  avant de rendre la main, comme avant.
- Si la fermeture est impossible (navigateur déconnecté, extension absente), le
  signaler dans le rapport final plutôt que de laisser la fenêtre ouverte.

## Tests obligatoires avec chaque correctif ou fonctionnalité

Aucun `fix` ni `feat` ne part en PR sans les tests qui le prouvent : le test
fait partie du diff et de la même PR — jamais une PR de suivi, jamais « plus
tard ». La logique est celle du skill `verify`
([`.claude/skills/verify/SKILL.md`](.claude/skills/verify/SKILL.md)) : **le test
d'abord**, il échoue sur l'ancien code, il passe avec le changement.

- **Correctif (`fix`)** : un test qui reproduit le bug tel qu'il se produisait
  avant le correctif, puis passe avec. Sans reproduction testée, le bug n'est
  pas réputé corrigé.
- **Fonctionnalité (`feat`)** : le comportement nominal, au moins un cas limite
  (entrée vide, valeur inconnue, échec réseau, concurrence…) et le cas où la
  nouveauté ne doit rien changer.
- **Vérifier que le test mord** : le lancer contre le code d'avant — ou casser
  volontairement l'invariant produit comme dans le skill `verify` — et le voir
  échouer avec un message exploitable. Un test qui ne peut pas échouer n'est
  pas un test.

### Où vit le test (suivre l'existant, ne pas inventer un harnais)

| Zone | Emplacement et style |
| --- | --- |
| Rust, logique privée | `#[cfg(test)] mod tests` en fin de module |
| Rust, comportement de crate | `crates/<crate>/tests/<sujet>.rs`, nommé par comportement (`invariants.rs`, `properties.rs`) |
| Rust, invariants et bornes | proptest dans `tests/properties.rs`, littéraux frontière épinglés (pas de kill probabiliste) |
| Overlay (JS) | `apps/lc-overlay/dev/tests/<feature>.test.mjs` : `node:test` + `node:assert/strict`, en-tête disant ce que le fichier protège, invariants plutôt que valeurs recopiées |
| Extension | `extension/src/**/<module>.test.ts` à côté du module, vitest (`describe`/`it` en français) |
| Console admin | `apps/lc-admin-web/src/**/<module>.test.ts(x)`, vitest + `@testing-library/preact` |
| Outils et release | `scripts/release/tests/*.Tests.ps1`, exécutés par `run-tests.ps1` |

Pour un changement Rust de fond, la batterie complète reste le gate :
`cargo xtask verify --offline` (propriétés, mutation kill-or-justify, voir
[`docs/rust/verification.md`](docs/rust/verification.md)).

### Interdits

- Affaiblir, `skip`/`ignore` ou supprimer un test existant pour faire passer le
  code ; un test désactivé exige une justification `fichier:ligne` dans la PR.
- Assertions recopiées de l'implémentation (le test refait le calcul), tests qui
  ne peuvent pas échouer, harnais testé à la place du produit.
- Un test ajouté que la CI ne lance pas sans le dire. Les jobs de
  `.github/workflows/` : overlay JS = `tests JS de l'overlay`
  (`overlay-tauri.yml`), console admin = `admin-web` (`rust.yml`), extension =
  workflow `extension` (`extension.yml`). Un test hors de ces jobs se lance à la
  main et se mentionne comme tel dans la PR.

### Dans la PR

`## Tests` liste les tests ajoutés (fichier + nom) et les commandes lancées avec
leur résultat ; aucun `## Tests` vide. Changement réellement non testable (doc,
commentaire, config de CI) : l'écrire (`Non applicable : …`) au lieu de laisser
croire qu'il est couvert.

## Limites connues : corriger, pas accepter

Une limite **connue et corrigeable** n'est pas une option à présenter ou à merger.
Avant de rendre une tâche ou d'ouvrir une PR :

- si le défaut est un test manquant, une couverture incomplète, une CI rouge
  volontaire, une baseline non mesurée, une duplication de logique, un P3
  fixable ou tout autre écart réparable, **le corriger dans la même tâche** ;
- si la correction est indépendante ou demande un autre périmètre, **lancer un
  agent dédié immédiatement**, sur la même branche ou un worktree dédié, sans
  attendre un lot suivant ;
- ne pas présenter une PR en disant « P3 accepté » quand le correctif est
  faisable ; une limite acceptée doit être une **limitation technique ou de
  configuration réelle** (OS, matériel, API externe, choix produit explicite),
  documentée avec sa preuve, son impact et son suivi ;
- une nightly rouge « par conception », un score non mesuré ou une couverture
  non exercée sont des défauts, pas des limites acceptables.

## Tests locaux avant d'ouvrir la PR

Lancer ce qui couvre le changement et le dire dans `## Tests` :

| Zone | Commande |
| --- | --- |
| Rust | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` |
| Rust ciblé (préféré) | `cargo test -p <crate>` ou `cargo nextest run -p <crate>` |
| Overlay (JS) | `node --test apps/lc-overlay/dev/tests/*.test.mjs` (la commande du job `tests JS de l'overlay`) |
| Extension | `npm --prefix extension run ci:check` |
| Console admin | `npm test` dans `apps/lc-admin-web` |

Le poste est partagé : cibler les crates, éviter deux compilations globales en
parallèle, laisser les commandes globales au Lead quand plusieurs agents tournent.

## Cas particuliers

- **Release** : suivre `.claude/skills/release/SKILL.md` ; la PR `release/*` suit
  quand même ce flux, et le « go » de release reste celui de l'utilisateur.
- **Instruction contraire explicite** (« commit sur cette branche », « ouvre la
  PR sans attendre ma validation », « ouvre la PR mais ne merge pas », plan
  approuvé qui attend un go) : elle l'emporte pour cette tâche ; le défaut reste
  le flux complet.
- **PR d'un autre agent** : ne pas la merger sans demande, ne pas réécrire sa
  branche.
