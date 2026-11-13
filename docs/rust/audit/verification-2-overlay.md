# Vérification indépendante 2 — overlay Tauri (correctifs P1 / COR-11)

> **Auditeur** : `overlay-bridge` (indépendant de la lane d'implémentation P1).
> **Date** : 2026-11-10. **Branche** : `feat/rust-rebuild`. **HEAD** : `63e12f5`
> (« fix(p1): close major audit findings (COR-05..COR-11) »).
> **Périmètre** : diff overlay de P1 — `apps/lc-overlay/**`,
> `docs/rust/overlay-bridge.md`, `docs/rust/overlay-release.md` ; vérification des
> correctifs **COR-11** de `docs/rust/audit/p1-correctifs.md` et du binaire
> `target/release/lc-overlay.exe`.
> **Mode** : audit adversarial en lecture seule — revue de code, `grep` de preuve,
> scan `strings` ASCII + UTF-16 du binaire release **déjà construit**. Aucun
> build, aucune exécution de l'application, aucune modification de code, aucun
> commit. Timebox ~45 min.
> **Livrable** : ce document. Findings `AUD-V2-O-xxx`.

---

## 1. Résumé exécutif

**Verdict global : correctifs P1/COR-11 substantiellement tenus — 7 des 8
sous-points vérifiés sont TENUS, 1 est PARTIEL. Aucun bypass critique ou élevé
n'a été trouvé.** Les deux seuls findings de sévérité Faible sont des écarts de
moindre privilège / de désactivation incomplète, sans exploitation démontrable
en l'état (défense en profondeur). Le reste relève de la précision documentaire
ou de recommandations Phase 9.

### 1.1 Verdicts COR-11

| # | Sous-point COR-11 | Verdict | Preuves principales |
|---|---|---|---|
| 1 | Appairage/WS refusent `http://` par défaut | **TENU** | `net/tls.rs:48-60`, `net/pairing.rs:121-136`, `net/ws.rs:460-480` |
| 2 | Dérogation `LIVECHAT_INSECURE_TLS=1` limitée au debug | **TENU** (statique) | `net/tls.rs:16-27`, absence de RUSTFLAGS/profile ; cf. §6 |
| 3 | Refus du downgrade `apiBaseUrl` https→http | **TENU** | `net/pairing.rs:159-186`, test `:336-354` |
| 4 | Carte tweet neutralisée + CSP `script-src 'self'`/`frame-src 'none'` | **TENU** | `legacy-tweet-card-renderer.js:8-18`, `legacy-twitter-widgets-utils.js:8-10`, `tauri.conf.json:54` |
| 5 | Capabilities Tauri par fenêtre + `COMMAND_MATRIX` | **TENU** (résidu O-002) | `capabilities/{overlay,board,pairing}.json:1-7`, `commands.rs:34-83`, `lib.rs:196-208` |
| 6 | Updater explicitement désactivé pour 3.0.0 | **PARTIEL** | `updater.rs:55/69-74/102-107` OK, mais contournement tray `tray.rs:413-430/445` (O-001) |
| 7 | `bridge-test.html`/`README.md` sortis du bundle | **TENU** | `dev/` (déplacés), `tauri.conf.json:7`, scan binaire §8.1 |
| 8 | Board via `Authorization: Bearer` | **TENU** | `web/bridge.js:630-656`, `legacy-item-utils.js`, `legacy-preview-media-utils.js`, `legacy-render-utils.js:19-39` |

### 1.2 Findings

| ID | Sévérité | Confiance | Titre |
|---|---|---|---|
| AUD-V2-O-001 | Faible | Élevée | L'updater reste joignable via le menu tray (contourne `UPDATER_ENABLED_FOR_RELEASE`, requête réseau sortante) |
| AUD-V2-O-002 | Faible | Élevée | `overlay_get_config` expose `clientToken` aux fenêtres overlay et pairing (moindre privilège A6-002 partiel) |
| AUD-V2-O-003 | Info | Élevée | Doc : opt-in `__LIVECHAT_TWEET_WIDGETS_ENABLED` documenté mais inexistant dans le code ; message de log « fallback to raw HTML » trompeur |
| AUD-V2-O-004 | Info | Élevée | Doc/dev : compte « 13 méthodes » alors que l'API en expose 14 (`authorizedFetch` ajouté) ; `dev/README.md` encore rédigé « Phase 0 » |
| AUD-V2-O-005 | Info | Moyenne | CSP large sur `connect-src`/`img-src`/`media-src` (`http:`/`https:` génériques) — nécessaire au renderer legacy, canal d'exfiltration résiduel |
| AUD-V2-O-006 | Info | Moyenne | `config.json` écrite de façon non atomique et sans permissions explicites (contient `clientToken` en clair) — antérieur à P1 |
| AUD-V2-O-007 | Info | Élevée | Diagnostics du bridge actifs en release (`__bridgeMockEmit`, `__bridgeCalls`, journal) |
| AUD-V2-O-008 | Info | Élevée | `app.withGlobalTauri: true` conservé — pas de bypass constaté grâce à la matrice + capabilities, mais surface inutile |

**Compteurs** : 0 Élevée, 0 Moyenne, 2 Faibles, 6 Info.

### 1.3 Actions prioritaires

1. **O-001** — faire passer le chemin tray par `updater::check_for_update` (qui
   respecte le flag) ou masquer l'entrée « Vérifier les mises à jour » tant que
   l'updater est désactivé (≈ 0,25 j-h).
2. **O-002** — ne plus exposer `clientToken` aux fenêtres `overlay`/`pairing`
   (projection de config dédiée au board) (≈ 0,5 j-h).
3. **Doc** — corriger O-003/O-004 et nuancer O-001 dans
   `overlay-bridge.md` §12.5 et `overlay-release.md` §6 (≈ 30 min).

---

## 2. Méthode et preuves

1. **Revue statique** ligne à ligne des fichiers overlay touchés par `63e12f5`
   (`capabilities/`, `src/commands.rs`, `src/net/{mod,tls,pairing,ws}.rs`,
   `src/updater.rs`, `src/tray.rs`, `src/lib.rs`, `tauri(.beta).conf.json`,
   `web/bridge.js`, shims `web/dist/renderer/{board,overlay}/**`,
   `dev/README.md`, `docs/rust/overlay-*.md`).
2. **Greps adversariaux** ciblés : `danger_accept`, `insecure_tls`,
   `require_secure_scheme`, `innerHTML`, `insertAdjacentHTML`, `outerHTML`,
   `document.write`, `srcdoc`, `createElement('script')`, `token=`,
   `platform.twitter`, `twttr`, `widgets.js`, `Authorization`,
   `COMMAND_MATRIX`, `ensure_allowed`, `updater`, `debug-assertions`,
   `RUSTFLAGS`.
3. **Scan du binaire release** `target/release/lc-overlay.exe`
   (18 486 272 o, mtime `2026-11-10 13:59:14`) en ASCII **et** UTF-16LE.
4. **Fraîcheur du binaire** : le fichier source/asset overlay le plus récent
   (`web/dist/renderer/overlay/legacy-twitter-widgets-utils.js`, mtime
   `13:58:23`) est **antérieur** à l'exe (`13:59:14`) ; `git status` propre sur
   `apps/lc-overlay` et les docs overlay ⇒ aucune divergence détectée entre le
   binaire inspecté et l'arbre de travail. Le commit `63e12f5` (14:14) est
   postérieur au build mais n'a pas modifié les fichiers overlay après leur
   compilation (mtimes antérieurs).
5. **Limite** : aucun build ni lancement réel n'a été effectué (pas de
   WebView2, pas de réseau) — le scan binaire et le raisonnement statique
   remplacent l'exécution ; cf. §6.

