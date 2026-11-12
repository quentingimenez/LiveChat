//! Glue serenity (feature `serenity`, **désactivée par défaut**).
//!
//! Ce module ne contient que des briques d'adaptation :
//!
//! 1. **Scraping du catalogue** : [`command_payloads`] produit les corps JSON
//!    exacts à envoyer à `PUT /applications/{id}/commands` (ou à passer à
//!    `serenity`, Phase 6b).
//! 2. **Parsing d'interaction** : [`parse_invocation`] convertit une
//!    [`CommandInteraction`] en [`Invocation`] neutre, prête pour
//!    [`crate::handlers::dispatch`].
//! 3. **Réponses** : [`deferral_response`] et [`message_response`] rendent les
//!    corps JSON de callback (`type: 5`, `type: 4`), et [`respond`] les envoie
//!    via `serenity` quand la feature est compilée.
//!
//! La boucle gateway (handler `interactionCreate`, enregistrement au démarrage)
//! reste à câbler en Phase 6b : ce module évite volontairement toute dépendance
//! au réseau dans les tests (la feature est éteinte par défaut).

use lc_domain::{GuildId, PairingMode};
use serde_json::{json, Value};
use serenity::builder::{CreateEmbed, CreateInteractionResponse, CreateInteractionResponseMessage};
use serenity::http::Http;
use serenity::model::application::{CommandDataOption, CommandDataOptionValue, CommandInteraction};

use crate::catalog::{self, CommandSpec, OptionKind};
use crate::context::Author;
use crate::i18n::Lang;
use crate::inputs::{
    CommandInput, DisplayFullInput, Invocation, LinkInput, MediaSubmission, PairCodeInput,
    TalkInput, TimeSettingInput,
};
use crate::reply::{CommandOutcome, Deferral, Embed};

// ---------------------------------------------------------------------------
// 1. Enregistrement des commandes
// ---------------------------------------------------------------------------

/// Corps JSON d'une commande slash pour l'API Discord (`type` = `1`).
pub fn command_payload(spec: &CommandSpec, lang: Lang) -> Value {
    let options: Vec<Value> = spec
        .options
        .iter()
        .map(|option| {
            let mut payload = json!({
                "type": option_type(option.kind),
                "name": option.name.get(lang),
                "description": option.description.get(lang),
                "required": option.required,
            });
            if !option.choices.is_empty() {
                let choices: Vec<Value> = option
                    .choices
                    .iter()
                    .map(|choice| {
                        json!({
                            "name": choice.name.get(lang),
                            "value": choice.value,
                        })
                    })
                    .collect();
                payload["choices"] = Value::Array(choices);
            }
            payload
        })
        .collect();

    json!({
        "name": spec.name_for(lang),
        "description": spec.description_for(lang),
        "options": options,
    })
}

/// Corps JSON de toutes les commandes enregistrées, dans l'ordre legacy.
pub fn command_payloads(lang: Lang, hide_commands_disabled: bool) -> Vec<Value> {
    catalog::registered_commands(hide_commands_disabled)
        .into_iter()
        .map(|spec| command_payload(spec, lang))
        .collect()
}

/// Type numérique d'option attendu par l'API Discord.
pub const fn option_type(kind: OptionKind) -> u8 {
    match kind {
        OptionKind::String => 3,
        OptionKind::Integer => 4,
        OptionKind::Boolean => 5,
        OptionKind::Attachment => 11,
    }
}

// ---------------------------------------------------------------------------
// 2. Parsing d'une interaction
// ---------------------------------------------------------------------------

/// Convertit une interaction Hub en [`Invocation`] neutre.
///
/// Renvoie `None` si le nom localisé de la commande ne correspond à aucune
/// commande enregistrée (commande inconnue ou masquée par
/// `HIDE_COMMANDS_DISABLED`).
pub fn parse_invocation(
    interaction: &CommandInteraction,
    lang: Lang,
    hide_commands_disabled: bool,
) -> Option<Invocation> {
    let spec = *catalog::find_by_name(lang, hide_commands_disabled, &interaction.data.name)?;
    let input = parse_input(interaction, &spec, lang)?;
    Some(Invocation::new(
        domain_guild_id(interaction),
        lang,
        parse_author(interaction),
        input,
    ))
}

