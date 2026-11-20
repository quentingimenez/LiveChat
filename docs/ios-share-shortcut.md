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

## Prérequis

- Le serveur LiveChat est joignable depuis l'iPhone, en **HTTPS** de
  préférence. Une adresse `localhost` ou `127.0.0.1` ne fonctionne pas depuis le
  téléphone : utilise l'URL publique du serveur, ou une adresse LAN/Tailscale
  accessible depuis l'iPhone.
- Ton compte est membre actif de la guilde et possède le droit d'envoyer un
  média (rôle membre ou admin).
- L'ingestion est activée sur le serveur (`/ingest` répond autre chose que
  `503 ingest_api_disabled`).
- Un ordinateur avec le terminal PowerShell ou `curl` pour créer le jeton une
  fois. La suite se fait sur l'iPhone.

## 1. Créer le jeton d'ingestion

1. Ouvre la board de l'overlay sur le PC.
2. Va dans **Mon profil** → section **Appareils** → carte **Extension
   navigateur** → **Créer un code**.
3. La board affiche l'adresse du serveur et un code à 8 caractères. Le code
   expire au bout de **10 minutes**.
4. Depuis le PC, échange ce code contre un jeton :

```powershell
$base = "https://livechat.example.com"   # adresse affichée par la board
$code = "ABCD-EFGH"                      # code affiché par la board
$body = @{ code = $code; deviceName = "iPhone de Quentin" } | ConvertTo-Json
$response = Invoke-RestMethod -Method Post -Uri "$base/ingest/pair/consume" `
  -ContentType "application/json" -Body $body
$response.ingestApiToken
```

Avec `curl` :

```bash
curl -sS -X POST "https://livechat.example.com/ingest/pair/consume" \
  -H "Content-Type: application/json" \
  -d '{"code":"ABCD-EFGH","deviceName":"iPhone de Quentin"}' \
  | jq -r .ingestApiToken