---

## 3. Vérification par sous-point COR-11

### 3.1 Appairage et WS refusent `http://` par défaut — TENU

- La dérogation est centralisée dans `apps/lc-overlay/src/net/tls.rs` :
  `INSECURE_TLS_ENV = "LIVECHAT_INSECURE_TLS"` (`:16`),
  `insecure_tls_enabled()` (`:19-27`) et
  `require_secure_scheme(scheme, context, allow_insecure)` (`:48-60`) qui
  n'accepte `http|ws` que si `allow_insecure`, sinon renvoie une erreur
  explicite (`https`/`wss` toujours acceptés).
- **Appairage** : `pairing_endpoint_with` valide le schéma via
  `tls::require_secure_scheme` (`net/pairing.rs:121-136`) ; le client HTTP n'est
  rendu « insecure » que si la dérogation est active
  (`build_client(insecure_tls)` `:137-147`, appel `:242`).
- **WS** : `ws_url_from_server_url` propage `tls::insecure_tls_enabled()`
  (`net/ws.rs:460-466`) ; `ws_url_from_server_url_with` refuse `http|ws` sans
  dérogation ; `build_connector` ne crée un connecteur permissif que si
  `insecure_tls` (`:492-505`, `danger_accept_invalid_certs` +
  `danger_accept_invalid_hostnames` uniquement dans cette branche).
