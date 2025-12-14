# OverlayClient – LiveChatCaCaBox Companion

**OverlayClient** est une application de bureau **Electron** conçue pour fonctionner de concert avec le projet **LiveChatCaCaBox**.

C’est un **overlay transparent**, *click-through* et **toujours au premier plan** qui permet d’afficher des alertes et de jouer des médias (vidéos, sons, YouTube) déclenchés via des commandes **Discord**.

---

## 📥 Téléchargement (Windows .exe)

Un exécutable Windows est disponible dans les **Releases GitHub**.

### Page de release *nightly*
https://github.com/Streamer/LiveChat-Overlay/releases/tag/nightly

### Lien direct de téléchargement  
*(si l’asset s’appelle `LiveChat.exe`)*
https://github.com/Streamer/LiveChat-Overlay/releases/download/nightly/LiveChat.exe

---

## ❓ Pourquoi ce client ?

Contrairement à une simple **Browser Source** dans OBS ou à un navigateur Chrome classique, ce client a été développé pour contourner plusieurs limitations techniques spécifiques :

### 🔧 Gestion des Headers Discord
Discord force le téléchargement des fichiers  
(`Content-Disposition: attachment`).

➡️ Le client intercepte les requêtes réseau pour modifier les en-têtes à la volée, permettant la **lecture en streaming direct** des fichiers audio/vidéo hébergés sur Discord.

### ▶️ Autoplay Forcé
Contourne les politiques strictes des navigateurs (Chrome / Edge) qui empêchent la lecture automatique des médias sans interaction utilisateur.

### 🪟 Transparence Totale
Fenêtre **sans bordure**, **transparente**, et **click-through** pour ne pas gêner l’utilisation du PC.

---

## 🧠 Technologies Utilisées

- **Electron** : Framework principal pour créer l’application de bureau
- **Node.js** : Backend de l’application
- **Electron Session & WebRequest API** : Utilisées pour manipuler les en-têtes HTTP et corriger les types MIME

---

## ✨ Fonctionnalités Clés

- **Mode Fantôme**  
  La fenêtre est invisible tant qu’aucun média ne joue.  
  Elle ignore les clics de souris (vous pouvez jouer / travailler en dessous).

- **Patch Audio Discord**  
  Correction automatique des headers `Content-Disposition` et `Content-Type` pour les fichiers venant de :
  - cdn.discordapp.com
  - media.discordapp.net

- **Support Multi-écrans**  
  Une icône dans la **System Tray** permet :
  - de déplacer l’overlay sur l’écran de votre choix
  - de régler le volume

- **Sécurité assouplie**  
  Configuration spécifique pour :
  - autoriser le contenu mixte (HTTP / HTTPS)
  - ignorer les erreurs de certificats auto-signés si nécessaire

---

## ⚙️ Configuration (URL de votre instance)

Dans `main.js`, configurez l’URL du client web à afficher :

    mainWindow.loadURL('https://votre-site.fr/client?guildId=VOTRE_ID_DISCORD');

---

## 🧑‍💻 Développement (depuis les sources)

### Prérequis
- **Node.js** installé sur votre machine

### Installation

    git clone https://github.com/Streamer/LiveChat-Overlay.git
    cd LiveChat-Overlay
    npm install

### Lancement (Développement)

    npm start

### Build local (.exe)

    npm run dist

---

## 🛠️ Détails Techniques (Configuration Electron)

    app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required'); // Force l'autoplay
    app.commandLine.appendSwitch('ignore-certificate-errors'); // Ignore les erreurs SSL
    app.commandLine.appendSwitch('disable-renderer-backgrounding'); // Empêche la mise en veille de l'onglet
