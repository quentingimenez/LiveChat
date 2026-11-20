# Release semi-automatisée (serveur, overlay, extension) — conception

Date : 2026-11-17

## Intention

C'est Claude qui sort les versions de LiveChat. L'utilisateur ne fait qu'une chose :
donner son feu vert. Le système doit permettre à Claude de sortir une release
vite et sans oubli (version d'un fichier pas mise à jour, `docker start` oublié
après `update_container`, overlay marqué *Latest* avant le déploiement du serveur),
et de **choisir lui-même le chiffre à incrémenter** de chaque composant.

## Objectifs

- Une demande (« fais une release », « sors le serveur ») suffit à lancer le processus.
- Claude détecte les composants qui ont changé depuis leur dernier tag et propose,
  pour chacun, la nouvelle version avec une raison d'une ligne.
- Un seul point de validation : l'utilisateur répond « go » à la proposition
  (ou corrige un numéro). Ce « go » vaut accord pour committer, pousser, merger,
  publier et déployer **cette** release.
- Le passage de version touche tous les fichiers d'un composant d'un coup ; la CI
  refuse une PR où ils divergent.
- Au merge sur `main`, les tags et les builds partent tout seuls.
- Claude finit : déploiement Unraid du serveur (sauvegarde DB incluse), vérifications,
  puis publication de l'overlay en *Latest*.

## Non-objectifs

- Pas de publication sur le Chrome Web Store : l'extension n'y est plus pour
  l'instant. Une release d'extension = une release GitHub avec le zip.
- Pas de déploiement Unraid depuis GitHub (pas de runner auto-hébergé : le serveur
  n'est joignable que depuis le réseau local de l'utilisateur).
- Pas de changelog versionné dans le dépôt : les notes vivent dans la PR et les
  releases GitHub.