- **Aucun autre `danger_accept*`** dans le crate (grep exhaustif `src/**/*.rs`).
- Tests : `net/pairing.rs:321-355` (refus http, downgrade, upgrade) et
  `net/ws.rs:1066-1084` (http refusé par défaut, accepté avec `true`).

### 3.2 Dérogation limitée au debug — TENU (statique)

- `insecure_tls_enabled()` lit l'environnement puis, si `!cfg!(debug_assertions)`,
  émet un `warn!` explicite (« dérogation TLS ignorée : les builds release
  exigent un certificat vérifié ») et **retourne `false`**
  (`net/tls.rs:19-27`).
- Les deux clients concernés consomment cette unique source
  (`pairing.rs:116-117/242`, `ws.rs:440/460-461/492`).
- Aucune configuration de compilation ne rétablit `debug-assertions` :
  `.cargo/config.toml` ne contient que `[build] jobs`/`[net] retry` (aucun
  `rustflags`) ; aucun `profile.release.debug-assertions` dans les
  `Cargo.toml` ; `apps/lc-overlay/scripts/release-build.ps1` n'exporte aucun
  `RUSTFLAGS`. Le binaire ciblé est `target/release/` ⇒ `debug_assertions =
  false` par défaut ⇒ la dérogation est inopérante en release.
- **Réserve de confiance** : ce comportement n'est pas couvert par un test
  automatisé en profil release (cf. §6 et O-001/§7).

### 3.3 Refus du downgrade `apiBaseUrl` https→http — TENU

- `apply_consume_response` (`net/pairing.rs:159-186`) parse l'URL demandée et
  l'`apiBaseUrl` renvoyée : schéma hors `http|https` refusé (`:173-177`),
  **`requested.scheme() == "https" && parsed.scheme() == "http"` ⇒ erreur
  explicite** (`:173-183`, message « le serveur a renvoyé un apiBaseUrl http
  incompatible avec un appairage HTTPS (mise à jour du serveur requise) »).
  L'upgrade http→https reste accepté.
- Tests : `api_base_url_cannot_downgrade_https_to_http` (`:336-354`),
  `api_base_url_https_upgrade_is_accepted` (`:355-...`).

### 3.4 Carte tweet neutralisée + CSP stricte — TENU

- `web/dist/renderer/overlay/legacy-tweet-card-renderer.js` : le HTML distant
  n'est plus injecté ; `setTweetCardTextContent` (`:10-18`) parse dans un
  document **inerte** (`new DOMParser().parseFromString(..., 'text/html')`) puis
  affecte uniquement `node.textContent` (`:18`). Les deux points d'entrée
  utilisent ce helper (`:65-69`, `:88-92`) ; aucun `innerHTML` de contenu.
