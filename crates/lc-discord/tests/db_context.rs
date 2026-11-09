//! Tests d'intégration du contexte de production [`DbBotContext`] : base
//! `lc-db` réelle en mémoire, horloge figée et backends de test injectés.
//!
//! Ces tests vérifient le câblage (injection, persistance SQLite, horloge)
//! sans dépendre d'implémentations réseau encore absentes (`lc-media`,
//! `lc-playback`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use chrono::Duration as ChronoDuration;
use lc_db::{Db, DbConfig};
use lc_discord::context::{BotConfig, BotContext, DbBotContext};
use lc_discord::handlers::dispatch;
use lc_discord::i18n::{strings, Lang};
use lc_discord::inputs::{
    CommandInput, DisplayFullInput, Invocation, PairCodeInput, TimeSettingInput,
};
use lc_discord::testing::{
    admin_author, member_author, sample_now, FakeMediaBackend, FakeMemeBackend,
    FakeOverlayDirectory, FakePlaybackBackend,
};
use lc_domain::{FixedClock, GuildId, PairingMode};

type Context = DbBotContext<FixedClock>;

struct Harness {
    context: Context,
    media: Arc<FakeMediaBackend>,
    playback: Arc<FakePlaybackBackend>,
    meme: Arc<FakeMemeBackend>,
    overlays: Arc<FakeOverlayDirectory>,
}

async fn harness() -> Harness {
    let db = Db::open_in_memory(&DbConfig::default())
        .await
        .expect("base mémoire");
    let media = Arc::new(FakeMediaBackend::new());
    let playback = Arc::new(FakePlaybackBackend::new());
    let meme = Arc::new(FakeMemeBackend::new());
    let overlays = Arc::new(FakeOverlayDirectory::new());
    let clock = FixedClock::new(sample_now());
    let context = DbBotContext::with_clock(
        db,
        clock,
        BotConfig::new("http://localhost:3333"),
        Arc::clone(&media) as Arc<dyn lc_discord::context::MediaBackend>,
        Arc::clone(&playback) as Arc<dyn lc_discord::context::PlaybackBackend>,
        Arc::clone(&meme) as Arc<dyn lc_discord::context::MemeBackend>,
        Arc::clone(&overlays) as Arc<dyn lc_discord::context::OverlayDirectory>,
    );
    Harness {
        context,
        media,
        playback,
        meme,
        overlays,
    }
}

fn guild() -> GuildId {
    GuildId::parse("123456789012345678").expect("identifiant de guilde valide")
}

fn invocation(input: CommandInput, admin: bool) -> Invocation {
    Invocation::new(
        Some(guild()),
        Lang::Fr,
        if admin {
            admin_author()
        } else {
            member_author()
        },
        input,
    )
}

#[tokio::test]
async fn db_context_persiste_un_code_d_appairage_consommable() {
    let harness = harness().await;
    let outcome = dispatch(
        &harness.context,
        invocation(
            CommandInput::PairCode(PairCodeInput {
                mode: PairingMode::Normal,
            }),
            true,
        ),
    )
    .await;
    assert!(outcome.reply.ephemeral);
    let description = outcome.reply.embed.description_text().unwrap_or_default();
    assert!(
        description.contains("http://localhost:3333"),
        "{description}"
    );
    assert!(
        description.contains("Expire dans 5 minute(s)."),
        "{description}"
    );

    let codes = harness
        .context
        .db()
        .pairing_codes()
        .list()
        .await
        .expect("codes");
    assert_eq!(codes.len(), 1);
    let code = codes[0].code.clone();
    let entry = harness
        .context
        .db()
        .pairing_codes()
        .get(&code)
        .await
        .expect("lecture")
        .expect("code présent");
    assert_eq!(entry.guild_id, guild());
    assert_eq!(entry.mode, PairingMode::Normal);
    assert_eq!(
        entry.expires_at,
        Some(sample_now() + ChronoDuration::minutes(5))
    );
    assert!(entry.used_at.is_none());

    // La consommation (handler d'appairage de l'API HTTP) marque le code.
    let consumed = harness
        .context
        .db()
        .pairing_codes()
        .consume(&code, sample_now())
        .await
        .expect("consommation");
    assert!(consumed.is_some());
    let used = harness
        .context
        .db()
        .pairing_codes()
        .get(&code)
        .await
        .expect("lecture")
        .expect("code présent");
    assert_eq!(used.used_at, Some(sample_now()));
    let again = harness
        .context
        .db()
        .pairing_codes()
        .consume(&code, sample_now())
        .await
        .expect("seconde consommation");
    assert!(again.is_none());
}

#[tokio::test]
async fn db_context_persiste_les_reglages_de_guilde() {
    let harness = harness().await;
    drop(
        dispatch(
            &harness.context,
            invocation(
                CommandInput::SetDefaultTime(TimeSettingInput { seconds: 12 }),
                true,
            ),
        )
        .await,
    );
    drop(
        dispatch(
            &harness.context,
            invocation(
                CommandInput::SetMaxTime(TimeSettingInput { seconds: 90 }),
                true,
            ),
        )
        .await,
    );
    let outcome = dispatch(
        &harness.context,
        invocation(
            CommandInput::SetDisplayFull(DisplayFullInput { value: true }),
            true,
        ),
    )
    .await;
    assert_eq!(
        outcome.reply.embed.description_text(),
        Some(strings(Lang::Fr).set_display_full_answer)
    );

    let row = harness
        .context
        .db()
        .guilds()
        .get(&guild())
        .await
        .expect("guilde")
        .expect("ligne créée");
    assert_eq!(row.default_media_time, Some(12));
    assert_eq!(row.max_media_time, Some(90));
    assert!(row.display_media_full);
}

#[tokio::test]
async fn db_context_refuse_les_reglages_sans_admin() {
    let harness = harness().await;
    let outcome = dispatch(
        &harness.context,
        invocation(
            CommandInput::SetDefaultTime(TimeSettingInput { seconds: 30 }),
            false,
        ),
    )
    .await;
    assert!(outcome.reply.ephemeral);
    assert_eq!(
        outcome.reply.embed.title_text(),
        Some(strings(Lang::Fr).not_allowed)
    );
    assert!(harness
        .context
        .db()
        .guilds()
        .get(&guild())
        .await
        .expect("lecture")
        .is_none());
    assert!(harness.media.requests().is_empty());
    assert!(harness.meme.requests().is_empty());
    assert!(harness.overlays.queried_guilds().is_empty());
}

#[tokio::test]
async fn db_context_delegue_le_stop_au_moteur_de_lecture() {
    let harness = harness().await;
    let outcome = dispatch(&harness.context, invocation(CommandInput::Stop, false)).await;
    assert!(outcome.reply.ephemeral);
    assert_eq!(harness.playback.stop_calls(), vec![guild()]);
}
