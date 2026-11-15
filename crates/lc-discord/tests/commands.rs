//! Tests d'intégration des 12 commandes contre [`FakeBotContext`].
//!
//! Chaque commande est exercée une fois sur son chemin heureux et, quand le
//! legacy a un comportement d'erreur observable, sur ce chemin aussi. Aucun
//! réseau, aucun Discord : la base est une SQLite en mémoire réelle.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::Duration as ChronoDuration;
use lc_discord::catalog::CommandKey;
use lc_discord::context::{BotConfig, BotContext, ConnectedOverlay};
use lc_discord::error::MediaErrorCode;
use lc_discord::handlers::dispatch;
use lc_discord::i18n::{strings, Lang};
use lc_discord::inputs::{CommandInput, Invocation, MediaSubmission, PairCodeInput, TalkInput};
use lc_discord::reply::{
    CommandOutcome, Deferral, COLOR_ERROR, COLOR_INFO, COLOR_SUCCESS, COLOR_WARNING,
};
use lc_discord::testing::{admin_author, member_author, sample_asset, sample_now, FakeBotContext};
use lc_domain::{GuildId, MediaKind, PairingMode};

const ADMIN_ID: &str = "424242424242424242";
const MEMBER_ID: &str = "515151515151515151";

fn guild() -> GuildId {
    GuildId::parse("123456789012345678").expect("identifiant de guilde valide")
}

fn invocation(
    input: CommandInput,
    author: lc_discord::context::Author,
    guild_id: Option<GuildId>,
    lang: Lang,
) -> Invocation {
    Invocation::new(guild_id, lang, author, input)
}

fn description(outcome: &CommandOutcome) -> String {
    outcome
        .reply
        .embed
        .description_text()
        .unwrap_or_default()
        .to_owned()
}

fn title(outcome: &CommandOutcome) -> Option<String> {
    outcome.reply.embed.title_text().map(str::to_owned)
}

// ---------------------------------------------------------------------------
// /alive, /help, /info
// ---------------------------------------------------------------------------

#[tokio::test]
async fn alive_repond_publiquement_avec_le_pseudo_mis_en_forme() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(CommandInput::Alive, member_author(), None, Lang::Fr),
    )
    .await;

    assert_eq!(outcome.command, CommandKey::Alive);
    assert_eq!(outcome.deferral, Deferral::None);
    assert!(!outcome.reply.ephemeral);
    assert_eq!(outcome.reply.embed.color, Some(COLOR_SUCCESS));
    assert_eq!(outcome.reply.embed.title, None);
    assert_eq!(description(&outcome), "John Doe, Je suis en vie !");
}

#[tokio::test]
async fn help_liste_les_commandes_enregistrees_dans_l_ordre_legacy() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(CommandInput::Help, member_author(), None, Lang::Fr),
    )
    .await;

    assert!(outcome.reply.ephemeral);
    assert_eq!(title(&outcome).as_deref(), Some("Commands :"));
    let description = description(&outcome);
    let expected = strings(Lang::Fr);
    assert!(description.contains(expected.send_name));
    assert!(description.contains("/dispo"));
    assert!(description.contains("/msg"));
    let first = description.find("/dispo").expect("dispo");
    let send = description.find("/msg").expect("msg");
    assert!(first < send, "l'ordre legacy doit être respecté");
}

#[tokio::test]
async fn help_masque_les_commandes_cachees_quand_configure() {
    let mut ctx = FakeBotContext::new().await.expect("contexte");
    let mut config = BotConfig::new("http://localhost:3333");
    config.hide_commands_disabled = true;
    ctx.set_config(config);

    let outcome = dispatch(
        &ctx,
        invocation(CommandInput::Help, member_author(), None, Lang::Fr),
    )
    .await;
    let description = description(&outcome);
    assert!(description.contains("/dispo"));
    assert!(!description.contains("/cmsg"), "{description}");
    assert!(!description.contains("/cdire"), "{description}");
}

