//! Bot Discord LiveChat : catalogue des commandes, handlers testables et glue
//! serenity optionnelle.
//!
//! ## Découpage
//!
//! - [`catalog`] : catalogue **de données pur** des 14 commandes slash legacy
//!   (9 commandes « générales » + 5 commandes d'envoi de médias), avec leurs
//!   noms/descriptions/options localisés fr/en et l'ordre d'enregistrement
//!   historique.
//! - [`i18n`] : textes fr/en identiques aux dictionnaires legacy
//!   (`server/src/services/i18n/{fr,en}.ts`), placeholders `{{…}}` compris.
//! - [`context`] : trait [`BotContext`] injecté dans les handlers (persistance
//!   `lc-db`, moteur `lc-playback`, média/TTS, horloge, aléa, configuration) et
//!   implémentation `DbBotContext` câblant `lc-db` + backends injectés.
//! - [`inputs`] : entrées typées par commande (ce qu'une interaction Discord
//!   fournit, déjà résolu par la couche d'adaptation).
//! - [`handlers`] : logique métier fidèle au legacy, async et **sans serenity**
//!   (donc testable hors réseau).
//! - [`reply`] : sortie typée (embed, éphémère, message différé) consommée par
//!   la glue serenity ou par un autre adaptateur.
//! - [`testing`] : [`testing::FakeBotContext`] pour les tests et les spike
//!   d'intégration.
//! - `serenity_glue` (feature `serenity`, désactivée par défaut) : scraper les
//!   commandes du catalogue, parser une `CommandInteraction`, envoyer les
//!   réponses.
//!
//! ## Hypothèses Phase 6a (voir `docs/rust/lc-discord.md`)
//!
//! Les crates `lc-media` et `lc-playback` sont encore des squelettes au moment
//! de l'écriture : les handlers ne dépendent donc que de traits
//! ([`context::MediaBackend`], [`context::PlaybackBackend`],
//! [`context::MemeBackend`]) dont les adaptateurs réels arrivent en Phase 6b.
//! Aucun réseau n'est fait dans ce crate (y compris dans les tests).

#![forbid(unsafe_code)]

pub mod catalog;
pub mod context;
pub mod error;
pub mod handlers;
pub mod i18n;
pub mod inputs;
pub mod reply;
pub mod rich_payload;
pub mod testing;
pub mod text;

#[cfg(feature = "serenity")]
pub mod serenity_glue;

pub use catalog::{CommandKey, CommandSpec, OptionKind, OptionSpec, COMMANDS};
pub use context::{Author, BotConfig, BotContext, DbBotContext, GuildSettingsPatch};
pub use error::{BotError, MediaError, MediaErrorCode, SpeechFailure};
pub use handlers::dispatch;
pub use i18n::{strings, Lang, Strings};
pub use inputs::Invocation;
pub use reply::{CommandOutcome, Deferral, Embed, Reply};
