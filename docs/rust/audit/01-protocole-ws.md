# Audit A1 — Protocole v2 & hub WebSocket (lc-server)

| Champ | Valeur |
|---|---|
| Audit | A1 — communication temps réel (protocole v2, hub WS, interaction moteur) |
| Auditeur | `poc-server` (indépendant de l'implémentation) |
| Date | 2026-11-10 |
| Commit audité | `e0a381f` (branche `feat/rust-rebuild`), aucun fichier du périmètre modifié au moment de l'audit |
| Spécification de référence | `docs/rust/protocol-v2.md`, **gelée** (Brouillon v2, 2026-11-09, 960 lignes) |
| Périmètre code | `apps/lc-server/src/ws/{mod,hub,session,tests}.rs`, `apps/lc-server/src/{app.rs,wiring.rs,lib.rs,shutdown.rs}`, `apps/lc-server/src/http/admin/{clients,guilds,hub}.rs`, `crates/lc-protocol/**`, `crates/lc-playback/src/{engine,store,payload}.rs` |
| Méthode | revue statique ligne à ligne + tests ciblés `cargo test -p lc-server ws --locked` + 37 probes dynamiques sur serveur isolé (port 3499, DB `.phase0/audit/A1/audit.db`) |
| Livrable | ce rapport uniquement — **aucun code modifié, aucun commit** |

## 0. Verdict de synthèse

Le chemin nominal du protocole v2 est **globalement conforme** : handshake, authentification, enveloppe, codes de fermeture, messages C→S/S→C, unicast/broadcast, lecture seule, reconnexion et isolation par guilde se comportent comme spécifié et sont couverts par 23 tests + probes dynamiques. **Aucun finding Bloquant** (pas de faille d'auth, pas de fuite de données inter-guilde, pas de corruption d'état persisté constatée).

**5 findings Majeurs** : trois écarts de comportement visibles côté produit (`play` texte-seul jamais émis, arrêt `revoked` absent, arrêt `purge` émis en `manual`), un plafond `peers` non documenté (8) qui contredit la spec, et une supervision du moteur de lecture absente (mort silencieuse possible après erreur DB).

| Sévérité | Nombre | IDs |
|---|---|---|
| Bloquant | 0 | — |
| Majeur | 5 | AUD-A1-001 … 005 |
| Mineur | 7 | AUD-A1-006 … 012 |
| Nit | 2 | AUD-A1-013, 014 |

Probes : **37 vérifications, 32 PASS, 5 FAIL** — les 5 FAIL correspondent exactement aux écarts AUD-A1-002, 003, 006, 007 (×2). Le finding 001 (job texte-seul) est prouvé par une probe *positive* (comportement défectueux reproduit de façon déterministe).

Recommandation de priorisation : **001/002/003 avant toute release 3.0** (contrat observé par l'overlay), **005** avant mise en production (risque d'arrêt silencieux de la lecture), **004** décision produit/doc, **006–014** à traiter en Phase 4/backlog.

## 1. Findings

Notation : **Sévérité** (Bloquant/Majeur/Mineur/Nit), **Confiance** (haute = prouvé statiquement *et* dynamiquement ; moyenne = prouvé statiquement, non reproduit dynamiquement ; basse = à confirmer). Preuve = `fichier:ligne` + commande/JSON reproductible. Effort : XS ≤ 0,5 j, S ≤ 1 j, M 1–2 j.

### AUD-A1-001 — Un job `play` sans média (texte seul) n'est jamais diffusé et est marqué `FAILED`

- **Sévérité** : Majeur — **Confiance** : haute (statique + dynamique).
- **Composant** : `apps/lc-server/src/ws/hub.rs:726-768` (`signed_play_payload`), `apps/lc-server/src/ws/hub.rs:264-299` (`unicast_play`), `crates/lc-playback/src/engine.rs:662-676` (`run_guild`).
- **Preuve statique** :
  - `hub.rs:732` — `let media = payload.media.as_ref()?;` : si `payload.media` est `null` (cas explicitement autorisé par la spec, §6.2 l.415-443 « media = null si le job n'a pas de média »), la fonction retourne `None` et `unicast_play` **saute silencieusement** la connexion (`hub.rs:280-291`).
  - `engine.rs:662-670` — `let recipients = self.sink.on_play(...).await; if recipients == 0 { warn!("aucun overlay connecté : job marqué FAILED"); finish(Failed); ... }` : le compteur ne compte que les `play` signés, donc un job texte-seul est marqué `FAILED` **même avec des overlays connectés**.
  - Conséquence secondaire : `hub.rs:308-321` (`signed_current_job`) retourne `None` pour un job texte-seul → `welcome.currentJob` **absent** alors qu'un job est `PLAYING`, contraire à §6.1 l.378-391.
- **Preuve dynamique** (probe `text-only-job-dropped`, `.phase0/audit/A1/probe-results.json`) : un client connecté (room non vide, `roomPeers:1`), job texte inséré en base (`44444444-4444-4444-8444-000000000001`, média `NULL`), attente 2,2 s → `playCount:0`, `jobStatus:"FAILED"`. Log serveur : `WARN lc_playback::engine: aucun overlay connecté : job marqué FAILED guild=… job=44444444-…0002` alors que le client était connecté.
- **Impact** : toute lecture sans média (mèmes texte, cartes riches, futurs jobs texte) est perdue et marquée en échec ; l'overlay n'affiche rien ; la reconnexion ne peut pas reprendre ce job. La fixture gelée `v2_play_text_author.json` (media `null`) est valide côté parsing mais ne peut jamais être produite par le serveur.
- **Correctif** : rendre la signature conditionnelle au média (`play` émis sans URL si `media == null`) ; compter dans `recipients` **tous** les sockets servis, média ou non ;
  `signed_current_job` doit renvoyer le payload tel quel quand `media == null`.
- **Tests à ajouter** : (1) test `ws` : job texte-seul + 1 client connecté → `play` reçu, statut `PLAYING` puis `stop` ; (2) test unitaire moteur : `on_play` retourne 1 pour un payload sans média ; (3) `welcome` avec currentJob texte-seul (media null).
- **Effort** : S.

### AUD-A1-002 — Révocation à chaud : fermeture 4402 sans `stop {reason:"revoked"}` préalable

- **Sévérité** : Majeur — **Confiance** : haute (statique + dynamique).
- **Composant** : `apps/lc-server/src/http/admin/clients.rs:28,31-73` (`revoke_overlay`), `apps/lc-server/src/ws/hub.rs:517-536` (`close_client`), `apps/lc-server/src/http/admin/hub.rs:37-60` (trait `AdminHub`).
- **Preuve statique** : `clients.rs:56` — `let closed = hub.close_client(&client.id.as_uuid(), REVOKED_CLOSE_CODE)` ; `hub.rs:522-536` n'envoie que `OutFrame::Close(4402, "revoked")`. Aucun appel à `stop_with_reason(..., StopReason::Revoked)` / `broadcast_stop` n'existe dans le chemin de révocation.
- **Preuve dynamique** (probe `revocation-stop-then-4402`) : `revoke` → HTTP 200 `{found:true,revoked:true}`, close reçu `{"code":4402,"reason":"revoked"}`, mais **aucune trame `stop`** (`stop:null`).
- **Spec violée** : §10 l.734-736 et §6.3 l.462-477 : « stop `revoked` (jobId null) puis close 4402 juste après » ; §13 l.756 même mapping.
- **Impact** : l'overlay ne reçoit pas l'ordre de stopper la lecture avant la fermeture ; un client lent ou qui traite le close après le rendu continue de jouer jusqu'à la fermeture du socket ; ordre garanti §1.5 cassé ; l'invariant « stop jamais avant play » reste respecté mais pas la séquence de révocation.
- **Correctif** : avant `close_client(..., 4402)`, appeler le moteur : `stop_with_reason(guild, None, StopReason::Revoked)` puis fermer (ajouter une méthode `broadcast_stop_with_reason` au trait `AdminHub`, voir AUD-A1-003).
- **Tests à ajouter** : test `ws` « revocation pendant lecture » : 2 clients, job en cours, révocation admin → socket ciblé reçoit `stop {reason:"revoked", jobId:null}` **puis** close 4402 ; l'autre socket reçoit aussi le stop (broadcast room).
- **Effort** : S (avec AUD-A1-003).

### AUD-A1-003 — Purge admin : stop diffusé en `manual` (au lieu de `purge`) et avec `jobId` non nul

- **Sévérité** : Majeur — **Confiance** : haute (statique + dynamique).
- **Composant** : `apps/lc-server/src/http/admin/guilds.rs:172-318` (purge, appel l.204), `apps/lc-server/src/ws/hub.rs:538-554` (`broadcast_stop`), `apps/lc-server/src/http/admin/hub.rs:51-55`.
- **Preuve statique** : `guilds.rs:204` — `let broadcast = hub.broadcast_stop(&guild_id).await;` ; `hub.rs:544-546` — `engine.stop_with_reason(guild_id, None, StopReason::Manual)` : la raison est **codée en dur** ; le trait `AdminHub::broadcast_stop(&GuildId)` ne permet pas de choisir la raison (`http/admin/hub.rs:55`).
- **Preuve dynamique** (probe `purge-stop-reason`) : purge 200 `{purged:true,…}` ; trame reçue : `{"reason":"manual","jobId":"8225d611-…"}` alors que la spec attend `reason:"purge"` et `jobId:null` (§6.3 l.462-477 ; §13 l.749 « purge admin → stop purge + libération »). La purge libère bien les jobs côté HTTP (`released_playing_jobs:1`) mais l'ordre client est faux.
- **Impact** : l'overlay ne peut pas distinguer une purge (réinitialisation complète, y compris texte/auteur et état de session) d'un arrêt manuel ; risque d'état résiduel affiché après purge ; non-conformité à un contrat gelé.
- **Correctif** : introduire `broadcast_stop_with_reason(&GuildId, Option<GuildId>, StopReason)` dans `AdminHub`/`Hub` (réutilisable pour 002) ; purge → `StopReason::Purge`, `jobId = None` ; stop manuel → `Manual` avec jobId si unique.
- **Tests à ajouter** : test admin purge : job en cours → `stop {reason:"purge", jobId:null}` avant la fermeture des sockets ; test unitaire du mapping `AdminHub`.
- **Effort** : S.

### AUD-A1-004 — Plafond `peers` = 8 non documenté et silencieux (9ᵉ overlay absent des listes)

- **Sévérité** : Majeur — **Confiance** : haute (dynamique + statique).
- **Composant** : `apps/lc-server/src/ws/hub.rs:36` (`MAX_PEERS = 8`), `hub.rs:184-208` (`peer_list`, `peers.truncate(MAX_PEERS)`).
- **Preuve statique** : `hub.rs:208` tronque la liste triée (`label`, puis `client_id`) ; la spec §6.4 l.479-487 définit la déduplication et le tri mais **ne mentionne aucun plafond** (l'exemple §6.4 a 2 pairs, la fixture `v2_peers_empty.json` gère la room vide).
- **Preuve dynamique** (probe `peers-cap-8-undocumented`) : 9 clients distincts connectés dans la même guilde → chaque client voit exactement 8 pairs (`lengths:[8,8,8,8,8,8,8,8,8]`), labels `author-01…author-08` triés ; le 9ᵉ (`audit-ro`) est absent **de sa propre liste**.
- **Impact** : dans une room à plus de 8 overlays (usage réel : plusieurs écrans/participants), la présence affichée est fausse pour tous les clients au-delà du 8ᵉ ; parité v1 rompue (v1 diffusait la liste complète) ; le comportement n'est pas documenté, donc un test de conformité v2 le considérera comme un bug.
- **Correctif** : soit retirer le plafond (le payload reste < 64 Kio : 8 → 100 pairs ≈ quelques Kio), soit le rendre configurable et **l'ajouter à la spec** + au payload (`truncated`/`total` impossible sans casser la spec → préférer le retrait ou un plafond très haut documenté).
- **Tests à ajouter** : 10 sockets même guilde → liste de 10 pairs identique et triée ; test de non-régression sur la taille du payload.
- **Effort** : S.

### AUD-A1-005 — Mort silencieuse du moteur de lecture : aucune supervision ni alerte après erreur de persistance

- **Sévérité** : Majeur — **Confiance** : moyenne (statique convaincante ; **non reproduit** dynamiquement — voir preuve).
- **Composant** : `crates/lc-playback/src/engine.rs:507-529` (`run`), `apps/lc-server/src/ws/hub.rs:399-479` (`ensure_runtime`, spawn l.439-443).
- **Preuve statique** :
  - `engine.rs:509-528` : `bootstrap().await?` puis boucle `select! { shutdown, sleep(tick_interval) => self.tick().await? }` ; **toute erreur de tick (DB verrouillée/pleine, I/O, migration) interrompt définitivement la boucle** et remonte à l'appelant. Le commentaire `engine.rs:507-508` précise : « le serveur décide de la politique de redémarrage ».
  - `hub.rs:439-443` : le spawn se contente de `if let Err(error) = … { tracing::error!(error = %error, "moteur de lecture arrêté sur erreur"); }` — **pas de redémarrage, pas de signal, pas d'état exposé**. `Hub::engine` est un `OnceCell` : une fois initialisé, `engine()` continue de renvoyer l'Arc du moteur mort ; `broadcast_stop`/`unicast_play` seront des no-ops silencieux et `/health` ne sonde pas le moteur (statut `ok` si DB/Discord ok).
- **Preuve négative (non reproduction)** : probe `engine-alive-after-wal-lock` — verrou `BEGIN EXCLUSIVE` de 9 s par une autre connexion (`.phase0/audit/A1/lockdb.py`) : en mode WAL, les **lectures** ne sont pas bloquées, le moteur a survécu et le job inséré par le verrou a été traité (`playAfterLock:1`, `lockJobStatus:"FAILED"` car texte-seul — cf. 001). La panne n'est donc pas reproductible avec un simple verrou WAL ; elle reste possible sur erreur SQLite réelle (disque plein, `database is locked` sur écriture pendant > `busy_timeout` 5 s, corruption).
- **Impact** : après une erreur transitoire unique, **toute la lecture s'arrête pour toutes les guildes jusqu'au redémarrage du process** ; un seul log ERROR, aucune alerte, `/health` reste `ok` ⇒ panne silencieuse en production.
- **Correctif** : superviser le moteur — boucle de redémarrage avec backoff dans `hub.rs` (ou `EngineSupervisor`) + exposition d'un état dans `/health` (`engine:"ok"|"down"`) ; à défaut, faire remonter l'erreur jusqu'à `serve()` pour arrêt contrôlé.
- **Tests à ajouter** : test d'intégration avec un store qui échoue une fois (`bootstrap`/`tick` en erreur) → le superviseur redémarre le moteur (ou `/health` passe `degraded`) ; test d'injection `database is locked` sur écriture > 5 s.
- **Effort** : M.

### AUD-A1-006 — Trame > 256 Kio fermée en `1006` (anormal) au lieu de `4408 frame_too_large`

- **Sévérité** : Mineur — **Confiance** : haute (dynamique).
- **Composant** : `apps/lc-server/src/ws/session.rs:51` (`MAX_UPGRADE_MESSAGE_BYTES = 256 KiB`), `session.rs:91-110` (`.max_message_size/.max_frame_size`).
- **Preuve dynamique** (probes `frame-100kio-4408` et `frame-300kio-expected-4408`) : trame de 100 Kio → close `{"code":4408,"reason":"frame_too_large"}` ✅ ; trame de 300 Kio → close `{"code":1006,"reason":""}` (tungstenite ferme anormalement avant notre vérification `raw.len() > MAX_FRAME_BYTES`).
- **Spec** : §2 l.186 et §4 l.231 : toute trame > 64 Kio côté C→S → close `4408` ; `1006` n'existe pas dans la table des codes §3 l.203-212.
- **Impact** : le client ne reçoit pas l'erreur protocolaire actionnable (bug client à corriger) mais une fermeture anormale ; non-détection d'un client défectueux/attaquant ; conformance des close codes.
- **Correctif** : relever `max_message_size` (ex. 1 Mio) et conserver la vérification applicative 64 Kio avant parsing (chemin déjà présent), afin que 64 Kio–1 Mio → 4408 ; documenter le comportement au-delà de la limite de transport (ou borner à 1 Mio avec 1009 documenté).
- **Tests à ajouter** : test `ws` trame `MAX_FRAME_BYTES+1` (existe ✅) ; ajouter 1 Mio et 300 Kio attendant 4408 (tant que sous la limite transport).
- **Effort** : S.

### AUD-A1-007 — Aucune validation de `id` (non vide, ≤ 64) : les trames invalides sont acceptées et `ack.ref` ré-émet l'id tel quel

- **Sévérité** : Mineur — **Confiance** : haute (dynamique + statique).
- **Composant** : `crates/lc-protocol/src/lib.rs:285-315` (`Deserialize` de `Envelope`), `apps/lc-server/src/ws/session.rs` (parsing direct, aucune validation supplémentaire).
- **Preuve statique** : `Envelope::deserialize` retire/valide seulement `v`, `id`, `ts` (types serde) puis parse le message ; **aucun contrôle de longueur/« non vide »** de `id`. Le text de doc `lc-protocol` (`lib.rs:239`) mentionne « non vide, unique par connexion » mais aucune règle n'est appliquée.
- **Preuve dynamique** (probes `after-welcome-id-65-spec-1002`, `after-welcome-id-empty-spec-1002`) : id de 65 caractères → `ack {"ok":true,"ref":"iiii…(65)"}` sans fermeture ; id vide → `ack {"ok":true,"ref":""}`.
- **Spec** : §2 l.180 (id non vide ≤ 64) et §4 l.243 (« id ≤ 64 sinon 1002 »).
- **Impact** : frames non conformes acceptées ; `ack.ref` peut ré-émettre un id très long (jusqu'à ~65 Kio dans les limites de trame) au point de frôler/dépasser la garantie « messages S→C < 64 Kio » (§2 l.186) et de faire échouer des clients stricts ; bruit de diagnostic.
- **Correctif** : valider dans `Deserialize` (`id.is_empty() || id.chars().count() > 64` → erreur custom) ; `session.rs` mappe déjà l'erreur de parse en `1002 protocol_error`.
- **Tests à ajouter** : fixtures/tests unitaires `Envelope` id vide/65 ; tests `ws` (même scénario que la probe) attendant 1002.
- **Effort** : XS.

### AUD-A1-008 — Liveness : toute trame (dont `heartbeat`) rafraîchit `last_activity`, donc un client sans Pong n'est pas fermé à 45 s

- **Sévérité** : Mineur — **Confiance** : haute (statique ; non observable avec le client Node qui auto-répond Pong).
- **Composant** : `apps/lc-server/src/ws/session.rs:208-209,220,290-291,314-321`.
- **Preuve statique** : `last_activity` est mis à jour sur **toute** trame texte (`session.rs:216-220`) et sur Ping/Pong (`session.rs:290-291`) ; le watchdog ne teste que `last_activity.elapsed() >= heartbeat_timeout` (`session.rs:314`), sans suivre de `last_pong` distinct.
- **Spec** : §1.3 l.101-106 : Ping toutes les 15 s ; « sans `Pong` (ni aucune trame) pendant 45 s → 1001 » et surtout « un client qui envoie des `heartbeat` mais ne répond pas aux `Ping` **est fermé** » — le heartbeat ne remplace pas le ping/pong. Le code traite le heartbeat comme une activité suffisante.
- **Impact** : un client applicatif qui envoie des heartbeats mais ne lit plus (réponse bloquée, event-loop gelée, client non conforme) n'est pas fermé : sockets zombies, buffers serveur qui se remplissent (file `OutFrame` non bornée), présence `lastSeenAt` faussement à jour.
- **Correctif** : suivre `last_pong` séparément ; fermer 1001 `heartbeat_timeout` si aucun Pong depuis 45 s (les heartbeats ne comptent pas), tout en gardant `last_activity` pour les autres diagnostics.
- **Tests à ajouter** : test `ws` avec un client qui envoie heartbeat mais ignore les Ping → close 1001 à ~45 s (test tokio temps virtuel si possible) ; test émission Ping toutes les 15 s.
- **Effort** : S.

### AUD-A1-009 — Course dans `ensure_runtime` : `engine()` peut être visible avant `db`/`config`

- **Sévérité** : Mineur — **Confiance** : moyenne (statique ; fenêtre très courte, non reproduite).
- **Composant** : `apps/lc-server/src/ws/hub.rs:399-479` (early-return l.403-405, publication l.475-476).
- **Preuve statique** : `get_or_try_init` crée le moteur et le publie dans `self.engine` **avant** `drop(self.db.set(db))` / `drop(self.config.set(config))` (l.475-476) ; `unicast_play` exige `self.config.get()` (l.265-270) et `signed_current_job` exige aussi la config (l.314). Une seconde session concurrente qui appelle `ensure_runtime` pendant la fenêtre prend l'early-return (`engine` déjà présent), puis un `play`/`welcome` peut trouver `config = None` → `unicast_play` retourne 0 (job dispatch marqué `FAILED`, cf. `engine.rs:662-676`) ou `currentJob` absent.
- **Impact** : à la toute première connexion (initialisation simultanée de plusieurs overlays), un job peut être perdu ou un `welcome.currentJob` omis ; pas de corruption durable mais comportement non déterministe difficile à diagnostiquer.
- **Correctif** : publier un seul `OnceCell<Arc<HubRuntime { engine, db, config }>>` (un seul point de visibilité), ou initialiser `db`/`config` avant de lancer le moteur, ou sérialiser `ensure_runtime` par un verrou d'initialisation `tokio::sync::Mutex`.
- **Tests à ajouter** : test concurrent (N connexions simultanées déclenchant `ensure_runtime` + un job en attente) → tous les welcome/currentJob et `play` corrects.
- **Effort** : S.

### AUD-A1-010 — Anomalie multi-`PLAYING` : le moteur conserve le plus ancien alors que la spec impose le plus récent

- **Sévérité** : Mineur — **Confiance** : haute (statique, ordre SQL vérifié).
- **Composant** : `crates/lc-playback/src/engine.rs:461-490` (`current_job`), `engine.rs:695-717` (`handle_multi_playing`), `crates/lc-playback/src/store.rs:47-64` (`playing_jobs` `ORDER BY started_at ASC, id ASC`).
- **Preuve statique** : la liste `playing_jobs` est **croissante** (plus ancien → plus récent). `current_job` fait `playing.pop()` → plus récent ✅ (conforme §6.1.1 l.393-413). Mais `handle_multi_playing` fait `for extra in playing.iter().skip(1) { finish(Done) }` → il **termine les plus récents et conserve le plus ancien**, alors que son warn (`engine.rs:707`) affirme « libération des plus anciens » et que la spec §6.1.1/§15.3 impose d'exposer le plus récent. Les jobs terminés le sont **sans trame `stop`** (silencieux).
- **Impact** : en cas d'anomalie de données (invariant cassé), l'overlay expose le **plus vieux** job et perd les récents sans notification ; l'état de récupération diverge du contrat de reprise.
- **Correctif** : terminer tous les éléments **sauf le dernier** (`playing.split_last()`/`[..len-1]`) ou inverser l'ordre ; aligner le message de warn ; émettre un `stop` interne si le job en cours est terminé pour une autre raison.
- **Tests à ajouter** : test unitaire store/engine avec 2 jobs PLAYING artificiels (started_at distincts) → `current_job` = plus récent, les autres passent DONE.
- **Effort** : XS.

### AUD-A1-011 — Le message S→C `error` n'est jamais émis ; `send_server_error` est du code mort

- **Sévérité** : Mineur — **Confiance** : haute (statique + `#[allow(dead_code)]`).
- **Composant** : `apps/lc-server/src/ws/session.rs:811-841` (`send_server_error`, `#[allow(dead_code)]`), chemins d'échec `meme.trigger`/`stop.request` (`session.rs:584-623` pour `stop.request`, `session.rs:631-757` pour `meme.trigger`) et dispatch moteur.
- **Preuve statique** : la fonction est marquée `#[allow(dead_code)]` et aucun appelant n'existe (`grep send_server_error` → seulement la définition) ; les échecs d'`engine.enqueue`/`engine.stop` sont seulement `warn!` loggés alors que l'ack a déjà été émis `ok:true` (l.611-627, l.680+). La spec §6.7 l.522-534 définit `error` (`dispatch_failed`, `media_unavailable`, `internal_error`) comme non fatal et optionnel, mais son absence totale empêche tout diagnostic client.
- **Impact** : en cas d'échec interne après un ack `ok:true` (DB en erreur, job rejeté), le client croit l'action acceptée et n'affiche rien ; l'overlay n'a aucun signal, le watchdog attend la fin du job fantôme.
- **Correctif** : émettre `ServerErrorPayload { code: "internal_error", message, job_id }` sur échec d'enqueue/stop (et/ou retourner un ack `ok:false` avant l'exécution) ; ajouter un test.
- **Tests à ajouter** : test `ws` avec DB forcée en erreur après connexion → ack `ok:false` **ou** `error` S→C reçu, socket maintenu.
- **Effort** : S.

### AUD-A1-012 — Champs inconnus des payloads `play`/`tweetCard` non conservés dans `extra` (doc gelée vs code)

- **Sévérité** : Mineur — **Confiance** : haute (statique).
- **Composant** : `crates/lc-protocol/src/lib.rs:729-776` (`TweetCardPayload`, `PlayPayload` — aucune carte `extra`, aucun `#[serde(flatten)] HashMap`), `crates/lc-protocol/tests/fixtures_roundtrip.rs:156-202` (round-trip exact uniquement sur les clés connues de la fixture `v2_play_tweet_card_rich.json`).
- **Preuve statique** : `grep flatten/HashMap/BTreeMap` dans `lc-protocol/src/lib.rs` → un seul `flatten` (`CurrentJobPayload.play`) et **aucune carte de champs inconnus**. La doc §2 l.189-191 et §6.2.1 l.454 affirment : « `lc-protocol` conserve ces champs dans `extra` pour les structures `play`/`tweetCard` (round-trip exact) ». Le test golden passe parce que la fixture `rich` n'utilise que les clés typées ajoutées (`currentStatusId`, `videoUrl`, `videos`…).
- **Impact** : perte silencieuse des futures clés riches (champs `tweetCard` ajoutés côté scraper v1) → round-trip non exact, pas de compatibilité ascendante réelle ; un overlay v3.1 ne recevrait pas les nouveaux champs attendus.
- **Correctif** : ajouter `#[serde(flatten)] pub extra: BTreeMap<String, Value>` sur `PlayPayload` et `TweetCardPayload` (serialize en dernier, skip si vide) ; conserver la sérialisation déterministe et compléter la fixture golden. **Décision doc** : si `extra` n'est pas voulu, amender la spec (§2/§6.2.1) — mais le contrat est gelé, donc préférer le code.
- **Tests à ajouter** : fixture golden avec clé inconnue (`"x-future":42`) dans `play` et `tweetCard` → parse puis re-sérialisation **identique**.
- **Effort** : S.

### AUD-A1-013 (Nit) — `hello_required` défini mais jamais émis

- **Sévérité** : Nit — **Confiance** : haute.
- **Preuve** : `crates/lc-protocol/src/lib.rs` (`close_reason::HELLO_REQUIRED`) ; `grep HELLO_REQUIRED` dans `apps/lc-server` → aucun usage ; le flux avant welcome utilise `1002 protocol_error` (conforme §1.2 l.96). Code mort/documentaire uniquement.
- **Impact** : aucun comportement fautif ; risque de divergence si un client/tool attend cette raison.
- **Correctif** : soit supprimer la constante, soit l'utiliser pour le cas « trame avant hello » si la spec le décide (la spec gelée impose 1002 → ne pas changer sans révision).
- **Tests** : néant.
- **Effort** : XS.

### AUD-A1-014 (Nit) — `ack {ok:true}` est émis avant l'exécution effective (stop.request, meme.trigger)

- **Sévérité** : Nit — **Confiance** : haute.
- **Preuve** : `session.rs:623` ack `ok:true` puis `engine.stop` ; `session.rs:757` ack `ok:true` puis `engine.enqueue` ; les erreurs sont seulement loggées (cf. AUD-A1-011). La spec §5.6/§5.7 décrit l'ordre « ack puis stop/play » sans définir le cas d'échec interne.
- **Impact** : en cas d'échec interne, le client a une confirmation mensongère (voir 011 pour le correctif global).
- **Correctif** : exécuter puis ack (au prix d'une latence DB) ou ack `ok:false` + `error` en cas d'échec ; s'aligner avec AUD-A1-011.
- **Tests** : couverts par le test de 011.
- **Effort** : XS (avec 011).

## 2. Couverture de la spécification (message par message)

Légende : ✅ conforme (preuve), ⚠️ écart (finding), ❔ non couvert (revue statique seulement ou test absent).

### Transport / handshake / liveness

| Élément | Statut | Preuve |
|---|---|---|
| `GET /ws` sans `Upgrade` → 426 `websocket_required` | ✅ | probe `ws-without-upgrade-426` |
| `/socket.io/` → 426 `overlay_outdated` + `protocolVersion:2` + `minOverlayVersion` | ✅ | probes `socketio-426` (2 runs) |
| Aucun token dans l'URL/header (auth uniquement dans `hello`) | ✅ | revue `session.rs:91-200` ; aucun extracteur de token ni query param côté handler |
| Hello sous 5 s sinon 4401 `auth_timeout` | ✅ | probe `hello-timeout-4401` : close 4401 après 5 027 ms |
| Token vide → 4401 `missing_token` | ✅ | probe `empty-token-4401` |
| Token inconnu → 4401 `invalid_token` | ✅ | probe `unknown-token-4401` |
| Token révoqué avant connexion → 4402 `revoked` | ✅ | test `revoked_client_closes_with_4402` |
| `clientId` fourni incohérent → 4401 | ✅ statique | `session.rs:144-151` (aucune probe dédiée) |
| `appVersion` obsolète → 4403 reason `{"minVersion":"3.0.0"}` | ✅ | probe `old-app-version-4403` + test |
| Trame avant `hello` autre que hello → 1002 | ✅ | probes `first-frame-*` (invalid JSON, non-hello) |
| Trame binaire avant/après welcome → 1003 | ✅ | probes `first-frame-binary-1003`, `after-welcome-binary-1003` |
| Second `hello` après welcome → 1002 | ✅ statique | `session.rs:776-780` (`ClientMessage::Hello(_) => Err(1002)`) ; pas de probe dédiée |
| `welcome` → `settings` → `peers` dans cet ordre | ✅ | probe `hello-welcome-settings-peers` (comparaison de `at`) |
| `welcome` : `protocolVersion`, `serverVersion`, `serverTime`, `config`, `sessionMode` | ✅ | probe `hello-welcome-settings-peers`, `welcome-serverVersion-v2` |
| `welcome.currentJob` absent sans job PLAYING | ✅ | probe `welcome-currentJob-absent-idle` (clé absente, pas `null`) |
| `welcome.currentJob` présent + URL signée par client en reconnexion | ✅ | probe `welcome-currentJob-replay-signed` (jobId, startedAt, remainingMs, sig vérifiée par client B) |
| Ping toutes les 15 s + close 1001 si pas de Pong/activité 45 s | ⚠️/❔ | code conforme sur l'émission (`session.rs:314-321`) mais sémantique Pong faussée (AUD-A1-008) ; non observable avec Node (auto-Pong) |
| Heartbeat applicatif accepté (présence) | ✅ | probe `heartbeat-and-error-keep-socket` + test `heartbeat_is_accepted_and_updates_last_seen` |
| `sessionMode` du heartbeat divergent = diagnostic log seulement | ✅ statique | `session.rs:519` (warn/debug), mode serveur autoritaire |
| `TRUST_PROXY_HEADERS`/rate-limit IP hors scope WS | ➖ | hors A1 (Phase 3 HTTP) |

### Enveloppe et limites

| Élément | Statut | Preuve |
|---|---|---|
| Enveloppe `{v,id,ts,type,payload}` aplatie | ✅ | tests fixtures round-trip (29) |
| `v ≠ 2` → 4403 | ✅ | probe `after-welcome-v3-4403` |
| JSON invalide → 1002 | ✅ | probes `first-frame-invalid-json-1002`, `after-welcome-invalid-json-1002` |
| Type inconnu → 1002 | ✅ | probe `after-welcome-unknown-type-1002` |
| Payload non-objet (null) → 1002 | ✅ | probe v2 (`payload:null` → 1002) |
| `id` non vide ≤ 64 → 1002 | ⚠️ | AUD-A1-007, probes id-65 / id-vide |
| Trame > 64 Kio → 4408 | ✅ ≤256 Kio | probe `frame-100kio-4408` ; au-delà de 256 Kio → 1006 (AUD-A1-006) |
| S→C garanti < 64 Kio | ⚠️ edge | dépend de AUD-A1-007 (ack ref) ; payloads normaux très en dessous |
| 20 msg/s fenêtre 1 s → 4409 | ✅ tests | test `flooding_after_welcome_closes_with_4409` (pas de probe) |
| `playback.state` ≤ 1/s, excédent ignoré | ✅ statique | `session.rs` throttle `PLAYBACK_STATE_MIN_INTERVAL_MS` ; test partiel |
| `peers.request` ≤ 1/2 s, excédent ignoré | ✅ | probe `peers-request-throttled-2s` |
| TTL média adaptatif image/audio/vidéo | ✅ | test `signed_play_uses_the_adaptive_media_ttl` + `media_url.rs` tests |
| `remainingMs` borné | ✅ statique | `session.rs` clamp MAX_PLAYBACK_REMAINING_MS |
| Texte mème 500 chars tronqué | ✅ statique | `session.rs:727` (`chars().take(MAX_MEME_TEXT_CHARS)`) |

### Messages C→S

| Message | Statut | Preuve |
|---|---|---|
| `hello` | ✅ | voir handshake |
| `heartbeat` | ✅ | probe + test |
| `playback.state` | ❔ | code revu (throttle, clamp, persistance par `engine.handle_playback_state`) ; test `playback_state_is_persisted_for_a_corresponding_job` ; **aucune probe dynamique** (état `paused` non exercé) |
| `playback.ended` | ❔ | code revu (idempotent, sans ack, met le job DONE via `engine.handle_playback_ended`) ; **aucun test** ni probe dans la suite `ws` |
| `error` | ✅ | probe `heartbeat-and-error-keep-socket` (socket maintenu) + revue log-only |
| `stop.request` | ✅ | probes `stop-request-throttle-1s`, `stop-request-manual-broadcast`, `invite-read-only-refused` |
| `meme.trigger` | ✅ | probes `meme-ack-then-play-unicast-both-sockets`, `invite-read-only-refused` ; cas d'erreur item/média seulement statiques |
| `peers.request` | ✅ | probes `peers-request-throttled-2s`, `peers-dedupe-same-clientId` |

### Messages S→C

| Message | Statut | Preuve |
|---|---|---|
| `welcome` | ✅ | probes ordre/champs/currentJob |
| `settings` | ✅ | probes reception + `settings-broadcast-after-guild-update` (PATCH admin → frame à 7 s) |
| `peers` | ⚠️ | probes dedupe/cap/isolation ; plafond 8 = AUD-A1-004 |
| `play` | ⚠️/✅ | probes unicast signé 2 sockets, média null, tweetCard null, guild isolation ; texte-seul jamais émis = AUD-A1-001 |
| `stop` | ⚠️ | `manual` ✅ (probe) ; `preempt` ❔ (pas de probe) ; `revoked` ⚠️ (002) ; `purge` ⚠️ (003) |
| `ack` | ✅ | probes stop/meme/peers ; codes `rate_limited`/`read_only` conformes |
| `error` | ⚠️ | jamais émis (AUD-A1-011) |

### Rooms, unicast/broadcast, modes, révocation, purge, moteur

| Élément | Statut | Preuve |
|---|---|---|
| Rooms par guilde (isolation inter-guilde) | ✅ | probe `guild-isolation` (`g2Play:0`, peers guilde 2 uniquement) |
| `play`/`currentJob` unicast avec URL signée par client | ✅ | probes sig vérifiée HMAC côté client ; deux sockets même clientId reçoivent la même URL (clientId identique, conforme) |
| `settings`/`stop`/`peers` broadcast room | ✅ | probes stop 2 sockets, settings 1 socket, peers room |
| `peers` dédupliqué par clientId | ✅ | probe `peers-dedupe-same-clientId` |
| `peers` self inclus dans la liste (v1 parity) | ✅ | probes peers (liste du socket contient son propre clientId) |
| `invite_read_only` : `meme.trigger` et `stop.request` refusés `read_only` | ✅ | probe `invite-read-only-refused` |
| `invite_read_only` : autres messages acceptés | ✅ statique | chemin `read_only` limité aux deux branches concernées |
| Révocation à chaud : `stop revoked` puis close 4402 | ⚠️ | AUD-A1-002 |
| Purge admin : `stop purge` + libération jobs + fermeture overlays | ⚠️ | AUD-A1-003 (`manual` observé) |
| Moteur lazy (`ensure_runtime`) + un seul PLAYING par guilde | ✅/⚠️ | probe+revue ; course d'initialisation = AUD-A1-009 ; anomalie multi-PLAYING = AUD-A1-010 |
| Supervis° moteur / mort silencieuse | ⚠️ | AUD-A1-005 (non reproduit) |
| `shutdown` gracieux WS (close 1001 `server_shutdown`) | ❔ | code revu (`session.rs:325`) mais **non testé dynamiquement** (Windows : pas de SIGTERM, le harnais `Stop-Process` ne déclenche pas l'arrêt gracieux) |
| Fuites de tâches / verrous (Mutex hub, verrou moteur) | ✅/⚠️ | revue : `std::sync::Mutex` hub jamais tenu à travers un `.await` dans les chemins chauds ; verrou `tokio::Mutex` moteur tenu pendant les awaits DB/sink (par conception, sérialise les guildes — coût de latence mais pas de deadlock observé) ; tâches spawnées : moteur + purge (durée de vie process), sessions (1 tâche/socket) ; subscription shutdown recréée à chaque itération de boucle (inefficacité mineure, pas une fuite) |

## 3. Points forts

- **Auth solide** : hachage SHA-256 hex minuscule conforme, lookup par `token_hash` + `revoked_at IS NULL`, distinguo revoked/inconnu, `clientId` cohérent vérifié, version minimale vérifiée après auth avec reason JSON conforme ; test vectoriel SHA-256 présent.
- **Codes et raisons de fermeture** : tous les cas testés dynamiquement renvoient le code **et la raison** exacts de la spec (4401 missing_token/invalid_token/auth_timeout, 4402 revoked, 4403 + JSON minVersion, 1002 protocol_error, 1003 unsupported_frame, 4408 frame_too_large) ; mapping centralisé testé (« close_reason_mapping_is_stable »).
- **Enveloppe** : sérialisation aplatie exacte (round-trip 29 fixtures), payloads stricts et tolérants aux champs inconnus (sauf conservation `extra`, cf. 012), validation de l'objet JSON.
- **Handshake** : séquence welcome → settings → peers conforme et ordonnée, `welcome.currentJob` absent/present correct, URLs signées par client en replay (HMAC vérifiée indépendamment par la probe).
- **Messages** : throttles exacts (stop.request 1 s → `rate_limited`, peers.request 2 s → silencieux), log-only de `error` C→S, read-only autoritaire appliqué uniquement aux deux messages concernés, présence mise à jour, unicast/broadcast conformes, isolation par guilde.
- **Qualité d'implémentation** : verrou hub tolérant au poisoning, `Weak<Hub>` dans le sink pour éviter les cycles, `HMAC` avec `subtle::ConstantTimeEq` côté vérification média, `OnceCell` pour initialisation lazy, deny lints workspace (pas d'`unwrap`/`expect`/`panic` dans les chemins WS), tests existants de bonne facture (23 tests ciblés, fixtures golden).
- **Infrastructure d'audit** : 37 probes reproductibles (`.phase0/audit/A1/`), serveur isolé, aucune dépendance à `.phase0/local-test`.

## 4. Zones non couvertes / limites de l'audit

- **Liveness 45 s / Ping** : non observable avec le client Node 24 (réponse Pong automatique au niveau protocole) ; findings 008 issus de la revue statique. À couvrir par un test `ws` en temps accéléré ou un client bas niveau qui ignore les Ping.
- **Arrêt gracieux WS** : pas de endpoint de shutdown ni de SIGTERM exploitable sous Windows ; la fermeture 1001 `server_shutdown` des sockets n'a pas été observée dynamiquement (chemin statique : `session.rs:325`).
- **`playback.state` / `playback.ended`** : pas de probe dynamique (état `paused`, `remainingMs`, reprise busyUntil) ; un test existe pour `playback.state` persisté, rien pour `playback.ended` n: le comportement est revu mais non prouvé de bout en bout.
- **Endpoint média HTTP** (`/overlay/media/:assetId`) : signature vérifiée hors ligne (HMAC recalculée par notre client), mais les réponses 200/206/403/416 et `Range`/`HEAD` relèvent de l'audit Phase 3 (HTTP) et n'ont pas été exercées ici.
- **Charge / concurrence** : probes ≤ 10 sockets ; pas de test de charge à 100+ clients, ni de contention hub/moteur sous charge (le POC serveur a mesuré 100 clients mais sur un sous-ensemble de cette implémentation).
- **Discord / admin auth / panneau** : hors périmètre A1 (A2/A3) ; les handlers admin n'ont été lus que pour les chemins révocation/purge/settings.
- **Purge vs moteur concurrent** : la purge admin supprime les lignes pendant que le moteur peut ticker ; aucune attestation d'absence de course (pas de probe concurrente). À couvrir par un test d'intégration.
- **`tweetCard` réel** : le serveur courant ne peut jamais produire de carte riche (`payload.rs:build_play_payload` met toujours `tweet_card: None`) ; la conformité du pipeline carte riche n'est donc pas testable de bout en bout (dépend de la lane media/ingest).
- **Fenêtre d'initialisation** (finding 009) : non reproduite dynamiquement (timing très court).

## 5. Reproductibilité et artefacts

### Commandes exactes

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

# build ciblé (sortie .phase0/audit/A1/build.log)
cargo build -p lc-server --bin lc-server --locked

# suite WS (23 tests ; .phase0/audit/A1/ws-tests.log)
cargo test -p lc-server ws --locked

# probes dynamiques complètes (serveur isolé port 3499, DB isolée, seed, dump DB)
powershell -NoProfile -ExecutionPolicy Bypass -File .phase0\audit\A1\run-probes.ps1
```

### Artefacts produits (non commités, sous `.phase0/audit/A1/`)

| Fichier | Contenu |
|---|---|
| `probe-results.json` | 37 vérifications + `finalHealth` (dernier run, 5 FAIL) |
| `server.out.log` / `server.err.log` | logs serveur du run isolé (dont `WARN lc_playback::engine: aucun overlay connecté : job marqué FAILED …`) |
| `ws-tests.log` | sortie `cargo test -p lc-server ws --locked` (23 tests, 0 échec) |
| `build.log` | sortie `cargo build` (0 erreur) |
| `seed.py`, `insert_text_job.py`, `job_status.py`, `lockdb.py`, `dump.py` | jeux de données isolés (guildes 349900000000000001/2, 9+1 clients, média READY, mème, jobs texte/UUID, verrou SQLite) |
| `probe.mjs`, `run-probes.ps1` | client WS Node 24 et orchestrateur |
| `audit.db`, `db-after.json` | base SQLite isolée et état final |

### Environnement

- `rustc 1.99.1`, `cargo 1.99.1`, host `x86_64-pc-windows-msvc` ; Node `v24.12.0` (WebSocket global) ; Python `3.14`.
- Dépendances clés : axum 0.8.9, tokio 1.54.1, tokio-tungstenite 0.29.0, sqlx 0.8.6, lc-protocol 3.0.0, lc-playback 3.0.0.
- Binaire testé : `target/debug/lc-server.exe` construit au commit `e0a381f` (le répertoire `ws/` et les crates protocole/playback n'avaient aucune modification locale pendant l'audit).

### Extraits de preuve (dernier run)

```json
// AUD-A1-001
{"name":"text-only-job-dropped","ok":true,
 "evidence":{"playCount":0,"jobStatus":"FAILED","roomPeers":1}}
// AUD-A1-002
{"name":"revocation-stop-then-4402","ok":false,
 "evidence":{"close":{"code":4402,"reason":"revoked"},"stop":null}}
// AUD-A1-003
{"name":"purge-stop-reason","ok":false,
 "evidence":{"stop":{"jobId":"8225d611-6035-4afd-adf8-ba13cee28603","reason":"manual"}}}
// AUD-A1-004
{"name":"peers-cap-8-undocumented","ok":true,
 "evidence":{"lengths":[8,8,8,8,8,8,8,8,8],"sample":["author-01",…,"author-08"]}}
// AUD-A1-006
{"name":"frame-300kio-expected-4408","ok":false,"evidence":{"code":1006,"reason":""}}
// AUD-A1-007
{"name":"after-welcome-id-65-spec-1002","ok":false,
 "evidence":{"ack":{"ok":true,"ref":"iiii…"}}}
```

## 6. Recommandations pour la Phase 4 (WS v2)

1. Corriger **AUD-A1-001, 002, 003** avant le cutover (contrats directement observés par l'overlay v3) ; ajouter les tests de non-régression associés au crate `lc-protocol`/`lc-server`.
2. Introduire dans `AdminHub` une API `broadcast_stop_with_reason` (002/003) plutôt que le `Manual` codé en dur.
3. Décider du **plafond peers** (004) : retirer, relever, ou documenter dans `protocol-v2.md` (révision de spec) avec un test de conformité.
4. Ajouter un superviseur moteur + indicateur `/health` (005) et un test d'injection d'erreur DB ; ne pas dépendre d'un log unique pour une panne silencieuse.
5. Compléter les tests manquants identifiés en §2 (liveness Pong, playback.ended, second hello, clientId incohérent, shutdown 1001, révocation pendant lecture) pour verrouiller la spec gelée.
6. Étudier la contention du verrou `tokio::Mutex` du moteur tenu pendant les awaits DB/sink (`engine.rs:564-573`) : sous charge multi-guildes, un dispatch lent bloque toutes les guildes ; mesure Phase 4 à prévoir.
7. Étendre `ws/tests.rs` pour couvrir les trous listés en §2 (liveness Pong, second hello, clientId incohérent, shutdown 1001, révocation pendant lecture) et brancher les probes dynamiques de `.phase0/audit/A1/` dans la CI si l'orchestration le permet (node/python requis).

## 7. Journal des vérifications

- Revue statique complète : `ws/mod.rs` (76 l.), `ws/hub.rs` (795), `ws/session.rs` (871), `ws/tests.rs` (~860), `lc-protocol/src/lib.rs` (823), `lc-protocol/tests/fixtures_roundtrip.rs` (469), `app.rs` (563), `wiring.rs` (180), `lib.rs` (280), `shutdown.rs` (209), `lc-playback/{engine,store,payload}.rs` (866/257/143), `http/admin/{clients,guilds,hub}.rs`, `migrations/0001_init.sql`.
- Tests ciblés : `cargo build -p lc-server --bin lc-server --locked` = 0 ; `cargo test -p lc-server ws --locked` = 23/23 verte en 0,13 s.
- Probes : **3 campagnes** (v1 puis v2/v3 après correction du format des ids de domaine — **les ids lc-domain sont des UUID, `GuildId` est un snowflake**) ; dernier run = 37 vérifications, 5 FAIL couverts par findings.
- Aucun code modifié (`git status` : seul `docs/rust/audit/` non suivi) ; aucun commit ; serveur isolé arrêté (0 process `lc-server`), artifacts sous `.phase0/audit/A1/` uniquement.
