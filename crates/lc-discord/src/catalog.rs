//! Catalogue **de données pur** des commandes Discord legacy.
//!
//! Reproduit `server/src/loaders/DiscordLoader.ts` (ordre d'enregistrement,
//! commandes « cachées ») et les `SlashCommandBuilder` de
//! `server/src/components/**/*Command.ts` : mêmes noms, descriptions, options,
//! options requises et choix, pour les deux langues supportées.
//!
//! Particularité legacy importante : les **noms de commandes et d'options sont
//! localisés** (`rosetty` résout au démarrage avec `env.I18N`) ; la bot
//! n'enregistre donc qu'une seule variante linguistique à la fois
//! (`/msg` en français, `/send` en anglais). Le catalogue expose les deux.
//!
//! Aucune dépendance à serenity ici : la glue de la feature `serenity`
//! convertit ces données en `CreateCommand`.

use crate::i18n::{Lang, EN, FR};

/// Texte disponible en français et en anglais.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalizedText {
    /// Variante française.
    pub fr: &'static str,
    /// Variante anglaise.
    pub en: &'static str,
}

impl LocalizedText {
    /// Construit un texte bilingue (utilisable en `const`).
    pub const fn new(fr: &'static str, en: &'static str) -> Self {
        Self { fr, en }
    }

    /// Variante correspondant à `lang`.
    pub const fn get(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::Fr => self.fr,
            Lang::En => self.en,
        }
    }
}

/// Type d'option Discord (sous-ensemble utilisé par le legacy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionKind {
    /// `addStringOption`.
    String,
    /// `addIntegerOption`.
    Integer,
    /// `addBooleanOption`.
    Boolean,
    /// `addAttachmentOption`.
    Attachment,
}

/// Choix d'une option `String` (legacy `addChoices`).
#[derive(Clone, Copy, Debug)]
pub struct ChoiceSpec {
    /// Libellé localisé affiché.
    pub name: LocalizedText,
    /// Valeur envoyée dans l'interaction (`normal`/`invite`).
    pub value: &'static str,
}

/// Option d'une commande slash.
#[derive(Clone, Copy, Debug)]
pub struct OptionSpec {
    /// Nom localisé (fait partie du contrat d'API Discord).
    pub name: LocalizedText,
    /// Description localisée.
    pub description: LocalizedText,
    /// Type d'option.
    pub kind: OptionKind,
    /// `setRequired(true)` dans le legacy.
    pub required: bool,
    /// Choix imposés (tableau vide sinon).
    pub choices: &'static [ChoiceSpec],
}

/// Identité stable d'une commande, indépendante de la langue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandKey {
    /// `/alive` (fr : `/dispo`).
    Alive,
    /// `/pair-code`.
    PairCode,
    /// `/help` (nom non localisé).
    Help,
    /// `/info` (nom non localisé).
    Info,
    /// `/media-add`.
    MediaAdd,
    /// `/lier` (en : `/link`).
    Link,
    /// `/overlays`.
    Overlays,
    /// `/config-defaulttime` (fr : `/config-defaut`).
    SetDefaultTime,
    /// `/config-displayfull`.
    SetDisplayFull,
    /// `/config-maxtime` (fr : `/config-max`).
    SetMaxTime,
    /// `/send` (fr : `/msg`).
    Send,
    /// `/hsend` (fr : `/cmsg`).
    HideSend,
    /// `/talk` (fr : `/dire`).
    Talk,
    /// `/htalk` (fr : `/cdire`).
    HideTalk,
    /// `/stop`.
    Stop,
}

impl CommandKey {
    /// Identifiant machine stable (logs, future persistance d'interactions).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::PairCode => "pair-code",
            Self::Help => "help",
            Self::Info => "info",
            Self::MediaAdd => "media-add",
            Self::Link => "link",
            Self::Overlays => "overlays",
            Self::SetDefaultTime => "set-default-time",
            Self::SetDisplayFull => "set-display-full",
            Self::SetMaxTime => "set-max-time",
            Self::Send => "send",
            Self::HideSend => "hide-send",
            Self::Talk => "talk",
            Self::HideTalk => "hide-talk",
            Self::Stop => "stop",
        }
    }

    /// `true` pour les commandes conditionnellement enregistrées
    /// (`HIDE_COMMANDS_DISABLED === 'true'` les masque).
    pub const fn is_hidden_command(self) -> bool {
        matches!(self, Self::HideSend | Self::HideTalk)
    }

    /// `true` pour les commandes qui envoient du contenu sur le stream.
    pub const fn is_media_command(self) -> bool {
        matches!(
            self,
            Self::Send | Self::HideSend | Self::Talk | Self::HideTalk | Self::Stop
        )
    }
}

