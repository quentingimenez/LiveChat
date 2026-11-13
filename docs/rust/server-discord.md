# lc-server — runtime Discord (Phase 6b)

Document de référence du module `apps/lc-server/src/discord/`, qui branche le
bot Discord du legacy (`server/src/loaders/DiscordLoader.ts`) sur le nouvel
arbre Rust : client serenity, enregistrement des commandes, adaptateurs
`BotContext` et événements gateway.

## Périmètre

Livré ici :

- démarrage/arrêt du client serenity (`DiscordRuntime`, `DiscordHandle`) ;
- enregistrement des commandes slash depuis le catalogue `lc-discord` ;
- adaptateurs `BotContext` au-dessus de `AppState` (`lc-db` pour les guilde,
  codes d'appairage, board média et overlays ; placeholders typés pour média et
  playback) ;
- événements `ready`, `guild_create`, `interaction_create` et `message` ;
- sonde de santé Discord (`DiscordStatusProbe`) ;
- tests unitaires sans réseau.

Hors périmètre (laissé aux lanes parallèles / Phase 9) :

- câblage dans `main`/`router` : fait par le Lead (voir § 4) ;
- `crates/lc-media` (ingestion) et `crates/lc-playback` (file de lecture) :
  adaptateurs réels à substituer aux placeholders ;
- hub WebSocket overlay (task-17) : `DbOverlayDirectory` lit la table
  `overlay_clients` en attendant ;
- édition de `app.rs`, `lib.rs`, `Cargo.toml` (périmètre d'écriture interdit).

## 1. Fichiers

| Fichier | Rôle |
| --- | --- |
| `mod.rs` | documentation, câblage, re-exports |
| `error.rs` | `DiscordError` (`Disabled`, `DatabaseUnavailable`, `Client`, `Registration`, `Gateway`, `ShutdownTimeout`) |
| `runtime.rs` | `DiscordRuntime::start`, `DiscordHandle`, `DiscordStatus`, `DiscordStatusProbe`, `SharedClock` |
| `event_handler.rs` | `DiscordEventHandler` : `ready`, `guild_create`, `interaction_create`, `message` ; embed « how to use » |
| `registration.rs` | construction des payloads JSON depuis `lc_discord::command_payloads` + `create_global_commands` |
| `adapters.rs` | `bot_config_from`, `DiscordAdapters`, `DbMediaBoardBackend`, `DbOverlayDirectory`, placeholders média/playback |
| `message_commands.rs` | classification des messages (fidélité : toujours `None`) |

## 2. Démarrage

`DiscordRuntime::start(config: Arc<Config>, state: AppState) -> Result<DiscordHandle, DiscordError>`

Séquence :

1. `discord_token` absent ou vide ⇒ `Err(DiscordError::Disabled)`. Le boot du
   serveur ne doit pas échouer : le Lead logge et continue sans bot.
2. `AppState.db` absent ⇒ `Err(DiscordError::DatabaseUnavailable)` : les
   handlers ont besoin de `lc-db` (mode sans base non supporté pour Discord).
3. Construit `BotConfig` (`bot_config_from`) puis les adaptateurs
   (`DiscordAdapters::with_clock`) et `DbBotContext<SharedClock>`
   (`SharedClock` est un newtype car `lc-domain` n'implémente pas `Clock`
   pour `Arc<dyn Clock>`).
4. `Client::builder(token, GatewayIntents::GUILDS)` + `event_handler`.
   Les intents sont **exactement** ceux du legacy : `GUILDS` uniquement (le
   legacy ne demandait ni `GUILD_MESSAGES` ni `MESSAGE_CONTENT`).
5. `tokio::spawn` : `tokio::select!` entre `client.start()` (boucle gateway) et
   `ShutdownSignal::wait()` (arrêt du serveur). Sur signal : `shutdown_all()`
   des shards puis fin de tâche.

`DiscordHandle` expose :

- `status() -> Arc<DiscordStatus>` : état partagé (ready, erreurs gateway /
  enregistrement, nombre de commandes enregistrées) ;
- `probe() -> Arc<dyn HealthProbe>` : sonde pour `AppState::with_discord_probe`
  (`HealthState::Ok` si `ready`, sinon `Down`) ;
- `async shutdown(self) -> Result<(), DiscordError>` : déclenche l'arrêt,
  ferme les shards et attend la tâche avec `SHUTDOWN_TIMEOUT` (= `DRAIN_TIMEOUT`,
  10 s) ; timeout ⇒ `DiscordError::ShutdownTimeout`.

## 3. Événements

### `ready`

- marque le statut `ready` ;
- logge le tag du bot, le nombre de guildes et les intents ;
- enregistre les commandes **une seule fois** (voir § 4) puis logge les noms.

### `guild_create`

- `upsert_guild_row` : insère la guilde si absente, met à jour `name` sinon,
  **en préservant** `busy_until`, `default_media_time`, `max_media_time`,
  `display_media_full` (ces réglages sont posés par les commandes admin) ;
- **sauf** si `resolve_discord` renvoie une autre guilde : le serveur est lié à
  une guilde indépendante, sa ligne native n'est ni créée ni modifiée (log
  `info`, retour immédiat ; voir § 4 bis) ;