#[tokio::test]
async fn info_affiche_l_annee_de_l_horloge_et_le_depot() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(CommandInput::Info, member_author(), None, Lang::Fr),
    )
    .await;

    assert!(outcome.reply.ephemeral);
    assert_eq!(title(&outcome).as_deref(), Some("LiveChat Server - 2024"));
    assert_eq!(
        description(&outcome),
        "[GitHub Repository](https://github.com/quentingimenez/LiveChat)"
    );
}

// ---------------------------------------------------------------------------
// /client (code d'appairage)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pair_code_normal_persiste_un_code_avec_expiration() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::PairCode(PairCodeInput {
                mode: PairingMode::Normal,
            }),
            admin_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    // Acquittement éphémère immédiat, puis édition avec le code créé.
    assert_eq!(outcome.deferral, Deferral::Ephemeral);
    assert!(outcome.reply.ephemeral);
    assert_eq!(outcome.reply.embed.color, Some(COLOR_INFO));
    let description = description(&outcome);
    assert!(
        description.contains("http://localhost:3333"),
        "{description}"
    );
    assert!(
        description.contains("Expire dans 5 minute(s)."),
        "{description}"
    );
    assert!(
        description.contains(strings(Lang::Fr).pair_code_grants_admin),
        "un admin Discord doit voir la ligne « admin » : {description}"
    );
    assert!(
        description.contains(strings(Lang::Fr).pair_code_personal),
        "a normal code says it is personal: {description}"
    );

    let codes = ctx.db().pairing_codes().list().await.expect("codes");
    assert_eq!(codes.len(), 1);
    let entry = &codes[0];
    assert_eq!(entry.guild_id, guild());
    assert_eq!(entry.mode, PairingMode::Normal);
    assert!(entry.grants_admin, "admin Discord en mode normal");
    assert_eq!(entry.created_by_discord_user_id, ADMIN_ID);
    assert_eq!(entry.author_name.as_deref(), Some("Admin"));
    assert_eq!(
        entry.expires_at,
        Some(sample_now() + ChronoDuration::minutes(5))
    );
    assert!(entry.used_at.is_none());
}

#[tokio::test]
async fn pair_code_invite_est_sans_expiration() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::PairCode(PairCodeInput {
                mode: PairingMode::InviteReadOnly,
            }),
            admin_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert!(outcome.reply.ephemeral);
    let description = description(&outcome);
    assert!(!description.contains("Expire"), "{description}");
    assert!(
        !description.contains(strings(Lang::Fr).pair_code_grants_admin),
        "un code invité ne donne jamais l'admin : {description}"
    );
    assert!(
        !description.contains(strings(Lang::Fr).pair_code_personal),
        "an invite code is meant for someone else: {description}"
    );
    let codes = ctx.db().pairing_codes().list().await.expect("codes");
    assert_eq!(codes[0].mode, PairingMode::InviteReadOnly);
    assert!(!codes[0].grants_admin);
    assert!(codes[0].expires_at.is_none());
    assert!(codes[0].is_usable_at(sample_now()));
}

#[tokio::test]
async fn pair_code_d_un_membre_n_accorde_pas_l_admin() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::PairCode(PairCodeInput {
                mode: PairingMode::Normal,
            }),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    // A non-administrator normal code grants nothing beyond the member role.
    assert!(
        !description(&outcome).contains(strings(Lang::Fr).pair_code_grants_admin),
        "{}",
        description(&outcome)
    );
    let codes = ctx.db().pairing_codes().list().await.expect("codes");
    assert_eq!(codes.len(), 1);
    assert!(!codes[0].grants_admin);
}

#[tokio::test]
async fn pair_code_sans_guilde_renvoie_une_erreur_generique() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::PairCode(PairCodeInput {
                mode: PairingMode::Normal,
            }),
            admin_author(),
            None,
            Lang::Fr,
        ),
    )
    .await;

    assert!(outcome.reply.ephemeral);
    assert_eq!(description(&outcome), strings(Lang::Fr).command_error);
}

// ---------------------------------------------------------------------------
// /media-add
// ---------------------------------------------------------------------------

#[tokio::test]
async fn media_add_sans_source_repond_missing_media() {
    let mut ctx = FakeBotContext::new().await.expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::MediaAdd(MediaSubmission::default()),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert_eq!(outcome.deferral, Deferral::None);
    assert!(outcome.reply.ephemeral);
    assert_eq!(outcome.reply.embed.color, Some(COLOR_ERROR));
    assert_eq!(
        description(&outcome),
        strings(Lang::Fr).media_add_missing_media
    );
    assert!(ctx.media_board_mut().requests().is_empty());
}