/// Spécification complète d'une commande slash.
#[derive(Clone, Copy, Debug)]
pub struct CommandSpec {
    /// Identité stable.
    pub key: CommandKey,
    /// Nom localisé envoyé à Discord.
    pub name: LocalizedText,
    /// Description localisée envoyée à Discord.
    pub description: LocalizedText,
    /// Options dans l'ordre legacy.
    pub options: &'static [OptionSpec],
}

impl CommandSpec {
    /// Nom pour `lang`.
    pub const fn name_for(self, lang: Lang) -> &'static str {
        self.name.get(lang)
    }

    /// Description pour `lang`.
    pub const fn description_for(self, lang: Lang) -> &'static str {
        self.description.get(lang)
    }
}

const NO_OPTIONS: &[OptionSpec] = &[];

const LINK_OPTIONS: &[OptionSpec] = &[OptionSpec {
    name: LocalizedText::new(FR.link_option_code, EN.link_option_code),
    description: LocalizedText::new(
        FR.link_option_code_description,
        EN.link_option_code_description,
    ),
    kind: OptionKind::String,
    required: true,
    choices: &[],
}];

const PAIR_CODE_CHOICES: &[ChoiceSpec] = &[
    ChoiceSpec {
        name: LocalizedText::new(FR.pair_code_choice_normal, EN.pair_code_choice_normal),
        value: "normal",
    },
    ChoiceSpec {
        name: LocalizedText::new(FR.pair_code_choice_invite, EN.pair_code_choice_invite),
        value: "invite",
    },
];

const PAIR_CODE_OPTIONS: &[OptionSpec] = &[OptionSpec {
    name: LocalizedText::new(FR.pair_code_option_mode, EN.pair_code_option_mode),
    description: LocalizedText::new(
        FR.pair_code_option_mode_description,
        EN.pair_code_option_mode_description,
    ),
    kind: OptionKind::String,
    required: false,
    choices: PAIR_CODE_CHOICES,
}];

const SEND_OPTIONS: &[OptionSpec] = &[
    OptionSpec {
        name: LocalizedText::new(FR.send_option_url, EN.send_option_url),
        description: LocalizedText::new(
            FR.send_option_url_description,
            EN.send_option_url_description,
        ),
        kind: OptionKind::String,
        required: false,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(FR.send_option_media, EN.send_option_media),
        description: LocalizedText::new(
            FR.send_option_media_description,
            EN.send_option_media_description,
        ),
        kind: OptionKind::Attachment,
        required: false,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(FR.send_option_text, EN.send_option_text),
        description: LocalizedText::new(
            FR.send_option_text_description,
            EN.send_option_text_description,
        ),
        kind: OptionKind::String,
        required: false,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(FR.send_option_force_refresh, EN.send_option_force_refresh),
        description: LocalizedText::new(
            FR.send_option_force_refresh_description,
            EN.send_option_force_refresh_description,
        ),
        kind: OptionKind::Boolean,
        required: false,
        choices: &[],
    },
];

const HIDE_SEND_OPTIONS: &[OptionSpec] = &[
    OptionSpec {
        name: LocalizedText::new(FR.hide_send_option_url, EN.hide_send_option_url),
        description: LocalizedText::new(
            FR.hide_send_option_url_description,
            EN.hide_send_option_url_description,
        ),
        kind: OptionKind::String,
        required: false,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(FR.hide_send_option_media, EN.hide_send_option_media),
        description: LocalizedText::new(
            FR.hide_send_option_media_description,
            EN.hide_send_option_media_description,
        ),
        kind: OptionKind::Attachment,
        required: false,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(FR.hide_send_option_text, EN.hide_send_option_text),
        description: LocalizedText::new(
            FR.hide_send_option_text_description,
            EN.hide_send_option_text_description,
        ),
        kind: OptionKind::String,
        required: false,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(
            FR.hide_send_option_force_refresh,
            EN.hide_send_option_force_refresh,
        ),
        description: LocalizedText::new(
            FR.hide_send_option_force_refresh_description,
            EN.hide_send_option_force_refresh_description,
        ),
        kind: OptionKind::Boolean,
        required: false,
        choices: &[],
    },
];

