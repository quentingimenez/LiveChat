//! Handlers des 15 commandes, indépendants de serenity.
//!
//! Chaque handler reproduit le comportement du legacy
//! (`server/src/components/**/*Command.ts`) : même texte, mêmes couleurs,
//! mêmes réponses éphémères, mêmes erreurs localisées. Le seul écart
//! volontaire est l'enrichissement des tweets de `/send` (réseau, Phase 6b).

use chrono::Duration as ChronoDuration;
use lc_db::LinkOutcome;
use lc_domain::{
    Guild, GuildId, MediaAsset, MediaKind, PairingCode, PairingCodeEntry, PairingMode,
};

use crate::catalog::{registered_command_names, CommandKey};
use crate::context::{
    load_guild, Author, BotConfig, BotContext, GuildSettingsPatch, MediaAddRequest, MediaRequest,
    PlaybackJobRequest, PAIRING_CODE_MAX_ATTEMPTS,
};
use crate::error::{BotError, MediaError, MediaErrorCode, MediaLimits};
use crate::i18n::{interpolate, strings, Lang};
use crate::inputs::{
    CommandInput, DisplayFullInput, Invocation, LinkInput, MediaSubmission, PairCodeInput,
    TalkInput, TimeSettingInput,
};
use crate::reply::{
    CommandOutcome, Deferral, Embed, Reply, COLOR_ERROR, COLOR_INFO, COLOR_SUCCESS, COLOR_WARNING,
};
use crate::rich_payload::build_media_text_payload;
use crate::text::{start_case, to_optional_text, youtube_start_offset_sec};

/// Exécute une invocation et renvoie la réponse complète.
pub async fn dispatch(ctx: &dyn BotContext, invocation: Invocation) -> CommandOutcome {
    let command = invocation.command();
    let deferral = plan_deferral(&invocation);
    let reply = match handle(ctx, invocation).await {
        Ok(reply) => reply,
        Err(error) => {
            tracing::warn!(?error, command = command.id(), "échec d'une commande");
            generic_error_reply(ctx.config().lang)
        }
    };
    CommandOutcome {
        command,
        deferral,
        reply,
    }
}

/// Accusé de réception à envoyer avant le traitement (comme le legacy).
pub fn plan_deferral(invocation: &Invocation) -> Deferral {
    match &invocation.input {
        CommandInput::Send(submission) => {
            if submission_has_content(submission) {
                Deferral::Public
            } else {
                Deferral::None
            }
        }
        CommandInput::HideSend(submission) => {
            if submission_has_content(submission) {
                Deferral::Ephemeral
            } else {
                Deferral::None
            }
        }
        CommandInput::MediaAdd(submission) => {
            if invocation.guild_id.is_some() && submission.has_source() {
                Deferral::Ephemeral
            } else {
                Deferral::None
            }
        }
        CommandInput::Talk(_) => Deferral::Public,
        CommandInput::HideTalk(_) => Deferral::Ephemeral,
        _ => Deferral::None,
    }
}