#[tokio::test]
async fn media_add_ingere_et_ajoute_au_board() {
    let mut ctx = FakeBotContext::with_media_asset(sample_asset(MediaKind::Image))
        .await
        .expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    ctx.media_board_mut().set_created(true);
    let submission = MediaSubmission {
        url: Some("https://example.com/media.png".to_owned()),
        text: Some("Mon média".to_owned()),
        ..MediaSubmission::default()
    };
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::MediaAdd(submission),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert_eq!(outcome.deferral, Deferral::Ephemeral);
    assert!(outcome.reply.ephemeral);
    assert_eq!(outcome.reply.embed.color, Some(COLOR_SUCCESS));
    assert_eq!(
        description(&outcome),
        strings(Lang::Fr).media_add_answer_created
    );

    let requests = ctx.media_board_mut().requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].guild_id, guild());
    assert_eq!(requests[0].title.as_deref(), Some("Mon média"));
    assert_eq!(
        requests[0].created_by_discord_user_id.as_deref(),
        Some(MEMBER_ID)
    );
    assert_eq!(requests[0].created_by_name.as_deref(), Some("John"));
}

#[tokio::test]
async fn media_add_deja_present_repond_exists() {
    let mut ctx = FakeBotContext::with_media_asset(sample_asset(MediaKind::Image))
        .await
        .expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    ctx.media_board_mut().set_created(false);
    let submission = MediaSubmission {
        media: Some("https://cdn.discordapp.com/attachments/x.png".to_owned()),
        ..MediaSubmission::default()
    };
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::MediaAdd(submission),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert_eq!(
        description(&outcome),
        strings(Lang::Fr).media_add_answer_exists
    );
}

// ---------------------------------------------------------------------------
// /send et /hidesend
// ---------------------------------------------------------------------------

#[tokio::test]
async fn send_cree_un_job_public_avec_auteur_et_texte() {
    let mut ctx = FakeBotContext::with_media_asset(sample_asset(MediaKind::Video))
        .await
        .expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    let submission = MediaSubmission {
        url: Some("https://example.com/video.mp4".to_owned()),
        text: Some("salut".to_owned()),
        ..MediaSubmission::default()
    };
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::Send(submission),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert_eq!(outcome.deferral, Deferral::Public);
    assert!(!outcome.reply.ephemeral);
    assert_eq!(outcome.reply.embed.color, Some(COLOR_SUCCESS));
    assert_eq!(description(&outcome), strings(Lang::Fr).send_answer);

    let requests = ctx.playback_mut().requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.source, "discord_send_command");
    assert_eq!(request.author_name.as_deref(), Some("john_doe"));
    assert_eq!(
        request.author_image.as_deref(),
        Some("https://cdn.discordapp.com/avatars/1/abc.png?size=256")
    );
    assert_eq!(request.text.as_deref(), Some("salut"));
    assert!(request.show_text);
    assert_eq!(request.duration_sec, 30);
    assert_eq!(request.resume_offset_sec, 0);
    assert!(request.media_asset_id.is_some());
}

#[tokio::test]
async fn send_sans_contenu_repond_missing_content_sans_defer() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::Send(MediaSubmission::default()),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert_eq!(outcome.deferral, Deferral::None);
    assert!(outcome.reply.ephemeral);
    assert_eq!(outcome.reply.embed.color, Some(COLOR_ERROR));
    assert_eq!(
        description(&outcome),
        strings(Lang::Fr).send_missing_content
    );
}

#[tokio::test]
async fn hide_send_cree_un_job_ephemere_sans_auteur() {
    let mut ctx = FakeBotContext::with_media_asset(sample_asset(MediaKind::Video))
        .await
        .expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    let submission = MediaSubmission {
        media: Some("https://cdn.discordapp.com/attachments/x.mp4".to_owned()),
        ..MediaSubmission::default()
    };
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::HideSend(submission),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert_eq!(outcome.deferral, Deferral::Ephemeral);
    assert!(outcome.reply.ephemeral);
    assert_eq!(description(&outcome), strings(Lang::Fr).hide_send_answer);

    let requests = ctx.playback_mut().requests();
    assert_eq!(requests[0].source, "discord_hidesend_command");
    assert!(requests[0].author_name.is_none());
    assert!(requests[0].author_image.is_none());
}

