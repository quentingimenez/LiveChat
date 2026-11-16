//! Who may run which command.
//!
//! The historical rule was hard-coded (`Requirement` / [`requirement`]); it is
//! still the reference for the **defaults** of the `discord` surface. The
//! effective decision now comes from the configurable [`RightsTable`]:
//!
//! | Commands | Action | Default |
//! |---|---|---|
//! | `/msg`, `/cmsg` | `msg` | linked member or admin |
//! | `/dire`, `/cdire` | `tts` | linked member or admin |
//! | `/media-add` | `mediaAdd` | linked member or admin |
//! | `/stop` | `stop` | admin |
//! | `/overlays` | `overlays` | admin |
//! | `/pair-code`, `/alive`, `/help`, `/info` | `pairCode` / `info` | everyone |
//! | `/lier` | — | Discord administrator (not configurable) |
//!
//! An unlinked author maps to the `none` role. `discordAdminIsAdmin` (default
//! `true`) keeps the historical `author.is_admin` shortcut: a Discord server
//! administrator counts as the `admin` role. For commands that historically
//! required a **linked member**, the shortcut still does not replace the link
//! (the member role is upgraded instead): a Discord administrator without a
//! linked LiveChat account still cannot send media, exactly as before.
//!
//! The check runs before Discord is acknowledged, so a refusal is always a
//! private (ephemeral) answer.

use lc_domain::rights::{Action, RightsRole, RightsTable, Surface};
use lc_domain::{GuildId, Permission};

use crate::catalog::CommandKey;
use crate::context::{Author, BotContext};
use crate::error::BotError;

/// What a command asks of its author (the legacy hard-coded table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// No condition.
    Anyone,
    /// A linked LiveChat member whose role grants the permission.
    LinkedMember(Permission),
    /// A Discord administrator or a linked LiveChat admin.
    Admin,
}

/// Why a command was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The Discord account is linked to no member of the guild.
    NotLinked,
    /// The linked member's role lacks the right (a guest).
    RoleTooLow,
    /// The command is for administrators.
    AdminOnly,
}

/// Historical requirement of each command.
///
/// Kept as the documented reference of the `discord` defaults (section 5 of
/// the admin contract), even though [`check`] now consults the rights table.
pub const fn requirement(command: CommandKey) -> Requirement {
    match command {
        CommandKey::Send | CommandKey::HideSend | CommandKey::Talk | CommandKey::HideTalk => {
            Requirement::LinkedMember(Permission::TriggerPlayback)
        }
        CommandKey::MediaAdd => Requirement::LinkedMember(Permission::AddMedia),
        CommandKey::Stop | CommandKey::Overlays => Requirement::Admin,
        CommandKey::Alive
        | CommandKey::PairCode
        | CommandKey::Help
        | CommandKey::Info
        | CommandKey::Link => Requirement::Anyone,
    }
}

/// Configurable action of a command; `None` for `/lier`, which stays reserved
/// to Discord administrators and is not configurable.
pub const fn command_action(command: CommandKey) -> Option<Action> {
    match command {
        CommandKey::Send | CommandKey::HideSend => Some(Action::Msg),
        CommandKey::Talk | CommandKey::HideTalk => Some(Action::Tts),
        CommandKey::MediaAdd => Some(Action::MediaAdd),
        CommandKey::Stop => Some(Action::Stop),
        CommandKey::Overlays => Some(Action::Overlays),
        CommandKey::PairCode => Some(Action::PairCode),
        CommandKey::Alive | CommandKey::Help | CommandKey::Info => Some(Action::Info),
        CommandKey::Link => None,
    }
}

/// Provider of the live configurable rights table.
///
/// The server implements it over its `RightsStore`; `lc-discord` tests use a
/// plain [`RightsTable`] (which implements this trait).
pub trait RightsSource: Send + Sync {
    /// Current table (a snapshot; cheap to clone).
    fn table(&self) -> RightsTable;
}

impl RightsSource for RightsTable {
    fn table(&self) -> RightsTable {
        self.clone()
    }
}

