# lc-discord — Phase 6a : catalogue et handlers des commandes Discord

- **Crate** : `crates/lc-discord`
- **Statut** : livrée (catalogue + handlers + logique métier + glue serenity optionnelle + tests + cette doc)
- **Portée** : Phase 6a uniquement. La boucle gateway Discord réelle, les
  adaptateurs `lc-media`/`lc-playback` et les workers de purge restent en Phase 6b.
- **Source de vérité legacy** : `server/src/components/**/commands/*Command.ts`,
  `server/src/loaders/DiscordLoader.ts`,
  `server/src/services/i18n/{fr,en}.ts` et les services utilisés par les
  commandes (`pairingCodes`, `memeBoard`, `playbackJobs`, `manualStop`, `gtts`,
  `mediaErrors`, `richOverlayPayload`, `mediaSourceResolver`).

## 1. Périmètre et non-objectifs

Le crate contient deux couches volontairement séparées :

1. **Le domaine de commandes** (`catalog`, `i18n`, `inputs`, `reply`, `handlers`,
   `context`, `error`, `text`, `rich_payload`) : données pures et logique async,
   **sans aucune dépendance à serenity**. Tout est testable hors réseau via un
   `BotContext` injecté.
2. **La glue serenity** (`serenity_glue`, feature `serenity` **désactivée par
   défaut**) : construction des payloads d'enregistrement, parsing d'une
   `CommandInteraction` vers `Invocation`, et envoi des réponses.

Non-objectifs assumés de cette phase :

- aucune connexion au gateway Discord, aucun appel réseau (y compris dans les
  tests) ;