- si `is_new == Some(true)`, envoie l'embed « how to use » du legacy dans le
  premier salon textuel où le bot peut écrire (voir § 6) ;
- toute erreur (DB/reste) est loggée, jamais fatale.

### `interaction_create`

- ignore tout ce qui n'est pas `Interaction::Command` (pas de boutons/menus
  dans le legacy) ;
- `parse_invocation(&command, lang, hide_commands_disabled)` résout la
  commande par son nom localisé et les options par position ;
- pour toutes les commandes **sauf `/lier`**, le handler remplace le snowflake
  brut par `GuildsRepository::resolve_discord(snowflake)` avant de dispatcher :
  un serveur lié à une guilde indépendante agit donc sur cette guilde
  (réglages, file, board média, overlays) ; `/lier` garde le snowflake brut ;
- `dispatch(context, invocation)` exécute le handler `lc-discord`, puis
  `respond(http, command, outcome)` envoie le defer/réponse ;
- commande inconnue ou échec de réponse ⇒ `warn!`, pas de panic.

### `message`

Volontairement no-op : le legacy n'a **aucun** handler `messageCreate` (vérifié
dans `DiscordLoader.ts`), les cinq « commandes message » du plan
(`send`/`hidesend`/`hidetalk`/`talk`/`stop`) sont en réalité des slash
commands. `message_commands::classify` renvoie toujours `None` ; le point
d'accroche existe pour une phase ultérieure mais les intents ne permettent pas
de lire le contenu.

## 4. Enregistrement des commandes

Fidélité legacy : enregistrement **global** (`PUT /applications/{id}/commands`)
au premier `ready`, pas par guilde. Le libellé de la tâche mentionnait « par
guilde comme legacy discord.js » : c'est inexact, `DiscordLoader.ts` utilisait
`Routes.applicationCommands(clientId)` (global) ; le choix ici suit le code
réel. À documenter comme écart conscient avec le libellé.

- payloads : `lc_discord::command_payloads(lang, hide_commands_disabled)`
  réutilisé tel quel (JSON `{name, description, options}`) ; l'ordre legacy est
  conservé (`alive, msg, dire, pair-code, help, info, config-defaut,
  config-displayfull, config-max, overlays, stop, media-add`, puis `lier`,
  puis `cmsg`, `cdire` si activées) ;
- `Http::create_global_commands` exige une map `Serialize + Sized` : la tranche
  est matérialisée en `Vec<serde_json::Value>` dans `register_global_commands` ;
- un `AtomicBool` empêche un double enregistrement (plusieurs `ready` après
  reconnexions) ;
- erreurs stockées dans `DiscordStatus::registration_error` et loggées, jamais
  fatales.

## 4 bis. Commande `/lier` (`/link`)

Nouvelle commande (hors legacy), enregistrée comme les autres : nom `lier` en
français, `link` en anglais, une option `code` requise (texte). Elle lie le
serveur Discord courant à une guilde indépendante :

1. un administrateur d'overlay émet un code court (10 minutes, usage unique)
   avec `POST /overlay/admin/discord-link-code` ;
2. un administrateur Discord exécute `/lier code:<CODE>` (ou `/link`) ;
3. `DiscordLinkCodesRepository::redeem` valide le code, vérifie que la cible est
   indépendante et que le serveur n'est pas déjà lié ailleurs, puis pose
   `guilds.discord_guild_id` et marque le code utilisé ;
4. `DELETE /overlay/admin/discord-link` retire la liaison.

