//! Handler gateway serenity : `ready`, `guild_create`, `interaction_create`,
//! `message`.
//!
//! Comportement aligné sur le legacy `server/src/loaders/DiscordLoader.ts` :
//!
//! | événement | legacy | ici |
//! |---|---|---|
//! | `READY` | log du tag + enregistrement global des commandes | log + enregistrement **global** une seule fois |
//! | `GUILD_CREATE` | upsert implicite (Prisma) + embed `howToUse` dans le premier salon textuel | upsert `guilds` (réglages préservés) + embed `howToUse` si nouvelle guilde |
//! | `INTERACTION_CREATE` | dispatch par nom, erreurs → embed rouge éphémère | `parse_invocation` → `dispatch` → `respond` |
//! | `MESSAGE_CREATE` | **aucun handler** (intent `Guilds` seul) | no-op documenté ([`super::message_commands`]) |
//!
//! Le handler ne fait aucune logique métier lui-même : il traduit les objets
//! serenity en [`lc_discord::Invocation`] et délègue au crate `lc-discord`
//! (Phase 6a), ce qui garde la logique testable hors réseau.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serenity::all::{
    ChannelType, Colour, Context, CreateEmbed, CreateMessage, EventHandler, GatewayIntents, Guild,
    Interaction, Message, Permissions, Ready,
};

use lc_db::Db;
use lc_discord::serenity_glue::{parse_invocation, respond};
use lc_discord::{dispatch, BotConfig, DbBotContext, Lang};

use super::message_commands;
use super::registration;
use super::runtime::{DiscordStatus, SharedClock};
use super::DiscordError;

/// Couleur de l'embed d'aide envoyé à l'arrivée sur une guilde (legacy
/// `0x3498db`).
const HOW_TO_USE_COLOR: u32 = 0x34_98_db;

/// Textes `howToUseTitle` / `howToUseDescription` du legacy, copiés à
/// l'identique depuis `server/src/services/i18n/{fr,en}.ts`.
const HOW_TO_USE_FR: (&str, &str) = (
    "Comment m'utiliser ?",
    "Utilisez `/pair-code` pour générer un code d'appairage à usage unique pour \
     l'Overlay EXE ou la LiveChat Extension, puis saisissez-le dans \
     l'application/extension. Vous pouvez utiliser `mode:invite` pour un code \
     overlay en lecture seule. Ensuite utilisez `/msg`, `/cmsg`, `/dire`, \
     `/cdire` pour envoyer des médias.",
);
const HOW_TO_USE_EN: (&str, &str) = (
    "How to use it?",
    "Use `/pair-code` to generate a one-time pairing code for the desktop \
     Overlay EXE or LiveChat Extension, then enter it in the app/extension. You \
     can use `mode:invite` for a read-only overlay pairing code. After pairing, \
     use `/send`, `/hsend`, `/talk`, `/htalk` to queue media playback.",
);

/// Handler gateway du bot.
pub struct DiscordEventHandler {
    db: Db,
    context: Arc<DbBotContext<SharedClock>>,
    bot_config: BotConfig,
    status: Arc<DiscordStatus>,
    registered: AtomicBool,
}

impl DiscordEventHandler {
    /// Assemble le handler.
    pub fn new(
        db: Db,
        context: Arc<DbBotContext<SharedClock>>,
        bot_config: BotConfig,
        status: Arc<DiscordStatus>,
    ) -> Self {
        Self {
            db,
            context,
            bot_config,
            status,
            registered: AtomicBool::new(false),
        }
    }