fn parse_input(
    interaction: &CommandInteraction,
    spec: &CommandSpec,
    lang: Lang,
) -> Option<CommandInput> {
    Some(match spec.key {
        crate::catalog::CommandKey::Alive => CommandInput::Alive,
        crate::catalog::CommandKey::Help => CommandInput::Help,
        crate::catalog::CommandKey::Info => CommandInput::Info,
        crate::catalog::CommandKey::Overlays => CommandInput::Overlays,
        crate::catalog::CommandKey::Stop => CommandInput::Stop,
        crate::catalog::CommandKey::PairCode => CommandInput::PairCode(PairCodeInput {
            mode: match string_option(interaction, spec, lang, 0).as_deref() {
                Some("invite") => PairingMode::InviteReadOnly,
                _ => PairingMode::Normal,
            },
        }),
        crate::catalog::CommandKey::SetDefaultTime => {
            CommandInput::SetDefaultTime(TimeSettingInput {
                seconds: integer_option(interaction, spec, lang, 0).unwrap_or(0),
            })
        }
        crate::catalog::CommandKey::SetMaxTime => CommandInput::SetMaxTime(TimeSettingInput {
            seconds: integer_option(interaction, spec, lang, 0).unwrap_or(0),
        }),
        crate::catalog::CommandKey::SetDisplayFull => {
            CommandInput::SetDisplayFull(DisplayFullInput {
                value: boolean_option(interaction, spec, lang, 0).unwrap_or(false),
            })
        }
        crate::catalog::CommandKey::Send => {
            CommandInput::Send(parse_media_submission(interaction, spec, lang))
        }
        crate::catalog::CommandKey::HideSend => {
            CommandInput::HideSend(parse_media_submission(interaction, spec, lang))
        }
        crate::catalog::CommandKey::MemeAdd => {
            // La glue legacy réutilise `text` pour le titre du meme board.
            CommandInput::MemeAdd(parse_media_submission(interaction, spec, lang))
        }
        crate::catalog::CommandKey::Link => {
            let code = string_option(interaction, spec, lang, 0)?;
            CommandInput::Link(LinkInput {
                // Le tiret affiché par l'overlay (`ABCD-2345`) est ignoré.
                code: code.trim().replace('-', "").to_ascii_uppercase(),
            })
        }
        crate::catalog::CommandKey::Talk => CommandInput::Talk(TalkInput {
            text: string_option(interaction, spec, lang, 1),
            voice: string_option(interaction, spec, lang, 0).unwrap_or_default(),
        }),
        crate::catalog::CommandKey::HideTalk => CommandInput::HideTalk(TalkInput {
            text: string_option(interaction, spec, lang, 0),
            voice: string_option(interaction, spec, lang, 1).unwrap_or_default(),
        }),
    })
}

fn parse_media_submission(
    interaction: &CommandInteraction,
    spec: &CommandSpec,
    lang: Lang,
) -> MediaSubmission {
    MediaSubmission {
        url: string_option(interaction, spec, lang, 0),
        media: attachment_option(interaction, spec, lang, 1),
        text: string_option(interaction, spec, lang, 2),
        force_refresh: boolean_option(interaction, spec, lang, 3).unwrap_or(false),
    }
}

/// Option de la commande à la position legacy (les noms sont localisés).
fn option_at<'a>(
    interaction: &'a CommandInteraction,
    spec: &'a CommandSpec,
    lang: Lang,
    index: usize,
) -> Option<&'a CommandDataOption> {
    let option_spec = spec.options.get(index)?;
    let name = option_spec.name.get(lang);
    interaction
        .data
        .options
        .iter()
        .find(|option| option.name == name)
}

