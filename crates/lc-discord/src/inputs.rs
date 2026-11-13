//! Entrées typées des handlers.
//!
//! La glue serenity convertirait `serenity::CommandInteraction` en
//! [`Invocation`] ; les handlers, eux, ne manipulent que ces structures
//! neutres (testables sans réseau ni Discord).

use lc_domain::{GuildId, PairingMode};

use crate::catalog::CommandKey;
use crate::context::Author;
use crate::i18n::Lang;

/// Options de `/send`, `/hidesend` et `/media-add` (partie média).
///
/// La pièce jointe est déjà résolue en URL (`attachment.url ||
/// attachment.proxyURL`) par la glue, comme dans le legacy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaSubmission {
    /// Option `url`.
    pub url: Option<String>,
    /// Pièce jointe résolue en URL.
    pub media: Option<String>,
    /// Texte overlay (`text` pour `/send`, `title` pour `/media-add`).
    pub text: Option<String>,
    /// Option `refresh`.
    pub force_refresh: bool,
}

impl MediaSubmission {
    /// `true` si une source (URL ou pièce jointe) est fournie.
    pub fn has_source(&self) -> bool {
        self.url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || self
                .media
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    }
}

/// Options de `/dire` et `/cdire`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TalkInput {
    /// Option `text` (optionnelle ; cache les contrôles si vide).
    pub text: Option<String>,
    /// Option `voice` (requise) : texte à synthétiser.
    pub voice: String,
}

/// Options de `/client` (`overlayCodeCommand`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairCodeInput {
    /// `normal` (expire) ou `invite` (usage unique, sans expiration).
    pub mode: PairingMode,
}

/// Options de `/config-defaut` et `/config-max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSettingInput {
    /// Durée en minutes telle que saisie (0 ⇒ valeur par défaut).
    pub seconds: i64,
}

/// Options de `/config-displayfull`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayFullInput {
    /// Afficher les médias en plein écran.
    pub value: bool,
}

/// Options de `/lier` (`/link`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkInput {
    /// Code de liaison affiché dans l'overlay de la guilde indépendante.
    pub code: String,
}

/// Options spécifiques à une commande.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandInput {
    /// `/alive`.
    Alive,
    /// `/send`.
    Send(MediaSubmission),
    /// `/hidesend`.
    HideSend(MediaSubmission),
    /// `/media-add`.
    MediaAdd(MediaSubmission),
    /// `/dire`.
    Talk(TalkInput),
    /// `/cdire`.
    HideTalk(TalkInput),
    /// `/lier`.
    Link(LinkInput),
    /// `/overlays`.
    Overlays,
    /// `/help`.
    Help,
    /// `/info`.
    Info,
    /// `/client`.
    PairCode(PairCodeInput),
    /// `/config-defaut`.
    SetDefaultTime(TimeSettingInput),
    /// `/config-max`.
    SetMaxTime(TimeSettingInput),
    /// `/config-displayfull`.
    SetDisplayFull(DisplayFullInput),
    /// `/stop`.
    Stop,
}

impl CommandInput {
    /// Commande associée à ces options.
    pub fn command(&self) -> CommandKey {
        match self {
            Self::Alive => CommandKey::Alive,
            Self::Send(_) => CommandKey::Send,
            Self::HideSend(_) => CommandKey::HideSend,
            Self::MediaAdd(_) => CommandKey::MediaAdd,
            Self::Talk(_) => CommandKey::Talk,
            Self::HideTalk(_) => CommandKey::HideTalk,
            Self::Link(_) => CommandKey::Link,
            Self::Overlays => CommandKey::Overlays,
            Self::Help => CommandKey::Help,
            Self::Info => CommandKey::Info,
            Self::PairCode(_) => CommandKey::PairCode,
            Self::SetDefaultTime(_) => CommandKey::SetDefaultTime,
            Self::SetMaxTime(_) => CommandKey::SetMaxTime,
            Self::SetDisplayFull(_) => CommandKey::SetDisplayFull,
            Self::Stop => CommandKey::Stop,
        }
    }
}

/// Invocation complète d'une commande.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    /// Guilde de l'interaction (`None` en message privé).
    pub guild_id: Option<GuildId>,
    /// Langue de réponse résolue par la glue (`env.I18N` du legacy).
    pub lang: Lang,
    /// Auteur de l'interaction.
    pub author: Author,
    /// Options typées.
    pub input: CommandInput,
}

impl Invocation {
    /// Construit une invocation.
    pub fn new(guild_id: Option<GuildId>, lang: Lang, author: Author, input: CommandInput) -> Self {
        Self {
            guild_id,
            lang,
            author,
            input,
        }
    }

    /// Commande reconnue.
    pub fn command(&self) -> CommandKey {
        self.input.command()
    }

    /// Guilde, si l'invocation a lieu dans un serveur.
    pub fn guild(&self) -> Option<&GuildId> {
        self.guild_id.as_ref()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use lc_domain::{GuildId, PairingMode};

    use super::{
        CommandInput, DisplayFullInput, Invocation, LinkInput, MediaSubmission, PairCodeInput,
        TalkInput,
    };
    use crate::catalog::CommandKey;
    use crate::context::Author;
    use crate::i18n::Lang;

    fn guild() -> GuildId {
        GuildId::parse("123456789012345678").unwrap()
    }

    #[test]
    fn commande_associee_aux_options() {
        assert_eq!(
            CommandInput::Send(MediaSubmission::default()).command(),
            CommandKey::Send
        );
        assert_eq!(
            CommandInput::PairCode(PairCodeInput {
                mode: PairingMode::InviteReadOnly
            })
            .command(),
            CommandKey::PairCode
        );
        assert_eq!(
            CommandInput::SetDisplayFull(DisplayFullInput { value: true }).command(),
            CommandKey::SetDisplayFull
        );
        assert_eq!(
            CommandInput::Link(LinkInput {
                code: "K7QXM2PD".to_owned()
            })
            .command(),
            CommandKey::Link
        );
    }

    #[test]
    fn invocation_guild_et_langue() {
        let invocation = Invocation::new(
            Some(guild()),
            Lang::En,
            Author::new("john"),
            CommandInput::Talk(TalkInput {
                text: None,
                voice: "bonjour".to_owned(),
            }),
        );
        assert_eq!(invocation.command(), CommandKey::Talk);
        assert_eq!(invocation.guild(), Some(&guild()));
        assert_eq!(invocation.lang, Lang::En);
        assert_eq!(invocation.author.username, "john");
    }

    #[test]
    fn source_media_optionnelle() {
        let mut submission = MediaSubmission::default();
        assert!(!submission.has_source());
        submission.media = Some("https://cdn.discordapp.com/x.mp4".to_owned());
        assert!(submission.has_source());
    }
}