Réponses : toutes éphémères. Succès `Serveur lié` / `Server linked` en vert ;
code invalide, expiré ou déjà utilisé et serveur déjà lié ailleurs en rouge.
Seul un administrateur Discord (`author.is_admin`) peut l'exécuter ; hors
serveur Discord, réponse d'erreur générique. Une guilde cible déjà liée à un
autre serveur est re-liée au nouveau (remplacement), mais un serveur déjà lié à
une autre guilde n'est jamais repris automatiquement.

## 5. Adaptateurs `BotContext`

`bot_config_from(&Config)` mappe le legacy :

| `lc-server::Config` | `lc_discord::BotConfig` |
| --- | --- |
| `api_url` | `api_url` (`Url::to_string()`) |
| `i18n` | `Lang::parse` (`fr` par défaut) |
| `pairing_code_ttl_minutes` | `pairing_code_ttl_minutes` (`saturating_i64`) |
| `default_duration` | `default_duration_sec` |
| `hide_commands_disabled` | `hide_commands_disabled` |
| `media_max_size_mb`, `media_cache_max_total_mb`, `media_board_max_total_mb` | `media` (`MediaLimits`, `saturating_i64`) |

Adaptateurs livrés :

- `DbMediaBoardBackend` : `MediaBoardBackend` sur `lc-db` (`media_board`). `add_to_board`
  renvoie `false` si l'association existe déjà, `true` sinon ; une violation
  d'unicité concurrente est convertie en `false` (elle est mappée par `lc-db`
  en `DbError::Domain(DomainError::Conflict)`). TODO Phase 9 : la capacité
  (`MEDIA_BOARD_MAX_TOTAL_MB`) et le « pinning » de l'expiration sont dans le
  legacy mais `MediaAssetsRepository` n'expose pas les setters nécessaires.
- `DbOverlayDirectory` : `OverlayDirectory` sur `overlay_clients` (clients
  actifs seulement, plafond `MAX_OVERLAYS_LISTED = 25`). C'est une
  approximation du hub WebSocket (task-17) : il faudra le brancher sur le vrai
  hub pour retrouver l'ordre/tri exact des sessions connectées.
- `UnavailableMediaBackend` / `UnavailablePlaybackBackend` : erreurs typées
  explicites (`UnsupportedSource`, `BotError::Playback`) tant que
  `crates/lc-media` (task-16) et `crates/lc-playback` n'exposent pas d'API.
  L'API publique `DiscordAdapters::with_media` / `with_playback` permet au Lead
  d'injecter les implémentations réelles plus tard sans toucher au module.

## 6. Embed « how to use » (GuildCreate)

Reproduit le legacy : une fois par nouvelle guilde, embed couleur `#3498db`
avec les textes exacts fr/en recopiés de `server/src/services/i18n/{fr,en}.ts`
(constantes `HOW_TO_USE_FR` / `HOW_TO_USE_EN`). Recherche du salon :
`http.get_channels(guild_id)` → premier `ChannelType::Text` ou `News` où le bot
a `SEND_MESSAGES` (permissions inconnues ⇒ on tente). Envoi via
`ChannelId::send_message(&ctx.http, CreateMessage::new().embed(...))`.

## 7. Statut / santé

- `DiscordStatus` : `ready`, `gateway_error`, `registration_error`,
  `registered_commands` ; le handler est seul à écrire, l'extérieur lit.
- `DiscordStatusProbe` implémente `HealthProbe` : `Ok` si ready, sinon `Down`.
- Si Discord est désactivé (pas de token), le Lead n'installe pas de sonde :
  `DiscordProbe::from_config` du module `http::health` renvoie `Disabled`.

## 8. Câblage attendu côté Lead

`apps/lc-server/src/lib.rs` contient déjà `pub mod discord;` (ligne 34, ajouté
pendant la tâche). Câblage dans `run`/`main` (à faire par le Lead) :

```rust
let mut state = AppState::new(config.clone(), db, shutdown.clone());
let mut discord = None;

match crate::discord::DiscordRuntime::start(Arc::clone(&config), state.clone()).await {
    Ok(handle) => {
        state = state.with_discord_probe(handle.probe());
        discord = Some(handle);
    }
    Err(error) if error.is_disabled() => {
        tracing::info!(%error, "Discord désactivé");
    }
    Err(error) => {
        tracing::error!(%error, "démarrage Discord échoué");
    }
}

// ... serve() ...

if let Some(handle) = discord {
    if let Err(error) = handle.shutdown().await {
        tracing::warn!(%error, "arrêt Discord incomplet");
    }
}
```

