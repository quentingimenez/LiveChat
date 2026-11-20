# Raccourcis iOS LiveChat

Trois raccourcis **signés**, prêts à importer, sont fournis ici :

- [LiveChat-Photo.shortcut](https://raw.githubusercontent.com/quentingimenez/LiveChat/main/ios/LiveChat-Photo.shortcut) : partage une image depuis Photos.
- [LiveChat-Video.shortcut](https://raw.githubusercontent.com/quentingimenez/LiveChat/main/ios/LiveChat-Video.shortcut) : partage une vidéo depuis Photos.
- [LiveChat-Lien.shortcut](https://raw.githubusercontent.com/quentingimenez/LiveChat/main/ios/LiveChat-Lien.shortcut) : partage un lien YouTube, TikTok, X, Instagram, etc.

Chaque raccourci ne reçoit qu'un seul type de média. Dans la feuille de partage,
iOS n'affiche donc que celui qui correspond à ce que tu partages : une image
montre **LiveChat Photo**, une vidéo montre **LiveChat Video**, et un lien montre
**LiveChat Lien**. Il n'y a pas de menu à choisir : le type du média fait la
sélection.

Les deux premiers envoient le fichier partagé à `POST /ingest/file`. Le troisième
envoie `{"url": "<lien partagé>"}` à `POST /ingest` : le serveur télécharge le
média avec yt-dlp, comme pour l'extension navigateur. Tous utilisent le jeton
d'ingestion comme `Authorization: Bearer ...`.

## Importer sur l'iPhone

1. Ouvre le lien du raccourci dans Safari. Le fichier se télécharge dans
   l'app **Fichiers**.
2. Touche le fichier téléchargé : iOS propose **Ajouter le raccourci**.
3. Ouvre le raccourci dans l'app **Raccourcis** et remplace les deux valeurs
   de configuration (voir ci-dessous).

Si iOS refuse l'import, active **Réglages → Apps → Raccourcis → Autoriser les
raccourcis non fiables**, puis réessaie. Les fichiers de ce dossier sont déjà
signés ; cette option ne devrait pas être nécessaire.

## Configurer après l'import

Dans l'éditeur de chaque raccourci, remplace :

- `TON-SERVEUR` par l'hôte public de LiveChat, sans slash final
  (par exemple `livechat.example.com`) ;
- `TON_JETON` par le jeton d'ingestion créé depuis la board
  (Mon profil → Extension navigateur → Créer un code, puis
  `POST /ingest/pair/consume`).

Le raccourci apparaît ensuite dans le menu de partage iOS :

- **LiveChat Photo** pour une image ;
- **LiveChat Video** pour une vidéo ;
- **LiveChat Lien** pour une URL. Depuis YouTube ou TikTok, utilise
  **Partager → LiveChat Lien** (ou **Copier le lien**, puis partage le lien).
  Le média est téléchargé puis joué immédiatement sur le stream.

Pour ajouter un fichier au board au lieu de le jouer, ajoute
`&saveToBoard=true&title=Mon%20titre` à l'URL de `LiveChat Photo` ou
`LiveChat Video`. Pour un lien, ajoute `saveToBoard` = `true` dans le corps JSON
du raccourci (`LiveChat Lien`), ou consulte la documentation détaillée.

## Sécurité et signature

Le jeton n'est **pas** présent dans les fichiers : il est saisi après l'import,
dans l'éditeur du raccourci. Ne partage pas un raccourci dans lequel tu as mis
ton jeton ; utilise plutôt un jeton dédié, révocable depuis la console
d'administration → **Clients d'ingestion**.

Les trois fichiers distribués ici sont signés par le service
[HubSign](https://hubsign.routinehub.services/sign) de RoutineHub, qui utilise
la signature officielle d'Apple. La signature est vérifiée par iOS à l'import.

La documentation détaillée est dans [`docs/ios-share-shortcut.md`](../docs/ios-share-shortcut.md).