/// Checks the configurable right of `command` for `author`.
///
/// `table` is the current rights table. Returns `Ok(Ok(()))` when the command is
/// allowed and `Ok(Err(refusal))` when it is refused.
pub async fn check(
    ctx: &dyn BotContext,
    guild_id: Option<&GuildId>,
    author: &Author,
    table: &RightsTable,
    command: CommandKey,
) -> Result<Result<(), Refusal>, BotError> {
    let Some(action) = command_action(command) else {
        // `/lier` checks the Discord administrator itself in its handler.
        return Ok(Ok(()));
    };

    let admin_only = matches!(action, Action::Stop | Action::Overlays);
    // A direct message has no guild: there is no member to link, so the author
    // maps to `none` (which is enough for `info` and `pairCode` by default).
    let member = match guild_id {
        Some(guild_id) if !author.id.is_empty() => {
            ctx.db()
                .guild_members()
                .find_active_by_discord_user(guild_id, &author.id)
                .await?
        }
        _ => None,
    };

    let mut role = match &member {
        Some(member) => RightsRole::from(member.role),
        None => RightsRole::None,
    };
    // Today's `author.is_admin` shortcut applied to `Requirement::Admin`
    // commands (and harmlessly to the `Anyone` ones). It must not replace the
    // linked member for `msg`/`tts`/`mediaAdd`: the historical behaviour kept
    // requiring a linked member there. A linked member is simply upgraded to
    // `admin`, which is what `discordAdminIsAdmin` enables.
    let shortcut = author.is_admin
        && table.discord_admin_is_admin()
        && (member.is_some() || !matches!(action, Action::Msg | Action::Tts | Action::MediaAdd));
    if shortcut {
        role = RightsRole::Admin;
    }

    if table.allowed(Surface::Discord, action, role) {
        return Ok(Ok(()));
    }
    Ok(Err(match (admin_only, member) {
        (true, _) => Refusal::AdminOnly,
        (false, None) => Refusal::NotLinked,
        (false, Some(_)) => Refusal::RoleTooLow,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use lc_domain::rights::{Action, RightsRole, RightsTable, Surface};
    use lc_domain::GuildRole;

    use super::{check, command_action, requirement, Refusal, Requirement};
    use crate::catalog::{CommandKey, COMMANDS};
    use crate::testing::{admin_author, member_author, FakeBotContext};
    use lc_domain::GuildId;

    fn guild() -> GuildId {
        GuildId::parse("123456789012345678").unwrap()
    }

    #[test]
    fn sending_and_moderation_commands_are_gated() {
        for command in COMMANDS {
            let expected = matches!(
                command.key,
                CommandKey::Send
                    | CommandKey::HideSend
                    | CommandKey::Talk
                    | CommandKey::HideTalk
                    | CommandKey::MediaAdd
                    | CommandKey::Stop
                    | CommandKey::Overlays
            );
            assert_eq!(
                requirement(command.key) != Requirement::Anyone,
                expected,
                "{}",
                command.key.id()
            );
        }
        assert_eq!(requirement(CommandKey::Stop), Requirement::Admin);
        assert_eq!(requirement(CommandKey::Overlays), Requirement::Admin);
    }

    #[test]
    fn command_actions_match_the_contract() {
        for command in COMMANDS {
            match command.key {
                CommandKey::Link => assert_eq!(command_action(command.key), None),
                other => assert!(
                    command_action(other).is_some(),
                    "{} must map to an action",
                    other.id()
                ),
            }
        }
        assert_eq!(command_action(CommandKey::Send), Some(Action::Msg));
        assert_eq!(command_action(CommandKey::HideSend), Some(Action::Msg));
        assert_eq!(command_action(CommandKey::Talk), Some(Action::Tts));
        assert_eq!(command_action(CommandKey::HideTalk), Some(Action::Tts));
        assert_eq!(command_action(CommandKey::MediaAdd), Some(Action::MediaAdd));
        assert_eq!(command_action(CommandKey::Stop), Some(Action::Stop));
        assert_eq!(command_action(CommandKey::Overlays), Some(Action::Overlays));
        assert_eq!(command_action(CommandKey::PairCode), Some(Action::PairCode));
        assert_eq!(command_action(CommandKey::Alive), Some(Action::Info));
        assert_eq!(command_action(CommandKey::Help), Some(Action::Info));
        assert_eq!(command_action(CommandKey::Info), Some(Action::Info));
    }

    #[test]
    fn defaults_match_the_legacy_requirement_for_every_command() {
        let table = RightsTable::new();
        for command in COMMANDS {
            let Some(action) = command_action(command.key) else {
                continue;
            };
            for role in Surface::Discord.columns() {
                let linked =
                    role.guild_role()
                        .is_some_and(|guild_role| match requirement(command.key) {
                            Requirement::LinkedMember(permission) => guild_role.can(permission),
                            _ => false,
                        });
                let expected = match requirement(command.key) {
                    Requirement::Anyone => true,
                    Requirement::Admin => matches!(*role, RightsRole::Admin),
                    Requirement::LinkedMember(_) => linked,
                };
                assert_eq!(
                    table.allowed(Surface::Discord, action, *role),
                    expected,
                    "{} for {}",
                    command.key.id(),
                    role.id()
                );
            }
        }
    }

    #[tokio::test]
    async fn none_guest_member_and_admin_are_mapped_from_the_linked_member() {
        let ctx = FakeBotContext::new().await.unwrap();
        let table = ctx.rights_table();
        // Unlinked author = `none`.
        assert_eq!(
            check(
                &ctx,
                Some(&guild()),
                &admin_author(),
                &table,
                CommandKey::MediaAdd
            )
            .await
            .unwrap(),
            Err(Refusal::NotLinked)
        );

        // Guest linked to a non-admin account.
        ctx.link_author(&guild(), &member_author(), GuildRole::Guest)
            .await
            .unwrap();
        let table = ctx.rights_table();
        assert_eq!(
            check(
                &ctx,
                Some(&guild()),
                &member_author(),
                &table,
                CommandKey::Send
            )
            .await
            .unwrap(),
            Err(Refusal::RoleTooLow)
        );

        // Member can send, admin can stop.
        let member_ctx = FakeBotContext::new().await.unwrap();
        member_ctx
            .link_author(&guild(), &member_author(), GuildRole::Member)
            .await
            .unwrap();
        let table = member_ctx.rights_table();
        assert_eq!(
            check(
                &member_ctx,
                Some(&guild()),
                &member_author(),
                &table,
                CommandKey::Send
            )
            .await
            .unwrap(),
            Ok(())
        );
        assert_eq!(
            check(
                &member_ctx,
                Some(&guild()),
                &member_author(),
                &table,
                CommandKey::Stop
            )
            .await
            .unwrap(),
            Err(Refusal::AdminOnly)
        );

        let admin_ctx = FakeBotContext::new().await.unwrap();
        admin_ctx
            .link_author(&guild(), &member_author(), GuildRole::Admin)
            .await
            .unwrap();
        let table = admin_ctx.rights_table();
        assert_eq!(
            check(
                &admin_ctx,
                Some(&guild()),
                &member_author(),
                &table,
                CommandKey::Stop
            )
            .await
            .unwrap(),
            Ok(())
        );
    }

    #[tokio::test]
    async fn unlinked_admin_needs_the_discord_admin_is_admin_flag_for_stop() {
        let mut ctx = FakeBotContext::new().await.unwrap();
        let mut table = ctx.rights_table();
        assert_eq!(
            check(
                &ctx,
                Some(&guild()),
                &admin_author(),
                &table,
                CommandKey::Stop
            )
            .await
            .unwrap(),
            Ok(())
        );

        table.set_discord_admin_is_admin(false);
        ctx.set_rights(table);
        let table = ctx.rights_table();
        assert_eq!(
            check(
                &ctx,
                Some(&guild()),
                &admin_author(),
                &table,
                CommandKey::Stop
            )
            .await
            .unwrap(),
            Err(Refusal::AdminOnly)
        );
    }

    #[tokio::test]
    async fn an_unlinked_discord_admin_still_cannot_send_media() {
        let ctx = FakeBotContext::new().await.unwrap();
        let table = ctx.rights_table();
        for command in [
            CommandKey::Send,
            CommandKey::HideSend,
            CommandKey::Talk,
            CommandKey::HideTalk,
            CommandKey::MediaAdd,
        ] {
            assert_eq!(
                check(&ctx, Some(&guild()), &admin_author(), &table, command)
                    .await
                    .unwrap(),
                Err(Refusal::NotLinked)
            );
        }
        assert!(ctx.rights_table().discord_admin_is_admin());
    }

    #[tokio::test]
    async fn overrides_change_the_effective_decision() {
        let mut ctx = FakeBotContext::new().await.unwrap();
        let mut table = ctx.rights_table();
        table.set_override(Surface::Discord, Action::Msg, RightsRole::Guest, true);
        ctx.set_rights(table);
        ctx.link_author(&guild(), &member_author(), GuildRole::Guest)
            .await
            .unwrap();

        let table = ctx.rights_table();
        assert_eq!(
            check(
                &ctx,
                Some(&guild()),
                &member_author(),
                &table,
                CommandKey::Send
            )
            .await
            .unwrap(),
            Ok(())
        );
    }

    #[tokio::test]
    async fn lier_is_never_checked_here() {
        let ctx = FakeBotContext::new().await.unwrap();
        let table = ctx.rights_table();
        assert_eq!(
            check(
                &ctx,
                Some(&guild()),
                &member_author(),
                &table,
                CommandKey::Link
            )
            .await
            .unwrap(),
            Ok(())
        );
    }
}
