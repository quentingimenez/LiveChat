# OverlayClient - LiveChatCaCaBox Companion

**OverlayClient** est une application de bureau **Electron** conçue pour fonctionner de concert avec le projet [LiveChatCaCaBox](https://github.com/qlaffont/LiveChatCaCaBox).

C'est un **overlay transparent, "click-through" et toujours au premier plan** qui permet d'afficher des alertes et de jouer des médias (Vidéos, Sons, YouTube) déclenchés via des commandes Discord.

## Pourquoi ce client ?

Contrairement à une simple source navigateur (Browser Source) dans OBS ou un navigateur Chrome classique, ce client a été développé pour contourner plusieurs limitations techniques spécifiques :

1.  **Gestion des Headers Discord** : Discord force le téléchargement des fichiers (Content-Disposition: attachment). Ce client intercepte les requêtes réseau pour modifier les en-têtes à la volée, permettant la **lecture en streaming direct** des fichiers audio/vidéo hébergés sur Discord.
2.  **Autoplay Forcé** : Contourne les politiques strictes des navigateurs (Chrome/Edge) qui empêchent la lecture automatique des médias sans interaction utilisateur.
3.  **Transparence Totale** : Fenêtre sans bordure, transparente, qui laisse passer les clics de souris (click-through) pour ne pas gêner l'utilisation du PC.

## Technologies Utilisées

* **[Electron](https://www.electronjs.org/)** : Framework principal pour créer l'application de bureau.
* **Node.js** : Backend de l'application.
* **Electron Session & WebRequest API** : Utilisé pour manipuler les en-têtes HTTP (onHeadersReceived) et corriger les types MIME (audio/mpeg3 -> audio/mpeg).

## Fonctionnalités Clés

* **Mode Fantôme** : La fenêtre est invisible tant qu'aucun média ne joue. Elle ignore les clics de souris (vous pouvez jouer/travailler en dessous).
* **Patch Audio Discord** : Correction automatique des headers Content-Disposition et Content-Type pour les fichiers venant de cdn.discordapp.com et media.discordapp.net.
* **Support Multi-écrans** : Une icône dans la zone de notification (System Tray) permet de déplacer l'overlay sur l'écran de votre choix.
* **Sécurité assouplie** : Configuration spécifique pour autoriser le contenu mixte (HTTP/HTTPS) et ignorer les erreurs de certificats auto-signés si nécessaire.

## Installation et Utilisation

### Prérequis
* [Node.js](https://nodejs.org/) installé sur votre machine.

### Installation

1.  Clonez le dépôt :
    ```bash
    git clone https://github.com/SuperKn4cky/LiveChat-Overlay.git
    cd LiveChat-Overlay
    ```

2.  Installez les dépendances :
    ```bash
    npm install
    ```

3.  Configurez l'URL de votre instance (dans main.js) :
    ```javascript
    // Cherchez cette ligne et mettez votre lien
    mainWindow.loadURL('https://votre-site.fr/client?guildId=VOTRE_ID_DISCORD');
    ```

### Lancement (Développement)

```bash
npm start
```

### Création de l'exécutable (.exe)

Pour générer un installateur ou un exécutable portable :

```bash
npm run make
# ou selon votre configuration package.json
npm run build
```

## Détails Techniques (Configuration Electron)

L'application utilise des drapeaux ("flags") Chromium spécifiques pour garantir la fluidité :

```javascript
app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required'); // Force l'autoplay
app.commandLine.appendSwitch('ignore-certificate-errors'); // Ignore SSL errors
app.commandLine.appendSwitch('disable-renderer-backgrounding'); // Empêche la mise en veille de l'onglet
```