#[tokio::test]
async fn send_erreur_de_lecture_est_localisee() {
    let mut ctx = FakeBotContext::with_media_asset(sample_asset(MediaKind::Video))
        .await
        .expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    ctx.playback_mut().set_enqueue_failure(true);
    let submission = MediaSubmission {
        url: Some("https://example.com/video.mp4".to_owned()),
        ..MediaSubmission::default()
    };
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::Send(submission),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    // Deferral public : l'édition d'erreur reste publique, comme le legacy.
    assert!(!outcome.reply.ephemeral);
    assert_eq!(outcome.reply.embed.color, Some(COLOR_ERROR));
    assert_eq!(
        description(&outcome),
        MediaErrorCode::DownloadFailed.message(Lang::Fr, ctx.config().media)
    );
}

// ---------------------------------------------------------------------------
// /dire et /cdire
// ---------------------------------------------------------------------------

#[tokio::test]
async fn talk_synthetise_et_cree_un_job_public_avec_auteur() {
    let mut ctx = FakeBotContext::with_media_asset(sample_asset(MediaKind::Audio))
        .await
        .expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::Talk(TalkInput {
                text: Some("bonjour".to_owned()),
                voice: "salut tout le monde".to_owned(),
            }),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert_eq!(outcome.deferral, Deferral::Public);
    assert!(!outcome.reply.ephemeral);
    assert_eq!(description(&outcome), strings(Lang::Fr).talk_answer);

    let speech = ctx.media_mut().speech_requests();
    assert_eq!(speech.len(), 1);
    assert_eq!(speech[0], (Lang::Fr, "salut tout le monde".to_owned()));

    let requests = ctx.playback_mut().requests();
    assert_eq!(requests[0].source, "discord_talk_command");
    assert_eq!(requests[0].author_name.as_deref(), Some("john_doe"));
    assert_eq!(requests[0].text.as_deref(), Some("bonjour"));
    assert!(requests[0].show_text);
    assert_eq!(requests[0].duration_sec, 30);
}

#[tokio::test]
async fn hide_talk_synthetise_et_cree_un_job_ephemere_sans_auteur() {
    let mut ctx = FakeBotContext::with_media_asset(sample_asset(MediaKind::Audio))
        .await
        .expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::HideTalk(TalkInput {
                text: None,
                voice: "bonjour".to_owned(),
            }),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert_eq!(outcome.deferral, Deferral::Ephemeral);
    assert!(outcome.reply.ephemeral);
    assert_eq!(description(&outcome), strings(Lang::Fr).hide_talk_answer);

    let requests = ctx.playback_mut().requests();
    assert_eq!(requests[0].source, "discord_hidetalk_command");
    assert!(requests[0].author_name.is_none());
    assert!(!requests[0].show_text);
}

#[tokio::test]
async fn talk_echec_tts_repond_voice_error() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::Talk(TalkInput {
                text: None,
                voice: "bonjour".to_owned(),
            }),
            member_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert_eq!(outcome.deferral, Deferral::Public);
    // Deferral public : l'édition d'erreur reste publique, comme le legacy.
    assert!(!outcome.reply.ephemeral);
    assert_eq!(outcome.reply.embed.color, Some(COLOR_ERROR));
    assert_eq!(description(&outcome), strings(Lang::Fr).talk_voice_error);
}

// ---------------------------------------------------------------------------
// /overlays
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overlays_vide_repond_un_avertissement() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::Overlays,
            admin_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert!(outcome.reply.ephemeral);
    assert_eq!(outcome.reply.embed.color, Some(COLOR_WARNING));
    assert_eq!(title(&outcome).as_deref(), Some("Overlays connectés (0)"));
    assert_eq!(
        description(&outcome),
        strings(Lang::Fr).overlays_answer_empty
    );
}

