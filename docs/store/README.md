# Fiche Chrome Web Store

Tout ce qui se copie à la main dans la
[console développeur](https://chrome.google.com/webstore/devconsole) : la CI ne
met à jour que le paquet (`.github/workflows/extension.yml`, job
`publish-webstore`), jamais la fiche.

Logo, captures et textes ci-dessous correspondent à l'extension à partir de la
version 0.2.2 (boutons sur X, TikTok, YouTube et Shorts).

## Ce qu'il faut corriger dans la console

L'article existe déjà et publie une version 0.1.x, avec une fiche ancienne :

| Onglet de la console | À faire |
| --- | --- |
| Store listing | Logo `icon-128.png`, les 5 captures dans l'ordre, description courte et longue ci-dessous, catégorie et langue. |
| Store listing → screenshots | Ajouter les images 1280×800, retirer les anciennes si elles montrent une interface qui n'existe plus. |
| Privacy practices | But unique, justifications des permissions (ci-dessous), déclaration « pas de code distant ». |
| Distribution | Visibilité et pays ; rien à changer si c'était déjà public. |

Le paquet n'est pas concerné : il arrive par la CI. Après approbation de la
relecture, Chrome met à jour les extensions installées sans action de
l'utilisateur.

> Attention au nom : « LiveChat » est une marque déposée d'une autre société. Le
> nom actuel de l'article a déjà passé une première relecture, donc le garder ;
> éviter d'en ajouter un qui se rapprocherait de la marque.

## Textes

**Nom** : `LiveChat Extension`

**Description courte** (132 caractères maximum) :

```
Envoie le média d'un post X, TikTok ou YouTube en un clic vers le board et l'overlay de ton serveur Discord.
```

**Description détaillée** :

```
LiveChat Extension ajoute un bouton sur les posts de X, les vidéos TikTok et les vidéos YouTube : un clic suffit pour envoyer le média au board et à l'overlay de ton serveur Discord.

CE QU'ELLE FAIT
• Un bouton « LC » sur chaque post X, chaque vidéo TikTok et chaque vidéo YouTube, y compris les Shorts.
• Un clic envoie le lien au serveur : l'image ou la vidéo apparaît sur l'overlay et dans le board média.
• Un appui long (ou un double-clic) ouvre une fenêtre pour ajouter un texte, régler la durée d'affichage des images, ignorer le cache ou garder le média dans le board.
• Menu contextuel « Envoyer vers LiveChat » sur un lien, et popup pour envoyer l'onglet courant.
• Page d'options : URL du serveur, code d'appairage Discord, nom et avatar affichés sur l'overlay.

PRÉREQUIS
• Un serveur LiveChat (bot Discord et application overlay) et un code d'appairage, généré depuis l'overlay ou avec /pair-code.

CONFIDENTIALITÉ
• Aucune analytique, aucun serveur tiers : l'extension ne contacte que le serveur que tu configures.
• Ce qui est envoyé se limite à l'URL du média, au texte saisi et au nom/avatar configurés.
```

**Catégorie** : Social & Communication · **Langue** : Français

## Captures (1280×800, PNG sans transparence)

| Fichier | Ce qu'elle montre |
| --- | --- |
| `screenshot-1-x.png` | Un post X : le bouton « LC » dans la rangée d'actions. |
| `screenshot-2-compose.png` | La fenêtre d'envoi (texte, durée d'affichage, options). |
| `screenshot-3-tiktok.png` | Une vidéo TikTok : le bouton « LC » en bas de la colonne d'actions. |
| `screenshot-4-shorts.png` | Un Short : le bouton « LC » au-dessus du « J'aime ». |
| `screenshot-5-youtube-watch.png` | Une vidéo YouTube : le bouton « LC » dans la barre d'actions. |

`icon-128.png` est le logo de l'extension (le même que `extension/public/icons/icon-128.png`).

## Pratiques de confidentialité

