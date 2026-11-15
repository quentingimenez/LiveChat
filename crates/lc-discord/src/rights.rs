//! Who may run which command.
//!
//! | Commands | Who |
//! |---|---|
//! | `/msg`, `/cmsg`, `/dire`, `/cdire` | a Discord account linked to a LiveChat member (not a guest) of the guild |
//! | `/media-add` | same, with the right to add media |
//! | `/stop`, `/overlays` | a Discord administrator, or a linked LiveChat admin |
//! | everything else | anyone (`/lier` checks the Discord administrator itself) |
//!
//! The check runs before Discord is acknowledged, so a refusal is always a
//! private (ephemeral) answer.

use lc_domain::{GuildId, GuildRole, Permission};

use crate::catalog::CommandKey;
use crate::context::{Author, BotContext};
use crate::error::BotError;

/// What a command asks of its author.
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
    /// The linked member's role lacks the permission (a guest).
    RoleTooLow,
    /// The command is for administrators.
    AdminOnly,
}

/// Requirement of each command.
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

/// Checks `requirement` for `author` in the (already resolved) guild.
pub async fn check(
    ctx: &dyn BotContext,
    guild_id: Option<&GuildId>,
    author: &Author,
    requirement: Requirement,
) -> Result<Result<(), Refusal>, BotError> {
    if requirement == Requirement::Anyone {
        return Ok(Ok(()));
    }
    let Some(guild_id) = guild_id else {
        // A direct message has no guild: nothing to act on.
        return Ok(Err(match requirement {
            Requirement::Admin => Refusal::AdminOnly,
            _ => Refusal::NotLinked,
        }));
    };
    if requirement == Requirement::Admin && author.is_admin {
        return Ok(Ok(()));
    }
    let member = if author.id.is_empty() {
        None
    } else {
        ctx.db()
            .guild_members()
            .find_active_by_discord_user(guild_id, &author.id)
            .await?
    };
    Ok(match (requirement, member) {
        (Requirement::Admin, Some(member)) if member.role == GuildRole::Admin => Ok(()),
        (Requirement::Admin, _) => Err(Refusal::AdminOnly),
        (Requirement::LinkedMember(_), None) => Err(Refusal::NotLinked),
        (Requirement::LinkedMember(permission), Some(member)) => {
            if member.role.can(permission) {
                Ok(())
            } else {
                Err(Refusal::RoleTooLow)
            }
        }
        (Requirement::Anyone, _) => Ok(()),
    })
}

#[cfg(test)]
mod tests {
    use super::{requirement, Requirement};
    use crate::catalog::{CommandKey, COMMANDS};

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
}
