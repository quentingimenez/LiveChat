# Raccourcis iOS LiveChat

Deux raccourcis sont fournis ici :

- `LiveChat-Photo.shortcut` : partage d'une image depuis Photos ou Fichiers.
- `LiveChat-Video.shortcut` : partage d'une vidéo depuis Photos ou Fichiers.

Ils envoient le fichier partagé à l'endpoint `POST /ingest/file` de LiveChat, avec
le jeton d'ingestion comme `Authorization: Bearer ...`.

## Avant l'import

Les fichiers de ce dossier sont les sources **non signées**. iOS 15 et versions
ultérieures refusent généralement d'importer un `.shortcut` non signé. Le
workflow GitHub Actions `Sign iOS shortcuts` les signe sur un runner macOS et
publie les fichiers signés dans l'artefact `signed-ios-shortcuts` :

1. Ouvrir l'onglet **Actions** du dépôt GitHub.
2. Sélectionner le workflow **Sign iOS shortcuts**, puis le dernier run vert.
3. Télécharger l'artefact `signed-ios-shortcuts`.
4. Ouvrir le `.shortcut` signé sur l'iPhone : le bouton **Ajouter le raccourci**
   doit apparaître.

Le workflow se relance automatiquement à chaque modification d'un `.shortcut`
de ce dossier, et peut aussi être lancé manuellement.

## Après l'import

Dans l'éditeur du raccourci, remplacer :

- `TON-SERVEUR` par l'hôte public de LiveChat, sans slash final
  (par exemple `livechat.example.com`) ;
- `TON_JETON` par le jeton d'ingestion créé depuis la board
  (Mon profil → Extension navigateur → Créer un code, puis
  `POST /ingest/pair/consume`).

Le raccourci apparaît ensuite dans le menu de partage iOS pour les images ou les
vidéos. Il joue le média immédiatement sur le stream. Pour l'ajouter au board au
lieu de le jouer, ajouter `&saveToBoard=true&title=Mon%20titre` à l'URL.

La documentation détaillée est dans [`docs/ios-share-shortcut.md`](../docs/ios-share-shortcut.md).