- `legacy-twitter-widgets-utils.js:8-10` : `ensureTwitterWidgets()` **jette
  systématiquement** `tweet_widgets_disabled` ; plus aucune création de
  `<script>`, ni URL `platform.twitter.com`/`twttr`. Le catch de
  `legacy-tweet-card-playback-utils.js:17-23` ne journalise qu'un avertissement
  (message « fallback to raw HTML » hérité, mais aucun fallback HTML n'existe).
- CSP (`tauri.conf.json:54`, identique `tauri.beta.conf.json:54`) :
  `script-src 'self'`, `frame-src 'none'`, `object-src 'none'`.
- Scan binaire : `platform.twitter.com` 0/0, `twttr` 0, `widgets.js` 0,
  `syndication.twitter` 0 (§8.1). Le seul `twitter-widgets` restant est le
  **nom de fichier local** du shim neutralisé (`/dist/renderer/overlay/
  legacy-twitter-widgets-utils.js`), pas une URL distante.
- Écart documentaire : O-003 / O-004.

### 3.5 Capabilities par fenêtre + `COMMAND_MATRIX` — TENU (résidu O-002)

- `capabilities/overlay.json:1-7`, `board.json:1-7`, `pairing.json:1-7` :
  une capability par label (`windows: ["overlay"|"board"|"pairing"]`),
  permissions réduites à **`core:default`** (plus aucun `fs:*`) ;
  `capabilities/default.json` **supprimé**. Labels confirmés
  (`src/windows.rs:14-16`) et fenêtres déclarées dans `tauri.conf.json`
  (overlay/pairing/board).
- `src/commands.rs:34-66` : `COMMAND_MATRIX` — overlay = config + télémétrie ;
  board = config + meme board + `overlay_check_updates` ; pairing = config +
  appairage. `command_allowed` (`:68-74`) et `ensure_allowed` (`:76-83`,
  warn + `Err`).
- Les **12 commandes** enregistrées (`src/lib.rs:196-208`) prennent
  `window: WebviewWindow<Wry>` et appellent `ensure_allowed` en première ligne
  (vérifié sur `overlay_get_config` `:199-212` et l'ensemble du fichier).
- `overlay_install_update` est enregistrée (`lib.rs:208`) mais **absente de
  toutes les lignes de la matrice** ⇒ refusée depuis les trois fenêtres
  (test `commands.rs:530+`).
- Test `command_matrix_is_window_scoped` (`commands.rs:530+`) couvre les
  combinaisons interdites, y compris commandes inconnues.
- **Résidu** : `overlay_get_config` est autorisé pour les 3 fenêtres et renvoie
  la projection 8 champs **incluant `clientToken`** (`config.rs:107-116`,
  `commands.rs:199-212`) ⇒ O-002.

### 3.6 Updater explicitement désactivé — PARTIEL (O-001)

- Côté commandes, c'est correct : `UPDATER_ENABLED_FOR_RELEASE = false`
  (`src/updater.rs:55`) ; `check_for_update` (`:69-74`) et `install_update`
  (`:102-107`) retournent `disabled(current)` **avant** d'appeler le plugin
  (aucune requête réseau par ces commandes).
- **Contournement** : le menu tray appelle un chemin parallèle
  `spawn_update_check` (`src/tray.rs:413-430`) qui utilise directement
  `tauri_plugin_updater::UpdaterExt` + `updater.check().await` et **ne consulte
  pas** `UPDATER_ENABLED_FOR_RELEASE` ; entrée de menu présente
  (`tray.rs:28`, `:207-213`) et dispatch `ID_CHECK_UPDATES` (`:445`).
- Conséquence : `check()` effectue une requête réseau vers l'endpoint GitHub
  configuré (`tauri.conf.json:74-77`). L'installation reste impossible en
  pratique (pubkey placeholder `:77`, `createUpdaterArtifacts: false` `:60`,
  artefacts non signés) ⇒ fail-closed, mais la désactivation n'est pas
  totale.
- Documentation : `overlay-release.md:11/366/396` affirme « explicitement
  désactivé » ; `:167-170` reconnaît qu'un clic renvoie une erreur explicite
  (donc l'appel réseau existe) ; `overlay-bridge.md:606-611` précise « sans
  requête réseau » pour les **commandes** seulement. Formulation globale
  imprécise ⇒ finding O-001 + correction doc.

### 3.7 Harnais de dev hors bundle — TENU

- `web/README.md` et `web/bridge-test.html` ont été déplacés vers
  `apps/lc-overlay/dev/` (`dev/bridge-test.html` 25 470 o,
  `dev/README.md` 9 452 o) ; `frontendDist` reste `"web"` (`tauri.conf.json:7`)
  donc `dev/` n'est pas embarqué.
- Scan binaire : `bridge-test.html` 0/0, `dev/bridge-test` 0, `/dev/` 0.
- `dev/README.md:1-4` documente explicitement l'exclusion (AUD-A6-015) — mais
  son contenu est par ailleurs obsolète (O-004).

### 3.8 Board via `Authorization: Bearer` — TENU

- `web/bridge.js:630-656` : `authorizedFetch(url, options)` lit la config via
  `getConfig()`, refuse `not_paired` sans token, pose
  `Authorization: Bearer <clientToken>` puis `fetch`. Méthode **additionnelle**
  (14<sup>e</sup> clé de l'API), les 13 méthodes legacy sont inchangées
  (`:625-711`).
- Shims board : `legacy-item-utils.js` utilise `authorizedFetch` pour
  `fetchItems`/`createItem`/`patchItemMetadata`/`deleteItem` ;
  `legacy-preview-media-utils.js` : `buildAuthedUrl` **ne pose plus**
  `?token=` ; `legacy-render-utils.js:19-39` récupère le binaire via le bridge
  et l'expose en `blob:` object URL (fallback sans bridge `:21-22`
  `mediaNode.src = mediaUrl`, inoffensif car les URLs média v2 sont signées et
  le bridge est toujours chargé par les pages).
- Grep `token=` sur `apps/lc-overlay/web/**` : une seule occurrence, dans un
  **commentaire** (`legacy-media-dom-utils.js:77`, « ne jamais ajouter
  `?token=` ») ⇒ aucune URL du renderer ne transporte le token.
- Le repli serveur `?token=` reste transitoire (hors périmètre overlay) et
  n'est plus utilisé par le client.

### 3.9 Vérifications adverses complémentaires

| Candidat de bypass | Résultat |
|---|---|
| Injection restante renderer | `innerHTML` restants = affectations `''` (nettoyage DOM) dans board/overlay ; **aucun** `insertAdjacentHTML`/`outerHTML`/`document.write`/`srcdoc` ; seul `createElement('script')` = `legacy-loader.js:1-7` (script **local** relatif à la page, `script-src 'self'`) |
| URLs/token en clair | Tokens absents des URLs renderer ; `overlay_get_config` reste un vecteur de lecture du token (O-002) |
| Updater | PARTIEL (O-001) |
| CSP trop large | `script-src`/`frame-src`/`object-src` stricts ; `connect-src`/`img-src`/`media-src` autorisent `http:`/`https:` (O-005) |
| Config non atomique/partagée | Mutations sérialisées par mutex (`state.rs:70-81`) mais écriture `fs::write` directe (`config.rs:190-196`) (O-006) |
| `withGlobalTauri` | `true` (`tauri.conf.json:10`) — les commandes restent filtrées par la matrice, les plugins par les capabilities ; pas de bypass trouvé (O-008) |
| Diagnostics bridge | `__bridgeMockEmit`/`__bridgeCalls` actifs en release ; nécessitent déjà une exécution de script dans la page (O-007) |

---

## 4. Findings détaillés

### AUD-V2-O-001 — Updater joignable via le tray (Faible / Élevée)

- **Preuve** : `apps/lc-overlay/src/tray.rs:28` (`ID_CHECK_UPDATES`),
  `:207-213` (entrée de menu active), `:413-430` (`spawn_update_check` appelle
  `handle.updater()` puis `updater.check().await` sans consulter le flag),
  `:445` (dispatch). À comparer à `src/updater.rs:55/69-74/102-107`.
  Configuration : `tauri.conf.json:60` (`createUpdaterArtifacts: false`),
  `:74-77` (endpoint GitHub + pubkey placeholder).
- **Impact** : egress réseau non désiré depuis le menu tray et incohérence avec
  l'affirmation « updater explicitement désactivé » ; **pas d'installation
  possible** (clé placeholder/artefacts non signés = fail-closed), pas
  d'élévation de privilège.
- **Correctif** : dans `spawn_update_check`, appeler
  `crate::updater::check_for_update(app)` (qui renvoie `disabled`) **ou**
  construire l'entrée de menu uniquement si `UPDATER_ENABLED_FOR_RELEASE` ; en
  complément, tracer explicitement l'état désactivé.
- **Test** : test unitaire/`#[cfg(test)]` vérifiant que le chemin tray
  n'invoque pas `UpdaterExt::updater()` quand le flag est `false` (extraire la
  décision dans une fonction pure testable).
- **Effort** : 0,25 j-h. **Doc** : nuancer `overlay-release.md:11/366/396`.

### AUD-V2-O-002 — `clientToken` exposé aux fenêtres overlay et pairing (Faible / Élevée)

- **Preuve** : `apps/lc-overlay/src/config.rs:107-116`
  (`OverlayConfigView.client_token: Option<String>`),
  `src/commands.rs:199-212` (`overlay_get_config` renvoie la projection
  complète), matrice autorisant `overlay_get_config` depuis les 3 fenêtres
  (`commands.rs:34-66`).
- **Impact** : moindre privilège — une compromission future de la page overlay
  ou pairing (contenu local, surface d'injection réduite par ailleurs)
  donnerait accès au Bearer board. A6-002 est donc corrigé pour le **transport**
  (plus de token en query) mais pas pour la **projection**.
- **Correctif** : exposer deux projections — `overlay_get_config` sans
  `clientToken` (overlay/pairing) et une commande `board_get_config` avec
  token, ou filtrer côté commande selon `window.label()` ; mettre à jour la
  matrice et le test.
- **Effort** : 0,5 j-h. Hors COR-11 strict (défense en profondeur).

### AUD-V2-O-003 — Doc : opt-in tweet documenté mais absent (Info / Élevée)

- **Preuve** : `docs/rust/overlay-bridge.md:574-576` affirme que
  `ensureTwitterWidgets` lève `tweet_widgets_disabled` « sauf si
  `window.__LIVECHAT_TWEET_WIDGETS_ENABLED === true` » ; le code
  (`web/dist/renderer/overlay/legacy-twitter-widgets-utils.js:8-10`) **jette
  toujours**, sans lire aucun flag. Message de log trompeur
  `legacy-tweet-card-playback-utils.js:22` (« fallback to raw HTML ») alors
  qu'aucun fallback HTML n'existe.
- **Impact** : aucun risque sécurité (CSP bloquerait de toute façon un script
  tiers) ; risque de régression/confusion pour un futur mainteneur qui
  activerait le flag sans effet.
- **Correctif** : doc → « toujours désactivé ; le flag n'existe pas » ; ou
  implémenter le flag derrière `#[cfg(debug_assertions)]` avec garde CSP.
  Renommer le message de log.
- **Effort** : 15 min.

### AUD-V2-O-004 — Doc/dev : compte des méthodes et README obsolètes (Info / Élevée)

- **Preuve** : `web/bridge.js:4/18` annoncent « 13 méthodes » ; l'API exposée en
  compte **14** (`authorizedFetch`, `bridge.js:632-656/711`) et les docs
  `overlay-bridge.md:32/41/529/551` parlent encore de « 13 méthodes » tandis
  que `:226-228/597-604` documentent l'extension. `apps/lc-overlay/dev/README.md:3-12/25-30`
  décrit encore « Phase 0 », un « squelette sans dépendance Tauri » et
  « 13 méthodes ».
- **Impact** : documentaire uniquement (pas de risque d'exécution) ; le contrat
  public consommé par le renderer reste les 13 méthodes legacy + extension
  additive.
- **Correctif** : clarifier « 13 méthodes legacy + `authorizedFetch` (14 au
  total) » dans la doc et le README de dev ; mettre à jour `dev/README.md`.
- **Effort** : 30 min.

### AUD-V2-O-005 — CSP large sur connect/img/media (Info / Moyenne)

- **Preuve** : `tauri.conf.json:54` et `tauri.beta.conf.json:54` :
  `img-src ... http: https:`, `media-src ... http: https:`,
  `connect-src ... http: https: ws: wss:`.
- **Impact** : nécessaire au renderer legacy (médias distants et serveur
  configurable) et `script-src`/`frame-src` restent stricts ; si une injection
  réapparaissait, ces directives offriraient un canal d'exfiltration
  (fetch/img). Risque résiduel, pas un bypass P1.
- **Correctif (Phase 9)** : restreindre `connect-src` à l'origine API et
  `img-src`/`media-src` aux CDN/médias nécessaires, avec validation runtime.
- **Effort** : 1–2 j-h (validation renderer legacy).

### AUD-V2-O-006 — Persistance config non atomique et sans permissions (Info / Moyenne)

- **Preuve** : `apps/lc-overlay/src/config.rs:190-196` (`fs::write` direct du
  JSON) ; mutations sérialisées par mutex (`src/state.rs:70-81`) mais pas de
  `tmp + rename` ni de permissions explicites ; `config.json` contient
  `clientToken` en clair dans `%APPDATA%\LiveChat\`.
- **Impact** : corruption possible si crash pendant l'écriture (les lecteurs
  tolèrent les erreurs), confidentialité dépendante des ACL Windows ; antérieur
  à P1, pas un bypass des correctifs.
- **Correctif** : écriture `tmp` + `rename` (comme `admin_runtime.json` côté
  serveur), permissions restreintes documentées.
- **Effort** : 0,5 j-h.

### AUD-V2-O-007 — Diagnostics du bridge actifs en release (Info / Élevée)

- **Preuve** : `web/bridge.js:716-735` expose `window.__livechatOverlayBridge`
  (journal, commandes, `resetCallLog`), `window.__bridgeCalls`,
  `window.__bridgeLastEvent` et `window.__bridgeMockEmit`.
- **Impact** : surface de test (simulation play/stop/settings au niveau
  listeners) accessible à tout script s'exécutant dans la page ; pas
  d'escalade sans injection préalable, et aucune donnée secrète n'y est
  journalisée (masquage `SENSITIVE_KEY_PATTERN`, `bridge.js:156-187`).
- **Correctif** : garder `__bridgeMockEmit` derrière un flag de dev
  (`!__TAURI_INTERNALS__` déjà vrai en mock) ou ne conserver que le journal.
- **Effort** : 0,25 j-h.

### AUD-V2-O-008 — `withGlobalTauri: true` conservé (Info / Élevée)

- **Preuve** : `tauri.conf.json:10` (et beta `:10`). Le bridge détecte
  `__TAURI_INTERNALS__.invoke` en premier (`web/bridge.js`), donc
  `withGlobalTauri` n'est pas nécessaire au fonctionnement.
- **Impact** : expose `window.__TAURI__` (APIs core/plugins) à la page ; les
  commandes applicatives restent filtrées par `COMMAND_MATRIX` et les plugins
  par les capabilities (`core:default` uniquement) ⇒ aucun bypass trouvé.
- **Correctif (Phase 9)** : passer `withGlobalTauri: false` après validation du
  renderer (les écoutes d'événements utilisent le repli interne documenté).
- **Effort** : 0,25 j-h + smoke.

---

## 5. Points forts

1. **TLS centralisé** : une seule source de décision
   (`net/tls.rs`) consommée par pairing et WS ; la dérogation est inopérante en
   release (raisonnement statique) et toujours journalisée en warn.
2. **Refus explicites** des schémas non chiffrés et **anti-downgrade**
   `apiBaseUrl` https→http, avec tests unitaires dédiés.
3. **Moindre privilège IPC réellement appliqué** : les 12 commandes prennent la
   fenêtre et passent par `ensure_allowed` ; `overlay_install_update` est
   refusée partout ; capabilities par fenêtre et suppression de `default.json`.
4. **Injection renderer éliminée** : plus aucun sink HTML avec donnée distante
   (carte tweet via `DOMParser` inerte + `textContent`), aucun
   `insertAdjacentHTML`/`srcdoc`, et aucun script tiers embarqué.
5. **CSP stricte sur les vecteurs de code** : `script-src 'self'`,
   `frame-src 'none'`, `object-src 'none'`, confirmée dans les deux configs.
6. **Board authentifié par en-tête** : `authorizedFetch` + migration des 4
   appels REST et des médias ; aucune URL renderer ne contient `?token=`.
7. **Harnais hors bundle** : déplacement `web/` → `dev/` + `frontendDist: web`
   confirmé par scan du binaire release.
8. **Binaire inspecté cohérent** : exe postérieur au dernier fichier overlay,
   `git status` propre, absence des motifs interdits en ASCII **et** UTF-16.

---

## 6. Zones non couvertes / limites

- **Pas d'exécution** du binaire release ni de WebView2 : la vérification de
  `cfg!(debug_assertions)` en release est **statique** (pas de test automatisé
  release), et l'application des capabilities/CSP n'a pas été observée à
  l'exécution.
- **Pas de build** : le binaire inspecté est celui déjà présent
  (`target/release/lc-overlay.exe`, 13:59:14) ; l'absence de `bridge-test.html`,
  `platform.twitter.com`, `twitter-widgets` y est vérifiée par scan de motifs,
  pas par exécution.
- **Réseau non testé** : endpoint GitHub de l'updater non contacté ; le
  contournement tray est établi par lecture de code, pas par capture réseau.
- **Pas de revue du code serveur** ni des autres lanes P1 (hors périmètre) ;
  le repli `?token=` serveur reste transitoire et hors overlay.
- **Scan binaire partiel** : motifs demandés + quelques variantes
  (`twttr`, `syndication.twitter`, `widgets.js`, `dev/`), pas une analyse
  complète de toutes les chaînes.
- **Confiance** : élevée sur les verdicts 1, 3, 4, 5, 7, 8 ; bonne mais
  statique sur 2 ; le verdict PARTIEL de 6 est certain (code), seule
  l'exploitabilité est nulle (fail-closed).

---

## 7. Recommandations

| Priorité | Action | Réf. | Effort |
|---|---|---|---|
| P1 | Tray : passer par `updater::check_for_update` ou masquer l'entrée si désactivé | O-001 | 0,25 j-h |
| P1 | Séparer la projection config board (token) des vues overlay/pairing | O-002 | 0,5 j-h |
| P2 | Corriger docs (`overlay-bridge.md` §12.2/§12.5, comptes 13/14, `dev/README.md`) | O-003/O-004 | 0,5 j-h |
| P2 | Désactiver les diagnostics de test en release | O-007 | 0,25 j-h |
| P3 | Resserrer CSP `connect/img/media` et `withGlobalTauri` | O-005/O-008 | 1–2 j-h |
| P3 | Config atomique + permissions ; événements d'audit updater | O-006/O-001 | 0,5 j-h |

**Conclusion** : les correctifs P1/COR-11 sont globalement tenus ; les deux
écarts Faibles sont des durcissements de moindre privilège/désactivation, sans
exploitation démontrable compte tenu de la CSP stricte, de l'absence
d'injection et du fail-closed de signature.

---

## 8. Annexes

### 8.1 Scan du binaire release

`target/release/lc-overlay.exe` — 18 486 272 o — mtime `2026-11-10 13:59:14`.
Recherche ASCII **et** UTF-16LE (nombre d'occurrences) :

| Motif | ASCII | UTF-16 | Interprétation |
|---|---|---|---|
| `bridge-test.html` | 0 | 0 | harnais non embarqué |
| `platform.twitter.com` | 0 | 0 | aucun script tiers |
| `twitter-widgets` | 1 | 0 | **chemin local** `/dist/renderer/overlay/legacy-twitter-widgets-utils.js` (nom de shim neutralisé) |
| `widgets.js` | 0 | 0 | aucun script widget |
| `twttr` | 0 | 0 | aucun global Twitter |
| `syndication.twitter` | 0 | 0 | aucun domaine Twitter |
| `dev/bridge-test` | 0 | 0 | harnais hors bundle |
| `renderer-errors.js` | 1 | 0 | page charge son script local (attendu) |
| `LIVECHAT_INSECURE_TLS` | 2 | 0 | constante compilée, gating runtime release |
| `overlay_install_update` | 1 | 0 | nom de commande enregistrée (refusée par la matrice) |

Fraîcheur : dernier fichier overlay modifié `13:58:23` < exe `13:59:14` ;
`git status` propre sur `apps/lc-overlay` et les deux docs overlay.

### 8.2 Commandes de preuve (réutilisables)

```powershell
# refus http / downgrade / dérogation
Select-String -Path apps/lc-overlay/src/net/*.rs -Pattern 'require_secure_scheme|danger_accept|insecure_tls_enabled'
# matrice et garde par fenêtre
Select-String -Path apps/lc-overlay/src/commands.rs -Pattern 'COMMAND_MATRIX|command_allowed|ensure_allowed'
# sinks d'injection restants
Get-ChildItem apps/lc-overlay/web -Recurse -Include *.js | Select-String -Pattern 'innerHTML|insertAdjacentHTML|outerHTML|document.write|srcdoc'
# token en URL
Get-ChildItem apps/lc-overlay/web -Recurse -Include *.js,*.html | Select-String -Pattern 'token='
# scan binaire ASCII + UTF-16
$b = [IO.File]::ReadAllBytes('target/release/lc-overlay.exe')
[Text.Encoding]::ASCII.GetString($b).Contains('platform.twitter.com')
```

### 8.3 Fichiers inspectés

`apps/lc-overlay/{tauri.conf.json,tauri.beta.conf.json}`, `apps/lc-overlay/capabilities/{overlay,board,pairing}.json`,
`apps/lc-overlay/src/{lib.rs,commands.rs,updater.rs,tray.rs,windows.rs,config.rs,state.rs}`,
`apps/lc-overlay/src/net/{mod.rs,tls.rs,pairing.rs,ws.rs}`,
`apps/lc-overlay/web/{bridge.js,overlay.html,board.html,pairing.html,renderer-errors.js}`,
`apps/lc-overlay/web/dist/renderer/**` (board + overlay, via grep ciblé),
`apps/lc-overlay/dev/{README.md,bridge-test.html}`,
`docs/rust/{overlay-bridge.md,overlay-release.md}`,
`docs/rust/audit/{p1-correctifs.md,06-overlay-tauri.md}`,
binaire `target/release/lc-overlay.exe`.
