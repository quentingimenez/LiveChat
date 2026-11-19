# Instructions agents — LiveChat

Dépôt personnel de l'utilisateur (`quentingimenez/LiveChat`). Ces consignes
s'appliquent à tout agent qui travaille ici — session DSH, sous-agent, worktree —
pour toute correction, ajout ou modification demandée par l'utilisateur. Une
instruction explicite de la demande en cours l'emporte (voir « Cas particuliers »).

`main` n'a pas de protection GitHub : le respect de ce flux est la seule barrière
avant un merge direct.

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
3. **PR en fin de tâche**, vers `main` :

   ```bash
   git push -u origin <branche>
   gh pr create --base main --title "<titre conventional>" --body-file <fichier>
   ```

   Corps en français avec `## Problème`, `## Correctif`, `## Tests` (voir les PR
   précédentes). Ne pas ouvrir de PR brouillon sans demande : une PR signifie
   « prêt à être vérifié et mergé ».
4. **Vérifier la CI de la PR avant tout merge** :

   ```bash
   gh pr checks --watch
   ```

   `release-check` tourne sur toute PR ; `rust`, `extension` et `overlay-tauri`
   s'ajoutent selon les chemins touchés (`.github/workflows/`). Check rouge :
   corriger sur la même branche, pousser, revérifier — jamais merger rouge.
   CI bloquée par l'infrastructure (runner indisponible, file d'attente) :
   rapporter et attendre, ne pas merger.
5. **Merger soi-même** quand tout est vert :

   ```bash
   gh pr merge <numéro> --merge
   ```

   Merge commit, comme l'historique (pas de squash ni de rebase). Supprimer la
   branche ensuite (`--delete-branch` au merge, ou
   `git push origin --delete <branche>`). Jamais `--admin`, jamais de merge sans
   checks verts.
6. **Après le merge** : mettre `main` à jour en fast-forward, supprimer la
   branche locale, et rapporter à l'utilisateur : branche, URL de la PR, checks,
   commit de merge.

## Tests locaux avant d'ouvrir la PR

Lancer ce qui couvre le changement et le dire dans `## Tests` :

| Zone | Commande |
| --- | --- |
| Rust | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` |
| Rust ciblé (préféré) | `cargo test -p <crate>` ou `cargo nextest run -p <crate>` |
| Overlay (JS) | `node --test apps/lc-overlay/dev/tests/*.mjs` |
| Extension | `npm --prefix extension run ci:check` |
| Console admin | `npm test` dans `apps/lc-admin-web` |

Le poste est partagé : cibler les crates, éviter deux compilations globales en
parallèle, laisser les commandes globales au Lead quand plusieurs agents tournent.

## Cas particuliers

- **Release** : suivre `.claude/skills/release/SKILL.md` ; la PR `release/*` suit
  quand même ce flux, et le « go » de release reste celui de l'utilisateur.
- **Instruction contraire explicite** (« commit sur cette branche », « ouvre la
  PR mais ne merge pas », plan approuvé qui attend un go) : elle l'emporte pour
  cette tâche ; le défaut reste le flux complet.
- **PR d'un autre agent** : ne pas la merger sans demande, ne pas réécrire sa
  branche.