**But unique** : envoyer le média d'une page (X, TikTok, YouTube ou n'importe
quel lien) vers le board et l'overlay du serveur Discord LiveChat de
l'utilisateur.

**Justification des permissions** :

- `storage` — enregistre la configuration locale (URL du serveur, jeton
  d'appairage, nom et avatar affichés).
- `contextMenus` — ajoute « Envoyer vers LiveChat » au menu du clic droit.
- `tabs` — lit l'URL de l'onglet actif pour l'envoi depuis la popup et le menu
  contextuel.
- `webRequest` — suit les redirections des médias TikTok pour retrouver l'URL
  directe du fichier, que la page ne publie pas dans son DOM.
- Hôtes `x.com`, `twitter.com`, `tiktok.com`, `youtube.com`, `youtu.be` —
  injectent le bouton d'envoi sur la page.
- Hôtes optionnels `https://*/*`, `http://*/*` — permettent d'activer l'envoi sur
  d'autres sites ; demandés à l'utilisateur, jamais accordés d'office.

**Code distant** : aucun (tout le code est dans le paquet).

**Données** : l'extension n'envoie des données qu'au serveur LiveChat configuré
par l'utilisateur ; rien n'est vendu, revendu ni transmis à un tiers.

**Politique de confidentialité** : `docs/store/privacy.md` (publiée à
<https://github.com/quentingimenez/LiveChat/blob/main/docs/store/privacy.md>).
C'est l'URL à coller dans *Privacy practices* de la fiche et dans *Branding* du
projet Google Cloud ; elle devra passer sur le domaine du serveur
(`https://livechat.singesupreme.fr/privacy`) quand la console exigera un domaine
autorisé.

## Secrets GitHub (une seule fois)

`Settings > Secrets and variables > Actions` :

| Secret | Où le trouver |
| --- | --- |
| `CWS_PUBLISHER_ID` | Premier segment de l'URL de la console : `devconsole/<publisherId>/...`. |
| `CWS_EXTENSION_ID` | Identifiant de l'article (32 caractères, dans l'URL de l'article). |
| `CWS_CLIENT_ID` | Identifiant du client OAuth « Desktop app » créé dans Google Cloud. |
| `CWS_CLIENT_SECRET` | Secret du même client OAuth. |
| `CWS_REFRESH_TOKEN` | Sortie de `scripts/webstore-refresh-token.ps1` (voir ci-dessous). |

### Créer le client OAuth et le refresh token

1. Google Cloud Console → nouveau projet (ou projet existant) → *APIs & Services*
   → activer **Chrome Web Store API**.
2. *OAuth consent screen* → type *External*, ajouter son compte dans *Test users*.
3. *Credentials* → *Create credentials* → *OAuth client ID* → type
   **Desktop app** → noter `CWS_CLIENT_ID` et `CWS_CLIENT_SECRET`.
4. Depuis la racine du dépôt :

   ```powershell
   pwsh -NoProfile -File scripts/webstore-refresh-token.ps1 `
     -ClientId "<client id>" -ClientSecret "<client secret>"
   ```

   Le navigateur s'ouvre sur l'écran de consentement ; accepter le scope
   `chromewebstore` : le script affiche le refresh token à copier dans GitHub.

## Envoyer une version

- Un tag `extension-v*` (créé par la CI de release) envoie le zip et le soumet à
  la relecture.
- Sans nouveau tag : *Actions* → **Extension Build and Release** →
  *Run workflow* → cocher **publier sur le Chrome Web Store**.

La version du manifest doit être strictement supérieure à celle publiée
(0.2.2 est en relecture), sinon l'upload est refusé. La relecture prend en
général quelques heures à quelques jours ; le job affiche
`submission: PENDING_REVIEW` quand tout est accepté.

Le refresh token expire au bout de 7 jours tant que l'application OAuth reste en
mode *Testing* : republier le client en *Production* (onglet *Audience*) ou
relancer le script de refresh token avant expiration.