    /// Enregistre les commandes globales une seule fois, comme le legacy.
    async fn register_commands_once(&self, ctx: &Context) {
        if self.registered.swap(true, Ordering::AcqRel) {
            return;
        }

        let payloads = registration::registration_payloads(
            self.bot_config.lang,
            self.bot_config.hide_commands_disabled,
        );
        match registration::register_global_commands(&ctx.http, &payloads).await {
            Ok(commands) => {
                self.status.mark_registered(commands.len());
                tracing::info!(
                    count = commands.len(),
                    names = ?commands
                        .iter()
                        .map(|command| command.name.clone())
                        .collect::<Vec<_>>(),
                    "commandes slash globales enregistrées"
                );
            }
            Err(DiscordError::Registration(error)) => {
                self.status.set_registration_error(error.to_string());
                tracing::error!(%error, "enregistrement des commandes slash échoué");
            }
            Err(error) => {
                self.status.set_registration_error(error.to_string());
                tracing::error!(%error, "enregistrement des commandes slash échoué");
            }
        }
    }

    /// Upsert de la guilde en conservant les réglages éventuels (défauts/max/
    /// affichage complet) : le legacy stockait ces valeurs sur la ligne `Guild`.
    async fn upsert_guild(&self, guild_id: &lc_domain::GuildId, name: &str) {
        match upsert_guild_row(&self.db, guild_id, name).await {
            Ok(()) => tracing::info!(guild = guild_id.as_str(), %name, "guilde synchronisée"),
            Err(error) => {
                tracing::warn!(guild = guild_id.as_str(), %error, "synchronisation de guilde échouée");
            }
        }
    }
}

/// Crée ou rafraîchit la ligne `guilds` sans écraser les réglages configurés par
/// `/config-defaut`, `/config-max` et `/config-displayfull`.
pub async fn upsert_guild_row(
    db: &Db,
    guild_id: &lc_domain::GuildId,
    name: &str,
) -> Result<(), lc_db::DbError> {
    let existing = db.guilds().get(guild_id).await?;
    let guild = lc_domain::Guild {
        id: guild_id.clone(),
        name: Some(name.to_owned()),
        busy_until: existing.as_ref().and_then(|guild| guild.busy_until),
        default_media_time: existing.as_ref().and_then(|guild| guild.default_media_time),
        max_media_time: existing.as_ref().and_then(|guild| guild.max_media_time),
        display_media_full: existing
            .as_ref()
            .is_some_and(|guild| guild.display_media_full),
    };
    db.guilds().upsert(&guild).await
}

/// Cible d'upsert d'un `guild_create`.
///
/// `None` quand le serveur Discord est lié à une guilde indépendante : sa ligne
/// native (`id` = snowflake) ne doit alors pas être créée, les commandes du bot
/// visant la guilde indépendante. `Some(id)` renvoie sinon l'identifiant natif à
/// synchroniser.
pub async fn native_upsert_target(
    db: &Db,
    discord_guild_id: &lc_domain::GuildId,
) -> Result<Option<lc_domain::GuildId>, lc_db::DbError> {
    let resolved = db.guilds().resolve_discord(discord_guild_id).await?;
    Ok((resolved == *discord_guild_id).then_some(resolved))
}

/// Envoie l'embed d'aide legacy dans le premier salon textuel où le bot peut
/// écrire.
async fn send_how_to_use(
    http: &serenity::all::Http,
    guild_id: serenity::all::GuildId,
    lang: Lang,
) -> Result<bool, Box<serenity::Error>> {
    let channels = http.get_channels(guild_id).await.map_err(Box::new)?;
    let target = channels.into_iter().find(|channel| {
        matches!(channel.kind, ChannelType::Text | ChannelType::News)
            && can_send(channel.permissions)
    });
    let Some(channel) = target else {
        return Ok(false);
    };

    let (title, description) = how_to_use_texts(lang);
    let embed = CreateEmbed::new()
        .title(title)
        .description(description)
        .color(Colour::from_rgb(
            ((HOW_TO_USE_COLOR >> 16) & 0xff) as u8,
            ((HOW_TO_USE_COLOR >> 8) & 0xff) as u8,
            (HOW_TO_USE_COLOR & 0xff) as u8,
        ));
    channel
        .id
        .send_message(http, CreateMessage::new().embed(embed))
        .await
        .map_err(Box::new)?;
    Ok(true)
}