#[tokio::test]
async fn overlays_liste_trie_par_label() {
    let mut ctx = FakeBotContext::new().await.expect("contexte");
    ctx.overlays_mut().set_overlays(vec![
        ConnectedOverlay {
            client_id: "z".to_owned(),
            label: "Zeta".to_owned(),
        },
        ConnectedOverlay {
            client_id: "a".to_owned(),
            label: "alpha".to_owned(),
        },
    ]);
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::Overlays,
            admin_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert_eq!(outcome.reply.embed.color, Some(COLOR_SUCCESS));
    assert_eq!(title(&outcome).as_deref(), Some("Overlays connectés (2)"));
    let description = description(&outcome);
    let alpha = description.find("• `alpha` (`a`)").expect("alpha");
    let zeta = description.find("• `Zeta` (`z`)").expect("zeta");
    assert!(alpha < zeta);
}

#[tokio::test]
async fn overlays_sans_guilde_repond_not_allowed() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(CommandInput::Overlays, member_author(), None, Lang::Fr),
    )
    .await;

    assert!(outcome.reply.ephemeral);
    assert_eq!(
        title(&outcome).as_deref(),
        Some(strings(Lang::Fr).not_allowed)
    );
    assert_eq!(description(&outcome), strings(Lang::Fr).admin_only);
}

#[tokio::test]
async fn overlays_erreur_de_l_annuaire_repond_erreur_generique() {
    let mut ctx = FakeBotContext::new().await.expect("contexte");
    ctx.overlays_mut().set_failure(true);
    let outcome = dispatch(
        &ctx,
        invocation(
            CommandInput::Overlays,
            admin_author(),
            Some(guild()),
            Lang::Fr,
        ),
    )
    .await;

    assert!(outcome.reply.ephemeral);
    assert_eq!(description(&outcome), strings(Lang::Fr).command_error);
}

// ---------------------------------------------------------------------------
// /stop
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stop_arrete_la_guilde_et_repond_en_ephemere() {
    let mut ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(CommandInput::Stop, admin_author(), Some(guild()), Lang::Fr),
    )
    .await;

    assert_eq!(outcome.deferral, Deferral::Ephemeral);
    assert!(outcome.reply.ephemeral);
    assert_eq!(outcome.reply.embed.color, Some(COLOR_SUCCESS));
    assert_eq!(description(&outcome), strings(Lang::Fr).stop_answer);
    assert_eq!(ctx.playback_mut().stop_calls(), vec![guild()]);
}

#[tokio::test]
async fn stop_sans_guilde_est_refuse() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let outcome = dispatch(
        &ctx,
        invocation(CommandInput::Stop, member_author(), None, Lang::Fr),
    )
    .await;

    assert!(outcome.reply.ephemeral);
    assert_eq!(description(&outcome), strings(Lang::Fr).admin_only);
}

// ---------------------------------------------------------------------------
// Rights
// ---------------------------------------------------------------------------

fn send_text() -> CommandInput {
    CommandInput::Send(MediaSubmission {
        text: Some("salut".to_owned()),
        ..MediaSubmission::default()
    })
}

#[tokio::test]
async fn an_unlinked_account_cannot_send_media() {
    let mut ctx = FakeBotContext::new().await.expect("contexte");
    for input in [
        send_text(),
        CommandInput::Talk(TalkInput {
            voice: "bonjour".to_owned(),
            text: None,
        }),
        CommandInput::MediaAdd(MediaSubmission {
            url: Some("https://example.com/a.png".to_owned()),
            ..MediaSubmission::default()
        }),
    ] {
        // Even a Discord administrator needs a linked LiveChat member.
        let outcome = dispatch(
            &ctx,
            invocation(input, admin_author(), Some(guild()), Lang::Fr),
        )
        .await;
        assert_eq!(outcome.deferral, Deferral::None);
        assert!(outcome.reply.ephemeral);
        assert_eq!(description(&outcome), strings(Lang::Fr).not_linked);
    }
    assert!(ctx.playback_mut().requests().is_empty());
    assert!(ctx.media_board_mut().requests().is_empty());
}

