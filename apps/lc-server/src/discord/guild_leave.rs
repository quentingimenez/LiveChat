//! Départ du bot d'un serveur Discord, déclenché par le panneau admin.
//!
//! La purge d'une guilde et le détachement d'une guilde indépendante peuvent
//! demander au bot de quitter le serveur Discord associé. Ce module isole cet
//! effet de bord derrière [`DiscordGuildLeaver`] afin que les handlers admin
//! restent testables sans réseau :
//!
//! - [`DisabledLeaver`] est l'implémentation par défaut (aucun token Discord) ;
//! - [`SerenityLeaver`] appelle `DELETE /users/@me/guilds/{guild_id}`.

#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use serenity::http::Http;
use serenity::model::id::GuildId;

/// Résultat d'une demande de départ d'un serveur Discord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveOutcome {
    /// Le bot a quitté le serveur.
    Left,
    /// Le bot n'était pas membre du serveur (déjà absent).
    NotMember,
}

/// Erreur d'une demande de départ.
#[derive(Debug, thiserror::Error)]
pub enum LeaveError {
    /// Aucun token Discord n'est configuré.
    #[error("Discord n'est pas configuré (DISCORD_TOKEN absent)")]
    Disabled,
    /// L'identifiant du serveur Discord n'est pas un snowflake numérique.
    #[error("identifiant de serveur Discord invalide")]
    InvalidId,
    /// Échec de l'appel à l'API Discord.
    #[error("Discord : {0}")]
    Api(String),
}

/// Fait quitter le bot d'un serveur Discord.
#[async_trait]
pub trait DiscordGuildLeaver: Send + Sync + std::fmt::Debug {
    /// Makes the bot leave the Discord server `discord_guild_id`.
    async fn leave(&self, discord_guild_id: &str) -> Result<LeaveOutcome, LeaveError>;
}

/// Implémentation sans Discord configuré : refuse toujours la demande.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisabledLeaver;

#[async_trait]
impl DiscordGuildLeaver for DisabledLeaver {
    async fn leave(&self, _discord_guild_id: &str) -> Result<LeaveOutcome, LeaveError> {
        Err(LeaveError::Disabled)
    }
}

/// Implémentation réelle au-dessus du client HTTP serenity.
#[derive(Debug, Clone)]
pub struct SerenityLeaver {
    http: Arc<Http>,
}

impl SerenityLeaver {
    /// Construit un client HTTP à partir du token du bot.
    #[must_use]
    pub fn new(token: &str) -> Self {
        Self {
            http: Arc::new(Http::new(token)),
        }
    }
}

#[async_trait]
impl DiscordGuildLeaver for SerenityLeaver {
    async fn leave(&self, discord_guild_id: &str) -> Result<LeaveOutcome, LeaveError> {
        let id = discord_guild_id
            .parse::<u64>()
            .map_err(|_| LeaveError::InvalidId)?;

        match self.http.leave_guild(GuildId::new(id)).await {
            Ok(()) => Ok(LeaveOutcome::Left),
            Err(error) if is_unknown_guild(&error) => Ok(LeaveOutcome::NotMember),
            Err(error) => Err(LeaveError::Api(error.to_string())),
        }
    }
}

/// `true` si Discord signale que le bot n'est pas membre du serveur.
///
/// L'API répond `10004 Unknown Guild` (souvent avec un statut 404) quand le bot
/// a déjà quitté le serveur ou n'y a jamais été invité.
fn is_unknown_guild(error: &serenity::Error) -> bool {
    match error {
        serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(response)) => {
            response.status_code.as_u16() == 404 || response.error.code == 10004
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[tokio::test]
    async fn disabled_leaver_returns_disabled() {
        let result = DisabledLeaver.leave("123456789012345678").await;

        assert!(matches!(result, Err(LeaveError::Disabled)));
    }

    #[tokio::test]
    async fn serenity_leaver_rejects_an_invalid_id_without_network() {
        let leaver = SerenityLeaver::new("test-token");

        let result = leaver.leave("abc").await;

        assert!(matches!(result, Err(LeaveError::InvalidId)));
    }
}