- Pas de convention de messages de commit imposée (les commits restent en phrases
  libres à l'impératif) : la décision de version est prise par Claude en lisant
  les changements, pas par un outil.

## Anti-objectifs

- Une release qui part sans le « go » explicite de l'utilisateur.
- Un overlay marqué *Latest* alors que le serveur déployé ne le supporte pas.
- Un token personnel (PAT) ou un secret de plus à créer pour que ça marche.
- Un tag posé sur un commit qui n'est pas sur `main`.

## Contraintes

- Windows PowerShell 5.1 en local (pas de `&&`, `??`, `?:`). Les scripts locaux
  sont en PowerShell, compatibles 5.1.
- Fichiers de version par composant :
  - serveur : `[workspace.package] version` de `Cargo.toml` (+ `Cargo.lock`, et
    `fuzz/Cargo.lock` : son workspace à part répète la version des crates liées
    par les cibles de fuzzing, testées en `--locked` par la nightly) ;
    tag `server-vX.Y.Z` ; image `ghcr.io/quentingimenez/livechat-server:X.Y.Z`.
  - overlay : `apps/lc-overlay/tauri.conf.json` `version` et
    `apps/lc-overlay/Cargo.toml` `version` (+ `Cargo.lock`) ; tag `overlay-vX.Y.Z`.
  - extension : `extension/public/manifest.json` et `extension/package.json`
    (+ `extension/package-lock.json`) ; tag `extension-vX.Y.Z`.
  - `tauri.beta.conf.json` (canal bêta `LiveChat Next`) n'est pas concerné.
- Un tag poussé avec le `GITHUB_TOKEN` ne déclenche aucun workflow, **sauf**
  `workflow_dispatch` / `repository_dispatch`. Le workflow de release lance donc
  lui-même les builds par `workflow_dispatch` sur la ref du tag.
- `server-image.yml` exige l'entrée `version` en `workflow_dispatch` ; il faut aussi
  lui passer `update_latest=true` pour une release.
- Règles du dépôt : branche dédiée depuis `main` (worktree si l'arbre a des
  modifications), messages de commit en anglais à l'impératif, pas de
  `Co-Authored-By`, pas de mention Claude dans les PR.
- Déploiement Unraid : procédure de la mémoire `unraid-deployment-livechat`
  (hôte SSH lu dans `LIVECHAT_SSH_HOST` et URL publique dans `LIVECHAT_PUBLIC_URL`
  — le dépôt public ne versionne pas l'infrastructure ; sauvegarde avant,
  `update_container LiveChat` puis `docker start LiveChat`, secrets jamais
  affichés).

## Approches envisagées

1. **Skill `/release` + script local** — Claude fait tout depuis la machine de
   l'utilisateur, y compris tags et déploiement.
2. **Tout en GitHub Actions** — workflow « Release » qui change les versions, ouvre
   la PR, tague au merge et publie ; le déploiement Unraid demanderait un runner
   auto-hébergé.
3. **Skill seul, sans script** — procédure écrite, commandes à la main.

**Choix de l'utilisateur : 1 + 2 combinés.** Le local fait ce que seul le local peut
faire (décider la version, préparer la PR, déployer sur Unraid) ; GitHub fait ce
qui doit être garanti à chaque fois (contrôle de cohérence, tags sur le bon commit,
lancement des builds). L'approche 3 est écartée : elle laisse les modifications de
version et le déploiement à la main, là où naissent les oublis.

## Conception

### Déroulé d'une release

1. **Plan** — `scripts/release.ps1 plan` liste, pour chaque composant, le dernier
   tag, la version actuelle et les commits/fichiers qui le touchent depuis ce tag.
   Claude lit ces changements (diffs inclus si besoin) et choisit le niveau.
2. **Proposition** — Claude montre à l'utilisateur, par composant : version
   actuelle → nouvelle, niveau, raison d'une ligne, et le résumé des changements.
   Les composants sans changement ne sont pas proposés. Il attend « go ».
3. **Préparation** — branche `release/<résumé>` depuis `main` (worktree si besoin),
   `release.ps1 bump <composant> <version>` pour chaque composant, `release.ps1 check`,
   tests du projet, commit `Release server 3.6.0 and overlay 3.6.0`, push, PR avec
   les notes de version.
4. **Merge** — dès que la CI de la PR est verte, Claude merge (couvert par le « go »).
5. **Tags + builds (GitHub)** — `release.yml` voit les versions changées, pose les
   tags sur le commit de merge et lance les builds.
6. **Fin (Claude)** — attend les builds ; serveur : `release.ps1 deploy-server <version>`,
   vérifications ; puis overlay : publication du brouillon en *Latest*. Extension :
   rien à faire (release GitHub créée par la CI). Compte rendu à l'utilisateur.

### Règles de choix de version (appliquées par Claude, écrites dans le skill)

Par composant, sur ses propres changements depuis son dernier tag :

- **patch** : uniquement des corrections ou du réusinage, rien de nouveau visible
  par l'utilisateur, aucune migration de base.
- **minor** : nouvelle fonction, commande Discord, réglage, route API, migration de
  base additive, ou changement visible de l'interface.
- **major** : casse les clients installés (relèvement de `MIN_OVERLAY_VERSION`,
  protocole incompatible), retrait d'une fonction utilisée, ou refonte.
- Le niveau le plus haut parmi les changements l'emporte.
- L'extension reste en 0.x (prochaine release 0.2.0) ; en 0.x, un changement
  cassant donne un minor.
- Si un changement de protocole impose de relever `MIN_OVERLAY_VERSION`, Claude le
  signale dans la proposition (serveur ET overlay concernés, ordre de déploiement).

### Script local `scripts/release.ps1`

Sous-commandes :

- `plan [-Json]` — par composant : dernier tag (`git describe`/`git tag --list`
  trié semver), version dans les fichiers, commits depuis le tag filtrés par
  chemins, fichiers changés. Ne modifie rien.
- `bump <server|overlay|extension> <X.Y.Z>` — écrit la version dans tous les
  fichiers du composant en préservant leur format (remplacement ciblé, pas de
  réécriture JSON complète), met à jour les lockfiles (`cargo update -w --offline`
  ou équivalent ciblé ; `npm install --package-lock-only` pour l'extension),
  refuse une version qui n'est pas strictement supérieure au dernier tag.
- `check` — vérifie que chaque composant a une seule version dans tous ses
  fichiers ; code de sortie ≠ 0 sinon. Utilisé aussi par la CI.
- `deploy-server <X.Y.Z>` — via SSH : pull de l'image, arrêt propre, sauvegarde
  `livechat.sqlite*` + template dans `livechat-backups/before-X.Y.Z-<date>/` avec
  `cmp`, édition de `<Repository>`, `update_container LiveChat`, `docker start
  LiveChat`, puis attente de `/health` (version = X.Y.Z, db ok, discord ok),
  `/admin/` public = 403, logs « Discord prêt ». S'arrête à la première erreur et
  affiche la commande de retour arrière (ancien tag dans le template + restauration
  de la sauvegarde).

Chemins par composant (pour `plan`) :

- serveur : `crates/**` (sauf ce qui ne sert qu'à l'overlay), `apps/lc-server/**`
  ou équivalent réel, `deploy/**`, `Cargo.toml`, `Cargo.lock`.
- overlay : `apps/lc-overlay/**`, `crates/lc-protocol/**`, `crates/lc-domain/**`.
- extension : `extension/**`.
- (Les chemins exacts sont à relever dans le dépôt au moment du plan d'implémentation.)

### Skill `.claude/skills/release/SKILL.md`

Procédure ci-dessus, règles de version, points d'arrêt (seul arrêt : le « go »),
quoi faire si une étape échoue (CI rouge → corriger sur la branche de release ;
build rouge après tag → ne pas déployer, ne pas publier, prévenir), et rappel des
vérifications post-déploiement.

### GitHub Actions

- **Contrôle sur PR** : un job (dans `rust.yml` ou un petit `release-check.yml`)
  lance la vérification de cohérence des versions. En CI Linux, il exécute la même
  logique que `release.ps1 check` (PowerShell 7 `pwsh` est disponible sur
  `ubuntu-latest`, donc le script doit aussi tourner sous `pwsh`).
- **`release.yml`** : sur `push` vers `main`. Il n'agit **que** si le commit poussé
  est le merge d'une PR dont la branche source commence par `release/` (vérifié par
  `gh api repos/{repo}/commits/{sha}/pulls`) ; tout autre push ne tague rien, même
  si une version a changé à la main. Pour chaque composant dont la version a changé
  dans ce merge (comparaison avec le premier parent) et dont le tag n'existe pas
  encore, il crée le tag (annoté) sur `github.sha`, le pousse, puis lance le build par
  `gh workflow run <workflow> --ref <tag>` :
  - serveur : `server-image.yml` avec `-f version=X.Y.Z -f update_latest=true` ;
  - overlay : `overlay-tauri.yml` ;
  - extension : `extension.yml`.
  Permissions : `contents: write`, `actions: write`. Rien à faire si aucune
  version n'a changé.
- **Ajustements des workflows existants** :
  - `extension.yml` : supprimer le job `publish-webstore` (l'extension n'est plus
    sur le store) ; ajouter le contrôle tag = version du manifeste sur
    `extension-v*`.
  - `server-image.yml` / `overlay-tauri.yml` : vérifier qu'un `workflow_dispatch`
    sur une ref de tag passe bien les contrôles existants (`github.ref` commence par
    `refs/tags/...`), et corriger si besoin.

### Ordre de fin de release

Serveur déployé et vérifié **avant** la publication de l'overlay en *Latest*. Si
le déploiement échoue, l'overlay reste en brouillon et Claude prévient.

Release cassante (relèvement de la version minimale d'overlay) : le minimum est
relevé **dans le code** (`DEFAULT_MIN_OVERLAY_VERSION`, `crates/lc-protocol/src/lib.rs`),
donc livré avec l'image serveur, pas par variable du template. Claude publie
l'overlay en *Latest* immédiatement après la vérification du serveur (fenêtre de
quelques minutes où les anciens overlays sont refusés ; l'updater de l'overlay ne
dépend pas du WebSocket, il propose donc la nouvelle version dès sa publication).
La proposition à l'utilisateur mentionne cette coupure.

### Couplage serveur / overlay

`crates/lc-protocol` et `crates/lc-domain` servent aux deux. Si `plan` voit un
changement dans ces crates depuis le dernier tag serveur, il le signale et Claude
propose une release serveur en même temps que l'overlay. Avant de publier un
overlay en *Latest*, Claude vérifie aussi `/health` du serveur déployé.

### Extension : point de départ

Aucun tag `extension-v*` n'existe (seulement la pré-release `extension-latest`).
« Pas de tag » = jamais publiée : `plan` liste tout l'historique de `extension/`,
`bump` exige seulement une version > version des fichiers (0.1.0). La première
release sera 0.2.0 ; 0.1.0 n'est jamais taguée.

## Laissé au constructeur

- Structure interne du script (fonctions, module partagé avec la CI).
- Forme exacte de la sortie de `plan` (texte lisible + `-Json` pour Claude).
- Où placer le contrôle de cohérence dans la CI.
- Méthode de mise à jour ciblée des lockfiles.
- Tests du script (Pester ou tests PowerShell simples sur des copies temporaires
  des fichiers de version).

## Décisions prises par Claude (à vérifier)

- Extension en 0.x, prochaine release 0.2.0.
- Le merge de la PR de release est fait par Claude après CI verte, couvert par le « go ».
- Suppression du job Chrome Web Store plutôt que sa désactivation par variable.
- Nom de branche `release/<résumé>`, message de commit `Release <composants>`.
- `release.yml` ne tague que les merges de branches `release/*` (un changement de
  version à la main ailleurs ne publie rien).
- Version minimale d'overlay relevée dans le code, overlay publié juste après le
  serveur en cas de release cassante.
- Changement des crates partagées → release serveur proposée avec l'overlay.
- Pas de tag `extension-v0.1.0` rétroactif.
- Chemins serveur : inclure aussi `apps/lc-admin-web` et `xtask`.
