---
name: release
description: Use when the user asks to release, ship or publish a new version of the LiveChat server, overlay or extension ("fais une release", "sors le serveur", "nouvelle version de l'overlay"). Picks the version numbers, prepares the release PR, then tags, builds, deploys to Unraid and publishes after one "go".
---

# Release LiveChat (serveur, overlay, extension)

Claude sort les versions ; l'utilisateur ne donne qu'**un seul feu vert**. Conception :
`docs/superpowers/specs/2026-11-17-release-automation-design.md`.

Outil : `scripts/release.ps1` (Windows PowerShell 5.1 ou pwsh 7), depuis la racine
du dépôt :

```powershell
powershell -NoProfile -File scripts/release.ps1 plan            # changements depuis le dernier tag
powershell -NoProfile -File scripts/release.ps1 bump overlay 3.6.0
powershell -NoProfile -File scripts/release.ps1 check           # une seule version par composant
powershell -NoProfile -File scripts/release.ps1 deploy-server 3.6.0
```

## 0. Configuration du déploiement

Le dépôt est public : l'hôte SSH et l'URL publique ne sont **pas** versionnés (les
valeurs du code sont des placeholders `192.0.2.59` / `livechat.example.com`).
`deploy-server` les lit dans l'environnement et refuse de tourner s'ils manquent :

| Variable | Rôle |
| --- | --- |
| `LIVECHAT_SSH_HOST` | hôte SSH du serveur Unraid, ex. `root@unraid.lan` (requis) |
| `LIVECHAT_PUBLIC_URL` | URL publique vérifiée après déploiement, ex. `https://livechat.example.com` (requis) |
| `LIVECHAT_SSH_EXE` | chemin de `ssh.exe` (facultatif) |
| `LIVECHAT_IMAGE_NAME` | image serveur (facultatif, pour un fork) |

Les définir une fois dans un fichier `.env` à la racine du dépôt (ignoré par
git, jamais versionné, voir `.env.example`) :

```
LIVECHAT_SSH_HOST=root@unraid.lan
LIVECHAT_PUBLIC_URL=https://livechat.example.com
```

`scripts/release.ps1` charge ce fichier au démarrage (`Import-DotEnv`) : les
commandes `plan`, `check` et `deploy-server` fonctionnent ensuite sans rien
exporter. Une variable déjà présente dans l'environnement garde la priorité,
pour une surcharge ponctuelle :

```powershell
$env:LIVECHAT_SSH_HOST = 'root@unraid.lan'
$env:LIVECHAT_PUBLIC_URL = 'https://livechat.example.com'
powershell -NoProfile -File scripts/release.ps1 deploy-server 3.6.0
```