fn can_send(permissions: Option<Permissions>) -> bool {
    permissions.is_none_or(|permissions| {
        permissions.contains(Permissions::SEND_MESSAGES)
            || permissions.contains(Permissions::ADMINISTRATOR)
    })
}

/// Textes de l'embed `howToUse` selon la langue configurée (`I18N`).
pub fn how_to_use_texts(lang: Lang) -> (&'static str, &'static str) {
    match lang {
        Lang::Fr => HOW_TO_USE_FR,
        Lang::En => HOW_TO_USE_EN,
    }
}

#[async_trait]
impl EventHandler for DiscordEventHandler {
    async fn ready(&self, ctx: Context, data_about_bot: Ready) {
        self.status.mark_ready();
        tracing::info!(
            user = %data_about_bot.user.name,
            guilds = data_about_bot.guilds.len(),
            intents = GatewayIntents::GUILDS.bits(),
            "Discord prêt"
        );
        self.register_commands_once(&ctx).await;
        tracing::info!(
            registered = self.status.registered_commands(),
            "runtime Discord opérationnel"
        );
    }

    async fn guild_create(&self, ctx: Context, guild: Guild, is_new: Option<bool>) {
        let Ok(guild_id) = lc_domain::GuildId::parse(&guild.id.get().to_string()) else {
            tracing::warn!(
                discord_id = guild.id.get(),
                "identifiant de guilde invalide"
            );
            return;
        };
        match native_upsert_target(&self.db, &guild_id).await {
            Ok(Some(_)) => self.upsert_guild(&guild_id, &guild.name).await,
            Ok(None) => {
                tracing::info!(
                    guild = guild_id.as_str(),
                    "serveur Discord lié à une guilde indépendante : ligne native non créée"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    guild = guild_id.as_str(),
                    %error,
                    "résolution de la guilde liée échouée"
                );
                return;
            }
        }

        // Le legacy n'envoyait cet embed qu'aux nouvelles guildes ; serenity
        // distingue ce cas via `is_new` (les guildes rechargées au démarrage ne
        // déclenchent donc pas de spam).
        if is_new == Some(true) {
            match send_how_to_use(&ctx.http, guild.id, self.bot_config.lang).await {
                Ok(true) => tracing::info!(guild = guild_id.as_str(), "embed d'aide envoyé"),
                Ok(false) => tracing::info!(
                    guild = guild_id.as_str(),
                    "aucun salon textuel accessible pour l'embed d'aide"
                ),
                Err(error) => {
                    tracing::warn!(guild = guild_id.as_str(), %error, "envoi de l'embed d'aide échoué")
                }
            }
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(command) = interaction else {
            return;
        };

        let Some(invocation) = parse_invocation(
            &command,
            self.bot_config.lang,
            self.bot_config.hide_commands_disabled,
        ) else {
            tracing::warn!(
                name = %command.data.name,
                "interaction ignorée : commande inconnue du catalogue"
            );
            return;
        };

        let outcome = dispatch(self.context.as_ref(), invocation).await;
        if let Err(error) = respond(&ctx.http, &command, &outcome).await {
            tracing::warn!(name = %command.data.name, %error, "réponse d'interaction échouée");
        }
    }

    async fn message(&self, _ctx: Context, new_message: Message) {
        // Fidélité legacy : aucun handler `messageCreate`, intent `Guilds` seul.
        // On jette un coup d'œil à la classification (toujours `None`) pour que
        // le point d'accroche reste branché si une phase ultérieure l'active.
        let trigger = message_commands::classify(&new_message.content);
        tracing::trace!(
            author = %new_message.author.name,
            content_len = new_message.content.len(),
            ?trigger,
            "message ignoré (aucune commande par message dans le legacy)"
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use lc_db::{Db, DbConfig};
    use lc_discord::inputs::{CommandInput, Invocation};
    use lc_discord::testing::{member_author, FakeBotContext};
    use lc_discord::{dispatch, Lang};
    use lc_domain::GuildId;

    use super::{
        how_to_use_texts, native_upsert_target, upsert_guild_row, HOW_TO_USE_EN, HOW_TO_USE_FR,
    };

    fn guild_id() -> GuildId {
        GuildId::parse("424242424242424242").unwrap()
    }

    #[tokio::test]
    async fn dispatch_sans_reseau_fonctionne_avec_fake_context() {
        // Vérifie que le point d'entrée utilisé par `interaction_create`
        // (dispatch + contexte injecté) ne dépend d'aucun accès réseau.
        let context = FakeBotContext::new().await.unwrap();
        let invocation = Invocation {
            guild_id: Some(guild_id()),
            lang: Lang::Fr,
            author: member_author(),
            input: CommandInput::Alive,
        };
        let outcome = dispatch(&context, invocation).await;
        assert!(!outcome.reply.ephemeral);
        assert_eq!(outcome.reply.embed.color, Some(0x002e_cc71));
    }

    #[tokio::test]
    async fn dispatch_stop_delegue_au_backend_playback() {
        let mut context = FakeBotContext::new().await.unwrap();
        let invocation = Invocation {
            guild_id: Some(guild_id()),
            lang: Lang::Fr,
            author: member_author(),
            input: CommandInput::Stop,
        };
        let outcome = dispatch(&context, invocation).await;
        assert_eq!(outcome.reply.embed.color, Some(0x002e_cc71));
        assert_eq!(context.playback_mut().stop_calls(), vec![guild_id()]);
    }

    #[tokio::test]
    async fn upsert_guild_preserve_les_reglages() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        upsert_guild_row(&db, &guild_id(), "Ancien nom")
            .await
            .unwrap();
        db.guilds()
            .update_settings(&guild_id(), Some(12), Some(90), true)
            .await
            .unwrap();

        upsert_guild_row(&db, &guild_id(), "Nouveau nom")
            .await
            .unwrap();
        let guild = db.guilds().get(&guild_id()).await.unwrap().unwrap();
        assert_eq!(guild.name.as_deref(), Some("Nouveau nom"));
        assert_eq!(guild.default_media_time, Some(12));
        assert_eq!(guild.max_media_time, Some(90));
        assert!(guild.display_media_full);
    }

    #[tokio::test]
    async fn upsert_guild_cree_une_ligne_neuve() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        upsert_guild_row(&db, &guild_id(), "Guilde").await.unwrap();
        let guild = db.guilds().get(&guild_id()).await.unwrap().unwrap();
        assert_eq!(guild.name.as_deref(), Some("Guilde"));
        assert_eq!(guild.default_media_time, None);
        assert_eq!(guild.max_media_time, None);
        assert!(!guild.display_media_full);
    }

    #[tokio::test]
    async fn serveur_lie_a_une_guilde_independante_sans_ligne_native() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let standalone = GuildId::generate();
        let now = lc_domain::ms_to_datetime(0).unwrap();
        db.guilds()
            .create_standalone(&standalone, "Les Sardines", now)
            .await
            .unwrap();

        // Serveur non lié : la ligne native doit être synchronisée.
        assert_eq!(
            native_upsert_target(&db, &guild_id()).await.unwrap(),
            Some(guild_id())
        );

        db.guilds()
            .link_discord(&standalone, Some(&guild_id()))
            .await
            .unwrap();
        assert_eq!(native_upsert_target(&db, &guild_id()).await.unwrap(), None);
        assert!(db.guilds().get(&guild_id()).await.unwrap().is_none());
    }

    #[test]
    fn how_to_use_textes_legacy_exacts() {
        assert_eq!(how_to_use_texts(Lang::Fr), HOW_TO_USE_FR);
        assert_eq!(how_to_use_texts(Lang::En), HOW_TO_USE_EN);
        assert!(HOW_TO_USE_FR.0.contains("Comment m'utiliser"));
        assert!(HOW_TO_USE_EN.0.contains("How to use it"));
    }
}