fn string_option(
    interaction: &CommandInteraction,
    spec: &CommandSpec,
    lang: Lang,
    index: usize,
) -> Option<String> {
    match &option_at(interaction, spec, lang, index)?.value {
        CommandDataOptionValue::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn integer_option(
    interaction: &CommandInteraction,
    spec: &CommandSpec,
    lang: Lang,
    index: usize,
) -> Option<i64> {
    match &option_at(interaction, spec, lang, index)?.value {
        CommandDataOptionValue::Integer(value) => Some(*value),
        _ => None,
    }
}

fn boolean_option(
    interaction: &CommandInteraction,
    spec: &CommandSpec,
    lang: Lang,
    index: usize,
) -> Option<bool> {
    match &option_at(interaction, spec, lang, index)?.value {
        CommandDataOptionValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

/// URL de la pièce jointe (`attachment.url || attachment.proxyURL` côté
/// legacy ; Discord fournit toujours `url`).
fn attachment_option(
    interaction: &CommandInteraction,
    spec: &CommandSpec,
    lang: Lang,
    index: usize,
) -> Option<String> {
    let option = option_at(interaction, spec, lang, index)?;
    let CommandDataOptionValue::Attachment(attachment_id) = &option.value else {
        return None;
    };
    let resolved = &interaction.data.resolved;
    let attachment = resolved.attachments.get(attachment_id)?;
    Some(attachment.url.clone())
}

fn domain_guild_id(interaction: &CommandInteraction) -> Option<GuildId> {
    let guild_id = interaction.guild_id?;
    GuildId::parse(&guild_id.get().to_string()).ok()
}

fn parse_author(interaction: &CommandInteraction) -> Author {
    let user = &interaction.user;
    let username = user.name.clone();
    let member = interaction.member.as_ref();
    let nickname = member
        .and_then(|member| member.nick.clone())
        .filter(|nickname| !nickname.trim().is_empty());
    let global_name = user
        .global_name
        .clone()
        .filter(|global_name| !global_name.trim().is_empty());
    let name = nickname.or(global_name).unwrap_or_else(|| username.clone());
    let image = member
        .and_then(|member| member.avatar.as_ref().map(ToString::to_string))
        .and_then(|hash| guild_avatar_url(interaction, &hash))
        .or_else(|| user_avatar_url(user));
    let is_admin = member
        .and_then(|member| member.permissions)
        .is_some_and(|permissions| permissions.administrator());
    Author {
        id: user.id.get().to_string(),
        username,
        name,
        image,
        is_admin,
    }
}

fn guild_avatar_url(interaction: &CommandInteraction, hash: &str) -> Option<String> {
    let guild_id = interaction.guild_id?.get();
    let user_id = interaction.user.id.get();
    let extension = if hash.starts_with("a_") { "gif" } else { "png" };
    Some(format!(
        "https://cdn.discordapp.com/guilds/{guild_id}/users/{user_id}/avatars/{hash}.{extension}?size=256"
    ))
}

fn user_avatar_url(user: &serenity::model::user::User) -> Option<String> {
    let hash = user.avatar.as_ref().map(ToString::to_string)?;
    let user_id = user.id.get();
    let extension = if hash.starts_with("a_") { "gif" } else { "png" };
    Some(format!(
        "https://cdn.discordapp.com/avatars/{user_id}/{hash}.{extension}?size=256"
    ))
}

// ---------------------------------------------------------------------------
// 3. Réponses d'interaction
// ---------------------------------------------------------------------------

/// Corps JSON de l'accusé de réception différé (`type: 5`).
pub fn deferral_response(deferral: Deferral) -> Option<Value> {
    match deferral {
        Deferral::None => None,
        Deferral::Public => Some(json!({"type": 5})),
        Deferral::Ephemeral => Some(json!({"type": 5, "data": {"flags": 64}})),
    }
}

/// Corps JSON du message de réponse final (`type: 4`).
pub fn message_response(outcome: &CommandOutcome) -> Value {
    let mut data = json!({});
    if let Some(embed) = embed_json(&outcome.reply.embed) {
        data["embeds"] = json!([embed]);
    }
    if outcome.reply.ephemeral {
        data["flags"] = json!(64);
    }
    json!({"type": 4, "data": data})
}

/// Représentation JSON d'un embed (`title`/`description`/`color`).
pub fn embed_json(embed: &Embed) -> Option<Value> {
    let mut payload = json!({});
    let mut filled = false;
    if let Some(title) = &embed.title {
        payload["title"] = json!(title);
        filled = true;
    }
    if let Some(description) = &embed.description {
        payload["description"] = json!(description);
        filled = true;
    }
    if let Some(color) = embed.color {
        payload["color"] = json!(color);
        filled = true;
    }
    filled.then_some(payload)
}

/// Convertit un [`Embed`] neutre en embed serenity.
pub fn create_embed(embed: &Embed) -> CreateEmbed {
    let mut builder = CreateEmbed::new();
    if let Some(title) = &embed.title {
        builder = builder.title(title);
    }
    if let Some(description) = &embed.description {
        builder = builder.description(description);
    }
    if let Some(color) = embed.color {
        builder = builder.color(color);
    }
    builder
}

/// Envoie le deferral puis la réponse via l'API HTTP de Discord.
///
/// La boucle gateway de Phase 6b appelle `dispatch` puis cette fonction ; les
/// tests, eux, restent sur [`deferral_response`] / [`message_response`].
pub async fn respond(
    http: &Http,
    interaction: &CommandInteraction,
    outcome: &CommandOutcome,
) -> Result<(), Box<serenity::Error>> {
    if outcome.deferral != Deferral::None {
        let deferral = CreateInteractionResponseMessage::new()
            .ephemeral(outcome.deferral == Deferral::Ephemeral);
        let response = CreateInteractionResponse::Defer(deferral);
        http.create_interaction_response(interaction.id, &interaction.token, &response, Vec::new())
            .await
            .map_err(Box::new)?;
    }

    let mut message = CreateInteractionResponseMessage::new()
        .embed(create_embed(&outcome.reply.embed))
        .ephemeral(outcome.reply.ephemeral);
    // `embed` n'est pas optionnel côté builder : un embed vide est accepté mais
    // on préfère ne rien ajouter quand la réponse n'a que du texte.
    if outcome.reply.embed.title.is_none() && outcome.reply.embed.description.is_none() {
        message = CreateInteractionResponseMessage::new().ephemeral(outcome.reply.ephemeral);
    }
    let response = CreateInteractionResponse::Message(message);
    http.create_interaction_response(interaction.id, &interaction.token, &response, Vec::new())
        .await
        .map_err(Box::new)?;
    Ok(())
}
