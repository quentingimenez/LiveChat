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
3. iOS demande **Adresse du serveur** puis **Jeton d'ingestion iOS** : colle
   les valeurs affichées par la board (voir ci-dessous). Ces deux questions
   d'import évitent d'éditer les actions du raccourci.

Si iOS refuse l'import, active **Réglages → Apps → Raccourcis → Autoriser les
raccourcis non fiables**, puis réessaie. Les fichiers de ce dossier sont déjà
signés ; cette option ne devrait pas être nécessaire.

## Configurer après l'import

Dans la board : **Mon profil → Appareils → Raccourci iOS → Configurer**. La
board crée un jeton dédié et affiche une seule fois :

- l'adresse publique du serveur (par exemple `https://livechat.example.com`) ;
- le jeton d'ingestion.

Colle ces deux valeurs dans les questions posées par iOS à l'import. Le client
s'appelle `Raccourci iOS - <pseudo>` ; le reconfigurer révoque l'ancien jeton et
en crée un nouveau.

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

Le jeton n'est **pas** présent dans les fichiers : il est saisi une fois à
l'import, dans les questions du raccourci. Le service de signature ne reçoit que
le modèle sans jeton. Utilise un jeton dédié, révocable depuis la console
d'administration → **Clients d'ingestion**.

### Durée de vie du jeton d'ingestion

Le jeton reste valable tant que le raccourci est utilisé. Le serveur le refuse
après **180 jours sans activité** (aucun envoi pendant cette période) : la
requête d'ingestion échoue alors avec `ingest_token_expired`. Une simple
utilisation du raccourci remet le compteur à zéro, mais un jeton déjà expiré ne
peut pas être « réveillé » : il faut reconfigurer le raccourci depuis la board
(**Mon profil → Appareils → Raccourci iOS → Configurer**) pour obtenir un
nouveau jeton et le coller dans le raccourci. Le jeton n'est affiché qu'une
seule fois par la board et n'est pas transféré automatiquement : il n'est pas
récupérable depuis l'ancien appareil ni depuis les fichiers signés. En cas de
perte de l'appareil, reconfigure le raccourci depuis la board.

Les trois fichiers distribués ici sont signés par le service
[HubSign](https://hubsign.routinehub.services/sign) de RoutineHub, qui utilise
la signature officielle d'Apple. La signature est vérifiée par iOS à l'import.

La documentation détaillée est dans [`docs/ios-share-shortcut.md`](../docs/ios-share-shortcut.md).