#[tokio::test]
async fn a_linked_guest_cannot_send_media() {
    let mut ctx = FakeBotContext::new().await.expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Guest)
        .await
        .expect("invité lié");
    let outcome = dispatch(
        &ctx,
        invocation(send_text(), member_author(), Some(guild()), Lang::Fr),
    )
    .await;
    assert!(outcome.reply.ephemeral);
    assert_eq!(description(&outcome), strings(Lang::Fr).guest_not_allowed);
    assert!(ctx.playback_mut().requests().is_empty());
}

#[tokio::test]
async fn a_linked_member_sends_media() {
    let mut ctx = FakeBotContext::new().await.expect("contexte");
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Member)
        .await
        .expect("membre lié");
    let outcome = dispatch(
        &ctx,
        invocation(send_text(), member_author(), Some(guild()), Lang::Fr),
    )
    .await;
    assert_eq!(outcome.reply.embed.color, Some(COLOR_SUCCESS));
    assert_eq!(ctx.playback_mut().requests().len(), 1);
}

#[tokio::test]
async fn stop_needs_an_admin() {
    let mut ctx = FakeBotContext::new().await.expect("contexte");
    let refused = dispatch(
        &ctx,
        invocation(CommandInput::Stop, member_author(), Some(guild()), Lang::Fr),
    )
    .await;
    assert_eq!(description(&refused), strings(Lang::Fr).admin_only);
    assert!(ctx.playback_mut().stop_calls().is_empty());

    // A LiveChat admin linked to the account may stop without the Discord
    // Administrator permission.
    ctx.link_author(&guild(), &member_author(), lc_domain::GuildRole::Admin)
        .await
        .expect("admin lié");
    let allowed = dispatch(
        &ctx,
        invocation(CommandInput::Stop, member_author(), Some(guild()), Lang::Fr),
    )
    .await;
    assert_eq!(description(&allowed), strings(Lang::Fr).stop_answer);
    assert_eq!(ctx.playback_mut().stop_calls(), vec![guild()]);
}

#[tokio::test]
async fn stop_echec_du_moteur_repond_erreur_generique() {
    let mut ctx = FakeBotContext::new().await.expect("contexte");
    ctx.playback_mut().set_stop_failure(true);
    let outcome = dispatch(
        &ctx,
        invocation(CommandInput::Stop, admin_author(), Some(guild()), Lang::Fr),
    )
    .await;

    assert!(outcome.reply.ephemeral);
    assert_eq!(description(&outcome), strings(Lang::Fr).command_error);
}

// ---------------------------------------------------------------------------
// Linked member profile
// ---------------------------------------------------------------------------

fn linked_member(discord_user_id: &str, profile_custom: bool) -> lc_domain::GuildMember {
    lc_domain::GuildMember {
        id: lc_domain::GuildMemberId::new_v4(),
        guild_id: guild(),
        display_name: "Ancien pseudo".to_owned(),
        avatar: None,
        role: lc_domain::GuildRole::Member,
        created_at: sample_now(),
        removed_at: None,
        discord_user_id: Some(discord_user_id.to_owned()),
        profile_custom,
        discord_synced_at: None,
    }
}

#[tokio::test]
async fn any_command_refreshes_the_linked_member_profile() {
    let ctx = FakeBotContext::new().await.expect("contexte");
    let follows = linked_member(MEMBER_ID, false);
    let custom = linked_member(ADMIN_ID, true);
    ctx.db().guild_members().create(&follows).await.unwrap();
    ctx.db().guild_members().create(&custom).await.unwrap();

    for author in [member_author(), admin_author()] {
        dispatch(
            &ctx,
            invocation(CommandInput::Alive, author, Some(guild()), Lang::Fr),
        )
        .await;
    }

    let stored = ctx
        .db()
        .guild_members()
        .get(follows.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.display_name, "John");
    assert_eq!(
        stored.avatar.as_deref(),
        Some("https://cdn.discordapp.com/avatars/1/abc.png?size=256")
    );
    assert_eq!(stored.discord_synced_at, Some(sample_now()));
    // A member who chose their own profile keeps it.
    let stored = ctx
        .db()
        .guild_members()
        .get(custom.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.display_name, "Ancien pseudo");
}