La sonde doit être installée **avant** `app::router` si le routeur est
construit après le démarrage du bot ; sinon `/health` gardera `Down`/`Disabled`.

## 9. Tests

Tous sans réseau (aucun jeton requis) :

- `error.rs` : 2 tests (messages, `is_disabled`) ;
- `adapters.rs` : 6 tests (mapping config, défauts legacy, placeholders typés,
  board média create/doublon, overlays actifs/révoqués, contexte complet) ;
- `registration.rs` : 3 tests (ordre et noms fr, filtrage des commandes
  cachées, options de `media-add`) ;
- `message_commands.rs` : 2 tests (toujours `None`) ;
- `runtime.rs` : 4 tests (sans token ⇒ `Disabled` sans réseau, token sans base
  ⇒ `DatabaseUnavailable`, `SharedClock` délègue, gateway réel `#[ignore]`
  conditionné par `DISCORD_TOKEN`) ;
- `event_handler.rs` : 5 tests (dispatch `alive` et `stop` sans réseau via
  `lc_discord::testing::FakeBotContext`, upsert conserve les réglages, upsert
  crée la ligne, textes « how to use » exacts).

Les tests gateway réels nécessiteraient un `DISCORD_TOKEN` et un réseau : un
test `#[ignore]` documenté les conditionne à la variable d'environnement ; les
briques réseau sont limitées au `Client` serenity et n'ont pas de logique
propre.

Vérification finale le 2026-11-10 : `cargo fmt -p lc-server -- --check` = 0,
`cargo clippy -p lc-server --all-targets -- -D warnings` = 0,
`cargo test -p lc-server` = 227 tests lib + 1 doctest verts, 1 test gateway
ignoré (soit 22 tests pour le module `discord`, dont 1 ignoré).

## 10. Écarts et points Phase 9

1. **Enregistrement global** au lieu de « par guilde » (fidélité au code
   legacy, écart avec le libellé de la tâche).
2. **Média / playback** : placeholders ; brancher `lc-media` (task-16) et
   `lc-playback` via `DiscordAdapters::with_media` / `with_playback`.
3. **Overlays** : lecture `lc-db` ; brancher le hub WebSocket (task-17) pour
   l'ordre réel des sessions connectées.
4. **Board média** : capacité + pinning d'expiration non implémentés (lc-db
   sans setters) ; à traiter côté `lc-media`/`lc-db` (Phase 9).
5. **Présence/statut Discord** : le legacy n'en posait pas (aucun
   `set_activity`) ⇒ rien à porter.
6. **Année `/info`** : `lc-discord` utilise l'horloge injectée (UTC) alors que
   le legacy utilisait `new Date().getFullYear()` (TZ locale). Écart d'un jour
   possible au réveillon du Nouvel An uniquement.
7. **`startCase`** des noms d'auteur : approximation Rust (voir doc
   `lc-discord.md`).
8. **Commande inconnue** : loggée et ignorée (legacy répondait une erreur
   générique) — sans impact utilisateur réel car l'ensemble des commandes
   enregistrées est connu.

## 11. Risques

- serenity 0.12.5 épinglée : `Cargo.lock` modifié par cette lane (ajout
  serenity + arbre de deps) ; toute montée de version exige de revalider la
  glue (`serenity_glue` de `lc-discord` et ce module).
- API `lc-media`/`lc-playback` non figées : les placeholders échouent
  explicitement plutôt que silencieusement, mais le bot répondra des erreurs
  média tant qu'ils ne sont pas branchés.
- Gateway non testé en conditions réelles (pas de token dans l'environnement
  de build) : reconnexions, `is_new`, rate limits sont délégués à serenity.
- Événements non portés du legacy : rien d'autre que `ready`, `guildCreate`,
  `interactionCreate` (et `error` implicite de discord.js) ; `ClientReady` ne
  faisait que loguer, `GuildCreate` est couvert.

## Références

- legacy : `server/src/loaders/DiscordLoader.ts`, `server/src/services/i18n/{fr,en}.ts` ;
- `docs/rust/lc-discord.md` (catalogue, handlers, glue serenity) ;
- `docs/rust/protocol-v2.md`, `docs/rust/lc-db.md`, `docs/rust/server-ws.md` (à venir).