Ne jamais committer ces valeurs ni les afficher (hôte = information
d'infrastructure, et le template du conteneur contient des secrets).

## 1. Plan

1. `git fetch --tags origin`, puis `release.ps1 plan` sur `origin/main` à jour
   (dans un worktree propre sur `origin/main` si l'arbre courant est sur une autre
   branche ou a des modifications).
2. Lire les commits listés pour chaque composant, et les diffs quand le titre ne
   suffit pas à classer le changement. Ignorer un commit qui ne fait que changer
   la version d'un autre composant.
3. Composant sans commit depuis son dernier tag : pas de release.

## 2. Choisir le numéro

Par composant, sur ses propres changements depuis son dernier tag ; le niveau le
plus haut l'emporte :

- **patch** : uniquement des corrections ou du réusinage, rien de nouveau visible
  par l'utilisateur, aucune migration de base.
- **minor** : nouvelle fonction, commande Discord, réglage, route API, migration de
  base additive, ou changement visible de l'interface.
- **major** : casse les clients installés (relèvement de `DEFAULT_MIN_OVERLAY_VERSION`
  dans `crates/lc-protocol/src/lib.rs`, protocole incompatible), retrait d'une
  fonction utilisée, ou refonte.
- Extension : reste en 0.x (première release : 0.2.0) ; en 0.x un changement
  cassant donne un minor.
- `plan` signale « shared crates changed since the last server release » : les
  crates `lc-protocol` / `lc-domain` ont changé, proposer **aussi** une release
  serveur.
- Release cassante : la version minimale est relevée dans le code (livrée avec
  l'image), et la proposition annonce la coupure de quelques minutes pour les
  anciens overlays (entre le déploiement du serveur et la publication de l'overlay).

## 3. Proposer et attendre le « go »

Message à l'utilisateur, une ligne par composant :

```
serveur   3.5.1 → 3.6.0 (minor) — bannissement de membres, envoi en morceaux
overlay   3.5.0 → 3.6.0 (minor) — vitesse d'édition, vignettes, sélection multiple
extension 0.1.0 → 0.2.0 (minor) — durée d'affichage des images
```

puis le résumé des changements. **Attendre « go »** (ou un numéro corrigé). Ce
« go » vaut accord pour committer, pousser, merger, taguer, publier et déployer
**cette** release, et rien d'autre.

## 4. Préparer la PR de release

1. Branche `release/<résumé>` (ex. `release/server-3.6.0-overlay-3.6.0`) depuis
   `origin/main`, dans un worktree si l'arbre courant a des modifications (jamais de
   lien vers `node_modules` du dépôt principal).
2. `release.ps1 bump <composant> <version>` pour chaque composant, puis `check`.
3. Tests : `cargo test --workspace --offline` si serveur ou overlay ; dans
   `extension/`, `npm ci` puis `npm run ci:check` si extension.
4. Commit `Release server 3.6.0 and overlay 3.6.0` (anglais, impératif, sans
   `Co-Authored-By`), push, PR vers `main` avec les notes de version (par
   composant : numéro, niveau, raison, changements), sans mention de Claude.
5. Attendre la CI de la PR (`gh pr checks <n> --watch`). Rouge : corriger sur la
   branche de release. Vert : `gh pr merge <n> --merge` (sans `--delete-branch` :
   lancé depuis un worktree, il tente de basculer sur `main` et peut renvoyer une
   erreur alors que le merge a réussi ; vérifier avec `gh pr view <n> --json state`),
   puis `git push origin --delete release/<résumé>` et retirer le worktree.

## 5. Tags et builds (GitHub)

Au merge, `release.yml` pose les tags `<composant>-vX.Y.Z` sur le commit de merge
et lance `server-image.yml`, `overlay-tauri.yml`, `extension.yml` sur ces tags.
Suivre : `gh run list --workflow release.yml -L 1`, puis vérifier dans son log
(`gh run view <id> --log`) une ligne `<tag> pushed, build started` par composant :
un run vert qui dit « not a release merge » ou « nothing tagged » n'a rien fait.
Alors relancer le job (`gh run rerun <id>`) ; il pose les tags manquants et relance
les builds des tags déjà posés sur ce commit (même reprise si un lancement de build
a échoué). Ensuite `gh run list --workflow <build>.yml -L 3` et
`gh run watch <id> --exit-status`.
Un build rouge : **ni déploiement ni publication**, prévenir l'utilisateur avec
l'erreur.

## 6. Finir (dans cet ordre)

1. **Serveur** (si publié) : définir `LIVECHAT_SSH_HOST` et `LIVECHAT_PUBLIC_URL`
   dans l'environnement (section 0), puis `release.ps1 deploy-server X.Y.Z`. Le script
   télécharge l'image, arrête proprement le conteneur, sauvegarde la base et le
   template dans `/mnt/user/appdata/livechat-backups/before-X.Y.Z-<date>/` (vérifiés
   avec `cmp`), change l'image du template, lance `update_container` puis
   `docker start`, attend que `/health` annonce X.Y.Z avec db et discord `ok`,
   vérifie « Discord prêt » dans les logs et le 403 de `/admin/` depuis une adresse
   non autorisée. En cas d'échec pendant le script hôte, il remet le template
   sauvegardé, recrée et relance le conteneur ; en cas d'échec des vérifications
   (nouvelle image en mauvaise santé), il affiche l'image précédente, le dossier de
   sauvegarde et la commande `ROLLBACK` (template + base) à lancer sur l'hôte. La
   rapporter telle quelle à l'utilisateur, ne pas la lancer sans son accord (elle
   remet une base plus ancienne), et **ne pas publier l'overlay**.
2. **Notes de version** (overlay et extension publiés) : la CI ne met qu'un texte
   fixe (installation). Ajouter **devant**, en français, un bloc
   `Changements depuis <version précédente> :` puis des sections (`Board`,
   `Overlay`, `Serveur` pour l'overlay ; `Extension`, et `Serveur` si utile, pour
   l'extension), une ligne par changement visible, rédigée pour l'utilisateur à
   partir des commits et diffs lus à l'étape 1 (pas les titres de commit
   anglais). Pour l'overlay, finir par la compatibilité (« Les overlays X.Y.Z
   continuent de fonctionner avec un serveur A.B.C. » ou la coupure annoncée).
   Écrire le corps complet (bloc + texte fixe existant) dans un fichier, puis
   `gh release edit <tag> --notes-file <fichier>`. Modèle : `overlay-v3.5.0`.
3. **Overlay** (si publié), seulement après un serveur vérifié (ou sans release
   serveur : vérifier `https://livechat.example.com/health`) :
   `gh release edit overlay-vX.Y.Z --draft=false --latest`. C'est ce qui déclenche
   la mise à jour de tous les overlays installés.
4. **Extension** : la release GitHub avec le zip est créée par la CI (notes
   ajoutées au point 2). Pas de Chrome Web Store pour l'instant. Ne jamais la
   marquer *Latest*.
5. Compte rendu : versions sorties, liens des releases, image déployée, dossier de
   sauvegarde, ce qui n'a pas pu être vérifié.

## Interdits

- Rien ne part sans le « go » de l'utilisateur pour cette release.
- Jamais d'overlay *Latest* avant la vérification du serveur.
- Jamais de bêta ni d'extension marquée *Latest*.
- Jamais afficher les variables d'environnement du conteneur (secrets).
