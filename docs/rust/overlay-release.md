# Release de l'overlay Tauri LiveChat 3.0.0

Ce document décrit la production, la signature, la publication et le rollback des
installateurs de l'overlay Tauri (`apps/lc-overlay`). Il complète
[lc-overlay-shell.md](lc-overlay-shell.md) (shell Phase 8a) et
[lc-overlay-8b.md](lc-overlay-8b.md) (client WS / bridge / renderer Phase 8b).

> État actuel : le canal stable pointe la version **3.0.1** (`LiveChat`,
> `com.livechat`) et le canal bêta **3.0.0-beta.1** (`LiveChat Next`,
> `com.livechat.next`, installation side-by-side avec l'Electron 2.0.1).
> L'auto-update est **explicitement désactivé pour 3.0.0** (clé de signature
> updater placeholder, `createUpdaterArtifacts: false` dans les deux configs,
> pas d'artefacts updater) : voir §6 et §7. L'installeur est construit par la CI
> `overlay-tauri.yml` (§7.1).

## 1. Canaux et identités

| | Stable | Bêta (side-by-side) |
|---|---|---|
| Fichier de config Tauri | `tauri.conf.json` | `tauri.beta.conf.json` |
| `productName` | `LiveChat` | `LiveChat Next` |
| `identifier` | `com.livechat` | `com.livechat.next` |
| Version | `3.0.1` | `3.0.0-beta.1` |
| Dossier d'installation (NSIS currentUser) | `%LOCALAPPDATA%\LiveChat` | `%LOCALAPPDATA%\LiveChat Next` |
| Clé de désinstallation | `HKCU\...\Uninstall\LiveChat` | `HKCU\...\Uninstall\LiveChat Next` |
| Endpoint updater | `https://github.com/quentingimenez/LiveChat/releases/latest/download/latest.json` | `https://github.com/quentingimenez/LiveChat/releases/download/next/latest.json` |
| Config partagée | `%APPDATA%\LiveChat\config.json` | idem (voir §11 pour l'isolation bêta) |
| Raccourci menu Démarrer | `LiveChat.lnk` | `LiveChat Next.lnk` |

Le bêta est volontairement une **application distincte** (productName, identifier,
dossier, clé de désinstallation et valeur `Run` autostart — dérivée du
`productName` — tous différents) afin de pouvoir tester la nouvelle pile à côté de
l'installation Electron 2.0.1 sans la toucher.

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
- **n'installe rien** et ne touche pas à l'installation Electron existante.

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
  `legacy-loader.js`), `styles/**`, icônes/favicon) — pas de fichiers `overlay/src/**`
  legacy ni de préchargement Electron ;
