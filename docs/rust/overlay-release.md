# Release de l'overlay Tauri LiveChat 3.0.0

Ce document décrit la production, la signature, la publication et le rollback des
installateurs de l'overlay Tauri (`apps/lc-overlay`). Il complète
[lc-overlay-shell.md](lc-overlay-shell.md) (shell Phase 8a) et
[lc-overlay-8b.md](lc-overlay-8b.md) (client WS / bridge / renderer Phase 8b).

> État actuel : le canal stable pointe la version **3.5.0** (`LiveChat`,
> `com.livechat`) et le canal bêta **3.0.0-beta.1** (`LiveChat Next`,
> `com.livechat.next`, installation side-by-side avec le stable).
> L'auto-update est **actif à partir de 3.1.1** : installateur signé par la clé
> minisign du projet, `latest.json` joint à chaque release (§6). Les 3.0.x et 3.1.0
> n'ont pas d'updater : ils passent une fois à la main sur une version récente.
> L'installeur est construit par la CI `overlay-tauri.yml` (§7.1).

## 1. Canaux et identités

| | Stable | Bêta (side-by-side) |
|---|---|---|
| Fichier de config Tauri | `tauri.conf.json` | `tauri.beta.conf.json` |
| `productName` | `LiveChat` | `LiveChat Next` |
| `identifier` | `com.livechat` | `com.livechat.next` |
| Version | `3.5.0` | `3.0.0-beta.1` |
| Dossier d'installation (NSIS currentUser) | `%LOCALAPPDATA%\LiveChat` | `%LOCALAPPDATA%\LiveChat Next` |
| Clé de désinstallation | `HKCU\...\Uninstall\LiveChat` | `HKCU\...\Uninstall\LiveChat Next` |
| Endpoint updater | `https://github.com/quentingimenez/LiveChat/releases/latest/download/latest.json` | `https://github.com/quentingimenez/LiveChat/releases/download/next/latest.json` |
| Config partagée | `%APPDATA%\LiveChat\config.json` | idem (voir §11 pour l'isolation bêta) |
| Raccourci menu Démarrer | `LiveChat.lnk` | `LiveChat Next.lnk` |

Le bêta est volontairement une **application distincte** (productName, identifier,
dossier, clé de désinstallation et valeur `Run` autostart — dérivée du
`productName` — tous différents) afin de pouvoir tester la nouvelle pile à côté de
l'installation stable sans la toucher.

## 2. Prérequis

- Windows 10/11 x64, **WebView2 Runtime** (Evergreen ; présent par défaut sur
  Windows 11, installé par le bootstrapper NSIS sur Windows 10 si absent — voir §13).
- Rust `stable` (1.99.1 utilisée ici) + toolchain `x86_64-pc-windows-msvc` +
  Visual Studio Build Tools (linker MSVC), `cargo` dans le `PATH`.
- `tauri-cli` 2.x : `cargo install tauri-cli --locked` (2.12.4 utilisée ici).
- NSIS : téléchargé automatiquement par `tauri build` au premier bundle
  (`nsis-3.13.zip` + `nsis_tauri_utils.dll`), donc accès réseau requis au premier build.
- (Publication uniquement) `gh` CLI authentifié sur le dépôt
  `quentingimenez/LiveChat`, et pour l'updater la clé privée minisign (§6).

Environnement recommandé pour les builds locaux :

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_BUILD_JOBS = 4   # évite de saturer la machine pendant les builds multi-agents
```

## 3. Commandes de build

Depuis la racine du dépôt (le `target/` du workspace est à la racine) :

```powershell
# Stable uniquement
cargo tauri build --manifest-path apps/lc-overlay/Cargo.toml   # (depuis apps/lc-overlay : cargo tauri build)

# Bêta side-by-side
cargo tauri build --config tauri.beta.conf.json               # depuis apps/lc-overlay

# Les deux canaux d'un coup, avec collecte des tailles/temps/versions
powershell -ExecutionPolicy Bypass -File apps/lc-overlay/scripts/release-build.ps1 -Channel both
```

Le script `apps/lc-overlay/scripts/release-build.ps1` :

- lance `cargo tauri build` pour chaque canal (log dans
  `target/release/bundle/nsis/build-<canal>.log`) ;
- mesure la durée, liste les installateurs trouvés, la taille du binaire
  `target/release/lc-overlay.exe` et les métadonnées PE (version produit) ;
- écrit `target/release/bundle/nsis/release-summary-<canal>.json` (chemins,
  tailles, temps, notes) ;
- **n'installe rien**.

## 4. Artefacts produits

| Artefact | Chemin |
|---|---|
| Binaire release | `target/release/lc-overlay.exe` (écrasé par le dernier canal buildé) |
| Copie binaire stable | `target/release/bundle/nsis/lc-overlay-stable.exe` |
| Copie binaire bêta | `target/release/bundle/nsis/lc-overlay-beta.exe` |
| Installeur stable | `target/release/bundle/nsis/LiveChat_3.0.0_x64-setup.exe` |
| Installeur bêta | `target/release/bundle/nsis/LiveChat Next_3.0.0-beta.1_x64-setup.exe` |
| Logs de build | `target/release/bundle/nsis/build-stable.log`, `build-beta.log` |
| Synthèse | `target/release/bundle/nsis/release-summary-stable.json`, `release-summary-beta.json` |

Les tailles/temps mesurés figurent dans les fichiers `release-summary-*.json`
(et sont repris dans le rapport de la tâche de packaging).

### 4.1 Chiffres mesurés (2026-11-10, machine du Lead)

| | Stable `LiveChat` | Bêta `LiveChat Next` |
|---|---|---|
| Version | 3.0.0 | 3.0.0-beta.1 |
| Build from scratch | 218,9 s | 58,9 s (deps déjà compilées) |
| Rebuild incrémental (run final) | 60,5 s | 64,5 s |
| Installeur | 4 185 840 o (3,99 Mio) | 4 188 395 o (3,99 Mio) |
| Binaire applicatif | 18 373 120 o (17,52 Mio) | 18 373 632 o (17,52 Mio) |
| SHA-256 installeur | `72F47B1909DE451004E493C39069A055B8557098022CD0A9A492E8015C51B252` | `87DC1AE9E931E2A42D42E2181ADBAB7A5231A4B206EBE58780D5EA81A808FFF4` |
| SHA-256 binaire | `7A41A8C817ABBF65F4E20DC319062C944EB8C21C82A5C1BF351B8D15340CEAF0` | `677599DA5094044940BEBC3336097A9823CF9350A06047D2F85F0DBF165A7E53` |

Les deux installeurs restent **sous le budget de 20 Mo** (Phase 0 : installeur < 20 Mo,
binaire < 25 Mo).

## 5. Contenu des bundles

L'installeur NSIS Tauri (currentUser, `downloadBootstrapper`) contient :

- le binaire applicatif `lc-overlay.exe` avec le frontend embarqué (tout
  `apps/lc-overlay/web/` : `overlay.html`, `board.html`, `pairing.html`,
  `bridge.js`, `renderer-errors.js`, css/js legacy, `dist/renderer/**` (shims ESM +
  `legacy-loader.js`), `styles/**`, icônes/favicon) ;
- l'icône d'application (`icons/icon.ico`, `icons/icon.png`) et l'icône de tray ;
- le lanceur WebView2 (`webviewInstallMode: downloadBootstrapper` : l'installeur
  télécharge le runtime s'il manque, pas de dépendance admin) ;
- le désinstalleur et les entrées de registre `HKCU` (clé `LiveChat`/`LiveChat Next`).

Le renderer est servi depuis le binaire via `frontendDist: "web"`.
Deux pages de développement (`web/bridge-test.html`, `web/README.md`) sont
embarquées par simplicité car `frontendDist` emporte tout le dossier `web/`
(~35 Ko, sans impact fonctionnel) ; voir §13.

Vérification rapide (sans installer) après build :

```powershell
Get-Item target\release\lc-overlay.exe | Select-Object Length,LastWriteTime
Get-Item target\release\bundle\nsis\*.exe | Select-Object Name,Length
(Get-Item target\release\bundle\nsis\LiveChat_3.0.0_x64-setup.exe).VersionInfo |
    Select-Object ProductName,ProductVersion,CompanyName,FileDescription
```

Preuves collectées sur les artefacts de la build du 2026-11-10 :

- métadonnées PE : installeur stable `ProductName=LiveChat`, `ProductVersion=3.0.0` ;
  installeur bêta `ProductName=LiveChat Next`, `ProductVersion=3.0.0-beta.1` ;
  binaires : `FileVersion=3.0.0` / `3.0.0-beta.1`, `CompanyName=LiveChat` (publisher configuré) ;
- scan binaire de `lc-overlay-stable.exe` (ASCII/UTF-16) : présence de
  `overlay.html`, `bridge.js`, `renderer-errors.js`, `overlay://play`, `WebView2` —
  confirme que le frontend `web/` et l'événement de bridge sont bien embarqués dans
  l'exécutable, et que la pile WebView2 est référencée (le payload de l'installeur
  NSIS est lui-même compressé, d'où peu de marqueurs visibles) ;
- smoke run **du binaire release** (sans installation, config temporaire) : process
  vivant 7 s, `renderer overlay prêt`, `.bak` créé, tray `main-tray` créé, overlay
  positionné `0,0 2560×1440`, styles `0x080C0138 → 0x080C01B8`, raccourcis
  réappliqués, RSS ~46,9 Mo, aucun process résiduel après kill.

## 6. Updater : signature minisign

### 6.1 Fonctionnement dans l'overlay

- `plugins.updater.endpoints` pointe vers
  `https://github.com/quentingimenez/LiveChat/releases/latest/download/latest.json` :
  la release marquée *Latest* fait foi (bêta : `releases/download/next/latest.json`).
- `updater.rs` vérifie dès l'ouverture de la board (à chaque ouverture), au
  démarrage puis toutes les 6 h (ces deux-là en build release seulement ; le
  bouton « Rechercher une mise à jour » des Paramètres marche partout). Une version
  plus récente est téléchargée et sa signature vérifiée en arrière-plan, puis
  gardée en mémoire.
- L'installation reste un clic de l'utilisateur, pour ne jamais couper un live :
  entrée tray « Redémarrer et installer la version X » ou bouton des Paramètres.
  L'installeur NSIS tourne en mode passif (`installMode: passive`, barre de
  progression sans question) et relance LiveChat.
- L'état (`idle`, `checking`, `downloading`, `up_to_date`, `ready`, `error`) est
  exposé dans `settings.update` (`overlay://device-settings`).

### 6.2 Clé de signature

Paire minisign générée le 2026-11-14 avec
`cargo tauri signer generate -w "$env:USERPROFILE\.tauri\livechat-overlay.key"` :

- clé publique : `plugins.updater.pubkey` de `tauri.conf.json` et
  `tauri.beta.conf.json` (copie dans `livechat-overlay.key.pub`) ;
- clé privée et mot de passe : `%USERPROFILE%\.tauri\livechat-overlay.key` et
  `livechat-overlay.key.password`, **hors dépôt**, et dans les secrets GitHub
  Actions `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Perdre la clé privée coupe l'auto-update de tous les overlays installés : il
faudrait republier une version à installer à la main avec une nouvelle clé
publique. La garder aussi dans un gestionnaire de mots de passe.

### 6.3 Builds

`bundle.createUpdaterArtifacts` vaut `true` : `cargo tauri build` signe
l'installeur et écrit `<installeur>.exe.sig` à côté ; sans clé privée il échoue.
Pour un build local sans clé (devtest, essais) :

```powershell
cargo tauri build --config tauri.unsigned.conf.json   # depuis apps/lc-overlay
```

Build signé local :

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content "$env:USERPROFILE\.tauri\livechat-overlay.key" -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = Get-Content "$env:USERPROFILE\.tauri\livechat-overlay.key.password" -Raw
cargo tauri build
```

### 6.4 `latest.json`

Généré par la CI sur tag (§7.1) et joint à la release. `signature` est le contenu
du `.sig`, `url` l'installeur `.exe` de la même release (format updater Tauri 2,
plus de `.nsis.zip`) :

```json
{
  "version": "3.5.0",
  "notes": "LiveChat overlay 3.5.0",
  "pub_date": "2026-11-14T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contenu de LiveChat_3.5.0_x64-setup.exe.sig>",
      "url": "https://github.com/quentingimenez/LiveChat/releases/download/overlay-v3.5.0/LiveChat_3.5.0_x64-setup.exe"
    }
  }
}
```

Tant que le brouillon n'est pas publié et marqué *Latest*, l'endpoint renvoie
toujours l'ancienne release : la publication manuelle est le feu vert de la mise
à jour pour tous les overlays.

## 7. Publication GitHub (dépôt `quentingimenez/LiveChat`)

```powershell
gh auth status   # doit être authentifié sur le dépôt

# Stable (releaseType release, tag v3.0.0)
gh release create v3.0.0 `
  --repo quentingimenez/LiveChat `
  --title "LiveChat 3.0.0" `
  --notes-file docs/rust/RELEASE_NOTES_3.0.0.md `
  target/release/bundle/nsis/LiveChat_3.0.0_x64-setup.exe

# Bêta side-by-side (pré-release, tag next — même tag réutilisé à chaque bêta)
gh release create next `
  --repo quentingimenez/LiveChat `
  --title "LiveChat Next (bêta)" `
  --prerelease --notes "Bêta LiveChat Next" `
  target/release/bundle/nsis/"LiveChat Next_3.0.0-beta.1_x64-setup.exe"
```

Chaque release stable porte l'installeur et `latest.json` (§6.4) ; une bêta
publiée à la main porte son propre `latest.json` dans la release `next`. Le endpoint stable
`.../releases/latest/download/latest.json` suit automatiquement la dernière release
marquée « latest » (donc ne pas marquer la bêta comme latest).

### 7.1 Automatisation CI : `overlay-tauri.yml`

Le workflow `.github/workflows/overlay-tauri.yml` (runner `windows-latest`,
actions épinglées par SHA) :

- se déclenche sur push/PR touchant `apps/lc-overlay/**`, `crates/lc-protocol/**`,
  `crates/lc-domain/**`, `Cargo.*` ou le workflow lui-même, sur tag `overlay-v*`
  et à la demande ;
- installe `tauri-cli` 2.12.4 puis lance `cargo tauri build --ci` : hors tag avec
  `--config tauri.unsigned.conf.json` (pas de clé, pas de `.sig`), sur tag avec les
  secrets de signature (échec explicite s'ils manquent) ;
- vérifie le budget de 20 Mo, calcule le SHA-256, puis fait un **smoke test** :
  installation silencieuse (`/S`) dans `%LOCALAPPDATA%\LiveChat`, lancement 8 s
  avec une config isolée (`LIVECHAT_CONFIG_PATH`), désinstallation silencieuse ;
- publie l'installeur et son `.sha256` en artefact du run (14 jours) ;
- sur tag `overlay-v<version>` : refuse un tag différent de la version de
  `tauri.conf.json`, écrit `latest.json` à partir du `.sig`, puis crée un
  **brouillon** de release GitHub avec l'installeur, son `.sha256` et
  `latest.json` (job séparé, seul à avoir `contents: write`). Publier le brouillon
  et le marquer *Latest* déclenche la mise à jour des overlays installés ; ne
  jamais marquer une bêta *Latest*.

La CI Rust (`rust.yml`) continue de vérifier que `apps/lc-overlay` compile (clippy,
tests) sous Linux et Windows ; le bloc Win32 (`win32.rs`, crate `windows`) n'est
compilé que sous Windows.

## 8. Politique de versions

- **Stable** : semver `3.x.y`, `productName` LiveChat ; toute nouvelle version
  doit dépasser la version installée pour que l'updater la propose.
- **Bêta** : `3.x.y-beta.N` sous `LiveChat Next` / `com.livechat.next`, publiée en
  pré-release tag `next` ; peut être installée en parallèle du stable.
- Le serveur impose une **version minimale d'overlay** (`MIN_OVERLAY_VERSION`,
  défaut `3.0.0`) : un client trop ancien reçoit `close 4403` avec
  `{"minVersion":"x.y.z"}` et n'essaie pas de se reconnecter en boucle.
- La version embarquée dans le binaire est `version` de `tauri.conf.json`, alignée
  sur `version` du `Cargo.toml` du crate (le reste du workspace reste en `3.0.0`).

## 9. Smoke test de l'installeur (VM, jamais sur la machine de build)

Consigne de la Phase 9d : **ne pas installer sur la machine de développement**.
Utiliser une VM Windows 11
propre ou Windows Sandbox :

1. Copier `LiveChat_3.0.0_x64-setup.exe` dans la VM.
2. Installer silencieusement : `.\LiveChat_3.0.0_x64-setup.exe /S` (NSIS Tauri
   supporte `/S`), ou en interactif.
3. Vérifier : dossier `%LOCALAPPDATA%\LiveChat`, entrée « LiveChat » dans
   Applications installées, raccourci menu Démarrer, `HKCU\...\Uninstall\LiveChat`.
4. Lancer : overlay visible, tray présent, menu « Appairer l'overlay » → code
   d'appairage → serveur → overlay connecté (`status=connecté` dans le tray).
5. Vérifier la config `%APPDATA%\LiveChat\config.json` et `config.json.bak`.
6. Désinstaller (Applications installées → LiveChat, ou
   `%LOCALAPPDATA%\LiveChat\uninstall.exe /S`) ; vérifier que le dossier
   d'installation a disparu et que la config `%APPDATA%\LiveChat` est conservée.
7. Bêta : mêmes étapes ; vérifier la coexistence avec l'installateur stable
   (dossiers/clés distincts).

## 10. Coexistence stable / bêta

- Stable : `%LOCALAPPDATA%\LiveChat`, clé `LiveChat`, autostart `LiveChat` ; bêta :
  `LiveChat Next` partout, donc pas de collision.
- Side-by-side stable + bêta : possible (identités distinctes) ; utile pour
  comparer, mais la config `%APPDATA%\LiveChat` est partagée (voir §11).

## 11. Configuration et migration

Décision d'architecture (Phase 0/POC, conservée en 8a/8b) : le chemin de config
est **forcé** à `%APPDATA%\LiveChat\config.json` (le fichier qu'utilisait déjà
l'overlay 2.x), et non `app_config_dir()` (`%APPDATA%\com.livechat`). Conséquences :

- au premier écrit, un `.bak` one-shot (`config.json.bak`) est créé : il sert de
  filet de sécurité et de point de restauration ;
- la variable `LIVECHAT_CONFIG_PATH` permet d'isoler complètement une instance
  (utile pour tester le bêta sans toucher la config stable) :
  `$env:LIVECHAT_CONFIG_PATH = "$env:TEMP\livechat-next\config.json"`.

> Écart assumé vs la lettre de Task 26 : il n'y a **pas** de migration
> `%APPDATA%\LiveChat` → `%APPDATA%\com.livechat` ; le choix du chemin est
> documenté dans `overlay-bridge.md` §8.6.

## 12. Rollback

1. **Bêta** : désinstaller LiveChat Next (Applications installées) ; le stable
   éventuel n'est pas affecté.
2. **Stable → version précédente** : désinstaller LiveChat via Applications
   installées (per-user, pas d'admin), réinstaller l'installeur 3.x précédent depuis
   sa release GitHub ; la config `%APPDATA%\LiveChat` est intacte ; si besoin
   restaurer `config.json` depuis `config.json.bak`.
3. **Config corrompue** : fermer l'app, copier `%APPDATA%\LiveChat\config.json.bak`
   vers `config.json`, relancer (les valeurs legacy par défaut sont appliquées
   sinon).
4. Ne jamais supprimer `%APPDATA%\LiveChat` lors d'un rollback : il contient
   l'appairage et les réglages.

## 13. Écarts et limites connus

| Sujet | État | Impact / mitigation |
|---|---|---|
| Signature Authenticode | absente | SmartScreen peut afficher un avertissement au premier lancement ; prévoir un certificat de signature de code en Phase 9 finale. |
| Clé updater | placeholder (`createUpdaterArtifacts: false`, aucun workflow de release) | **auto-update explicitement désactivé pour 3.0.0** (§6) ; activation = travail 3.1 (clé réelle, artifacts signés, workflow Tauri). |
| Workflows de release | `overlay-tauri.yml` seul (§7.1) | un seul propriétaire de tags par artefact (AUD-A7-003). |
| Installation locale | **non testée** (consigne : ne pas installer) | procédure et smoke test VM décrits §9 ; à exécuter en Phase 9 finale. |
| WebView2 | **requis** | Windows 11 : présent. Windows 10 : `downloadBootstrapper` télécharge le runtime à l'installation (réseau requis). Variante hors-ligne possible : `webviewInstallMode: { "type": "offlineInstaller" }` (installeur plus gros, ~100 Mo). |
| MSI / machine-wide | non produit | NSIS per-user choisi (pas d'admin, updater NSIS) ; `cargo tauri build --bundles msi` possible mais nécessite WiX et une stratégie d'élévation. |
| Pages de dev | hors bundle | `bridge-test.html` + `README.md` déplacés dans `apps/lc-overlay/dev/` (AUD-A6-015) ; `frontendDist` reste `web/`, vérifié par scan du binaire release. |
| Code de release | non commité | artefacts laissés dans `target/release/bundle/nsis/` pour le Lead. |

## 14. Checklist « release finale » (Phase 9)

- [x] Clé minisign réelle générée, pubkey injecté, `createUpdaterArtifacts: true`,
      `latest.json` généré par la CI de release (§6, 2026-11-14).
- [ ] Certificat Authenticode disponible (optionnel mais recommandé) et commande
      `signtool` intégrée au pipeline.
- [ ] `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test -p lc-overlay` verts.
- [ ] Builds stable + bêta, tailles notées, `latest.json` généré et signé.
- [ ] Smoke install VM (stable + bêta + désinstallation + config préservée).
- [ ] Publication GitHub `quentingimenez/LiveChat` (stable releaseType release,
      bêta pré-release tag `next`).

## 15. Durcissement P1-11 (audit A6) — impact release

Les correctifs de la task-43 (`AUD-A6-002/003/004/005/008/015`) modifient le
contenu et le comportement du bundle 3.0.0 :

- **Updater désactivé dans le code pour 3.0.x et 3.1.0** (constante
  `UPDATER_ENABLED_FOR_RELEASE`), retirée en 3.1.1 quand l'auto-update a été
  activé (§6).
- **Harnais de dev hors bundle** : `apps/lc-overlay/dev/bridge-test.html` et
  `dev/README.md` ne sont plus dans `frontendDist` ; le binaire release ne doit
  plus contenir `bridge-test.html` (vérification : scan ASCII/UTF-16 du binaire).
- **CSP resserrée** : plus de `platform.twitter.com` (`script-src 'self'`,
  `frame-src 'none'`) ; le rendu de carte tweet est statique.
- **Capabilities par fenêtre** : `capabilities/{overlay,board,pairing}.json`
  remplacent `default.json` ; la matrice IPC est appliquée côté Rust.
- **Appairage/WS HTTPS** : `http://` refusé par défaut ; la dérogation
  `LIVECHAT_INSECURE_TLS=1` est limitée aux builds debug et ne doit jamais être
  utilisée pour un installeur de release.
- **Board** : authentification par en-tête `Authorization: Bearer` via
  `window.livechatOverlay.authorizedFetch` ; plus de `?token=` dans les URLs.

## 16. Durcissement P2-E (audit AUD-COR-17 / verification-2) — impact release

Les correctifs de la task-52 (`AUD-A6-006/007/009/010/011/016`, nits O-005..O-008)
modifient encore le contenu et le comportement du bundle 3.0.0 :

- **`enabled=false` strict** : au boot comme sur `show_overlay`, aucune fenêtre
  overlay n'est affichée tant que l'overlay n'est pas activé **et** appairé
  (`windows::overlay_should_show`). Le tray reflète l'état (« Overlay désactivé
  (aucune lecture) » / « Appairage requis »), l'entrée « Afficher l'overlay » est
  grisée et la désactivation coupe la lecture (`overlay://stop`) puis masque la
  fenêtre. Aucun événement `play`/`stop` n'est bufferisé ni rejoué dans cet état.
- **Config atomique** : `config.json` est écrit dans un fichier temporaire voisin
  puis renommé (`replace_file`, 3 essais) ; la copie `.bak` suit le même schéma.
  Plus de fichier partiellement écrit en cas de coupure. Permissions et partage
  du répertoire `%APPDATA%\LiveChat` : voir §11 (isolation par
  `LIVECHAT_CONFIG_PATH`).
- **CSP resserrée** : `img-src`/`media-src` conservent `http:`/`https:`/`blob:`
  (URLs média signées dynamiques) mais plus de `asset:` ni de
  `http://asset.localhost` ; `connect-src` ne liste plus `ws:`/`wss:` (le
  WebSocket vit côté Rust). Le renderer reste compatible (aucun accès direct au
  réseau autre que `fetch` board/média via `authorizedFetch`).
- **`withGlobalTauri: false`** : bridge.js utilise `window.__TAURI_INTERNALS__`
  (voie officielle Tauri 2) ; plus d'API `window.__TAURI__` exposée au contenu
  des pages. Les pages de dev hors bundle ne sont pas concernées.
- **Diagnostics bridge conditionnels** : `window.__livechatOverlayBridge`,
  `__bridgeCalls`, `__bridgeLastEvent`, `__bridgeMockEmit` ne sont créés qu'en
  mode mock (harnais `dev/bridge-test.html`) ou si
  `window.__LIVECHAT_BRIDGE_DEBUG = true` est posé avant `bridge.js`. Un
  installeur release n'expose donc aucun journal d'appels.
- **Matrice IPC complétée** : `overlay_install_update` est autorisée pour la
  fenêtre board (bouton de mise à jour) et reste refusée pour l'overlay ; la
  matrice par fenêtre est testée (`command_matrix_is_window_scoped`).

Contrôle release associé : `cargo tauri build --debug --no-bundle` (ou release),
puis scan du binaire pour `bridge-test.html`, `platform.twitter.com`,
`twitter-widgets-js` (absents) et présence de `overlay.html`/`renderer-errors.js`.