const MEDIA_ADD_OPTIONS: &[OptionSpec] = &[
    OptionSpec {
        name: LocalizedText::new(FR.media_add_option_url, EN.media_add_option_url),
        description: LocalizedText::new(
            FR.media_add_option_url_description,
            EN.media_add_option_url_description,
        ),
        kind: OptionKind::String,
        required: false,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(FR.media_add_option_media, EN.media_add_option_media),
        description: LocalizedText::new(
            FR.media_add_option_media_description,
            EN.media_add_option_media_description,
        ),
        kind: OptionKind::Attachment,
        required: false,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(FR.media_add_option_title, EN.media_add_option_title),
        description: LocalizedText::new(
            FR.media_add_option_title_description,
            EN.media_add_option_title_description,
        ),
        kind: OptionKind::String,
        required: false,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(
            FR.media_add_option_force_refresh,
            EN.media_add_option_force_refresh,
        ),
        description: LocalizedText::new(
            FR.media_add_option_force_refresh_description,
            EN.media_add_option_force_refresh_description,
        ),
        kind: OptionKind::Boolean,
        required: false,
        choices: &[],
    },
];

const TALK_OPTIONS: &[OptionSpec] = &[
    OptionSpec {
        name: LocalizedText::new(FR.talk_option_voice, EN.talk_option_voice),
        description: LocalizedText::new(
            FR.talk_option_voice_description,
            EN.talk_option_voice_description,
        ),
        kind: OptionKind::String,
        required: true,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(FR.talk_option_text, EN.talk_option_text),
        description: LocalizedText::new(
            FR.talk_option_text_description,
            EN.talk_option_text_description,
        ),
        kind: OptionKind::String,
        required: false,
        choices: &[],
    },
];

// Same order as the legacy `hidetalkCommand.ts`: Discord requires the required
// `voice` option before the optional `text` one.
const HIDE_TALK_OPTIONS: &[OptionSpec] = &[
    OptionSpec {
        name: LocalizedText::new(FR.hide_talk_option_voice, EN.hide_talk_option_voice),
        description: LocalizedText::new(
            FR.hide_talk_option_voice_description,
            EN.hide_talk_option_voice_description,
        ),
        kind: OptionKind::String,
        required: true,
        choices: &[],
    },
    OptionSpec {
        name: LocalizedText::new(FR.hide_talk_option_text, EN.hide_talk_option_text),
        description: LocalizedText::new(
            FR.hide_talk_option_text_description,
            EN.hide_talk_option_text_description,
        ),
        kind: OptionKind::String,
        required: false,
        choices: &[],
    },
];

const INTEGER_REQUIRED_OPTION: &[OptionSpec] = &[OptionSpec {
    name: LocalizedText::new(
        FR.set_default_time_option_text,
        EN.set_default_time_option_text,
    ),
    description: LocalizedText::new(
        FR.set_default_time_option_text_description,
        EN.set_default_time_option_text_description,
    ),
    kind: OptionKind::Integer,
    required: true,
    choices: &[],
}];

const MAX_TIME_OPTIONS: &[OptionSpec] = &[OptionSpec {
    name: LocalizedText::new(FR.set_max_time_option_text, EN.set_max_time_option_text),
    description: LocalizedText::new(
        FR.set_max_time_option_text_description,
        EN.set_max_time_option_text_description,
    ),
    kind: OptionKind::Integer,
    required: true,
    choices: &[],
}];

const DISPLAY_FULL_OPTIONS: &[OptionSpec] = &[OptionSpec {
    name: LocalizedText::new(
        FR.set_display_full_option_text,
        EN.set_display_full_option_text,
    ),
    description: LocalizedText::new(
        FR.set_display_full_option_text_description,
        EN.set_display_full_option_text_description,
    ),
    kind: OptionKind::Boolean,
    required: true,
    choices: &[],
}];

