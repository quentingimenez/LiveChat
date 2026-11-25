# Raccourci iOS : envoyer un média depuis le menu Partage

Le menu Partage d'iOS ne peut pas appeler directement l'API d'un site : il faut
une **Share Extension** (une app native) ou un **Raccourci**. LiveChat n'a pas
d'app iOS, mais son API d'ingestion accepte déjà les clients légers. Ce guide
installe donc un Raccourci qui envoie une photo, une vidéo ou un lien à
LiveChat depuis n'importe quelle app iOS.

Deux usages :

- **Jouer maintenant** : le média part dans la file de lecture et s'affiche sur
  le stream.
- **Ajouter à la board** : le média rejoint la bibliothèque du serveur, sans
  être joué.

Le Raccourci n'a aucun droit d'administration : il utilise un **jeton
d'ingestion** limité à l'envoi de médias (`Surface::Extension`,
`Action::Send`). Ce jeton est révocable à tout moment.

> **Fichiers prêts à importer** — Trois raccourcis signés sont fournis dans
> [`ios/`](../ios/) : `LiveChat-Photo.shortcut`, `LiveChat-Video.shortcut` et
> `LiveChat-Lien.shortcut`. Chacun ne reçoit qu'un seul type de média, donc iOS
> n'affiche dans la feuille de partage que celui qui correspond au partage ; il
> n'y a pas de menu à choisir. À l'import, iOS demande **Adresse du serveur**
> puis **Jeton d'ingestion iOS** : colle les valeurs affichées par la board.
> Aucune action du raccourci n'est à modifier. La procédure ci-dessous reste
> utile pour créer une variante (board, légende) ou comprendre les actions.

## Prérequis

- Le serveur LiveChat est joignable depuis l'iPhone, en **HTTPS** de
  préférence. Une adresse `localhost` ou `127.0.0.1` ne fonctionne pas depuis le
  téléphone : utilise l'URL publique du serveur, ou une adresse LAN/Tailscale
  accessible depuis l'iPhone.
- Ton compte est membre actif de la guilde et possède le droit d'envoyer un
  média (rôle membre ou admin).
- L'ingestion est activée sur le serveur (`/ingest` répond autre chose que
  `503 ingest_api_disabled`).
- Un ordinateur avec la board de l'overlay (ou la console d'administration) pour
  créer le jeton une fois. La suite se fait sur l'iPhone.

## 1. Créer le jeton d'ingestion

1. Ouvre la board de l'overlay sur le PC.
2. Va dans **Mon profil** → section **Appareils** → carte **Raccourci iOS** →
   **Configurer**.
3. La board crée un client d'ingestion dédié et affiche une seule fois :
   - l'adresse publique du serveur ;
   - le jeton d'ingestion.
4. Si tu préfères la console d'administration : **Clients d'ingestion** →
   **Créer un client**, puis utilise l'URL du serveur et le jeton affiché.

Le client s'appelle `Raccourci iOS - <pseudo>` : le reconfigurer révoque l'ancien
jeton et en crée un nouveau. Les clients actifs apparaissent dans **Mon profil**
→ **Mes appareils**, où chacun peut être révoqué sans passer par la console
d'administration. Pour plusieurs téléphones, crée des clients séparés dans la
console d'administration.

La carte affiche aussi les boutons **Photo**, **Vidéo** et **Lien** : chacun
copie l'adresse du raccourci correspondant, à ouvrir depuis l'iPhone (section 2).

> **Sécurité** — ce jeton permet d'envoyer des médias et d'ajouter des éléments
> à la board. Il ne permet pas d'administrer le serveur. Ne le partage pas, ne
> le commit pas, et révoque-le depuis **Mon profil** → **Mes appareils**, ou
> depuis la console d'administration → **Clients d'ingestion** si tu le perds.

## 2. Importer un raccourci signé