- aucune implémentation réelle de `lc-media`/`lc-playback` (squelettes au moment
  de l'écriture) : les handlers dépendent de traits ;
- pas de portage des branches « tweets » de `/send` et `/hidesend` (elles
  exigent oEmbed / vxtwitter, donc du réseau) — voir §8 ;
- pas de worker de purge (codes d'appairage, médias, jobs) : il appartient au
  serveur (`lc-server`), pas à la couche commandes.

## 2. Fichiers livrés

| Fichier | Rôle |
|---|---|
| `src/catalog.rs` | Catalogue de données pur des 14 commandes, ordre legacy, filtrage `HIDE_COMMANDS_DISABLED`. |
| `src/i18n.rs` | Textes fr/en copiés des dictionnaires legacy, placeholders `{{…}}` et interpolation. |
| `src/inputs.rs` | Entrées typées par commande (`CommandInput`, `Invocation`, `MediaSubmission`, etc.). |
| `src/reply.rs` | Sortie neutre (`Embed`, `Reply`, `Deferral`, `CommandOutcome`) et couleurs legacy. |
| `src/handlers.rs` | `dispatch` + 14 handlers fidèles au legacy + helpers de durée/erreurs. |
| `src/context.rs` | Trait `BotContext`, backends injectés (`MediaBackend`, `PlaybackBackend`, `MediaBoardBackend`, `OverlayDirectory`), `DbBotContext`. |
| `src/error.rs` | `BotError`, `MediaError`, `MediaErrorCode` (11 codes) et messages localisés. |
| `src/text.rs` | Équivalents legacy : `startCase`, offsets YouTube (`t`/`start`), troncature. |
| `src/rich_payload.rs` | Format `__LC_OVERLAY_RICH__::` + base64(JSON) du texte overlay. |
| `src/testing.rs` | `FakeBotContext` et faux backends pour les tests. |
| `src/serenity_glue.rs` | Glue serenity (feature `serenity`). |
| `tests/commands.rs` | 28 tests d'intégration commande par commande (FakeBotContext). |
| `tests/db_context.rs` | 4 tests du contexte réel (`lc-db` en mémoire + horloge figée). |

Dépendances ajoutées dans `crates/lc-discord/Cargo.toml` : `async-trait`,
`base64`, `rand`, plus `serenity 0.12` **optionnelle**. `chrono`, `serde`,
`serde_json`, `thiserror`, `tracing`, `lc-db`, `lc-domain` viennent du
workspace.

## 3. Catalogue

L'ordre ci-dessous est l'**ordre d'enregistrement legacy** (`DiscordLoader`),
qui est aussi l'ordre affiché par `/help` :

`alive, send, talk, pair-code, help, info, config-defaut, config-displayfull,
config-max, overlays, stop, media-add, hidesend, hidetalk`

| Commande | Nom fr | Nom en | Options | Réponse | Couleur |
|---|---|---|---|---|---|
| alive | `dispo` | `alive` | — | publique (pas de defer) | vert |
| send | `msg` | `send` | `url` (str), `media` (attachment), `text` (str), `refresh` (bool) | defer public | vert |
| talk | `dire` | `talk` | `voice` (str, requis), `text` (str) | defer public | vert |
| pair-code | `pair-code` | `pair-code` | `mode` (str, choix `normal`/`invite`) | éphémère | bleu |
| help | `help` | `help` | — | éphémère | — |
| info | `info` | `info` | — | éphémère | — |
| config-defaut | `config-defaut` | `config-defaulttime` | `nombre` (int, requis) | éphémère | vert |
| config-displayfull | `config-displayfull` | `config-displayfull` | `value` (bool, requis) | éphémère | vert |
| config-max | `config-max` | `config-maxtime` | `nombre` (int, requis) | éphémère | vert |
| overlays | `overlays` | `overlays` | — | éphémère | vert / jaune |
| stop | `stop` | `stop` | — | éphémère | vert |
| media-add | `media-add` | `media-add` | `url`, `media`, `title`, `refresh` | defer éphémère | vert |
| hidesend | `cmsg` | `hsend` | `url`, `media`, `text`, `refresh` | defer éphémère | vert |
| hidetalk | `cdire` | `htalk` | `text` (str), `voice` (str, requis) | defer éphémère | vert |

Points de fidélité :

- `/help` et `/info` gardent les noms/descriptions **codés en dur en anglais**
  du legacy (`help` / `List of Commands`, `info` / `Bot Informations`).
- `hidesend` et `hidetalk` ne sont enregistrées que si
  `HIDE_COMMANDS_DISABLED != 'true'` (même sémantique que le legacy, variable
  conservée : le nom est contre-intuitif mais historique).
- Les options `media` sont de type `Attachment` (`type: 11` côté Discord), les
  autres types (`3/4/5`) viennent de `OptionKind`.

L'API du catalogue est de la donnée pure : `COMMANDS`, `CommandKey`,
`CommandSpec`, `OptionSpec`, `OptionKind`, `registered_commands(hide)`,
`registered_command_names(lang, hide)`, `find_by_name(lang, hide, name)`,
`find_option`.

## 4. API publique et injection

```rust
pub async fn dispatch(ctx: &dyn BotContext, invocation: Invocation) -> CommandOutcome;
pub fn plan_deferral(invocation: &Invocation) -> Deferral;
```

- `Invocation { guild_id: Option<GuildId>, lang, author, input }` : tout ce
  qu'une interaction apporte, déjà résolu par l'adaptateur.
- `Author { id, username, name, image, is_admin }` : `is_admin` est calculé par
  la couche Discord (permission `Administrator`) ; les handlers le croient.
- `CommandInput` : une variante par commande ; les commandes média portent une
  `MediaSubmission` (`url`, `media`, `text`, `force_refresh`).
- `CommandOutcome { command, deferral, reply }` ; `Reply { embed, ephemeral }` ;
  `Embed { title, description, color }` ; couleurs legacy :
  vert `0x2ecc71`, rouge `0xe74c3c`, bleu `0x3498db`, jaune `0xf1c40f`.
- `BotContext` expose : `db() -> &lc_db::Db`, `clock() -> &dyn Clock`,
  `config() -> &BotConfig`, `media()`, `playback()`, `media_board()`,
  `list_connected_overlays(guild)`, `update_guild_settings(guild, patch)` et
  `generate_pairing_code()`.
- `DbBotContext` câble `lc-db` + `Clock` + les quatre backends injectés ;
  `DbBotContext::with_clock(...)` permet les tests d'intégration avec
  `FixedClock`.
- `BotConfig { api_url, lang, pairing_code_ttl_minutes, default_duration_sec,
  hide_commands_disabled, media: MediaLimits }` ; `MediaLimits {
  max_size_mb, max_cache_total_mb, max_board_total_mb }` (défauts legacy
  2048/5120/15360).
- Traits backends (à implémenter en Phase 6b) :
  - `MediaBackend::ingest_from_source(MediaRequest) -> Result<Option<MediaAsset>, MediaError>`
    (`None` = source non résolue, comme le legacy) et
    `ingest_speech(lang, text) -> Result<MediaAsset, SpeechFailure>` ;
  - `PlaybackBackend::enqueue(PlaybackJobRequest)` et
    `stop_guild(&GuildId) -> Result<u64, BotError>` (nombre de jobs libérés) ;
  - `MediaBoardBackend::add_to_board(MediaAddRequest) -> Result<bool, BotError>`
    (`true` = créé, `false` = déjà présent) ;
  - `OverlayDirectory::list_connected(&GuildId) -> Result<Vec<ConnectedOverlay>, BotError>`.

## 5. Comportement métier fidèle au legacy

### `/alive`
Réponse publique, sans titre, verte : `startCase(username Discord) + ", Je suis
en vie !"`. `startCase` reproduit lodash sur les cas courants (séparateurs,
camelCase, sigles, accents, apostrophes) ; voir limites §9.

### `/pair-code` (`client` en fr)
Guilde obligatoire (sinon erreur générique éphémère). Génère un code de 6
caractères dans `ABCDEFGHJKLMNPQRSTUVWXYZ23456789`, avec au plus 5 tentatives
d'unicité (`pairing_codes().get`). Mode `normal` : expiration
`now + max(1, PAIRING_CODE_TTL_MINUTES)` minutes ; mode `invite` : aucune
expiration, `PairingMode::InviteReadOnly` (usage unique, overlay EXE seulement).
La ligne `PairingCodeEntry` est créée dans `lc-db` avec l'auteur Discord
(id, nom, image). La description est exactement celle du legacy (`API URL`,
code, « Expire dans N minute(s). », N = TTL effectif, minimum 1 minute). Embed
bleu éphémère.

### `/help`
Titre littéral `Commands :`, description = liste des noms enregistrés dans
l'ordre du catalogue, séparés par `, ` ; `HIDE_COMMANDS_DISABLED` respecté ;
éphémère, sans couleur.

### `/info`
Titre `LiveChat Server - {année}` (année fournie par l'horloge injectée, donc
déterministe en test), description = lien GitHub historique, éphémère.

### `/media-add`
Guilde obligatoire, puis au moins une source (`url` ou pièce jointe, la pièce
jointe prime : `media || url`). Sans source : erreur éphémère
`media_add_missing_media`, sans defer (comme le legacy). Sinon defer
éphémère, ingestion par `MediaBackend` (le backend gère le cache et
`forceRefresh`), puis `MediaBoardBackend::add_to_board` avec `title` tronqué à
240 caractères et l'auteur résolu. Réponse verte `media_add_answer_created` ou
`...exists`. Les erreurs d'ingestion sont localisées via `MediaErrorCode`.

### `/overlays`
Guilde obligatoire. `OverlayDirectory` renvoie les clients connectés ; 0
client → embed jaune avec `overlaysAnswerEmpty` ; sinon embed vert éphémère,
titre `Overlays connectés (N)`, lignes ``• `label` (`clientId`)``, triées par
label insensible à la casse (équivalent `localeCompare` legacy, sans collation
ICU).

### `/config-defaut`, `/config-max`, `/config-displayfull`
Guilde obligatoire et auteur administrateur (sinon embed rouge `notAllowed`,
sans description). **Aucune borne** n'est appliquée dans le handler (fidélité :
le legacy stockait la valeur brute, y compris négative). Convention legacy
respectée : `0` → `None` pour les durées (remise au défaut global), booléen
stocké tel quel pour `displayFull`. La mise à jour passe par
`update_guild_settings`, qui préserve les autres champs. Réponse verte
éphémère.

### `/stop`
Guilde obligatoire. `PlaybackBackend::stop_guild` doit reproduire
`executeManualStopForGuild` : émission `overlay:stop` (`jobId: 'manual-stop'`),
remise à zéro de `busyUntil`, passage des jobs `PLAYING` en `DONE` avec
`finishedAt = now`, notification au scheduler, retour du nombre de jobs libérés.
Réponse verte éphémère `stopCommandAnswer`.

### `/send` et `/hidesend`
Au moins une source (`url`/`media`) ou un texte, sinon erreur éphémère
`sendCommandMissingContent` sans defer. Sinon defer public (`/send`) ou
éphémère (`/hidesend`). Si `url`/`media` : offset YouTube extrait de la source
(`t`/`start` en query ou fragment, formats `90`, `90s`, `1h2m3s`, `1:30`,
`1:02:03`), texte transformé via `build_media_text_payload` (texte brut si pas
d'offset, sinon payload riche), ingestion, puis création du job avec :
- `source = discord_send_command` / `discord_hidesend_command` ;
- auteur (`authorName = interaction.user.username`, `authorImage =
  avatarURL()`) **uniquement** pour `/send` ;
- `showText = text non vide` ;
- durée : durée explicite > durée du média (sauf image) > défaut guilde >
  `DEFAULT_DURATION`, puis clamp `maxMediaTime` de la guilde, minimum 1 s ;
- offset : `startOffsetSec` retiré de la durée restante et reporté dans
  `resumeOffsetSec`.

Les erreurs d'ingestion/lecture sont rendues comme messages média localisés
(rouge).

### `/dire` et `/cdire`
`voice` requis. Defer public (`/dire`) ou éphémère (`/cdire`). Synthèse via
`MediaBackend::ingest_speech(lang, voice)` : le backend doit reproduire le
legacy (`translate_tts`, `ie=UTF-8`, `client=tw-ob`, découpage ≤ 180
caractères, MP3 concaténé, asset `gtts:<hash>`) — ce crate ne fait aucun
réseau. Puis job média avec `text` brut, `showText = text non vide`, source
`discord_talk_command` / `discord_hidetalk_command`, et auteur uniquement pour
`/dire`. Échec TTS → message `talkCommandVoiceError` ; autre échec → message
média localisé avec repli `TRANSCODE_FAILED`.

### Erreurs localisées
11 codes : `UNSUPPORTED_SOURCE`, `PRIVATE_OR_AUTH_REQUIRED`, `DRM_PROTECTED`,
`MEDIA_NOT_FOUND`, `DOWNLOAD_TIMEOUT`, `FILE_TOO_LARGE`,
`CACHE_STORAGE_LIMIT_REACHED`, `BOARD_STORAGE_LIMIT_REACHED`, `INVALID_MEDIA`,
`TRANSCODE_FAILED`, `DOWNLOAD_FAILED`. Les messages interpolent les limites
configurées (`maxSizeMb`, `maxCacheTotalMb`, `maxBoardTotalMb`), avec repli
`DOWNLOAD_FAILED` pour un code inconnu, comme `getLocalizedMediaErrorMessage`.

## 6. Format de texte overlay riche

Les textes destinés à l'overlay restent compatibles avec le legacy :
`__LC_OVERLAY_RICH__::` + base64 standard (fallback sans padding en lecture) de
`{"type":"media","caption":<string|null>,"startOffsetSec":<entier>}`. Le
variant `tweet` n'est pas implémenté (voir §8). Tests : préfixe exact,
aller-retour, refus des payloads invalides.

## 7. Glue serenity (feature `serenity`)

Activée par `--features serenity` ; compile et passe clippy avec la feature.
Elle apporte :

- `command_payloads(lang, hide) -> Vec<serde_json::Value>` : corps exacts pour
  `PUT /applications/{id}/commands` (ou autre transport), avec types d'options
  Discord (`3/4/5/11`), `required` et `choices` ;
- `parse_invocation(&CommandInteraction, lang, hide) -> Option<Invocation>` :
  résolution de la commande par nom localisé, options positionnelles, pièces
  jointes via `data.resolved.attachments` (`url`), auteur (nick sinon
  `global_name` sinon username ; avatar de guilde ou global au format
  `...?size=256`), `is_admin = member.permissions.administrator()` ;
- `deferral_response` / `message_response` : corps JSON `type: 5` / `type: 4`
  (flags éphémère `64`), utiles pour les tests d'API et un autre transport ;
- `respond(&Http, &CommandInteraction, &CommandOutcome)` : envoie le defer puis
  le message via `http.create_interaction_response` (erreur boxée pour clippy).
  ⚠ `serenity::Error` est boxé dans la signature publique de cette fonction.

Câblage Phase 6b (non fait ici) : créer le `Client` (`Guilds`), enregistrer
`command_payloads`, router `interactionCreate` vers `parse_invocation` →
`dispatch` → `respond`, puis brancher `GuildCreate` (message d'accueil legacy,
hors commandes). La boucle gateway n'est volontairement pas dans ce crate.

## 8. Écarts, hypothèses et points Phase 6b

1. **Tweets non portés** : `/send` et `/hidesend` acceptant une URL tweet
   passaient par `resolveTweetVideoMediasFromUrl` / `resolveTweetCardFromUrl`
   (réseau, oEmbed/vxtwitter, payload riche `type: "tweet"`, durée
   `TWITTER_CARD_DURATION_SEC`). En Phase 6a l'URL est traitée comme une source
   média standard ; à porter dans un backend réseau (probablement `lc-media`)
   avec le variant `TweetCard` du protocole v2. C'est l'écart fonctionnel
   principal.
2. **Adaptateurs réels absents** : `lc-media` et `lc-playback` étaient des
   squelettes. `BotContext`/`MediaBackend`/`PlaybackBackend`/`MediaBoardBackend`/
   `OverlayDirectory` fixent le contrat ; les adaptateurs Phase 6b devront
   notamment implémenter TTS + cache/modèle média et le scheduler de lecture.
3. **Board média** : le pinning legacy (`expiresAt = now + 100 ans`), le contrôle
   de capacité (`MEDIA_BOARD_MAX_TOTAL_MB`) et la somme des tailles ne sont pas
   dans `lc-db` (pas de setter d'expiration ni de liste par guilde côté
   `MediaAssetsRepository`). Tout est délégué à `MediaBoardBackend::add_to_board` ;
   l'adaptateur devra soit étendre `lc-db`, soit gérer ces invariants.
4. **Workers de purge** (codes d'appairage toutes les 60 s, médias expirés,
   jobs, etc.) : restent côté serveur, hors commandes. Les handlers ne purgent
   jamais.
5. **Présence overlay** : le contrat `OverlayDirectory` renvoie
   `ConnectedOverlay { client_id, label }` ; le mapping legacy
   (`overlayAuthorName` → `overlayClientLabel` → `unknown-device`, et
   `overlayClientId` → `unknown-client`) est à faire dans l'adaptateur
   Socket.IO, pas dans le handler.
6. **Année de `/info`** : lue sur l'horloge injectée (heure locale serveur dans
   le legacy via `new Date().getFullYear()` ; ici `DateTime<Utc>` — écart d'un
   an possible au nouvel an si l'adaptateur ne fournit pas une horloge locale ;
   à trancher en 6b).
7. **Pas de message d'accueil `GuildCreate`** (embed `howToUseTitle/Description`,
   bleu, premier salon texte autorisé) : c'était hors commandes, non porté.