- l'icône d'application (`icons/icon.ico`, `icons/icon.png`) et l'icône de tray ;
- le lanceur WebView2 (`webviewInstallMode: downloadBootstrapper` : l'installeur
  télécharge le runtime s'il manque, pas de dépendance admin) ;
- le désinstalleur et les entrées de registre `HKCU` (clé `LiveChat`/`LiveChat Next`).

Le bundle ne contient **aucun asset runtime Electron** (`runtime-assets/preload/dis`t
Electron) : le renderer est servi depuis le binaire via `frontendDist: "web"`.
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

## 6. Updater : signature minisign (désactivé pour 3.0.0)

État actuel (décision P0-4, 2026-11-10) : l'auto-update est **désactivé pour la
version 3.0.0**, de façon cohérente sur toute la chaîne :

- `plugins.updater.pubkey` contient un **placeholder explicite**
  (`RWQPLACEHOLDER_MINISIGN_PUBLIC_KEY_A_REMPLACER_EN_PHASE_9=`) dans
  `tauri.conf.json` et `tauri.beta.conf.json` ;
- `bundle.createUpdaterArtifacts` vaut `false` dans les deux configs : les builds
  ne produisent ni `.nsis.zip` signé ni `.sig` ;
- les workflows legacy Electron (`overlay-release.yml` sur tag `overlay-v*` avec
  `--latest`, `overlay-nightly.yml`) ont été supprimés en P0-4 ; le workflow Tauri
  `overlay-tauri.yml` (§7.1) construit l'installeur NSIS sans artefacts updater et
  ne crée qu'un **brouillon** de release : la publication reste une décision
  manuelle (§7).

L'application démarre normalement ; un clic sur « Vérifier les mises à jour »
renvoie une **erreur explicite** (clé publique invalide) au lieu de planter —
comportement voulu et documenté. Aucun client 3.0.0 ne peut donc être mis à jour
automatiquement : c'est un choix assumé pour cette première release.

Pour activer réellement l'updater :

1. Générer une paire de clés minisign (hors dépôt !) :

   ```powershell
   cargo tauri signer generate -w "$env:USERPROFILE\.tauri\livechat-3x.key"
   ```

   Noter la clé publique affichée ; la clé privée reste sur la machine de release
   (ou dans les secrets GitHub Actions `TAURI_SIGNING_PRIVATE_KEY` /
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`).

2. Renseigner `plugins.updater.pubkey` avec la clé publique dans
   `tauri.conf.json` et `tauri.beta.conf.json`.

3. Passer `bundle.createUpdaterArtifacts` à `true` dans les deux configs.

4. Builder avec la clé privée :

   ```powershell
   $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content "$env:USERPROFILE\.tauri\livechat-3x.key" -Raw
   $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "<mot de passe de la clé>"
   cargo tauri build            # produit aussi .nsis.zip + .sig
   cargo tauri build --config tauri.beta.conf.json
   ```

   La signature updater **exige** la clé privée : sans elle, `cargo tauri build`
   échoue quand `createUpdaterArtifacts: true`. C'est la raison pour laquelle la
   configuration actuelle produit des installateurs non signés pour l'updater.

5. Template `latest.json` (à publier à côté de l'installeur). La valeur
   `signature` est le **contenu intégral du fichier `.sig`** ; l'URL pointe vers
   l'archive `.nsis.zip` du binaire (pas l'installeur `.exe`), qui est ce que le
   plugin updater télécharge et vérifie :

   ```json
   {
     "version": "3.0.0",
     "notes": "LiveChat 3.0.0",
     "pub_date": "2026-11-10T00:00:00Z",
     "platforms": {
       "windows-x86_64": {
         "signature": "<contenu de LiveChat_3.0.0_x64-setup.nsis.zip.sig>",
         "url": "https://github.com/quentingimenez/LiveChat/releases/download/v3.0.0/LiveChat_3.0.0_x64-setup.nsis.zip"
       }
     }
   }
   ```

   Bêta : `version` = `3.0.0-beta.N`, architecture `windows-x86_64` identique,
   URL sous le tag `next` ; `latest.json` placé en asset de la release `next`.

6. Le client overlay vérifie l'endpoint configuré au démarrage/à la demande
   (`overlay_check_updates`), affiche la version disponible, télécharge et
   installe via le plugin updater (restart géré par Windows). En cas de pubkey
   placeholder, l'erreur est remontée dans le tray.

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

Avec updater signé, ajouter à chaque release les fichiers `.nsis.zip`, `.sig` et
`latest.json` (stable : asset `latest.json` de la release `latest` ; bêta : asset
`latest.json` de la release `next`). Le endpoint stable
`.../releases/latest/download/latest.json` suit automatiquement la dernière release
marquée « latest » (donc ne pas marquer la bêta comme latest).

### 7.1 Automatisation CI : `overlay-tauri.yml`

Le workflow `.github/workflows/overlay-tauri.yml` (runner `windows-latest`,
actions épinglées par SHA) :

- se déclenche sur push/PR touchant `apps/lc-overlay/**`, `crates/lc-protocol/**`,
  `crates/lc-domain/**`, `Cargo.*` ou le workflow lui-même, sur tag `overlay-v*`
  et à la demande ;
- installe `tauri-cli` 2.12.4 puis lance `cargo tauri build --ci` (installeur NSIS
  stable `LiveChat_<version>_x64-setup.exe`, sans `.nsis.zip` ni `.sig`) ;
- vérifie le budget de 20 Mo, calcule le SHA-256, puis fait un **smoke test** :
  installation silencieuse (`/S`) dans `%LOCALAPPDATA%\LiveChat`, lancement 8 s
  avec une config isolée (`LIVECHAT_CONFIG_PATH`), désinstallation silencieuse ;
- publie l'installeur et son `.sha256` en artefact du run (14 jours) ;
- sur tag `overlay-v<version>` : refuse un tag différent de la version de
  `tauri.conf.json`, puis crée un **brouillon** de release GitHub avec les deux
  fichiers (job séparé, seul à avoir `contents: write`). Publier le brouillon
  reste manuel ; ne jamais le marquer *Latest* pour une bêta.

La CI Rust (`rust.yml`) continue de vérifier que `apps/lc-overlay` compile (clippy,
tests) sous Linux et Windows ; le bloc Win32 (`win32.rs`, crate `windows`) n'est
compilé que sous Windows.

Pour activer l'auto-update (3.1), il faudra en plus : les secrets
`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`,
`createUpdaterArtifacts: true`, la vraie clé publique (§6) et la génération de
`latest.json` dans le job de release.

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

Consigne de la Phase 9d : **ne pas installer sur la machine de développement**
(l'installation Electron 2.0.1 doit rester intacte). Utiliser une VM Windows 11
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
   (dossiers/clés distincts) et que l'installation Electron 2.0.1 (si présente)
   n'a pas bougé.

## 10. Coexistence avec l'Electron 2.0.1 et upgrade

- Electron 2.0.1 : installé dans `%LOCALAPPDATA%\Programs\livechat`, clé
  de désinstallation `HKCU\...\Uninstall\ec5c36a6-04f8-5099-a257-77071e875a50`,
  autostart `HKCU\...\Run\livechat`.
- Tauri stable 3.0.0 : `%LOCALAPPDATA%\LiveChat`, clé `LiveChat`, autostart
  `LiveChat` (même nom que l'Electron à la casse près → les deux applications ne
  doivent pas activer l'autostart simultanément ; le bêta utilise `LiveChat Next`,
  donc pas de collision).
- Un **upgrade in-place** Electron → Tauri n'est pas possible par l'installeur
  (clés de désinstallation différentes) : le chemin officiel est
  bêta side-by-side → validation → désinstallation explicite d'Electron 2.0.1 →
  installation de LiveChat 3.0.0. La config `%APPDATA%\LiveChat\config.json` est
  partagée et donc reprise automatiquement.
- Side-by-side stable + bêta : possible (identités distinctes) ; utile pour
  comparer, mais la config `%APPDATA%\LiveChat` est partagée (voir §11).

## 11. Configuration et migration

Décision d'architecture (Phase 0/POC, conservée en 8a/8b) : le chemin de config
est **forcé** à `%APPDATA%\LiveChat\config.json` (le même fichier que l'Electron),
et non `app_config_dir()` (`%APPDATA%\com.livechat`). Conséquences :

- pas de migration de chemin à faire lors de l'upgrade Electron → Tauri : le
  fichier est déjà au bon endroit, les 16 champs et normalisations legacy sont
  repris tels quels ;
- au premier écrit, un `.bak` one-shot (`config.json.bak`) est créé : il sert de
  filet de sécurité et de point de restauration pour un **rollback** vers
  Electron ;
- la variable `LIVECHAT_CONFIG_PATH` permet d'isoler complètement une instance
  (utile pour tester le bêta sans toucher la config Electron/stable) :
  `$env:LIVECHAT_CONFIG_PATH = "$env:TEMP\livechat-next\config.json"`.

> Écart assumé vs la lettre de Task 26 : il n'y a **pas** de migration
> `%APPDATA%\LiveChat` → `%APPDATA%\com.livechat` ; le choix du chemin partagé est
> documenté dans `overlay-bridge.md` §8.6 et conservé pour permettre le rollback.

## 12. Rollback

1. **Bêta** : désinstaller LiveChat Next (Applications installées) ; l'Electron
   2.0.1 et le stable éventuel ne sont pas affectés.
2. **Stable 3.0.0 → Electron 2.0.1** : désinstaller LiveChat via Applications
   installées (per-user, pas d'admin), réinstaller `LiveChat-Setup-2.0.1.exe`
   (electron-builder, clé GUID conservée), la config `%APPDATA%\LiveChat` est
   intacte ; si besoin restaurer `config.json` depuis `config.json.bak`.
3. **Config corrompue** : fermer l'app, copier `%APPDATA%\LiveChat\config.json.bak`
   vers `config.json`, relancer (les valeurs legacy par défaut sont appliquées
   sinon).
4. Ne jamais supprimer `%APPDATA%\LiveChat` lors d'un rollback : c'est ce qui
   permet à Electron et Tauri de partager l'appairage et les réglages.

## 13. Écarts et limites connus

| Sujet | État | Impact / mitigation |
|---|---|---|
| Signature Authenticode | absente | SmartScreen peut afficher un avertissement au premier lancement ; prévoir un certificat de signature de code en Phase 9 finale. |
| Clé updater | placeholder (`createUpdaterArtifacts: false`, aucun workflow de release) | **auto-update explicitement désactivé pour 3.0.0** (§6) ; activation = travail 3.1 (clé réelle, artifacts signés, workflow Tauri). |
| Workflows de release | Electron legacy supprimés (`overlay-release.yml`, `overlay-nightly.yml`) ; aucun workflow Tauri | publication 3.0.0 manuelle (§7) ; un seul propriétaire de tags par artefact (AUD-A7-003). |
| Installation locale | **non testée** (consigne : ne pas installer) | procédure et smoke test VM décrits §9 ; à exécuter en Phase 9 finale. |
| WebView2 | **requis** | Windows 11 : présent. Windows 10 : `downloadBootstrapper` télécharge le runtime à l'installation (réseau requis). Variante hors-ligne possible : `webviewInstallMode: { "type": "offlineInstaller" }` (installeur plus gros, ~100 Mo). |
| MSI / machine-wide | non produit | NSIS per-user choisi (pas d'admin, updater NSIS, cohérent avec Electron electron-builder per-user) ; `cargo tauri build --bundles msi` possible mais nécessite WiX et une stratégie d'élévation. |
| Pages de dev | hors bundle | `bridge-test.html` + `README.md` déplacés dans `apps/lc-overlay/dev/` (AUD-A6-015) ; `frontendDist` reste `web/`, vérifié par scan du binaire release. |
| Autostart vs Electron | valeur `Run` `LiveChat` ≡ `livechat` (casse ignorée) | ne pas activer l'autostart Tauri tant qu'Electron 2.0.1 est encore installé ; bêta `LiveChat Next` sans collision. |
| Config partagée Electron/Tauri | voulue | permet le rollback ; isoler une instance de test via `LIVECHAT_CONFIG_PATH`. |
| Code de release | non commité | artefacts laissés dans `target/release/bundle/nsis/` pour le Lead. |

## 14. Checklist « release finale » (Phase 9)

- [ ] (3.1 uniquement, auto-update désactivé en 3.0.0 — §6) Clé minisign réelle
      générée, pubkey injecté, `createUpdaterArtifacts: true`, workflow Tauri de
      release ajouté avec actions épinglées par SHA.
- [ ] Certificat Authenticode disponible (optionnel mais recommandé) et commande
      `signtool` intégrée au pipeline.
- [ ] `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test -p lc-overlay` verts.
- [ ] Builds stable + bêta, tailles notées, `latest.json` généré et signé.
- [ ] Smoke install VM (stable + bêta + désinstallation + config préservée).
- [ ] Publication GitHub `quentingimenez/LiveChat` (stable releaseType release,
      bêta pré-release tag `next`).
- [ ] Upgrade in-place validé sur VM : Electron 2.0.1 → bêta side-by-side →
      désinstallation Electron → stable 3.0.0 → auto-update 3.0.1.

## 15. Durcissement P1-11 (audit A6) — impact release

Les correctifs de la task-43 (`AUD-A6-002/003/004/005/008/015`) modifient le
contenu et le comportement du bundle 3.0.0 :

- **Updater explicitement désactivé dans le code** : en plus de
  `createUpdaterArtifacts: false` et de la pubkey placeholder, `updater.rs`
  expose `UPDATER_ENABLED_FOR_RELEASE = false`. Les commandes
  `overlay_check_updates` / `overlay_install_update` renvoient l'état `disabled`
  sans appel réseau ni erreur de clé. Pour activer l'auto-update (3.1) : clé
  minisign réelle, `createUpdaterArtifacts: true`, artefacts `.nsis.zip`/`.sig`,
  `latest.json` signé, puis passer la constante à `true` **et** vérifier la
  signature de bout en bout sur VM.
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
  du répertoire `%APPDATA%\LiveChat` : voir §11 (chemin partagé avec Electron
  2.0.1, isolation par `LIVECHAT_CONFIG_PATH`).
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