/// Catalogue complet, dans l'**ordre d'enregistrement legacy**.
///
/// `DiscordLoader.loadDiscordCommands` ajoute d'abord les 12 commandes
/// générales du legacy puis `hidesend`/`hidetalk` (si `HIDE_COMMANDS_DISABLED
/// !== 'true'`). `/lier` (nouvelle commande, hors legacy) s'insère juste avant
/// les commandes cachées. Cet ordre est aussi celui affiché par `/help`
/// (`global.commandsLoaded`).
pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        key: CommandKey::Alive,
        name: LocalizedText::new(FR.alive_name, EN.alive_name),
        description: LocalizedText::new(FR.alive_description, EN.alive_description),
        options: NO_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::Send,
        name: LocalizedText::new(FR.send_name, EN.send_name),
        description: LocalizedText::new(FR.send_description, EN.send_description),
        options: SEND_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::Talk,
        name: LocalizedText::new(FR.talk_name, EN.talk_name),
        description: LocalizedText::new(FR.talk_description, EN.talk_description),
        options: TALK_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::PairCode,
        name: LocalizedText::new(FR.pair_code_name, EN.pair_code_name),
        description: LocalizedText::new(FR.pair_code_description, EN.pair_code_description),
        options: PAIR_CODE_OPTIONS,
    },
    // `/help` et `/info` sont codés en dur en anglais dans le legacy
    // (`helpCommand.ts:4-9`, `infoCommand.ts:4-9`).
    CommandSpec {
        key: CommandKey::Help,
        name: LocalizedText::new("help", "help"),
        description: LocalizedText::new("List of Commands", "List of Commands"),
        options: NO_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::Info,
        name: LocalizedText::new("info", "info"),
        description: LocalizedText::new("Bot Informations", "Bot Informations"),
        options: NO_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::SetDefaultTime,
        name: LocalizedText::new(FR.set_default_time_name, EN.set_default_time_name),
        description: LocalizedText::new(
            FR.set_default_time_description,
            EN.set_default_time_description,
        ),
        options: INTEGER_REQUIRED_OPTION,
    },
    CommandSpec {
        key: CommandKey::SetDisplayFull,
        name: LocalizedText::new(FR.set_display_full_name, EN.set_display_full_name),
        description: LocalizedText::new(
            FR.set_display_full_description,
            EN.set_display_full_description,
        ),
        options: DISPLAY_FULL_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::SetMaxTime,
        name: LocalizedText::new(FR.set_max_time_name, EN.set_max_time_name),
        description: LocalizedText::new(FR.set_max_time_description, EN.set_max_time_description),
        options: MAX_TIME_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::Overlays,
        name: LocalizedText::new(FR.overlays_name, EN.overlays_name),
        description: LocalizedText::new(FR.overlays_description, EN.overlays_description),
        options: NO_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::Stop,
        name: LocalizedText::new(FR.stop_name, EN.stop_name),
        description: LocalizedText::new(FR.stop_description, EN.stop_description),
        options: NO_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::MediaAdd,
        name: LocalizedText::new(FR.media_add_name, EN.media_add_name),
        description: LocalizedText::new(FR.media_add_description, EN.media_add_description),
        options: MEDIA_ADD_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::Link,
        name: LocalizedText::new(FR.link_name, EN.link_name),
        description: LocalizedText::new(FR.link_description, EN.link_description),
        options: LINK_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::HideSend,
        name: LocalizedText::new(FR.hide_send_name, EN.hide_send_name),
        description: LocalizedText::new(FR.hide_send_description, EN.hide_send_description),
        options: HIDE_SEND_OPTIONS,
    },
    CommandSpec {
        key: CommandKey::HideTalk,
        name: LocalizedText::new(FR.hide_talk_name, EN.hide_talk_name),
        description: LocalizedText::new(FR.hide_talk_description, EN.hide_talk_description),
        options: HIDE_TALK_OPTIONS,
    },
];

/// Commandes effectivement enregistrées (`HIDE_COMMANDS_DISABLED`).
///
/// Reproduit la condition `env.HIDE_COMMANDS_DISABLED !== 'true'` du legacy :
/// les commandes cachées ne sont ajoutées que si l'option n'est pas activée.
pub fn registered_commands(hide_commands_disabled: bool) -> Vec<&'static CommandSpec> {
    COMMANDS
        .iter()
        .filter(|command| !(hide_commands_disabled && command.key.is_hidden_command()))
        .collect()
}

/// Noms localisés des commandes enregistrées, dans l'ordre de `/help`.
pub fn registered_command_names(lang: Lang, hide_commands_disabled: bool) -> Vec<&'static str> {
    registered_commands(hide_commands_disabled)
        .into_iter()
        .map(|command| command.name_for(lang))
        .collect()
}