8. **`/help` dynamique** : l'ordre et le nom viennent du catalogue ; il n'y a
   pas de fallback « commandes chargées par le gateway » comme
   `global.commandsLoaded` (le catalogue est la source).
9. **`startCase`** est une approximation fidèle des cas usuels, pas un port de
   lodash : pas de normalisation Unicode complète (NFKD), de gestion des
   apostrophes typographiques autres, ni de segmentation des scripts non
   latins. Risque limité aux pseudos exotiques.
10. **Bilingue** : `BotConfig::lang` pilote noms/textes ; le legacy utilisait
    `env.I18N` (défaut `fr`). Le serveur devra passer la langue au contexte.

## 9. Tests et vérifications

Commandes exécutées (toutes vertes) :

```
cargo fmt -p lc-discord --check
cargo clippy -p lc-discord --all-targets -- -D warnings
cargo clippy -p lc-discord --all-targets --features serenity -- -D warnings
cargo check  -p lc-discord --features serenity
cargo test   -p lc-discord
```

Résultat `cargo test -p lc-discord` : **66 tests, 0 échec**.

- 34 tests unitaires (catalogue, i18n, reply, text, rich_payload, context,
  inputs, error, handlers/deferral) ;
- 28 tests d'intégration `tests/commands.rs` : un chemin heureux par commande,
  plus les erreurs observables (guilde manquante, non-admin, source manquante,
  TTS en échec, enqueue en échec, annuaire overlay en échec, stop sans guilde,
  masquage `cmsg`/`cdire`, tri des overlays, persistance des réglages…) ;