async fn handle(ctx: &dyn BotContext, invocation: Invocation) -> Result<Reply, BotError> {
    let lang = invocation.lang;
    let raw_guild_id = invocation.guild_id.clone();
    let is_link = invocation.command() == CommandKey::Link;
    let author = invocation.author;
    // Toutes les commandes sauf `/lier` visent la guilde résolue : un serveur
    // Discord lié à une guilde indépendante laisse le snowflake au seul
    // `/lier`, qui doit justement lier ce serveur brut.
    let guild_id = match raw_guild_id.as_ref() {
        Some(raw_guild_id) if !is_link => {
            Some(ctx.db().guilds().resolve_discord(raw_guild_id).await?)
        }
        _ => raw_guild_id.clone(),
    };
    match invocation.input {
        CommandInput::Alive => Ok(alive_reply(lang, &author)),
        CommandInput::Send(submission) => {
            handle_media_command(
                ctx,
                lang,
                guild_id.as_ref(),
                &author,
                submission,
                false,
                true,
                "discord_send_command",
                strings(lang).send_answer,
            )
            .await
        }
        CommandInput::HideSend(submission) => {
            handle_media_command(
                ctx,
                lang,
                guild_id.as_ref(),
                &author,
                submission,
                true,
                false,
                "discord_hidesend_command",
                strings(lang).hide_send_answer,
            )
            .await
        }
        CommandInput::MediaAdd(submission) => {
            handle_media_add(ctx, lang, guild_id.as_ref(), &author, submission).await
        }
        CommandInput::Talk(input) => {
            handle_talk(
                ctx,
                lang,
                guild_id.as_ref(),
                &author,
                input,
                false,
                "discord_talk_command",
                strings(lang).talk_answer,
            )
            .await
        }
        CommandInput::HideTalk(input) => {
            handle_talk(
                ctx,
                lang,
                guild_id.as_ref(),
                &author,
                input,
                true,
                "discord_hidetalk_command",
                strings(lang).hide_talk_answer,
            )
            .await
        }
        CommandInput::Link(input) => {
            handle_link(ctx, lang, raw_guild_id.as_ref(), &author, input).await
        }
        CommandInput::Overlays => Ok(handle_overlays(ctx, lang, guild_id.as_ref()).await),
        CommandInput::Help => Ok(handle_help(ctx, lang)),
        CommandInput::Info => Ok(handle_info(ctx, lang)),
        CommandInput::PairCode(input) => {
            handle_pair_code(ctx, lang, guild_id.as_ref(), &author, input).await
        }
        CommandInput::SetDefaultTime(input) => {
            handle_time_setting(
                ctx,
                lang,
                guild_id.as_ref(),
                &author,
                input,
                TimeSetting::Default,
            )
            .await
        }
        CommandInput::SetMaxTime(input) => {
            handle_time_setting(
                ctx,
                lang,
                guild_id.as_ref(),
                &author,
                input,
                TimeSetting::Max,
            )
            .await
        }
        CommandInput::SetDisplayFull(input) => {
            handle_display_full(ctx, lang, guild_id.as_ref(), &author, input).await
        }
        CommandInput::Stop => handle_stop(ctx, lang, guild_id.as_ref()).await,
    }
}

fn submission_has_content(submission: &MediaSubmission) -> bool {
    submission.has_source()
        || submission
            .text
            .as_deref()
            .is_some_and(|text| !text.is_empty())
}

// ---------------------------------------------------------------------------
// /alive, /help, /info
// ---------------------------------------------------------------------------

fn alive_reply(lang: Lang, author: &Author) -> Reply {
    let dict = strings(lang);
    let username = start_case(&author.username);
    let description = interpolate(dict.alive_answer, &[("username", &username)]);
    Reply::public(
        Embed::new()
            .with_description(description)
            .with_color(COLOR_SUCCESS),
    )
}

fn handle_help(ctx: &dyn BotContext, lang: Lang) -> Reply {
    let names = registered_command_names(lang, ctx.config().hide_commands_disabled);
    let description = names
        .iter()
        .map(|name| format!("/{name}"))
        .collect::<Vec<_>>()
        .join(", ");
    Reply::ephemeral(
        Embed::new()
            .with_title("Commands :")
            .with_description(description),
    )
}

fn handle_info(ctx: &dyn BotContext, _lang: Lang) -> Reply {
    let year = ctx.clock().now().format("%Y").to_string();
    let title = format!("LiveChat Server - {year}");
    let description = "[GitHub Repository](https://github.com/quentingimenez/LiveChat)";
    Reply::ephemeral(Embed::new().with_title(title).with_description(description))
}

// ---------------------------------------------------------------------------
// Ingestion média (/send, /hidesend, /media-add) et /dire, /cdire
// ---------------------------------------------------------------------------

