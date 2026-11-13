//! Erreurs du runtime Discord.
//!
//! Deux familles :
//!
//! - [`DiscordError::Disabled`] : `DISCORD_TOKEN` absent ou vide. **Ce n'est pas
//!   une panne** : le serveur continue de fonctionner sans bot (comme le legacy,
//!   dont le `loadDiscord` était asynchrone et non bloquant) ;
//! - les autres variantes : échec effectif (base absente, client serenity, mise
//!   à jour des commandes, arrêt du shard, timeout d'arrêt).

use thiserror::Error;

/// Erreur de démarrage ou d'arrêt du runtime Discord.
#[derive(Debug, Error)]
pub enum DiscordError {
    /// `DISCORD_TOKEN` absent ou vide : Discord est désactivé.
    #[error("Discord désactivé : DISCORD_TOKEN est absent ou vide")]
    Disabled,

    /// Une base SQLite est indispensable pour un [`lc_discord::DbBotContext`].
    #[error("Discord impossible sans base SQLite (AppState.db = None)")]
    DatabaseUnavailable,

    /// Construction du client serenity (`Client::builder` / `event_handler`).
    #[error("client Discord impossible : {0}")]
    Client(#[source] Box<serenity::Error>),

    /// Mise à jour des commandes globales échouée au `ready`.
    #[error("enregistrement des commandes Discord échoué : {0}")]
    Registration(#[source] Box<serenity::Error>),

    /// Échec du gateway (login ou boucle `Client::start`).
    #[error("gateway Discord en erreur : {0}")]
    Gateway(#[source] Box<serenity::Error>),

    /// L'arrêt du shard n'a pas abouti dans le délai imparti.
    #[error("arrêt Discord non confirmé dans le délai imparti")]
    ShutdownTimeout,
}

impl DiscordError {
    /// `true` si le runtime n'a pas démarré parce que Discord est désactivé.
    ///
    /// Le câblage serveur doit logguer ce cas en `info`/`warn` et continuer le
    /// démarrage sans runtime, contrairement aux autres variantes qui méritent
    /// un log d'erreur.
    pub fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }
}

#[cfg(test)]
mod tests {
    use super::DiscordError;

    #[test]
    fn seul_disabled_est_un_etat_nominal() {
        assert!(DiscordError::Disabled.is_disabled());
        assert!(!DiscordError::DatabaseUnavailable.is_disabled());
        assert!(!DiscordError::ShutdownTimeout.is_disabled());
    }

    #[test]
    fn messages_francais_stables() {
        assert_eq!(
            DiscordError::Disabled.to_string(),
            "Discord désactivé : DISCORD_TOKEN est absent ou vide"
        );
        assert_eq!(
            DiscordError::DatabaseUnavailable.to_string(),
            "Discord impossible sans base SQLite (AppState.db = None)"
        );
        assert_eq!(
            DiscordError::ShutdownTimeout.to_string(),
            "arrêt Discord non confirmé dans le délai imparti"
        );
    }
}
