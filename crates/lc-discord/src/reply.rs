//! Modèles de réponse Discord produits par les handlers.
//!
//! Les handlers ne dépendent pas de serenity : ils renvoient des [`Embed`] et
//! des [`Reply`] neutres, que la glue (feature `serenity`) convertit en
//! `serenity::CreateEmbed` / `CreateInteractionResponse`. Les couleurs
//! reprennent exactement celles du legacy.

use crate::catalog::CommandKey;

/// Vert legacy `0x2ecc71` (succès).
pub const COLOR_SUCCESS: u32 = 0x002e_cc71;
/// Rouge legacy `0xe74c3c` (erreur).
pub const COLOR_ERROR: u32 = 0x00e7_4c3c;
/// Bleu legacy `0x3498db` (code d'appairage, infos de démarrage).
pub const COLOR_INFO: u32 = 0x0034_98db;
/// Jaune legacy `0xf1c40f` (avertissement, liste vide).
pub const COLOR_WARNING: u32 = 0x00f1_c40f;

/// Embed Discord neutre.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Embed {
    /// Titre optionnel.
    pub title: Option<String>,
    /// Description optionnelle.
    pub description: Option<String>,
    /// Couleur RGB optionnelle (voir les constantes `COLOR_*`).
    pub color: Option<u32>,
}

impl Embed {
    /// Embed vide.
    pub fn new() -> Self {
        Self::default()
    }

    /// Définit le titre.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Définit la description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Définit la couleur.
    #[must_use]
    pub fn with_color(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }

    /// Titre sous forme de chaîne éventuelle.
    pub fn title_text(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Description sous forme de chaîne éventuelle.
    pub fn description_text(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

/// Réponse à une interaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// Contenu de la réponse.
    pub embed: Embed,
    /// `true` si la réponse est éphémère (visible uniquement de l'auteur).
    pub ephemeral: bool,
}

impl Reply {
    /// Réponse éphémère.
    pub fn ephemeral(embed: Embed) -> Self {
        Self {
            embed,
            ephemeral: true,
        }
    }

    /// Réponse publique.
    pub fn public(embed: Embed) -> Self {
        Self {
            embed,
            ephemeral: false,
        }
    }
}

/// Accusé de réception préalable (`deferReply`) à effectuer.
///
/// Le legacy appelait `deferReply()` pour les commandes longues (ingestion
/// média, synthèse vocale) afin de respecter la limite de 3 secondes de
/// Discord, puis répondait avec `editReply`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Deferral {
    /// Aucun accusé de réception.
    #[default]
    None,
    /// `deferReply()` public.
    Public,
    /// `deferReply({ ephemeral: true })`.
    Ephemeral,
}

/// Résultat complet d'une commande.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutcome {
    /// Commande reconnue.
    pub command: CommandKey,
    /// Accusé de réception à envoyer avant le traitement long.
    pub deferral: Deferral,
    /// Réponse finale.
    pub reply: Reply,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Deferral, Embed, Reply, COLOR_ERROR, COLOR_INFO, COLOR_SUCCESS, COLOR_WARNING};

    #[test]
    fn embeds_vides_par_defaut() {
        let embed = Embed::new();
        assert_eq!(embed, Embed::default());
        assert_eq!(embed.title_text(), None);
        assert_eq!(embed.description_text(), None);
        assert_eq!(embed.color, None);
    }

    #[test]
    fn constructeur_d_embed() {
        let embed = Embed::new()
            .with_title("Titre")
            .with_description("Description")
            .with_color(COLOR_SUCCESS);
        assert_eq!(embed.title_text(), Some("Titre"));
        assert_eq!(embed.description_text(), Some("Description"));
        assert_eq!(embed.color, Some(0x002e_cc71));
    }

    #[test]
    fn reponses_ephemeres_et_publiques() {
        let ephemeral = Reply::ephemeral(Embed::new());
        assert!(ephemeral.ephemeral);
        let public = Reply::public(Embed::new());
        assert!(!public.ephemeral);
    }

    #[test]
    fn couleurs_legacy_exactes() {
        assert_eq!(COLOR_SUCCESS, 3_066_993);
        assert_eq!(COLOR_ERROR, 15_158_332);
        assert_eq!(COLOR_INFO, 3_447_003);
        assert_eq!(COLOR_WARNING, 15_844_367);
    }

    #[test]
    fn deferral_par_defaut() {
        assert_eq!(Deferral::default(), Deferral::None);
    }
}