1. Depuis l'iPhone, ouvre le lien du raccourci voulu :
   - [LiveChat-Photo.shortcut](https://raw.githubusercontent.com/quentingimenez/LiveChat/main/ios/LiveChat-Photo.shortcut)
   - [LiveChat-Video.shortcut](https://raw.githubusercontent.com/quentingimenez/LiveChat/main/ios/LiveChat-Video.shortcut)
   - [LiveChat-Lien.shortcut](https://raw.githubusercontent.com/quentingimenez/LiveChat/main/ios/LiveChat-Lien.shortcut)
2. Touche le fichier téléchargé : iOS propose **Ajouter le raccourci**. Les
   fichiers sont déjà signés ; l'option « Raccourcis non fiables » ne devrait
   pas être nécessaire.
3. iOS demande **Adresse du serveur** puis **Jeton d'ingestion iOS** : colle les
   deux valeurs affichées par la board. Le raccourci est prêt, sans éditer
   d'action.

## 3. Créer un raccourci personnalisé (facultatif)

Dans l'app **Raccourcis** :

1. **+** → nomme le raccourci `Jouer sur LiveChat`.
2. Ouvre les réglages du raccourci (icône ⓘ) → active **Afficher dans le menu
   Partage** → dans **Recevoir**, choisis **Images** (et **Vidéos** si proposé).
   Pour les fichiers, crée une variante du raccourci avec **Recevoir** =
   **Fichiers** ; pour les URL, voir la section 5.
3. Ajoute deux actions **Texte** avec l'adresse du serveur et le jeton, ou
   réutilise les questions d'import du raccourci fourni. Pour une variante,
   **Demander une entrée** fonctionne aussi.
4. Pour envoyer le fichier partagé, convertis-le si besoin (photo JPEG) puis
   passe-le directement au corps **Fichier**. Le raccourci fourni utilise un nom
   fixe (`photo.jpg` / `video.mp4`) ; l'action **Nom** n'est utile que si tu
   veux garder le nom d'origine.
5. Ajoute **Obtenir le contenu de l'URL** :
   - **URL** : `https://livechat.example.com/ingest/file?filename=photo.jpg`
     (ajoute `&text=<Légende>` seulement si tu as une légende)
   - **Méthode** : `POST`
   - **En-têtes** : `Authorization` = `Bearer <Jeton>` ; ajoute aussi
     `Content-Type` si tu connais le type (`image/jpeg` pour une photo,
     `video/mp4` ou `video/quicktime` pour une vidéo). Sinon le serveur détecte
     le format à partir du fichier.
   - **Corps de la requête** : `Fichier`
   - **Fichier** : le fichier partagé (ou le fichier converti à l'étape 4).
6. Pour une légende, ajoute **Demander une entrée** (texte) avant l'URL, puis
   insère la réponse dans `text=`. Applique **Encoder l'URL** au nom, à la
   légende et au titre avant de les insérer : les espaces, accents et `&` cassent
   sinon la query.
7. Termine par **Afficher le résultat** (facultatif) pour voir l'`ingestionId`
   renvoyé par le serveur.

Le serveur répond `202` dès le fichier reçu, puis le convertit en tâche de
fond : le média rejoint la file de lecture quand il est prêt, même si le
Raccourci s'est déjà fermé. Le nom du fichier sert de titre au média ; la
légende est affichée avec lui.

## 4. Ajouter à la board plutôt que jouer

Duplique le Raccourci (`Ajouter à la board`) et change l'URL :

```text
https://livechat.example.com/ingest/file?filename=<Nom>&saveToBoard=true&title=<Titre>
```

- `saveToBoard=true` ajoute le média à la bibliothèque sans créer de job.
- `title` est le titre affiché dans la board (240 caractères max). Sans
  `title`, le nom du fichier est utilisé.
- `durationSec` (1 à 600) règle la durée d'affichage d'une image dans l'overlay.
- `text` n'est pas conservé sur un élément de board : il ne sert qu'à la
  lecture. Pour un média ajouté à la board, utilise `title`.

## 5. Partager un lien YouTube, TikTok ou X

Le fichier binaire ne concerne que les médias locaux. Un raccourci signé
`LiveChat-Lien.shortcut` est fourni dans [`ios/`](../ios/) : il envoie déjà
`{"url": <URL partagée>}` et demande l'adresse du serveur et le jeton à
l'import. Pour une variante (légende, ajout à la board), crée un troisième Raccourci
(`Lien sur LiveChat`) avec **Recevoir** = **URL**, puis envoie une requête JSON à
`/ingest` :

- **URL** : `https://livechat.example.com/ingest`
- **Méthode** : `POST`
- **En-têtes** :
  - `Authorization` = `Bearer <Jeton>`
  - `Content-Type` = `application/json`
- **Corps de la requête** : `JSON`
  - `url` : l'URL partagée
  - `text` : la légende (facultatif)
  - `saveToBoard` : `false` pour jouer, `true` pour ajouter à la board
  - `forceRefresh` : `true` pour ignorer le cache (facultatif)

Le serveur télécharge le média avec yt-dlp, comme pour l'extension navigateur.

**Contrôle du statut, indispensable.** L'action « Obtenir le contenu de l'URL »
réussit même quand le serveur répond `401` : le Raccourci affiche alors « Lien
envoyé » alors que rien n'est arrivé. Ajoute donc après l'envoi :

1. **Obtenir les détails des URL** sur la sortie de la requête, puis **Code de
   statut** ;
2. **Si** le code de statut **n'est pas** `202`, **Afficher une alerte** avec le
   corps de la réponse (`{"error":"unauthorized"}`…) ;
3. sinon seulement, **Afficher une notification** « Lien envoyé à LiveChat ».

Sans cette vérification, une erreur d'authentification ou de droits (`403`)
est totalement silencieuse côté téléphone.

Le `202` signifie « accepté » : le téléchargement et la conversion se font
ensuite sur le serveur, sans faire attendre le Raccourci. Un lien que le
serveur ne parvient pas à récupérer (vidéo privée, supprimée) échoue donc après
la notification : l'échec apparaît dans la console d'administration
(**Ingestions**) et sur `GET /ingest/ingestions/{ingestionId}` avec le même
jeton.

## Tester sans Raccourci

Commence par le contrôle le moins coûteux : `GET /ingest/whoami` vérifie le
jeton seul — il ne télécharge rien, n'écrit aucun job et répond exactement les
mêmes statuts que `POST /ingest` (`401 unauthorized` si le jeton est inconnu,
`401 ingest_token_expired` s'il a expiré, `503 ingest_api_disabled` si
l'ingestion est coupée, `200` avec le libellé de l'appareil s'il est valide).

```bash
read -rsp "Jeton d'ingestion : " TOKEN && echo
curl -sS "https://livechat.example.com/ingest/whoami" \
  -H "Authorization: Bearer $TOKEN"
unset TOKEN
```

```powershell
$secure = Read-Host "Jeton d'ingestion" -AsSecureString
$token = [Net.NetworkCredential]::new('', $secure).Password
try {
  Invoke-RestMethod -Method Get `
    -Uri "https://livechat.example.com/ingest/whoami" `
    -Headers @{ Authorization = "Bearer $token" }
} finally {
  Remove-Variable token, secure -ErrorAction SilentlyContinue
}
```

Réponse d'un jeton valide :

```json
{
  "ok": true,
  "clientId": "…",
  "label": "Extension-…",
  "guildId": "…"
}
```

`label` et `guildId` confirment **quel** appairage le jeton utilise : deux
appareils du même profil portent des libellés différents, et un `guildId`
inattendu signifie que le raccourci pointe vers une autre board.

Une fois le jeton vérifié, teste le serveur depuis le PC. Saisis-le **sans
écho** : ne le recopie jamais dans une ligne de commande (PowerShell et bash
conservent l'historique, et la valeur serait visible dans la liste des
processus). Sous PowerShell 5.1, `Read-Host -AsSecureString` évite l'écho et
`[Net.NetworkCredential]` convertit la valeur pour l'en-tête ; le `finally`
efface la variable même si la requête échoue :

```powershell
$secure = Read-Host "Jeton d'ingestion" -AsSecureString
$token = [Net.NetworkCredential]::new('', $secure).Password
try {
  Invoke-RestMethod -Method Post `
    -Uri "https://livechat.example.com/ingest/file?filename=photo.jpg&text=Bonjour" `
    -Headers @{ Authorization = "Bearer $token" } `
    -ContentType "image/jpeg" -InFile .\photo.jpg
} finally {
  Remove-Variable token, secure -ErrorAction SilentlyContinue
}
```

Avec `curl` (bash) :

```bash
read -rsp "Jeton d'ingestion : " TOKEN && echo
curl -sS -X POST \
  "https://livechat.example.com/ingest/file?filename=photo.jpg&text=Bonjour" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: image/jpeg" \
  --data-binary @photo.jpg
unset TOKEN
```

`read -s` supprime l'écho et `unset` retire la variable du shell, mais **la
valeur reste visible dans les arguments du processus** (`ps`, gestionnaire de
tâches) pour le même utilisateur ou root : sur une machine partagée, utilise
un jeton de test révocable, jamais le jeton de production. Ne mets jamais de
jeton dans un script versionné ni dans un message.

### Sécurité du jeton

Le jeton d'ingestion est un **secret d'écriture** créé par la board, lié à
l'appareil et révocable depuis **Mon profil → Appareils** (reconfigurer
l'appareil révoque l'ancien jeton). Il **expire après 180 jours sans activité**
et l'ingestion répond alors `401 ingest_token_expired` ; un envoi remet le
compteur à zéro. Voir [`extension/README.md`](../extension/README.md) et
[`ios/README.md`](../ios/README.md) pour le détail du stockage et de la durée
de vie. Ne le partage pas, ne le commit pas.

Réponse d'un envoi (`202`, à jouer ou pour la board) :

```json
{
  "accepted": true,
  "ingestionId": "…",
  "guildId": "…"
}
```

Le résultat se lit ensuite avec le même jeton :

```bash
curl -sS "https://livechat.example.com/ingest/ingestions/<ingestionId>"   -H "Authorization: Bearer $TOKEN"
```

```json
{ "ingestionId": "…", "status": "ready", "created": false, "jobId": "…" }
```

`status` vaut `processing` pendant la conversion, `ready` à la fin (`jobId`
pour une lecture, `itemId` pour un ajout à la board) ou `failed` avec
`error.mediaCode` (`PRIVATE_OR_AUTH_REQUIRED`, `MEDIA_NOT_FOUND`…).

## Limites et dépannage

| Symptôme | Cause probable | Solution |
| --- | --- | --- |
| `401 unauthorized` | Jeton absent, faux ou révoqué | Reconfigure le raccourci depuis **Mon profil → Raccourci iOS** et recolle le nouveau jeton |
| `403 forbidden_role` | Le membre n'a pas le droit d'envoyer un média | Demande à un admin de vérifier le rôle |
| `413 payload_too_large` | Fichier au-dessus de la limite serveur | Regarde `maxBytes` ; compresse la vidéo ou passe par la board du PC |
| `422 media_ingestion_failed` | Lien refusé par le serveur (adresse privée, schéma non HTTP) | Partage le lien public du média |
| `202` mais rien n'est joué | La conversion a échoué après la réponse (format non reconnu, vidéo privée ou supprimée) | Regarde l'ingestion dans la console d'administration ou sur `GET /ingest/ingestions/{id}` ; convertis en JPEG/MP4 ou réessaie |
| `503 ingest_api_disabled` | Ingestion désactivée sur le serveur | Active l'ingestion côté serveur |
| `400 invalid_payload` | `filename` invalide, corps vide, `durationSec` hors 1–600 | Corrige la query ou le fichier |
| `504 ingest_timeout` | Le fichier n'a pas fini d'arriver dans le budget du serveur (300 s par défaut, réseau très lent) | Réessaie avec un fichier plus léger ou sur un réseau stable |
| Le Raccourci affiche « Lien envoyé » mais rien n'arrive | Le Raccourci n'inspecte pas le code de statut : un `401`/`403` passe inaperçu ; ou la conversion a échoué après le `202` | Vérifie le jeton avec `GET /ingest/whoami` (§ Tester sans Raccourci), puis ajoute le contrôle de statut (§5) ; regarde l'ingestion dans la console d'administration |
| Le Raccourci reste bloqué | Envoi d'un gros fichier sur un réseau lent | Réduis la vidéo (1080p), réessaie sur un réseau stable ; le serveur abandonne de son côté au bout du budget `/ingest` (`504 ingest_timeout`) |

La limite effective est la plus basse entre :

- `MEDIA_MAX_SIZE_MB` du serveur (2048 Mo par défaut) ;
- la limite du reverse proxy / Cloudflare (100 Mo par requête en offre
  gratuite).

Le serveur écrit le fichier dans un dossier temporaire et le hache en SHA-256 :
un même fichier envoyé deux fois réutilise le cache média.

## Révoquer l'accès

Console d'administration → **Clients d'ingestion** → sélectionne le client
`Raccourci iOS - <pseudo>` → **Révoquer**. Le Raccourci recevra alors `401
unauthorized`. Reconfigurer depuis la board révoque aussi l'ancien client et
crée un nouveau jeton.

## Pourquoi pas une app native ?

Une vraie Share Extension iOS demanderait une app SwiftUI, Xcode et un compte
Apple Developer payant pour une installation durable (TestFlight). Le Raccourci
n'a besoin ni de Mac ni de compte payant, s'installe directement depuis
l'iPhone et s'intègre au menu Partage. C'est la solution retenue pour ce dépôt.

L'endpoint utilisé est `POST /ingest/file`, ajouté au serveur pour les clients
légers. Il réutilise l'authentification, les droits et le pipeline média de
`POST /ingest`, sans nouvelle table de jetons.
