//! Commandes « message » : fidélité legacy (aucun déclencheur).
//!
//! Le legacy n'avait **aucun** handler `messageCreate` : `server/src/loaders/
//! DiscordLoader.ts` ne déclarait que l'intent `Guilds` (ni `GuildMessages` ni
//! `MessageContent`) et n'écoutait que `interactionCreate`. Les cinq commandes
//! appelées « message » dans la Phase 6a (`send`, `hidesend`, `talk`/`dire`,
//! `hidetalk`/`cdire`, `stop`) sont en réalité des **commandes slash**, comme
//! les neuf autres ; elles sont toutes dispatchées par `interaction_create`.
//!
//! Ce module existe pour :
//!
//! 1. documenter ce point (la tâche Phase 6b parle de « message commands ») ;
//! 2. rester le point d'accroche si une Phase ultérieure ajoute un vrai
//!    déclencheur par message (préfixe, mention) : la classification est isolée
//!    ici et renvoie [`None`] aujourd'hui, sans lire le contenu Discord (qui
//!    exigerait l'intent privilégié `MESSAGE_CONTENT`, absent du legacy).
//!
//! Aucune commande n'est donc exécutable par message : `/alive`, `!alive`, etc.
//! ne déclenchent rien, exactement comme le bot legacy.

/// Commande reconnue dans un message texte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageTrigger {
    /// `/stop` par message — jamais produit aujourd'hui.
    Stop,
}

/// Classe un message entrant en commande « message ».
///
/// Renvoie toujours [`None`] : fidélité legacy (aucun handler `messageCreate`).
/// La fonction existe comme point d'extension documenté et testé.
pub fn classify(_content: &str) -> Option<MessageTrigger> {
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{classify, MessageTrigger};

    #[test]
    fn aucun_preffixe_ni_mention_n_declenche_de_commande() {
        for content in [
            "/alive",
            "!alive",
            "/stop",
            "stop",
            "!stop",
            "<@123456789012345678> help",
            "bonjour",
            "",
        ] {
            assert_eq!(classify(content), None, "message : {content:?}");
        }
    }

    #[test]
    fn le_variant_stop_reste_reserve_a_une_phase_ulterieure() {
        let trigger = MessageTrigger::Stop;
        assert_eq!(trigger, MessageTrigger::Stop);
    }
}
