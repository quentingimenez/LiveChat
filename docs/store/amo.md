# Firefox — publication sur addons.mozilla.org (AMO)

L'extension est publiée comme add-on **listé** sur
[addons.mozilla.org](https://addons.mozilla.org/firefox/) : la fiche est publique,
les utilisateurs l'installent depuis AMO, et Firefox la met à jour automatiquement
à chaque version approuvée. Il n'y a **pas** d'`update_url` : AMO l'interdit pour
un add-on hébergé par Mozilla, et le manifeste Firefox ne doit pas en contenir.

## Pré-requis

- Un compte AMO avec la double authentification activée.
- L'identifiant `livechat-extension@livechat` est réservé une fois pour toutes :
  la première soumission crée l'add-on, les suivantes ajoutent une version au
  même add-on grâce à `browser_specific_settings.gecko.id`.
- Les secrets GitHub `AMO_JWT_ISSUER` et `AMO_JWT_SECRET`, générés sur
  <https://addons.mozilla.org/developers/addon/api/key/> (clé JWT, pas le mot de
  passe du compte).

## Ce que contient le manifeste Firefox

`extension/manifests/firefox.json` ajoute au manifeste commun :

```json
"background": { "scripts": ["assets/background.js"], "type": "module" },
"browser_specific_settings": {
  "gecko": {
    "id": "livechat-extension@livechat",
    "strict_min_version": "140.0",
    "data_collection_permissions": {
      "required": ["browsingActivity", "authenticationInfo"]
    }
  }
}
```

- Firefox n'exécute pas de service worker d'extension : `background.scripts` est
  utilisé à la place de `background.service_worker` (Chromium garde le service
  worker). Le build produit donc deux manifestes, un par cible.
- `strict_min_version: 140.0` est le premier Firefox qui comprend
  `data_collection_permissions`, obligatoire pour toute nouvelle soumission AMO.
- `data_collection_permissions.required` déclare les données traitées :
  - `browsingActivity` : l'URL de la page que l'utilisateur envoie ;
  - `authenticationInfo` : le jeton d'appairage LiveChat, stocké localement et
    envoyé en `Authorization: Bearer` au serveur configuré.
- La permission d'hôte pour l'API n'est **pas** dans `host_permissions` : elle
  est demandée à l'exécution depuis la page d'options
  (`optional_host_permissions`), après saisie de l'URL du serveur. Firefox exige
  un geste utilisateur au moment de `browser.permissions.request()` ; les
  handlers d'options demandent donc la permission avant tout `await`.

## Métadonnées de la fiche

`extension/amo-metadata.json` est passé à `web-ext sign` :

- `summary` : résumé bilingue (`en-US`, `fr`), obligatoire pour une première
  version listée ;
- `categories` : `["social-communication"]` ;
- `version.license` : `all-rights-reserved` par défaut (aucun fichier `LICENSE`
  dans le dépôt). Remplacer par `MIT`, `MPL-2.0`, etc. avant la première
  soumission si le projet doit être open source ;
- `version.approval_notes` : notes pour les relecteurs Mozilla (pas de serveur
  éditeur, aucune télémétrie, build Firefox non minifié, permissions expliquées).

La fiche publique (nom, description, captures, politique de confidentialité) se
complète dans le Developer Hub. Les visuels sont dans `docs/store/`, et la
politique de confidentialité dans [`privacy.md`](privacy.md) — publier son URL
(GitHub, Pages ou site du projet) dans le champ « Privacy Policy ».

## Première soumission et mises à jour

Depuis `extension/` :

```bash
npm run build:firefox
npx --yes web-ext@8 sign \
  --source-dir dist/firefox \
  --channel listed \
  --api-key "$AMO_JWT_ISSUER" \
  --api-secret "$AMO_JWT_SECRET" \
  --amo-metadata amo-metadata.json \
  --approval-timeout 0
```

`--approval-timeout 0` rend la main après la soumission sans attendre la revue.
La première version listée crée l'add-on avec les métadonnées ; les suivantes
ajoutent une version au même add-on.

Avant la première publication, incrémenter `version` dans
`extension/package.json` : le job vérifie que le tag `extension-v*` correspond, et
le tag `extension-v0.3.0` existant ne contient pas encore le support Firefox.

En CI, le job `publish-amo` de
[`extension.yml`](../../.github/workflows/extension.yml) fait exactement cela à
chaque tag `extension-v*`, ou manuellement avec `publish_to_amo`. Il ignore la
fiche (texte, captures) : celle-ci reste éditée à la main dans le Developer Hub.

## Mise à jour des installations

AMO sert de canal de mise à jour : après approbation, Firefox met à jour les
installations listées, sans `update_url` ni fichier `updates.json`. Il suffit de
publier une version supérieure (le numéro vient de `extension/package.json`).
AMO peut demander une revue manuelle : la mise à jour n'arrive qu'après
approbation, contrairement à une distribution auto-hébergée.

## Vérifications avant soumission

```bash
npm run ci:check
npx --yes web-ext@8 lint --source-dir dist/firefox
```

Le linter AMO doit sortir **0 erreur**. Les warnings `UNSAFE_VAR_ASSIGNMENT`
restants viennent de `innerHTML` avec un balisage statique dans les modales et le
logo ; ils sont acceptés.

Pour tester le paquet Firefox sans l'installer durablement :

1. `npm run build:firefox` ;
2. `about:debugging#/runtime/this-firefox` ;
3. « Charger un module temporaire » → `dist/firefox/manifest.json`.

Une installation temporaire disparaît à la fermeture de Firefox ; elle sert
uniquement aux tests locaux.

## Erreurs AMO fréquentes

- `update_url` refusé : le retirer du manifeste Firefox, AMO gère les mises à
  jour des add-ons listés.
- `data_collection_permissions` manquant : la clé est obligatoire pour les
  nouvelles soumissions ; `required` ne peut pas mélanger `"none"` avec une
  catégorie.
- Licence manquante : `version.license` doit être un slug de la liste AMO
  (`all-rights-reserved`, `MPL-2.0`, `MIT`, …).
- Code minifié : AMO exige alors une archive de sources. Le build Firefox est
  volontairement non minifié (`scripts/build.mjs`), donc le paquet envoyé est
  lisible ; ne pas réactiver la minification pour Firefox sans ajouter
  `--upload-source-code`.