async fn ingest_asset(
    ctx: &dyn BotContext,
    lang: Lang,
    submission: &MediaSubmission,
    ephemeral: bool,
) -> Result<MediaAsset, Reply> {
    let request = MediaRequest::new(
        submission.url.clone(),
        submission.media.clone(),
        submission.force_refresh,
    );
    match ctx.media().ingest_from_source(request).await {
        Ok(Some(asset)) => Ok(asset),
        Ok(None) => Err(media_error_reply(
            lang,
            &MediaError::new(MediaErrorCode::MediaNotFound),
            ctx.config().media,
            ephemeral,
        )),
        Err(error) => Err(media_error_reply(
            lang,
            &error,
            ctx.config().media,
            ephemeral,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_media_command(
    ctx: &dyn BotContext,
    lang: Lang,
    guild_id: Option<&GuildId>,
    author: &Author,
    submission: MediaSubmission,
    ephemeral: bool,
    include_author: bool,
    source: &'static str,
    success_answer: &'static str,
) -> Result<Reply, BotError> {
    let dict = strings(lang);
    if !submission_has_content(&submission) {
        // Le legacy répond immédiatement (avant tout defer) en éphémère.
        return Ok(error_reply(lang, dict.send_missing_content, true));
    }
    let Some(guild_id) = guild_id else {
        // Le legacy déréférençait `interaction.guildId!` juste après le defer :
        // l'erreur tombait dans le catch et produisait l'erreur média générique.
        return Ok(media_error_reply(
            lang,
            &MediaError::new(MediaErrorCode::DownloadFailed),
            ctx.config().media,
            ephemeral,
        ));
    };

    let offset = youtube_start_offset_sec(submission.url.as_deref(), submission.media.as_deref());
    let text_payload = build_media_text_payload(submission.text.as_deref(), offset);

    let guild = match load_guild(ctx, guild_id).await {
        Ok(guild) => guild,
        Err(error) => {
            return Ok(failure_reply(
                lang,
                &error,
                MediaErrorCode::DownloadFailed,
                ctx.config().media,
                ephemeral,
            ));
        }
    };

    let asset = if submission.has_source() {
        match ingest_asset(ctx, lang, &submission, ephemeral).await {
            Ok(asset) => Some(asset),
            Err(reply) => return Ok(reply),
        }
    } else {
        None
    };

    let base_duration = effective_duration(ctx.config(), guild.as_ref(), asset.as_ref(), None);
    let (duration_sec, resume_offset_sec) =
        adjusted_duration(base_duration, offset, asset.as_ref());

    let request = PlaybackJobRequest {
        guild_id: guild_id.clone(),
        media_asset_id: asset.as_ref().map(|asset| asset.id),
        text: text_payload,
        show_text: submission
            .text
            .as_deref()
            .is_some_and(|text| !text.is_empty()),
        author_name: include_author.then(|| author.username.clone()),
        author_image: include_author.then(|| author.image.clone()).flatten(),
        duration_sec,
        priority: 0,
        resume_offset_sec,
        source: source.to_owned(),
    };

    match ctx.playback().enqueue(request).await {
        Ok(()) => Ok(success_reply(lang, success_answer, ephemeral)),
        Err(error) => {
            tracing::warn!(?error, "échec de création du job de lecture");
            Ok(failure_reply(
                lang,
                &error,
                MediaErrorCode::DownloadFailed,
                ctx.config().media,
                ephemeral,
            ))
        }
    }
}

async fn handle_media_add(
    ctx: &dyn BotContext,
    lang: Lang,
    guild_id: Option<&GuildId>,
    author: &Author,
    submission: MediaSubmission,
) -> Result<Reply, BotError> {
    let dict = strings(lang);
    let Some(guild_id) = guild_id else {
        return Ok(not_allowed_reply(lang));
    };
    if !submission.has_source() {
        return Ok(error_reply(lang, dict.media_add_missing_media, true));
    }
    let asset = match ingest_asset(ctx, lang, &submission, true).await {
        Ok(asset) => asset,
        Err(reply) => return Ok(reply),
    };

    let title = to_optional_text(submission.text.as_deref(), 240);
    let request = MediaAddRequest {
        guild_id: guild_id.clone(),
        media_asset_id: asset.id,
        title,
        message: None,
        created_by_discord_user_id: Some(author.id.clone()),
        created_by_name: Some(author.name.clone()),
    };
    match ctx.media_board().add_to_board(request).await {
        Ok(created) => {
            let answer = if created {
                dict.media_add_answer_created
            } else {
                dict.media_add_answer_exists
            };
            Ok(success_reply(lang, answer, true))
        }
        Err(error) => {
            tracing::warn!(?error, "échec d'ajout au board média");
            Ok(failure_reply(
                lang,
                &error,
                MediaErrorCode::DownloadFailed,
                ctx.config().media,
                true,
            ))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_talk(
    ctx: &dyn BotContext,
    lang: Lang,
    guild_id: Option<&GuildId>,
    author: &Author,
    input: TalkInput,
    ephemeral: bool,
    source: &'static str,
    success_answer: &'static str,
) -> Result<Reply, BotError> {
    let dict = strings(lang);
    let Some(guild_id) = guild_id else {
        return Ok(media_error_reply(
            lang,
            &MediaError::new(MediaErrorCode::TranscodeFailed),
            ctx.config().media,
            ephemeral,
        ));
    };

    let asset = match ctx.media().ingest_speech(lang, &input.voice).await {
        Ok(asset) => asset,
        Err(_) => return Ok(error_reply(lang, dict.talk_voice_error, ephemeral)),
    };

    let guild = match load_guild(ctx, guild_id).await {
        Ok(guild) => guild,
        Err(error) => {
            return Ok(failure_reply(
                lang,
                &error,
                MediaErrorCode::TranscodeFailed,
                ctx.config().media,
                ephemeral,
            ));
        }
    };
    let duration_sec = effective_duration(ctx.config(), guild.as_ref(), Some(&asset), None);

    let request = PlaybackJobRequest {
        guild_id: guild_id.clone(),
        media_asset_id: Some(asset.id),
        text: input.text.clone(),
        show_text: input.text.as_deref().is_some_and(|text| !text.is_empty()),
        author_name: (!ephemeral).then(|| author.username.clone()),
        author_image: (!ephemeral).then(|| author.image.clone()).flatten(),
        duration_sec,
        priority: 0,
        resume_offset_sec: 0,
        source: source.to_owned(),
    };
    match ctx.playback().enqueue(request).await {
        Ok(()) => Ok(success_reply(lang, success_answer, ephemeral)),
        Err(error) => {
            tracing::warn!(?error, "échec de création du job vocal");
            Ok(failure_reply(
                lang,
                &error,
                MediaErrorCode::TranscodeFailed,
                ctx.config().media,
                ephemeral,
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// /overlays
// ---------------------------------------------------------------------------

async fn handle_overlays(ctx: &dyn BotContext, lang: Lang, guild_id: Option<&GuildId>) -> Reply {
    let dict = strings(lang);
    let Some(guild_id) = guild_id else {
        return not_allowed_reply(lang);
    };
    match ctx.list_connected_overlays(guild_id).await {
        Ok(mut overlays) => {
            let count = overlays.len();
            let title = interpolate(dict.overlays_answer_title, &[("count", &count.to_string())]);
            if overlays.is_empty() {
                return Reply::ephemeral(
                    Embed::new()
                        .with_title(title)
                        .with_description(dict.overlays_answer_empty)
                        .with_color(COLOR_WARNING),
                );
            }
            overlays.sort_by_key(|overlay| overlay.label.to_lowercase());
            let description = overlays
                .iter()
                .map(|overlay| format!("• `{}` (`{}`)", overlay.label, overlay.client_id))
                .collect::<Vec<_>>()
                .join("\n");
            Reply::ephemeral(
                Embed::new()
                    .with_title(title)
                    .with_description(description)
                    .with_color(COLOR_SUCCESS),
            )
        }
        Err(error) => {
            tracing::warn!(?error, "échec de lecture des overlays connectés");
            generic_error_reply(lang)
        }
    }
}

// ---------------------------------------------------------------------------
// /client (code d'appairage)
// ---------------------------------------------------------------------------

async fn handle_pair_code(
    ctx: &dyn BotContext,
    lang: Lang,
    guild_id: Option<&GuildId>,
    author: &Author,
    input: PairCodeInput,
) -> Result<Reply, BotError> {
    let dict = strings(lang);
    let Some(guild_id) = guild_id else {
        return Ok(generic_error_reply(lang));
    };

    let code = match generate_unique_code(ctx).await? {
        Some(code) => code,
        None => return Ok(generic_error_reply(lang)),
    };
    let now = ctx.clock().now();
    let ttl_minutes = ctx.config().pairing_code_ttl_minutes.max(1);
    let (mode, expires_at, template) = match input.mode {
        PairingMode::Normal => (
            PairingMode::Normal,
            Some(now + ChronoDuration::minutes(ttl_minutes)),
            dict.pair_code_answer_description,
        ),
        PairingMode::InviteReadOnly => (
            PairingMode::InviteReadOnly,
            None,
            dict.pair_code_answer_description_invite,
        ),
    };
    let entry = PairingCodeEntry {
        code,
        guild_id: guild_id.clone(),
        mode,
        created_by_discord_user_id: author.id.clone(),
        author_name: Some(author.name.clone()),
        author_image: author.image.clone(),
        expires_at,
        used_at: None,
        created_at: now,
    };
    ctx.db().pairing_codes().create(&entry).await?;

    let api_url = ctx.config().api_url.clone();
    let code_text = entry.code.as_str().to_owned();
    let expires_in = ttl_minutes.to_string();
    let description = interpolate(
        template,
        &[
            ("apiUrl", &api_url),
            ("code", &code_text),
            ("expiresIn", &expires_in),
        ],
    );
    Ok(Reply::ephemeral(
        Embed::new()
            .with_title(dict.pair_code_answer_title)
            .with_description(description)
            .with_color(COLOR_INFO),
    ))
}

async fn generate_unique_code(
    ctx: &dyn BotContext,
) -> Result<Option<lc_domain::PairingCode>, BotError> {
    for _ in 0..PAIRING_CODE_MAX_ATTEMPTS {
        let code = ctx.generate_pairing_code();
        if ctx.db().pairing_codes().get(&code).await?.is_none() {
            return Ok(Some(code));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Réglages de guilde
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimeSetting {
    Default,
    Max,
}

async fn handle_time_setting(
    ctx: &dyn BotContext,
    lang: Lang,
    guild_id: Option<&GuildId>,
    author: &Author,
    input: TimeSettingInput,
    setting: TimeSetting,
) -> Result<Reply, BotError> {
    let dict = strings(lang);
    let Some(guild_id) = guild_id else {
        return Ok(generic_error_reply(lang));
    };
    if !author.is_admin {
        return Ok(not_allowed_reply(lang));
    }
    let current = load_guild(ctx, guild_id).await?;
    let requested = if input.seconds == 0 {
        None
    } else {
        Some(input.seconds)
    };
    let mut patch = GuildSettingsPatch {
        default_media_time: current.as_ref().and_then(|guild| guild.default_media_time),
        max_media_time: current.as_ref().and_then(|guild| guild.max_media_time),
        display_media_full: current
            .as_ref()
            .is_some_and(|guild| guild.display_media_full),
    };
    let answer = match setting {
        TimeSetting::Default => {
            patch.default_media_time = requested;
            dict.set_default_time_answer
        }
        TimeSetting::Max => {
            patch.max_media_time = requested;
            dict.set_max_time_answer
        }
    };
    ctx.update_guild_settings(guild_id, patch).await?;
    Ok(success_reply(lang, answer, true))
}

async fn handle_display_full(
    ctx: &dyn BotContext,
    lang: Lang,
    guild_id: Option<&GuildId>,
    author: &Author,
    input: DisplayFullInput,
) -> Result<Reply, BotError> {
    let dict = strings(lang);
    let Some(guild_id) = guild_id else {
        return Ok(generic_error_reply(lang));
    };
    if !author.is_admin {
        return Ok(not_allowed_reply(lang));
    }
    let current = load_guild(ctx, guild_id).await?;
    let patch = GuildSettingsPatch {
        default_media_time: current.as_ref().and_then(|guild| guild.default_media_time),
        max_media_time: current.as_ref().and_then(|guild| guild.max_media_time),
        display_media_full: input.value,
    };
    ctx.update_guild_settings(guild_id, patch).await?;
    Ok(success_reply(lang, dict.set_display_full_answer, true))
}

// ---------------------------------------------------------------------------
// /lier (liaison d'un serveur Discord à une guilde indépendante)
// ---------------------------------------------------------------------------

async fn handle_link(
    ctx: &dyn BotContext,
    lang: Lang,
    raw_guild_id: Option<&GuildId>,
    author: &Author,
    input: LinkInput,
) -> Result<Reply, BotError> {
    let dict = strings(lang);
    let Some(discord_guild_id) = raw_guild_id else {
        return Ok(generic_error_reply(lang));
    };
    if !author.is_admin {
        return Ok(not_allowed_reply(lang));
    }
    let Ok(code) = PairingCode::parse(&input.code) else {
        return Ok(error_reply(lang, dict.link_invalid_code, true));
    };
    let outcome = ctx
        .db()
        .discord_link_codes()
        .redeem(&code, discord_guild_id, ctx.clock().now())
        .await?;
    Ok(match outcome {
        LinkOutcome::Linked { guild_name, .. } => {
            let name = guild_name.as_deref().unwrap_or("LiveChat");
            let description = interpolate(dict.link_success_description, &[("guildName", name)]);
            Reply::ephemeral(
                Embed::new()
                    .with_title(dict.link_success_title)
                    .with_description(description)
                    .with_color(COLOR_SUCCESS),
            )
        }
        LinkOutcome::InvalidCode => error_reply(lang, dict.link_invalid_code, true),
        LinkOutcome::AlreadyLinkedElsewhere => error_reply(lang, dict.link_already_linked, true),
    })
}

// ---------------------------------------------------------------------------
// /stop
// ---------------------------------------------------------------------------

async fn handle_stop(
    ctx: &dyn BotContext,
    lang: Lang,
    guild_id: Option<&GuildId>,
) -> Result<Reply, BotError> {
    let dict = strings(lang);
    let Some(guild_id) = guild_id else {
        return Ok(generic_error_reply(lang));
    };
    let released = ctx.playback().stop_guild(guild_id).await?;
    tracing::info!(
        guild_id = guild_id.as_str(),
        released,
        "arrêt manuel demandé par commande"
    );
    Ok(success_reply(lang, dict.stop_answer, true))
}

// ---------------------------------------------------------------------------
// Calculs de durée (fidèles à `createPlaybackJob` + `getDurationFromGuildId`)
// ---------------------------------------------------------------------------

fn effective_duration(
    config: &BotConfig,
    guild: Option<&Guild>,
    media: Option<&MediaAsset>,
    explicit: Option<i64>,
) -> i64 {
    let candidate = explicit
        .or_else(|| {
            media.and_then(|asset| match asset.kind {
                MediaKind::Image => None,
                _ => asset.duration_sec,
            })
        })
        .or_else(|| media.map(|_| config.default_duration_sec.max(1)));
    let base = candidate
        .or_else(|| guild.and_then(|guild| guild.default_media_time))
        .unwrap_or(config.default_duration_sec)
        .max(1);
    match guild.and_then(|guild| guild.max_media_time) {
        Some(max) if base > max => max.max(1),
        _ => base,
    }
}

fn adjusted_duration(base: i64, offset: Option<i64>, media: Option<&MediaAsset>) -> (i64, i64) {
    match (offset, media) {
        (Some(offset), Some(asset)) if offset > 0 && asset.kind != MediaKind::Image => {
            ((base - offset).max(1), offset)
        }
        _ => (base, 0),
    }
}

// ---------------------------------------------------------------------------
// Fabriques de réponses
// ---------------------------------------------------------------------------

fn success_reply(lang: Lang, description: &str, ephemeral: bool) -> Reply {
    let embed = Embed::new()
        .with_title(strings(lang).success)
        .with_description(description)
        .with_color(COLOR_SUCCESS);
    if ephemeral {
        Reply::ephemeral(embed)
    } else {
        Reply::public(embed)
    }
}

fn error_reply(lang: Lang, description: &str, ephemeral: bool) -> Reply {
    let embed = Embed::new()
        .with_title(strings(lang).error)
        .with_description(description)
        .with_color(COLOR_ERROR);
    if ephemeral {
        Reply::ephemeral(embed)
    } else {
        Reply::public(embed)
    }
}

fn media_error_reply(
    lang: Lang,
    error: &MediaError,
    limits: MediaLimits,
    ephemeral: bool,
) -> Reply {
    error_reply(lang, &error.message(lang, limits), ephemeral)
}

fn failure_reply(
    lang: Lang,
    error: &BotError,
    fallback: MediaErrorCode,
    limits: MediaLimits,
    ephemeral: bool,
) -> Reply {
    let message = match error {
        BotError::Media(media) => media.message(lang, limits),
        _ => fallback.message(lang, limits),
    };
    error_reply(lang, &message, ephemeral)
}

fn not_allowed_reply(lang: Lang) -> Reply {
    Reply::ephemeral(
        Embed::new()
            .with_title(strings(lang).not_allowed)
            .with_color(COLOR_ERROR),
    )
}

fn generic_error_reply(lang: Lang) -> Reply {
    Reply::ephemeral(
        Embed::new()
            .with_title(strings(lang).error)
            .with_description(strings(lang).command_error)
            .with_color(COLOR_ERROR),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use chrono::Duration as ChronoDuration;
    use lc_domain::entities::DiscordLinkCode;
    use lc_domain::ids::{GuildId, PairingCode};

    use super::{dispatch, plan_deferral, Deferral, Invocation};
    use crate::context::{Author, BotContext};
    use crate::i18n::{strings, Lang};
    use crate::inputs::{CommandInput, LinkInput, MediaSubmission, TalkInput, TimeSettingInput};
    use crate::reply::COLOR_SUCCESS;
    use crate::testing::{admin_author, member_author, sample_now, FakeBotContext};

    fn invocation(input: CommandInput) -> Invocation {
        Invocation::new(None, Lang::Fr, Author::new("john"), input)
    }

    async fn standalone_guild(ctx: &FakeBotContext, raw: &str, name: &str) -> GuildId {
        let guild_id = GuildId::parse(raw).unwrap();
        ctx.db()
            .guilds()
            .create_standalone(&guild_id, name, sample_now())
            .await
            .unwrap();
        guild_id
    }

    async fn seed_link_code(ctx: &FakeBotContext, guild_id: &GuildId, raw: &str) {
        let now = sample_now();
        ctx.db()
            .discord_link_codes()
            .replace_for_guild(&DiscordLinkCode {
                code: PairingCode::parse(raw).unwrap(),
                guild_id: guild_id.clone(),
                created_by_member_id: None,
                expires_at: now + ChronoDuration::minutes(10),
                used_at: None,
                created_at: now,
            })
            .await
            .unwrap();
    }

    #[test]
    fn deferral_send_selon_contenu() {
        let empty = invocation(CommandInput::Send(MediaSubmission::default()));
        assert_eq!(plan_deferral(&empty), Deferral::None);

        let text_only = invocation(CommandInput::Send(MediaSubmission {
            text: Some("salut".to_owned()),
            ..MediaSubmission::default()
        }));
        assert_eq!(plan_deferral(&text_only), Deferral::Public);
    }

    #[test]
    fn deferral_hide_et_talk() {
        let hide = invocation(CommandInput::HideSend(MediaSubmission {
            url: Some("https://example.com/x".to_owned()),
            ..MediaSubmission::default()
        }));
        assert_eq!(plan_deferral(&hide), Deferral::Ephemeral);

        let talk = invocation(CommandInput::Talk(TalkInput {
            text: None,
            voice: "bonjour".to_owned(),
        }));
        assert_eq!(plan_deferral(&talk), Deferral::Public);
    }

    #[test]
    fn deferral_media_add() {
        let no_source = invocation(CommandInput::MediaAdd(MediaSubmission::default()));
        assert_eq!(plan_deferral(&no_source), Deferral::None);

        let with_source = invocation(CommandInput::MediaAdd(MediaSubmission {
            media: Some("https://cdn.discordapp.com/x.png".to_owned()),
            ..MediaSubmission::default()
        }));
        // Sans guilde le legacy répond immédiatement not_allowed.
        assert_eq!(plan_deferral(&with_source), Deferral::None);
    }

    #[test]
    fn deferral_simples() {
        assert_eq!(
            plan_deferral(&invocation(CommandInput::Help)),
            Deferral::None
        );
        assert_eq!(
            plan_deferral(&invocation(CommandInput::Stop)),
            Deferral::None
        );
        assert_eq!(
            plan_deferral(&invocation(CommandInput::Link(LinkInput {
                code: "K7QXM2PD".to_owned(),
            }))),
            Deferral::None
        );
    }

    #[tokio::test]
    async fn link_lie_le_serveur_et_repond_en_ephemere() {
        let ctx = FakeBotContext::new().await.unwrap();
        let guild_id = standalone_guild(&ctx, "111", "Les Sardines").await;
        let discord = GuildId::parse("123456789012345678").unwrap();
        seed_link_code(&ctx, &guild_id, "K7QXM2PD").await;

        let outcome = dispatch(
            &ctx,
            Invocation::new(
                Some(discord.clone()),
                Lang::Fr,
                admin_author(),
                CommandInput::Link(LinkInput {
                    code: "k7qxm2pd".to_owned(),
                }),
            ),
        )
        .await;

        assert!(outcome.reply.ephemeral);
        assert_eq!(outcome.reply.embed.color, Some(COLOR_SUCCESS));
        assert_eq!(outcome.reply.embed.title.as_deref(), Some("Serveur lié"));
        assert!(outcome
            .reply
            .embed
            .description
            .as_deref()
            .is_some_and(|text| text.contains("Les Sardines")));
        assert_eq!(
            ctx.db().guilds().resolve_discord(&discord).await.unwrap(),
            guild_id
        );
    }

    #[tokio::test]
    async fn link_refuse_les_non_admins() {
        let ctx = FakeBotContext::new().await.unwrap();
        let guild_id = standalone_guild(&ctx, "111", "Les Sardines").await;
        let discord = GuildId::parse("123456789012345678").unwrap();
        seed_link_code(&ctx, &guild_id, "K7QXM2PD").await;

        let outcome = dispatch(
            &ctx,
            Invocation::new(
                Some(discord.clone()),
                Lang::Fr,
                member_author(),
                CommandInput::Link(LinkInput {
                    code: "K7QXM2PD".to_owned(),
                }),
            ),
        )
        .await;

        assert!(outcome.reply.ephemeral);
        assert_eq!(outcome.reply.embed.color, Some(crate::reply::COLOR_ERROR));
        assert_eq!(
            outcome.reply.embed.title.as_deref(),
            Some(strings(Lang::Fr).not_allowed)
        );
        assert_eq!(
            ctx.db().guilds().resolve_discord(&discord).await.unwrap(),
            discord
        );
    }

    #[tokio::test]
    async fn link_refuse_un_code_invalide() {
        let ctx = FakeBotContext::new().await.unwrap();
        let discord = GuildId::parse("123456789012345678").unwrap();

        for code in ["K7QXM2PD", "pas un code !"] {
            let outcome = dispatch(
                &ctx,
                Invocation::new(
                    Some(discord.clone()),
                    Lang::Fr,
                    admin_author(),
                    CommandInput::Link(LinkInput {
                        code: code.to_owned(),
                    }),
                ),
            )
            .await;

            assert!(outcome.reply.ephemeral);
            assert_eq!(outcome.reply.embed.color, Some(crate::reply::COLOR_ERROR));
            assert_eq!(
                outcome.reply.embed.description.as_deref(),
                Some(strings(Lang::Fr).link_invalid_code)
            );
        }
    }

    #[tokio::test]
    async fn commande_sur_serveur_lie_vise_la_guilde_independante() {
        let ctx = FakeBotContext::new().await.unwrap();
        let guild_id = standalone_guild(&ctx, "111", "Les Sardines").await;
        let discord = GuildId::parse("123456789012345678").unwrap();
        ctx.db()
            .guilds()
            .link_discord(&guild_id, Some(&discord))
            .await
            .unwrap();

        let outcome = dispatch(
            &ctx,
            Invocation::new(
                Some(discord.clone()),
                Lang::Fr,
                admin_author(),
                CommandInput::SetDefaultTime(TimeSettingInput { seconds: 10 }),
            ),
        )
        .await;

        assert_eq!(outcome.reply.embed.color, Some(COLOR_SUCCESS));
        let row = ctx
            .db()
            .guilds()
            .get(&guild_id)
            .await
            .unwrap()
            .expect("ligne de la guilde indépendante");
        assert_eq!(row.default_media_time, Some(10));
        assert!(ctx.db().guilds().get(&discord).await.unwrap().is_none());
    }
}
