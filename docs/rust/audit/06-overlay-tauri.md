# Audit A6 — Overlay Tauri : fenêtres Win32, CSP/XSS, bridge, WS, config/token, packaging

| Champ | Valeur |
|---|---|
| Date | 2026-11-10 |
| Branche | `feat/rust-rebuild` (dépôt `C:\Users\SuperKnacky\code\LiveChat`) |
| Auditeur | `db-core` (lane A6, audit indépendant — n'a pas écrit `apps/lc-overlay` ni ses docs) |
| Périmètre | `apps/lc-overlay/**` (`src/**`, `web/**` dont `bridge.js`, renderer legacy, `tauri.conf.json`, `tauri.beta.conf.json`, `capabilities/default.json`, `scripts/release-build.ps1`, `build.rs`, `Cargo.toml`) ; docs `docs/rust/{overlay-bridge,lc-overlay-8b,overlay-release,lc-overlay-shell,poc-tauri-report}.md` ; binaire release déjà construit `target/release/lc-overlay.exe` et installateurs NSIS présents |
| Méthode | Lecture statique critique exhaustive + reproduction dynamique contrôlée du binaire release (aucun build, aucune installation) ; vérification croisée par `grep` ; hash SHA-256 avant/après pour prouver la non-altération de l'installation Electron |
| Artefacts | `.phase0/audit/A6/` (`evidence-01-binary.txt`, `evidence-02-run.txt`, `config-temp.json`, `config-temp.json.bak`) — répertoire ignoré par git |
| Code modifié | **Aucun** (seul livrable écrit : ce rapport) ; aucun commit |

> Objet de l'audit : établir ce que garantit réellement l'overlay Tauri en release
> (fenêtres topmost/click-through, isolation XSS du contenu serveur, surface IPC,
> transport du jeton d'appairage, chaîne de mise à jour), et ce qui relève de
> revendications de documentation non vérifiées. L'auditeur n'a pas écrit cette lane :
> le ton est volontairement critique.

---

## 1. Résumé exécutif

L'overlay Tauri est un **assemblage solide pour un POC avancé** : l'isolation des
données serveur en texte est globalement correcte (`textContent`), la CSP de
`tauri.conf.json` n'autorise pas `'unsafe-inline'` pour les scripts, le client WS
implémente correctement heartbeat/backoff/codes de fermeture 4401-4403 et le
rejeu de `current_job`, les styles `WS_EX_NOACTIVATE|WS_EX_TRANSPARENT|WS_EX_TOOLWINDOW`
sont posés et réassertés toutes les 2 s, et la config est écrite dans le dossier
`%APPDATA%\LiveChat` partagé avec Electron avec préservation des clés inconnues
(`serde(flatten)`) pour permettre le rollback.

En revanche, l'audit met en évidence **trois failles hautes** — deux touchant le jeton
d'appairage (fuite en query string, transport en clair) et une touchant l'exécution de
code non fiable dans le webview privilégié (carte tweet + script tiers Twitter) — ainsi
qu'un **updater non prêt pour la release** (clé minisign placeholder, artefacts non
signés, installateurs non signés). Plusieurs défenses en profondeur sont insuffisantes :
commandes IPC accessibles aux 3 fenêtres et faiblement validées, diffusion du
`clientToken` à la fenêtre board, fichier de config non écrit atomiquement et partagé
entre canaux sans verrou, outil de test `bridge-test.html` embarqué dans le binaire.

**Compteurs** : 0 critique, 3 hautes, 6 moyennes, 7 basses, 2 info (**18 findings**).
**Top 3 risque** :
1. `AUD-A6-002` — script tiers `platform.twitter.com/widgets.js` exécuté dans le webview
   privilégié (IPC Tauri accessible) si une carte tweet est rendue ; **dormant**
   aujourd'hui (le serveur v2 ne remplit jamais `tweet_card`), mais le code est livré.
2. `AUD-A6-001` — `clientToken` en query string (`?token=`) sur toutes les requêtes
   board + absence de header `Authorization` ⇒ fuite du jeton et board cassé contre le
   serveur v2 Bearer-only.
3. `AUD-A6-003` — appairage/WS acceptent `http://` sans exigence TLS ⇒ `clientToken`
   en clair dans le `hello` WS, et downgrade `server_url` possible via `apiBaseUrl`.

Bloquants signalés immédiatement au Lead (message séparé) : `AUD-A6-001`,
`AUD-A6-002`, `AUD-A6-003` et `AUD-A6-005` (updater).

| ID | Sévérité | Confiance | Titre |
|---|---|---|---|
| `AUD-A6-001` | **Haute** | Haute (statique, trivial) | Board : `clientToken` en query string, pas de Bearer ⇒ fuite + board cassé v2 |
| `AUD-A6-002` | **Haute** | Haute (statique + doc serveur) | Script tiers Twitter + `innerHTML` carte tweet dans webview privilégié (latent) |
| `AUD-A6-003` | **Haute** | Haute (statique) | Appairage/WS acceptent `http://` ⇒ jeton en clair (`hello` WS) + downgrade `apiBaseUrl` |
| `AUD-A6-004` | Moyenne | Haute | `LIVECHAT_INSECURE_TLS` désactive cert **et** hostname ⇒ MITM du WS |
| `AUD-A6-005` | Moyenne | Haute (preuves binaire) | Updater : pubkey placeholder, `createUpdaterArtifacts:false`, installateurs non signés |
| `AUD-A6-006` | Moyenne | Haute | Overlay affiché/topmost même `enabled=false` et sans appairage ; toggle ne le cache pas |
| `AUD-A6-007` | Moyenne | Moyenne | Config non atomique, `.bak` one-shot périmé, dossier partagé Electron/stable/bêta sans verrou |
| `AUD-A6-008` | Moyenne | Haute | Commandes IPC accessibles aux 3 fenêtres, payloads peu validés, raccourcis arbitraires persistés |
| `AUD-A6-009` | Moyenne | Haute | `withGlobalTauri:true` + CSP `connect-src` large : exfiltration/IPC directs si une injection existe |
| `AUD-A6-010` | Basse | Haute | `window.__bridgeMockEmit` et diagnostics actifs en release (mode Tauri) |
| `AUD-A6-011` | Basse | Haute | `emit_settings` diffuse `clientToken` à la fenêtre board (exposition inutile) |
| `AUD-A6-012` | Basse | Haute (accepté) | Jeton en clair au repos dans `config.json`, permissions héritées |
| `AUD-A6-013` | Basse | Haute | `innerHTML` carte tweet : primitives d'injection résiduelles (img/media http(s), iframe) même sans script |
| `AUD-A6-014` | Basse | Moyenne | Aucune détection plein écran exclusif → overlay invisible/DPR variable, guard topmost toutes les 2 s |
| `AUD-A6-015` | Basse | Haute (preuve binaire) | `web/bridge-test.html` (25 Ko) embarqué dans le binaire release |
| `AUD-A6-016` | Basse | Moyenne | Aucune limite locale aux `pairing_consume`/`reportError`/`check_updates` (amplification) |
| `AUD-A6-017` | Info | Haute | Pas de vérification de mise à jour au démarrage (tray uniquement) |
| `AUD-A6-018` | Info | Haute | `LIVECHAT_PRODUCT_NAME` contrôle le nom autostart (cosmétique) |

Effort de remédiation estimé : **8 à 12 j-h** (les 3 hautes + updater ≈ 4–6 j-h).

---

## 2. Méthode et preuves

1. **Lecture statique** : tous les fichiers de `apps/lc-overlay/src/` (14 modules +
   `net/`), `web/bridge.js`, `web/{overlay,board,pairing}.html`,
   `web/renderer-errors.js`, les renderers legacy `web/dist/renderer/**`
   (points d'entrée XSS, chargement dynamique de scripts, URLs média),
   `tauri.conf.json`, `tauri.beta.conf.json`, `capabilities/default.json`,
   `Cargo.toml`, `build.rs`, `scripts/release-build.ps1`, et les docs
   `overlay-bridge.md`, `lc-overlay-8b.md`, `overlay-release.md`,
   `lc-overlay-shell.md`, `poc-tauri-report.md`, `server-overlay.md`.
2. **Vérification croisée** par `grep` : usages de `innerHTML`, `?token=`,
   `Authorization`, `platform.twitter.com`, `danger_accept_invalid`,
   `LIVECHAT_CONFIG_PATH`, points d'écriture du token.
3. **Reproduction dynamique contrôlée** (aucun build, aucune installation) :
   - binaire release déjà construit par la lane packaging :
     `target\release\lc-overlay.exe` (18 373 632 o, `FileVersion=3.0.0-beta.1`,
     `ProductName=LiveChat Next`, `CompanyName=Babz`, **Authenticode=NotSigned**) ;
   - `findstr` sur le binaire : `bridge-test.html`, `RWQPLACEHOLDER_MINISIGN`,
     `LIVECHAT_INSECURE_TLS`, `overlay_get_config` présents (assets/chaînes
     embarqués) ;
   - lancement de `target\release\bundle\nsis\lc-overlay-beta.exe` avec
     `LIVECHAT_CONFIG_PATH=.phase0\audit\A6\config-temp.json`
     (`{"enabled": false, "serverUrl": null, "volume": 0.8}`) : processus vivant après
     8 s, `MainWindowTitle=LiveChat Next Overlay`, arrêt par `Stop-Process` ;
   - **preuve de non-altération de l'installation Electron** :
     `%APPDATA%\LiveChat\config.json` SHA-256 avant = après
     `03B8CB7553536FFBB8AEF185DEC35110114B9D5BD0639ABFE727DFC9BA3AB678`,
     `.bak` `633619D0205953225A0AB047704396CBD8B100D9AF1AFE8AFCD7C3802E68A32D`
     inchangés ; `config-temp.json.bak` créé par l'application (preuve du `.bak`
     one-shot) ;
   - `Get-AuthenticodeSignature` sur les installateurs
     `LiveChat_3.0.0_x64-setup.exe` (4 185 840 o) et
     `LiveChat Next_3.0.0-beta.1_x64-setup.exe` (4 188 395 o) : **NotSigned** ;
     `release-summary-beta.json` confirme « pas de certificat Authenticode »,
     `createUpdaterArtifacts=false` (clé privée attendue en Phase 9) et NSIS
     `currentUser`.
4. **Hors exécution** (marqué statique) : installateur NSIS (installation/
   désinstallation réellement exécutée), test de mise à jour signée, réseau réel
   (TLS/proxy/MITM), jeu en plein écran exclusif. Tout ce qui n'est pas prouvé
   dynamiquement est signalé comme tel (colonne confiance).

Limites de méthode : pas de build (consigne d'audit read-only), donc pas de
`cargo test -p lc-overlay` exécuté ; les « 51 tests » de `lc-overlay-8b.md:147`
sont une revendication de documentation, non rejouée ici.

---

## 3. Findings détaillés

### AUD-A6-001 — Board : `clientToken` en query string, aucun header `Authorization` (Haute)
- **Preuve** : `web/dist/renderer/board/legacy-preview-media-utils.js:21-37`
  (`buildAuthedUrl` : `url.searchParams.set('token', clientToken)` l.27) ;
  utilisé par `legacy-item-utils.js:9,14` (GET items), `:34,35` (PATCH),
  `:51,52` (POST), `:70,71` (DELETE), média board
  `legacy-preview-render-state-utils.js:38`, contexte `legacy-runtime-context-utils.js:18-22`.
  Aucun de ces `fetch` n'envoie d'en-tête `Authorization`.
  Serveur cible v2 : Bearer-only, `?token=` ignoré
  (`docs/rust/server-overlay.md:18,95,399`).
- **Impact** : (a) le board est **fonctionnellement cassé** contre le serveur Rust v2
  (401 sur toutes les opérations, y compris la lecture des items et le média board) ;
  (b) le jeton d'appairage — credential qui authentifie aussi le WS `hello` — fuit
  dans les URLs (journaux serveur/proxy, historique,Referer si navigation, rapports
  de crash). L'overlay, lui, est conforme (URL média verbatim, sans `?token=` :
  `legacy-media-dom-utils.js:74-80`). L'écart board est documenté/assumé
  (`lc-overlay-8b.md:85,174`, `legacy-ts-removal.md:192`) mais **contredit la décision
  explicite** `overlay-bridge.md:398,448` (supprimer l'ajout de token en Phase 8,
  overlay ET board).
- **Correctif** : dans les helpers board, remplacer `token` en query par
  `Authorization: Bearer <clientToken>` (et supprimer `buildAuthedUrl` ou n'y laisser
  que le base URL) ; aligner `OverlayConfigView`/events sur ce flux ; mettre à jour la
  doc. Test : unit test renderer (les URLs générées ne contiennent jamais `token=` ;
  les fetch portent le header) + test d'intégration board contre `lc-server` v2.
- **Effort** : M (0,5–1 j).

### AUD-A6-002 — Script tiers Twitter + `innerHTML` carte tweet dans le webview privilégié (Haute, latent)
- **Preuve** : `web/dist/renderer/overlay/legacy-tweet-card-renderer.js:55,78`
  (`widgetContent.innerHTML = model.html;`) ; `legacy-tweet-card-playback-utils.js:16-23`
  (charge `ensureTwitterWidgets()` puis `twitter.widgets.load(...)`) ;
  `legacy-twitter-widgets-utils.js:25-35` injecte
  `<script id="twitter-widgets-js" src="https://platform.twitter.com/widgets.js">` ;
  CSP autorise ce script (`tauri.conf.json:54`, `script-src 'self'
  https://platform.twitter.com`) et l'iframe Twitter (`frame-src ... twitter.com`) ;
  le webview overlay a `withGlobalTauri: true` (`tauri.conf.json:10`) et toutes les
  commandes applicatives sont hors ACL Tauri 2 (`capabilities/default.json:3`) ;
  `overlay_get_config` renvoie le `clientToken` (`src/commands.rs:135-145`).
- **État actuel** : **dormant** — le serveur v2 ne remplit jamais `tweet_card`
  (`apps/lc-server/src/media/assets.rs:349` `tweet_card: None` ;
  `apps/lc-server/src/ws/tests.rs:882`); le type existe pourtant
  (`crates/lc-protocol/src/lib.rs:716-773`) et le code de rendu est embarqué dans le
  binaire release. Toute réactivation du champ (Phase 9, rollback legacy) arme le
  chemin.
- **Impact** : un script tiers exécuté dans ce webview obtient l'IPC Tauri :
  lecture du `clientToken`, `pairing_consume` (ré-appairage vers un serveur
  attaquant), `meme_board_set_bindings` (Persistance de raccourcis globaux), voire
  `overlay_install_update`. C'est **de l'exécution de code non fiable dans un contexte
  privilégié** (chaîne d'approvisionnement Twitter/CDN + HTML serveur non assaini).
- **Correctif** : ne plus charger `widgets.js` ni rendre la carte par `innerHTML`
  (construire un DOM statique à partir de champs structurés : texte, auteur, lien) ;
  retirer `platform.twitter.com` de `script-src` et `frame-src` ; si une carte riche
  est indispensable, l'ouvrir dans le navigateur externe. Test : vérifier dans le
  binaire release l'absence de `platform.twitter.com` en `script-src`, et un test
  renderer que le chemin carte tweet n'exécute aucun script.
- **Effort** : M (0,5–1 j). **À trancher avant release.**

### AUD-A6-003 — Appairage/WS acceptent `http://` sans exigence TLS ⇒ jeton en clair et downgrade (Haute)
- **Preuve** : `src/net/pairing.rs:108-120` (`pairing_endpoint` accepte `http` ET
  `https`, aucune exception loopback) ; `src/net/pairing.rs:147-153`
  (`config.server_url = response.api_base_url` **fourni par le serveur**, sinon l'URL
  demandée) ; `src/net/ws.rs:460-477` (`http|ws → ws`, `https|wss → wss`) puis
  `hello` incluant `client_token` (`ws.rs:431-457`) ; UI `web/pairing.html:112`
  (`type="url"`, préremplissage, aucune validation de schéma ;
  `pairing-controller.js:17-22` envoie `input.value.trim()`) ; persistance en clair
  (`config.rs`).
- **Impact** : l'utilisateur peut appairer en `http://` ; le jeton est alors échangé
  en POST en clair puis **renvoyé en clair dans chaque `hello` WS** (reconnexions et
  heartbeats) — interception triviale sur réseau partagé (Wi-Fi public, hôtel, LAN).
  Un serveur compromis peut de plus renvoyer `apiBaseUrl: http://…` et forcer un
  downgrade même si l'utilisateur a saisi `https://`. Le jeton donne le contrôle
  overlay (meme board, média, WS).
- **Correctif** : exiger `https` (ou `wss`) sauf loopback + confirmation explicite ;
  rejeter un `apiBaseUrl` de schéma différent / d'origine différente ; afficher un
  avertissement UI ; test unitaire sur `pairing_endpoint`/`ws_url_from_server_url`
  et test d'intégration (http → refus).
- **Effort** : S–M (0,25–0,5 j).

### AUD-A6-004 — `LIVECHAT_INSECURE_TLS` désactive cert **et** hostname (Moyenne)
- **Preuve** : `src/net/ws.rs:45` (`INSECURE_TLS_ENV="LIVECHAT_INSECURE_TLS"`),
  `:479-487` (parse `1|true|yes|on`), `:489-499`
  (`danger_accept_invalid_certs(true)` **et** `danger_accept_invalid_hostnames(true)`) ;
  même mécanisme pour le client pairing (`pairing.rs:122-132`,
  `danger_accept_invalid_certs` seul).
- **Impact** : toute machine où cette variable est positionnée (script d'installation,
  debug oublié, malware) accepte n'importe quel certificat ⇒ MITM total du WS/TLS
  (vol du jeton, injection de commandes de lecture). La doc
  (`lc-overlay-8b.md:27,179`) présente cela comme « debug uniquement », mais rien
  dans le code ne le restreint à un build debug / n'alerte l'utilisateur.
- **Correctif** : n'activer l'option qu'en `debug_assertions` ou derrière un flag
  `--insecure-tls` explicite + log `tracing::warn` + bannière UI ; documenter le
  risque ; test unitaire (env absent → vérification stricte).
- **Effort** : S (0,25 j).

### AUD-A6-005 — Updater : clé placeholder, artefacts non signés, installateurs non signés (Moyenne)
- **Preuve** : `tauri.conf.json:60` `createUpdaterArtifacts: false` ; `:73-78`
  endpoint `https://github.com/quentingimenez/LiveChat/releases/latest/download/latest.json`
  + `pubkey = "RWQPLACEHOLDER_MINISIGN_PUBLIC_KEY_A_REMPLACER_EN_PHASE_9="` ;
  `tauri.beta.conf.json` même placeholder, endpoint `.../next/latest.json` ;
  `scripts/release-build.ps1:186-190` (« pas de certificat Authenticode », « artefacts
  de signature minisign exigent la clé privée en Phase 9 ») ; `src/updater.rs:48-71`
  (`app.updater()` en erreur → statut error fail-closed), `:78-114` (installation
  seulement si update signé trouvé), pas de vérification au démarrage ;
  preuves binaire : binaire et installateurs `Authenticode=NotSigned`,
  `release-summary-beta.json` notes identiques.
- **Impact** : l'auto-update **ne peut pas fonctionner** (clé placeholder ⇒ toutes les
  installations échoueront en fail-closed), et les installateurs non signés déclenchent
  SmartScreen/AV et ne permettent pas d'établir la chaîne de confiance de mise à jour.
  Le risque sécurité actuel est faible (fail-closed) mais **rendre l'updater actif
  sans clé réelle + signature + CI dédiée** serait un risque majeur. Le remote
  `origin` GitHub correspond à l'endpoint (vérifié `git remote -v`), donc pas de
  divergence d'organisation.
- **Correctif** : générer la clé minisign (`tauri signer generate`), stocker la clé
  privée en secret CI, `createUpdaterArtifacts: true`, publier `latest.json` + `.sig`,
  signer Authenticode les installateurs (certificat), test d'upgrade sur VM avant
  activation. Tant que ce n'est pas fait : laisser l'updater explicitement désactivé
  et documenter.
- **Effort** : M–L (1–2 j + coût certificat).

### AUD-A6-006 — Overlay visible/topmost même désactivé et sans appairage ; toggle ne le cache pas (Moyenne)
- **Preuve** : `src/windows.rs:266-271` `bootstrap_windows()` applique
  systématiquement `apply_overlay_flags()` (show l.167-169) sans tester
  `config.enabled` ; `src/windows.rs:134-176` pose topmost/click-through/full-display
  bounds ; `src/tray.rs:327-341` `toggle_enabled` persiste la config puis publie les
  settings/reconnecte mais **ne cache pas** l'overlay ; repro : config temporaire
  `{"enabled": false, ...}` ⇒ fenêtre « LiveChat Next Overlay » présente
  (`.phase0/audit/A6/evidence-02-run.txt`).
- **Impact** : couche transparente plein écran topmost présente en permanence au
  démarrage, même sans appairage ni activation ; l'utilisateur ne peut pas la
  désactiver (le toggle ne masque pas) ; couche inutile qui interagit avec d'autres
  overlays (Discord, Steam) et les thèmes/fullscreen. Pas de vol de clic
  (`WS_EX_TRANSPARENT` + `pointer-events:none`) ni de vol de focus
  (`WS_EX_NOACTIVATE` + `focus:false`), donc pas de blocage d'entrée constaté.
- **Correctif** : ne `show()` la fenêtre que si `enabled && has_pairing_config` ;
  `toggle_enabled(false)` ⇒ `hide()` immédiat (+ arrêt du guard) ; test unitaire
  `bootstrap_windows` avec config disabled (fenêtre non montrée).
- **Effort** : S (0,25 j).

### AUD-A6-007 — Config non atomique, `.bak` one-shot périmé, dossier partagé sans verrou (Moyenne)
- **Preuve** : `src/config.rs:190-198` `write_to` = `create_dir_all` + `fs::write`
  (troncature puis écriture, pas de fichier temporaire ni `rename`, pas de `fsync`,
  permissions héritées par défaut) ; `:395-401` `ensure_backup` copie le `.bak` **une
  seule fois** et ne le rafraîchit jamais ; `:405-411` `migrate_from` ;
  `:414-432` chemin forcé `%APPDATA%\LiveChat\config.json` (env
  `LIVECHAT_CONFIG_PATH` prioritaire) ; docs `overlay-release.md:24,286-294,309-313,328`
  : le fichier est **volontairement partagé** entre Electron, stable et bêta (rollback),
  et stable+bêta peuvent tourner simultanément (identités/single-instance distinctes).
- **Impact** : (a) crash/power-loss pendant `fs::write` ⇒ config tronquée/corrompue
  (perte de l'appairage, pas de récupération : le `.bak` est une vieille copie
  Electron, pas une copie de la dernière config Tauri valide) ; (b) deux applications
  qui écrivent le même fichier = lost-update silencieux (le dernier écrase les
  réglages de l'autre) ; (c) permissions héritées : tout processus du même
  utilisateur peut lire le jeton (attendu, parité Electron, cf. AUD-A6-012).
- **Correctif** : écriture atomique (temp + `rename` + fsync), rafraîchir `.bak` avant
  chaque écriture réussie (ou garder N backups), verrou fichier inter-processus
  (`.lock`) ou dossier par canal avec migration, durcir les ACL (`icacls` équivalent
  via API Windows : héritage désactivé, utilisateur courant seul). Tests : écriture
  interrompue simulée, deux writers concurrents, backup rafraîchi.
- **Effort** : M (0,5–1 j).

### AUD-A6-008 — Commandes IPC accessibles aux 3 fenêtres et faiblement validées (Moyenne)
- **Preuve** : `src/lib.rs:196-209` enregistre les 12 commandes sans distinction de
  fenêtre ; `capabilities/default.json:3` confirme qu'elles sont **hors ACL Tauri 2**
  et donc appelables depuis `overlay`, `board` et `pairing` ; validations internes :
  `overlay_get_config` retourne le token (`commands.rs:135-145`) ;
  `pairing_consume` lit `serverUrl`/`code`/`deviceName` via `Value::as_str` sans
  validation locale (`commands.rs:254-267`) ; `meme_board_set_bindings` persiste des
  accélérateurs arbitraires fournis par le renderer (`commands.rs:321-341`,
  `shortcuts.rs`) ; `overlay_report_error` fallback `payload.to_string()` non borné
  (`commands.rs:171`) ; `overlay_report_playback_state` n'utilise pas de `job_id`
  validé (`commands.rs:184-220`).
- **Impact** : toute injection script dans n'importe laquelle des 3 pages (cf.
  AUD-A6-002/013) obtient les mêmes pouvoirs que l'overlay : lecture du jeton,
  ré-appairage, persistance de raccourcis globaux arbitraires (surface de capture
  clavier), flood de messages WS. La validation faible ne crée pas la faille mais
  **amplifie** toute compromission du renderer et empêche une défense en profondeur.
- **Correctif** : séparer les commandes par fenêtre (`overlay_*` uniquement pour
  overlay, board/handlers dédiés) ou vérifier `WebviewWindow::label()` en début de
  commande ; valider schémas/longueurs (serverUrl https, code format, deviceName
  borné, message ≤ 2 Ko, bindings whitelist) ; ne pas persister un binding sans
  validation ; test : appels IPC depuis board/pairing refusés.
- **Effort** : M (0,5 j).

### AUD-A6-009 — `withGlobalTauri:true` + CSP large : exfiltration/IPC directs si injection (Moyenne)
- **Preuve** : `tauri.conf.json:10` `withGlobalTauri: true` (expose
  `window.__TAURI__`) ; `:54` CSP `connect-src 'self' ipc: http://ipc.localhost http:
  https: ws: wss:` et `img-src/media-src ... http: https:` — quasiment toutes les
  destinations réseau sont autorisées.
- **Impact** : si une injection (même sans script, via HTML de carte tweet, ou une
  future feature) permet d'exécuter du JS, l'exfiltration est directe (fetch/WS vers
  n'importe quel hôte) et l'IPC Tauri est joignable sans import. Aujourd'hui la CSP
  sans `'unsafe-inline'` et l'usage de `textContent` limitent l'entrée ; il s'agit de
  défense en profondeur, à traiter avec AUD-A6-002.
- **Correctif** : restreindre `connect-src` aux hôtes réellement nécessaires (API
  LiveChat + ws/wss connus), `img/media-src` aux domaines média/CDN ; désactiver
  `withGlobalTauri` et passer par l'API `@tauri-apps/api` bundlée ; `object-src
  'none'` conservé.
- **Effort** : S–M (0,25–0,5 j).

### AUD-A6-010 — `window.__bridgeMockEmit` et diagnostics actifs en release (Basse)
- **Preuve** : `web/bridge.js:688-704` expose `window.__livechatOverlayBridge`
  (jusqu'à 200 appels, arguments « sanitizés ») et `:705+` `window.__bridgeMockEmit`
  qui émet play/stop/settings localement **même en mode Tauri** (commentaire « outil
  de test ») ; assets servis tels quels (pas de build qui les retire).
- **Impact** : tout script injecté peut fabriquer des événements de lecture/stop ou
  modifier l'UI sans passer par le serveur (spoofing d'état local) ; les diagnostics
  gardent en mémoire des extraits d'appels (sanitizés, mais présents). Ne donne pas de
  privilege supplémentaire au-delà du contexte JS.
- **Correctif** : supprimer ou conditionner ces objets à `debug_assertions` (ou à un
  flag de dev) ; ne pas embarquer les outils de test dans la release.
- **Effort** : S (0,1 j).

### AUD-A6-011 — `emit_settings` diffuse le `clientToken` à la fenêtre board (Basse)
- **Preuve** : `src/events.rs:24` (`EVENT_SETTINGS`), `:139-152` `emit_settings`
  cible `OVERLAY_LABEL` **et** `BOARD_LABEL` avec `OverlayConfigView` qui contient
  `client_token` (`config.rs:104-116`, l.109).
- **Impact** : le jeton est exposé au renderer board alors que (i) v2 ne l'accepte
  pas en query et (ii) le board n'en a pas besoin s'il utilise le Bearer du WS.
  Surface d'exposition supplémentaire (logs/erreurs board, audit XSS).
- **Correctif** : envoyer une vue sans token (`OverlayConfigView` public ou flag
  `include_token:false`) à la fenêtre board ; ne conserver le token qu'en mémoire du
  processus/overlay.
- **Effort** : S (0,1 j).

### AUD-A6-012 — Jeton en clair au repos, permissions héritées (Basse, accepté)
- **Preuve** : `src/config.rs:190-198` `fs::write` sans ACL explicite ;
  `%APPDATA%\LiveChat\config.json` contient `clientToken` ; dossier partagé avec les
  données Electron (Cache, Local Storage, Preferences…), cf.
  `evidence-01-binary.txt`.
- **Impact** : tout processus du même utilisateur (malware non élevé, backup
  cloud/sync, autre app) peut lire le jeton et usurper l'overlay. Norme desktop
  (Electron faisait pareil), pas de DPAPI/safeStorage.
- **Correctif** : protéger avec DPAPI (`CryptProtectData`, portée utilisateur) ou
  Credential Manager, à évaluer en Phase 9 ; au minimum ACL `%APPDATA%\LiveChat`
  restreintes au compte courant (héritage désactivé).
- **Effort** : M (0,5 j, optionnel).

### AUD-A6-013 — `innerHTML` carte tweet : primitives d'injection résiduelles (Basse)
- **Preuve** : `legacy-tweet-card-renderer.js:55,78` injecte le HTML fourni par le
  serveur ; CSP bloque les scripts non Twitter mais autorise `img-src/media-src
  http: https:` et `frame-src platform.twitter.com|syndication.twitter.com` ; liens
  externes `target=_blank rel=noopener noreferrer` (l.1-7) ; vidéos inline
  `video.src = inlineVideo.url` (l.35). Dormant comme AUD-A6-002
  (`tweet_card: None`).
- **Impact** : même sans script, un `tweet_card.html` contrôlé par le serveur (ou un
  futur proxy) peut charger des images/médias/iframes distants ⇒ pistage, SSRF
  côté client (requêtes sortantes), déni de service visuel. Avec `frame-src
  twitter`, une iframe Twitter peut charger du contenu arbitraire de twitter.com.
- **Correctif** : supprimer l'injection HTML ; construire le DOM à partir des champs
  structurés (`textContent`, liens validés) ; restreindre `img/media-src` aux hôtes
  média ; retirer twitter de `frame-src`.
- **Effort** : S–M (0,25–0,5 j, avec AUD-A6-002).

### AUD-A6-014 — Aucune détection plein écran exclusif ; guard topmost toutes les 2 s (Basse)
- **Preuve** : `src/display.rs` (213 l.) ne fait que lister/sélectionner les écrans
  (`key()` `"{x},{y},{w},{h},{index}"`) ; `windows.rs:102-130`
  `overlay_bounds()` = **bounds entiers de l'écran** ; `spawn_topmost_guard`
  (`windows.rs:179-198`) réasserte `set_always_on_top(true)` + `SetWindowPos(HWND_TOPMOST)`
  toutes les 2 s (`KEEP_ON_TOP_INTERVAL` l.19).
- **Impact** : dans un jeu en plein écran exclusif (DWM bypass), une fenêtre topmost
  n'est pas composée : l'overlay est invisible alors que l'utilisateur l'attend
  (écart fonctionnel, pas sécurité) ; en multi-écrans/DPI/HDR le placement plein écran
  peut mal tomber (non testé) ; le guard périodique peut interférer avec d'autres
  overlays topmost de l'utilisateur (Discord/Steam) et empêche un autre overlay de
  passer devant. Repro non exécutée (pas de jeu disponible en audit).
- **Correctif** : documenter la limitation (fenêtré/borderless sans bordure
  recommandé), option « désactiver le guard topmost », détection best-effort
  fullscreen/borderless si faisable sans injection.
- **Effort** : S (0,25 j) / M si détection réelle.

### AUD-A6-015 — `bridge-test.html` embarqué dans le binaire release (Basse)
- **Preuve** : `findstr` sur `target\release\lc-overlay.exe` : `bridge-test.html`
  **MATCH** (`evidence-01-binary.txt`) ; fichier présent `web/bridge-test.html`
  (25 470 o) et documenté (`overlay-bridge.md:7,525,547`, `lc-overlay-shell.md:37`) ;
  non déclaré comme fenêtre Tauri (`tauri.conf.json` : 3 fenêtres overlay/pairing/board),
  donc non atteignable par l'UI prévue, mais servi depuis l'origine `tauri.localhost`.
- **Impact** : page de test 25 Ko dans la release (surface d'attaque hors produit ;
  peut exercer le bridge, lister/écrire des données si ouverte ; risque de confusion
  de bug report). Faible aujourd'hui car non routée.
- **Correctif** : exclure `web/bridge-test.html` du `frontendDist` de release
  (build conditionnel ou dossier de dev séparé) ; `grep` en CI sur l'artefact pour
  vérifier son absence.
- **Effort** : S (0,1 j).

### AUD-A6-016 — Aucune limite locale aux `pairing_consume`/`reportError`/`check_updates` (Basse)
- **Preuve** : `commands.rs:247-311` (pairing) sans throttle ni longueur maximale ;
  `commands.rs:159-182` reportError sans cap (`payload.to_string()` l.171) ;
  `bridge.js:646-652` reportError sans validation de taille ;
  `renderer-errors.js:61-78` (max 20 erreurs × 1000 chars) limite déjà les erreurs
  console, mais un script injecté peut appeler directement le bridge.
- **Impact** : un renderer compromis peut spamer le serveur (pairing 422/400, erreurs
  WS) jusqu'à épuiser son rate limit (100 req/15 min côté serveur), créant un
  self-DoS et polluant les logs. Effet amplifié par AUD-A6-008.
- **Correctif** : throttle côté commandes (1 pairing/10 s, reportError 5/min, taille
  ≤2 Ko), et validation du bridge (`assert` de type/longueur).
- **Effort** : S (0,2 j).

### AUD-A6-017 — Pas de vérification de mise à jour au démarrage (Info)
- **Preuve** : `src/updater.rs` (vérification uniquement à la demande) ;
  `tray.rs:413-430` `spawn_update_check` déclenché par le menu tray
  (`ID_CHECK_UPDATES`), pas au démarrage.
- **Impact** : produit (utilisateurs sur protocole obsolète voient une coupure 4403
  sans chemin de mise à jour guidé). Choix assumé mais à noter pour la Phase 9
  (auto-check opt-in + notification).
- **Effort** : S.

### AUD-A6-018 — `LIVECHAT_PRODUCT_NAME` contrôle le nom autostart (Info)
- **Preuve** : `src/lib.rs:69` `autostart_app_name()` lit l'env puis retombe sur
  `productName` ; utilisé par le plugin autostart (`.app_name(...)` l.183-188) ;
  `tray.rs:355-371` toggle via plugin.
- **Impact** : cosmétique (nom de la clé Run). Éviter les variables d'environnement
  pour des identités de sécurité, mais pas de risque réel.
- **Effort** : S (documentation).

---

## 4. Couverture de la checklist d'audit

| Item demandé | Couvert | Preuve / finding |
|---|---|---|
| Fenêtres Win32 : transparent, click-through, topmost, no-activate, skip-taskbar | Oui | `tauri.conf.json:13-27` (overlay), `windows.rs:134-176`, `win32.rs` (WS_EX_LAYERED/TRANSPARENT/NOACTIVATE/TOOLWINDOW, SetWindowPos), repro `evidence-02-run.txt` ; AUD-A6-006/014 |
| Topmost guard 2 s | Oui | `windows.rs:19,179-198` (boucle 2 s), `win32.rs` reassert |
| Plein écran exclusif | Partiel | `windows.rs:102-130` bounds écran entier, `display.rs` aucune détection ; non testé dynamiquement ; AUD-A6-014 |
| Clic traversant vers jeu/site sensible | Oui (code) | `set_ignore_cursor_events(true)` + `WS_EX_TRANSPARENT` + CSS `pointer-events:none` (`web/styles/overlay/base.css:24`) ; pas de test jeu réel |
| CSP des 3 pages | Oui | `tauri.conf.json:54` (CSP unique) ; aucune meta CSP dans les pages ; pas d'inline script ; `pairing.html` inline `<style>` explique `style-src 'unsafe-inline'` ; AUD-A6-002/009/013 |
| XSS titres/messages/auteurs | Oui | `legacy-overlay-text-utils.js:55,96,128` (`textContent`) ; avatar `img.src` non validé (URL serveur) ; board sans innerHTML ; AUD-A6-002/013 |
| Twitter iframe / carte tweet | Oui | AUD-A6-002/013 : `innerHTML = model.html` + `widgets.js` tiers ; dormant (`tweet_card: None`) |
| Bridge 13 méthodes / validation IPC / contexte / invoke non validés | Oui | `bridge.js:39-50,625-681`, transport 62-95, `getConfig` token l.627, mock l.705+ ; AUD-A6-008/010 |
| Client WS : TLS strict + `LIVECHAT_INSECURE_TLS` | Oui | `ws.rs:45,479-499` ; AUD-A6-003/004 |
| WS : backoff, closes 4401/4402/4403, replay `currentJob`, heartbeat, buffer 5 s | Oui | `ws.rs:370-413` close_step, heartbeat 15 s, backoff 1/2/3 s, `welcome` adoption offset, `events.rs:27,30,` buffer ; pas de test live |
| Config/token disque : permissions, `.bak`, chemin partagé `%APPDATA%\LiveChat`, `LIVECHAT_CONFIG_PATH` | Oui | `config.rs:22-28,104-116,190-198,395-401,414-432` ; `overlay-release.md:24,286-294,309-313,328` ; repro `.bak` créé + Electron intact ; AUD-A6-007/012 |
| Autostart / single-instance / tray | Oui | `lib.rs:178-188` plugins, `windows.rs:256-263` second instance, `tray.rs:433-457` ; AUD-A6-018 (info) |
| Updater : pubkey placeholder, artifacts, endpoints, release non signée | Oui | `tauri.conf.json:60,73-78`, bêta idem, `updater.rs:48-114`, `release-build.ps1:186-190`, preuves NotSigned ; AUD-A6-005/017 |
| Board token en query | Oui | AUD-A6-001 (preuve complète) |
| Isolation bêta | Oui | `tauri.beta.conf.json` (identité/endpoint séparés), config partagée voulue ; AUD-A6-007 |
| Page dev `bridge-test.html` embarquée | Oui | `findstr` binaire MATCH ; AUD-A6-015 |
| Validateurs Tauri / capabilities | Oui | `capabilities/default.json` (fs scopes, 3 fenêtres), ACL app commands absente l.3 ; AUD-A6-008/009 |

---

## 5. Points forts

- **Isolation du texte serveur correcte** : titres, messages, noms d'auteurs rendus par
  `textContent` ; board sans `innerHTML` sur les données ; seule la carte tweet (non
  alimentée en v2) utilise `innerHTML`.
- **CSP sans `'unsafe-inline'` pour `script-src`** et `object-src 'none'` ; aucun
  script inline dans les 3 pages ; la seule entorse `style-src 'unsafe-inline'` est
  justifiée par un `<style>` local dans `pairing.html`.
- **Client WS discipliné** : messages JSON validés (`ServerEnvelope`, trame invalide →
  `Failed`), heartbeat 15 s, backoff 1/2/3 s borné, codes de fermeture 4401 (purge du
  token + retour appairage), 4402 (révoqué), 4403 (protocole obsolète avec
  `minVersion`), rejeu `current_job` avec offset calculé, buffer d'événements 5 s.
- **Fenêtres Win32 faites sérieusement** : styles ex posés et vérifiés par
  `GetWindowLongPtrW`/`SetWindowLongPtrW`, `SetWindowPos(HWND_TOPMOST,
  SWP_NOACTIVATE)` sans vol de focus, close intercepté pour minimiser dans le tray.
- **Config pensée pour le rollback** : chemin forcé partagé avec Electron,
  `serde(flatten) extra` préserve les clés inconnues, parsing tolérant avec warnings,
  `.bak` one-shot ; la repro a prouvé que l'installation Electron n'était pas modifiée.
- **Rust propre** : `unsafe` confiné à `win32.rs` (allow local), lints
  `unwrap_used/expect_used/panic/dbg` deny, `unsafe_code = "deny"` local ;
  `withGlobalTauri` reste le principal écart de durcissement.
- **Fail-closed sur l'updater** : la clé placeholder produit une erreur explicite
  plutôt qu'une mise à jour silencieuse ; l'installation n'est tentée que si un
  update signé est trouvé.
- **Single-instance** : évite les overlays en double et route la seconde instance vers
  board/pairing.

---

## 6. Zones non couvertes

- **Pas d'installation NSIS exécutée** (ni installation, ni désinstallation, ni
  upgrade in-place Electron 2.0.1 → Tauri) : comportement du registre/Run key,
  `%LOCALAPPDATA%`, désinstalleur non vérifiés dynamiquement.
- **Pas de test réseau réel** : TLS (validation stricte par défaut, proxies, MITM,
  certificats internes), WebSocket contre le serveur v2 non démarré, rate limit
  serveur — conclusions statiques basées sur le code et les docs.
- **Pas de test dynamique click-through/topmost sur jeu réel** (plein écran exclusif,
  borderless, multi-écrans, DPI 125-200 %, HDR) ni de test anti-cheat.
- **Pas d'analyse binaire approfondie** (strings seulement, pas de désobfuscation) ;
  pas de vérification de la signature des DLL/chargements dynamiques.
- **Pas de `cargo test -p lc-overlay`** ni de clippy (audit sans build) ; les « 51
  tests verts » de `lc-overlay-8b.md:147` ne sont pas rejoués.
- **Updater de bout en bout impossible** sans clé minisign réelle + release GitHub.
- **Pas d'audit du serveur v2** (autres lanes) : interactions supposées conformes à
  `docs/rust/server-overlay.md`, notamment Bearer-only et suppression de
  `/overlay/config`.
- **Renderer board non audité exhaustivement** (~90 fichiers legacy) : échantillon
  ciblé XSS/URL/token ; pas de revue de tous les `innerHTML`.
- **Pas de vérification ACL Windows** (`icacls`) du dossier `%APPDATA%\LiveChat`
  (audit read-only, pas de modification des permissions).
- **Pas de fuzzing** des payloads IPC/WS.

---

## 7. Recommandations priorisées

### P0 — avant toute release / activation d'auto-update
1. **Supprimer l'exécution de code tiers** (AUD-A6-002/013) : plus de
   `platform.twitter.com/widgets.js`, plus de `innerHTML` de HTML serveur ; rendu
   statique construit en DOM ; retirer twitter de `script-src`/`frame-src` ; grep CI
   sur l'artefact.
2. **Board en Bearer** (AUD-A6-001) : supprimer `?token=`, header `Authorization` ;
   réparer le board v2 ; tests unitaires « aucune URL ne contient token= ».
3. **TLS obligatoire à l'appairage/WS** (AUD-A6-003) : refuser `http://` hors
   loopback, rejeter un `apiBaseUrl` de schéma différent ; tests.
4. **Chaîne de mise à jour** (AUD-A6-005) : clé minisign réelle en secret CI,
   `createUpdaterArtifacts:true`, signature Authenticode des installateurs, test
   d'upgrade VM ; sinon laisser l'updater désactivé et le documenter.

### P1 — durcissement (sprint suivant)
5. Masquer l'overlay quand `enabled=false` / non appairé, et sur toggle (AUD-A6-006).
6. Restreindre `LIVECHAT_INSECURE_TLS` aux builds debug + avertissement (AUD-A6-004).
7. Séparer/valider les commandes IPC par fenêtre, bornes de payload, whitelist des
   bindings (AUD-A6-008/016).
8. Écriture atomique de la config, `.bak` rafraîchi, verrou inter-processus, ACL
   durcies (AUD-A6-007/012).
9. Retirer le token de `emit_settings` pour board (AUD-A6-011) ; désactiver
   `withGlobalTauri` et resserrer CSP `connect-src`/`img-src`/`media-src`
   (AUD-A6-009).
10. Retirer `bridge-test.html` et les outils de mock du binaire release
    (AUD-A6-010/015).

### P2 — qualité / produit
11. Détection plein écran/borderless + option de désactivation du guard topmost
    (AUD-A6-014).
12. Vérification de mise à jour opt-in au démarrage (AUD-A6-017).
13. DPAPI/ACL pour le jeton et audit de la cohabitation stable/bêta (AUD-A6-012/007).
14. Fuzzing IPC/WS et tests réseau réels (table ci-dessus, §6).

---

## 8. Annexe — artefacts et commandes de reproduction

- `.phase0/audit/A6/evidence-01-binary.txt` : `Get-Item`/`VersionInfo` du binaire
  release, `Get-AuthenticodeSignature`, `findstr` (assets et chaînes), inventaire
  `%APPDATA%\LiveChat`, inventaire du bundle NSIS.
- `.phase0/audit/A6/evidence-02-run.txt` : exécution contrôlée
  (`LIVECHAT_CONFIG_PATH` temporaire), PID, fenêtre, hashes SHA-256 avant/après de la
  config Electron et de son `.bak`, création de `config-temp.json.bak`, signature
  Authenticode des installateurs, `release-summary-beta.json`.
- `.phase0/audit/A6/config-temp.json` (+ `.bak` généré par l'application).
- Commandes clés (PowerShell) : `Get-AuthenticodeSignature`,
  `Get-FileHash -Algorithm SHA256`, `Start-Process ... -PassThru` + `Stop-Process`,
  `findstr /C:"..." target\release\lc-overlay.exe`, `Select-String` sur la config
  Tauri/capabilities.
- Aucun build, aucune installation, aucune modification de l'installation Electron
  (hash vérifiés) ; `git status --short` ne contient que les artefacts d'autres lanes
  et ce rapport.
- Remote vérifié (`git remote -v`) : `https://github.com/quentingimenez/LiveChat`,
  cohérent avec l'endpoint updater configuré.

---

*Fin de l'audit A6 — 18 findings (0 critique, 3 hautes, 6 moyennes, 7 basses, 2 info).
Les findings hauts `AUD-A6-001` à `AUD-A6-003` et la chaîne updater `AUD-A6-005` ont
été signalés au Lead avant rédaction de ce rapport.*