- 4 tests `tests/db_context.rs` : `DbBotContext<FixedClock>` + `lc-db` SQLite en
  mémoire (persistance d'un code d'appairage et consommation atomique,
  persistance des réglages, refus non-admin, délégation du stop).

Les faux backends (`testing.rs`) enregistrent les requêtes reçues
(`requests()`, `speech_requests()`, `stop_calls()`, `queried_guilds()`) ; les
assertions portent sur les contenus exacts (titres, descriptions, couleurs,
flags éphémère, sources de jobs, durées, offsets, auteurs). Aucun réseau,
aucune variable d'environnement, aucune horloge réelle.

## 10. Risques et limites

- **Fidélité texte** : les dictionnaires fr/en ont été recopiés du legacy ; les
  tests épinglent les chaînes françaises principales, mais toute correction
  i18n devra être faite des deux côtés si le legacy évolue.
- **API `lc-playback` supposée** : `enqueue`/`stop_guild` sont des traits
  provisoires ; le contrat réel (DTO, gestion `resumeOffsetSec`, notification
  scheduler) peut imposer des ajustements d'adaptateur, pas de handler.
- **Glue non testée contre un vrai gateway** : compile et sa logique de mapping
  est revue, mais seul un test d'intégration Phase 6b (ou un test avec des
  payloads JSON d'interaction figés) validera le parsing.
- **`Cargo.lock`** a été mis à jour (ajout `rand`, `serenity` optionnelle) ;
  aucun autre crate/app n'a été modifié par cette tâche.
- `serenity` est épinglée par le workspace au 0.12.x ; toute montée de version
  devra être refaite dans `serenity_glue.rs` (API `create_interaction_response`,
  `ImageHash`, `CreateInteractionResponse` côté `builder`).

## 11. Références

- Legacy : `server/src/loaders/DiscordLoader.ts`,
  `server/src/components/**/commands/*Command.ts`,
  `server/src/services/i18n/{fr,en}.ts`,
  `server/src/services/{pairingCodes,memeBoard,playbackJobs,manualStop,gtts,mediaErrors,richOverlayPayload,mediaSourceResolver}.ts`,
  `server/src/loaders/commands/utils.ts`, `server/src/services/discord-utils.ts`.
- Plan : `docs/rust/migration-plan.md` (Annexe B, commandes Discord).
- Base : `docs/rust/lc-db.md` ; protocole overlay : `docs/rust/protocol-v2.md`,
  `docs/rust/overlay-bridge.md`.