/// Retrouve une commande par son nom **localisé** (nom envoyé par Discord).
pub fn find_by_name(
    lang: Lang,
    hide_commands_disabled: bool,
    name: &str,
) -> Option<&'static CommandSpec> {
    registered_commands(hide_commands_disabled)
        .into_iter()
        .find(|command| command.name_for(lang) == name)
}

/// Retrouve une option par son nom localisé.
pub fn find_option<'a>(command: &'a CommandSpec, lang: Lang, name: &str) -> Option<&'a OptionSpec> {
    command
        .options
        .iter()
        .find(|option| option.name.get(lang) == name)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{find_by_name, registered_command_names, CommandKey, OptionKind, COMMANDS};
    use crate::i18n::Lang;

    #[test]
    fn noms_localises_fideles_au_legacy() {
        let fr = registered_command_names(Lang::Fr, false);
        assert_eq!(
            fr,
            vec![
                "dispo",
                "msg",
                "dire",
                "pair-code",
                "help",
                "info",
                "config-defaut",
                "config-displayfull",
                "config-max",
                "overlays",
                "stop",
                "media-add",
                "lier",
                "cmsg",
                "cdire",
            ]
        );

        let en = registered_command_names(Lang::En, false);
        assert_eq!(
            en,
            vec![
                "alive",
                "send",
                "talk",
                "pair-code",
                "help",
                "info",
                "config-defaulttime",
                "config-displayfull",
                "config-maxtime",
                "overlays",
                "stop",
                "media-add",
                "link",
                "hsend",
                "htalk",
            ]
        );
    }

    #[test]
    fn commandes_cachees_exclues_quand_desactivees() {
        let names = registered_command_names(Lang::En, true);
        assert!(!names.contains(&"hsend"));
        assert!(!names.contains(&"htalk"));
        assert_eq!(names.len(), 13);
        assert!(find_by_name(Lang::En, true, "hsend").is_none());
        assert!(find_by_name(Lang::En, false, "hsend").is_some());
    }

    #[test]
    fn ordre_et_types_doptions() {
        let send = find_by_name(Lang::En, false, "send").expect("send doit exister");
        assert_eq!(send.key, CommandKey::Send);
        assert_eq!(send.options.len(), 4);
        assert_eq!(send.options[1].kind, OptionKind::Attachment);
        assert!(!send.options.iter().any(|option| option.required));

        let talk = find_by_name(Lang::Fr, false, "dire").expect("dire doit exister");
        assert_eq!(talk.options[0].kind, OptionKind::String);
        assert!(talk.options[0].required);
        assert!(!talk.options[1].required);

        let pair = find_by_name(Lang::Fr, false, "pair-code").expect("pair-code doit exister");
        let mode = &pair.options[0];
        assert_eq!(mode.kind, OptionKind::String);
        assert!(!mode.required);
        assert_eq!(mode.choices.len(), 2);
        assert_eq!(mode.choices[1].value, "invite");
        assert_eq!(mode.choices[0].name.get(Lang::Fr), "normal");

        let config = find_by_name(Lang::En, false, "config-maxtime").expect("config-maxtime");
        assert_eq!(config.options[0].kind, OptionKind::Integer);
        assert!(config.options[0].required);

        let link = find_by_name(Lang::Fr, false, "lier").expect("lier doit exister");
        assert_eq!(link.key, CommandKey::Link);
        assert_eq!(link.options.len(), 1);
        assert_eq!(link.options[0].kind, OptionKind::String);
        assert!(link.options[0].required);
        assert_eq!(link.options[0].name.get(Lang::Fr), "code");
        assert_eq!(link.options[0].name.get(Lang::En), "code");
    }

    /// Discord rejects the whole bulk registration ("Required options must be
    /// placed before non-required options") when one command breaks this rule.
    #[test]
    fn required_options_come_before_optional_ones() {
        for command in COMMANDS {
            let first_optional = command.options.iter().position(|option| !option.required);
            if let Some(first_optional) = first_optional {
                assert!(
                    command.options[first_optional..]
                        .iter()
                        .all(|option| !option.required),
                    "{}: required option after an optional one",
                    command.name_for(Lang::En)
                );
            }
        }
    }

    #[test]
    fn catalogue_contient_quinze_commandes() {
        assert_eq!(COMMANDS.len(), 15);
        assert_eq!(
            COMMANDS
                .iter()
                .filter(|command| command.key.is_hidden_command())
                .count(),
            2
        );
    }
}