```

La réponse contient notamment :

```json
{
  "apiBaseUrl": "https://livechat.example.com",
  "ingestApiToken": "…64 caractères hexadécimaux…",
  "ingestClientId": "…",
  "guildId": "…",
  "deviceName": "iPhone de Quentin",
  "authorName": "…",
  "authorImage": null
}
```

Garde le `ingestApiToken` : il n'est affiché qu'une fois. Utilise un
`deviceName` unique et stable (`iPhone de Quentin`, pas `iPhone`), car
re-appairer avec le même libellé révoque l'ancien client.

> **Sécurité** — ce jeton permet d'envoyer des médias et d'ajouter des éléments
> à la board. Il ne permet pas d'administrer le serveur. Ne le mets pas dans un
> Raccourci partagé, ne le commit pas, et révoque-le depuis la console
> d'administration → **Clients d'ingestion** si tu le perds.

## 2. Déposer le jeton sur l'iPhone

Les Raccourcis n'ont pas de trousseau (Keychain) : le jeton doit être lu dans un
fichier. Crée un fichier texte `ingest-token.txt` contenant uniquement le jeton,
puis dépose-le dans un dossier accessible depuis l'app **Fichiers** de
l'iPhone — par exemple `iCloud Drive/LiveChat/ingest-token.txt`, ou un dossier
OneDrive/Google Drive connecté à Fichiers.

Évite de laisser le jeton dans une note synchronisée ou dans le corps du
Raccourci : un Raccourci se partage facilement.

## 3. Créer le Raccourci « Jouer sur LiveChat »

Dans l'app **Raccourcis** :

1. **+** → nomme le raccourci `Jouer sur LiveChat`.
2. Ouvre les réglages du raccourci (icône ⓘ) → active **Afficher dans le menu
   Partage** → dans **Recevoir**, choisis **Images** (et **Vidéos** si proposé).
   Pour les fichiers, crée une variante du raccourci avec **Recevoir** =
   **Fichiers** ; pour les URL, voir la section 5.
3. Ajoute **Obtenir le contenu du fichier** et sélectionne
   `iCloud Drive/LiveChat/ingest-token.txt`. Ajoute ensuite une action de
   nettoyage (`Rogner les espaces` / Trim) pour retirer le retour à la ligne
   final, puis garde le résultat dans une variable `Jeton`.
4. Récupère le nom du fichier partagé :
   - **photo** : **Convertir l'image** (JPEG, qualité moyenne) →
     **Enregistrer le fichier** dans un dossier temporaire → **Obtenir le
     fichier** → **Obtenir les détails du fichier** → **Nom** ;
   - **vidéo** : **Encoder le média** (720p) → **Enregistrer le fichier** →
     **Obtenir le fichier** → **Obtenir les détails du fichier** → **Nom** ;
   - **fichier** : **Obtenir les détails du fichier** → **Nom**.
5. Ajoute **Obtenir le contenu de l'URL** :
   - **URL** : `https://livechat.example.com/ingest/file?filename=<Nom>&text=<Légende>`
   - **Méthode** : `POST`
   - **En-têtes** : `Authorization` = `Bearer <Jeton>` ; ajoute aussi
     `Content-Type` si tu connais le type (`image/jpeg` pour une photo,
     `video/mp4` pour une vidéo). Sinon le serveur détecte le format à partir du
     fichier.
   - **Corps de la requête** : `Fichier`
   - **Fichier** : le fichier partagé (ou le fichier converti à l'étape 4).
6. Pour une légende, ajoute **Demander une entrée** (texte) avant l'URL, puis
   insère la réponse dans `text=`. Applique **Encoder l'URL** au nom, à la
   légende et au titre avant de les insérer : les espaces, accents et `&` cassent
   sinon la query.
7. Termine par **Afficher le résultat** (facultatif) pour voir le `jobId` et
   les champs `hasMedia` / `hasText` renvoyés par le serveur.

Le média part immédiatement dans la file de lecture. Le nom du fichier sert de
titre au média ; la légende est affichée avec lui.

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

Le fichier binaire ne concerne que les médias locaux. Crée un second Raccourci
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

## Tester sans Raccourci

Une fois le jeton créé, vérifie le serveur depuis le PC :

```powershell
$token = Get-Content .\ingest-token.txt -Raw
Invoke-RestMethod -Method Post `
  -Uri "https://livechat.example.com/ingest/file?filename=photo.jpg&text=Bonjour" `
  -Headers @{ Authorization = "Bearer $($token.Trim())" } `
  -ContentType "image/jpeg" -InFile .\photo.jpg
```

Avec `curl` :

```bash
curl -sS -X POST \
  "https://livechat.example.com/ingest/file?filename=photo.jpg&text=Bonjour" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: image/jpeg" \
  --data-binary @photo.jpg
```

Réponse d'un envoi joué :

```json
{
  "accepted": true,
  "jobId": "…",
  "guildId": "…",
  "hasMedia": true,
  "hasText": true
}
```

Réponse d'un ajout à la board :

```json
{
  "accepted": true,
  "savedToBoard": true,
  "created": true,
  "guildId": "…",
  "mediaBoardItemId": "…",
  "mediaAssetId": "…"
}
```

## Limites et dépannage

| Symptôme | Cause probable | Solution |
| --- | --- | --- |
| `401 unauthorized` | Jeton absent, faux ou révoqué | Recrée un code et un client, remplace le fichier du jeton |
| `403 forbidden_role` | Le membre n'a pas le droit d'envoyer un média | Demande à un admin de vérifier le rôle |
| `413 payload_too_large` | Fichier au-dessus de la limite serveur | Regarde `maxBytes` ; compresse la vidéo ou passe par la board du PC |
| `422 media_ingestion_failed` | Format non reconnu ou fichier corrompu | Convertis en JPEG/MP4, réessaie |
| `503 ingest_api_disabled` | Ingestion désactivée sur le serveur | Active l'ingestion côté serveur |
| `400 invalid_payload` | `filename` invalide, corps vide, `durationSec` hors 1–600 | Corrige la query ou le fichier |
| Le Raccourci reste bloqué | Fichier trop gros ou réseau lent | Réduis la vidéo (1080p), réessaie sur un réseau stable |

La limite effective est la plus basse entre :

- `MEDIA_MAX_SIZE_MB` du serveur (2048 Mo par défaut) ;
- la limite du reverse proxy / Cloudflare (100 Mo par requête en offre
  gratuite).

Le serveur écrit le fichier dans un dossier temporaire et le hache en SHA-256 :
un même fichier envoyé deux fois réutilise le cache média.

## Révoquer l'accès

Console d'administration → **Clients d'ingestion** → sélectionne le client
`iPhone de Quentin` → **Révoquer**. Le Raccourci recevra alors `401
unauthorized`. Re-appairer avec le même `deviceName` révoque aussi l'ancien
client et crée un nouveau jeton.

## Pourquoi pas une app native ?

Une vraie Share Extension iOS demanderait une app SwiftUI, Xcode et un compte
Apple Developer payant pour une installation durable (TestFlight). Le Raccourci
n'a besoin ni de Mac ni de compte payant, s'installe directement depuis
l'iPhone et s'intègre au menu Partage. C'est la solution retenue pour ce dépôt.

L'endpoint utilisé est `POST /ingest/file`, ajouté au serveur pour les clients
légers. Il réutilise l'authentification, les droits et le pipeline média de
`POST /ingest`, sans nouvelle table de jetons.
