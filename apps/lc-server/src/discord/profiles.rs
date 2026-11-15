//! Name and avatar of a member linked to their Discord account.
//!
//! A member who paired with their own `/pair-code` follows their Discord
//! account (`guild_members.discord_user_id`) until they choose their own
//! profile. Every Discord command they type already refreshes it (see
//! `lc_discord::handlers`); this module covers members who never type one:
//! their profile is read from Discord when one of their overlays connects, at
//! most every [`REFRESH_INTERVAL`], and right away when they switch back to
//! Discord from « Mon profil ».
//!
//! The REST call sits behind [`DiscordProfileSource`] so the handlers stay
//! testable without network:
//!
//! - [`DisabledProfiles`] is the default (no Discord token);
//! - [`SerenityProfiles`] calls `GET /guilds/{guild}/members/{user}`, which
//!   needs no privileged gateway intent.

#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use lc_db::Db;
use lc_discord::identity;
use lc_domain::{GuildMember, OverlayClientId};
use serenity::http::Http;
use serenity::model::id::{GuildId, UserId};

use crate::app::AppState;

/// Shortest delay between two Discord reads for the same member on overlay
/// connections. Commands typed in Discord refresh the profile anyway.
pub const REFRESH_INTERVAL: Duration = Duration::hours(6);

/// Profile of an account in a Discord server, as Discord shows it there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscordProfile {
    /// Server nickname, else global name, else username.
    pub name: String,
    /// Server avatar, else account avatar.
    pub image: Option<String>,
}

/// Failure to read a profile from Discord.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// No Discord token is configured.
    #[error("Discord n'est pas configuré (DISCORD_TOKEN absent)")]
    Disabled,
    /// An identifier is not a numeric snowflake.
    #[error("identifiant Discord invalide")]
    InvalidId,
    /// The Discord API call failed.
    #[error("Discord : {0}")]
    Api(String),
}

/// Reads member profiles from Discord.
#[async_trait]
pub trait DiscordProfileSource: Send + Sync + std::fmt::Debug {
    /// Profile of `user_id` in the Discord server `discord_guild_id`;
    /// `Ok(None)` when the account is not (or no longer) in that server.
    async fn member_profile(
        &self,
        discord_guild_id: &str,
        user_id: &str,
    ) -> Result<Option<DiscordProfile>, ProfileError>;
}

/// Without Discord: every read is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisabledProfiles;

#[async_trait]
impl DiscordProfileSource for DisabledProfiles {
    async fn member_profile(
        &self,
        _discord_guild_id: &str,
        _user_id: &str,
    ) -> Result<Option<DiscordProfile>, ProfileError> {
        Err(ProfileError::Disabled)
    }
}

/// Reads over the serenity HTTP client.
#[derive(Debug, Clone)]
pub struct SerenityProfiles {
    http: Arc<Http>,
}

impl SerenityProfiles {
    /// HTTP client authenticated with the bot token.
    #[must_use]
    pub fn new(token: &str) -> Self {
        Self {
            http: Arc::new(Http::new(token)),
        }
    }
}

#[async_trait]
impl DiscordProfileSource for SerenityProfiles {
    async fn member_profile(
        &self,
        discord_guild_id: &str,
        user_id: &str,
    ) -> Result<Option<DiscordProfile>, ProfileError> {
        let guild = snowflake(discord_guild_id)?;
        let user = snowflake(user_id)?;
        match self
            .http
            .get_member(GuildId::new(guild), UserId::new(user))
            .await
        {
            Ok(member) => Ok(Some(DiscordProfile {
                name: identity::display_name(
                    member.nick.as_deref(),
                    member.user.global_name.as_deref(),
                    &member.user.name,
                ),
                image: identity::avatar_url(
                    guild,
                    user,
                    member.avatar.as_ref().map(ToString::to_string).as_deref(),
                    member
                        .user
                        .avatar
                        .as_ref()
                        .map(ToString::to_string)
                        .as_deref(),
                ),
            })),
            Err(error) if is_unknown_member(&error) => Ok(None),
            Err(error) => Err(ProfileError::Api(error.to_string())),
        }
    }
}

fn snowflake(value: &str) -> Result<u64, ProfileError> {
    value
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or(ProfileError::InvalidId)
}

/// `true` when Discord reports that the account (`10007 Unknown Member`) or
/// the server (`10004 Unknown Guild`) is out of reach.
fn is_unknown_member(error: &serenity::Error) -> bool {
    match error {
        serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(response)) => {
            response.status_code.as_u16() == 404 || matches!(response.error.code, 10004 | 10007)
        }
        _ => false,
    }
}

/// What a refresh did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// The profile was read from Discord and stored.
    Updated,
    /// The member chose their own profile, or is not linked to Discord.
    NotFollowing,
    /// Read less than [`REFRESH_INTERVAL`] ago.
    Fresh,
    /// The account left the Discord server (or the guild has none): the
    /// profile is kept as it is.
    OutOfReach,
    /// Discord could not be read (not configured, API failure).
    Unavailable,
}

/// Reads the member's profile from Discord and stores it, while the member
/// follows their Discord account. `force` ignores [`REFRESH_INTERVAL`].
pub async fn refresh_member(
    db: &Db,
    source: &dyn DiscordProfileSource,
    member: &GuildMember,
    now: DateTime<Utc>,
    force: bool,
) -> RefreshOutcome {
    let Some(user_id) = member.discord_user_id.as_deref() else {
        return RefreshOutcome::NotFollowing;
    };
    if !member.follows_discord() {
        return RefreshOutcome::NotFollowing;
    }
    if !force
        && member
            .discord_synced_at
            .is_some_and(|synced_at| now - synced_at < REFRESH_INTERVAL)
    {
        return RefreshOutcome::Fresh;
    }
    let members = db.guild_members();
    // Native guilds are the Discord server itself; a standalone guild may be
    // linked to one.
    let discord_guild_id = match db.guilds().access(&member.guild_id).await {
        Ok(Some(access)) if access.standalone => access.discord_guild_id,
        Ok(_) => Some(member.guild_id.as_str().to_owned()),
        Err(error) => {
            tracing::warn!(%error, member = %member.id, "lecture de la guilde impossible");
            return RefreshOutcome::Unavailable;
        }
    };
    let profile = match discord_guild_id {
        Some(discord_guild_id) => source.member_profile(&discord_guild_id, user_id).await,
        None => Ok(None),
    };
    let profile = match profile {
        Ok(Some(profile)) => profile,
        Ok(None) => {
            if let Err(error) = members.mark_discord_synced(member.id, now).await {
                tracing::warn!(%error, member = %member.id, "horodatage du profil Discord impossible");
            }
            return RefreshOutcome::OutOfReach;
        }
        Err(ProfileError::Disabled) => return RefreshOutcome::Unavailable,
        Err(error) => {
            tracing::warn!(%error, member = %member.id, "lecture du profil Discord impossible");
            return RefreshOutcome::Unavailable;
        }
    };
    let name = identity::member_name(&profile.name).unwrap_or_else(|| member.display_name.clone());
    match members
        .sync_discord_profile(member.id, &name, profile.image.as_deref(), now)
        .await
    {
        Ok(true) => RefreshOutcome::Updated,
        // « Mon profil » was saved meanwhile.
        Ok(false) => RefreshOutcome::NotFollowing,
        Err(error) => {
            tracing::warn!(%error, member = %member.id, "mise à jour du profil Discord impossible");
            RefreshOutcome::Unavailable
        }
    }
}

/// Refreshes, in the background, the profile of the member owning an overlay
/// that just connected.
pub fn spawn_refresh_on_connect(state: &AppState, client_id: OverlayClientId) {
    let Some(db) = state.db.clone() else {
        return;
    };
    let source = Arc::clone(&state.discord_profiles);
    let now = state.clock.now();
    tokio::spawn(async move {
        let member = match db.guild_members().get_for_overlay_client(client_id).await {
            Ok(Some(member)) => member,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%error, client = %client_id, "lecture du membre de l'overlay impossible");
                return;
            }
        };
        let outcome = refresh_member(&db, source.as_ref(), &member, now, false).await;
        tracing::debug!(?outcome, member = %member.id, "profil Discord à la connexion");
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[tokio::test]
    async fn disabled_profiles_return_disabled() {
        let result = DisabledProfiles.member_profile("1", "2").await;
        assert!(matches!(result, Err(ProfileError::Disabled)));
    }

    #[tokio::test]
    async fn serenity_profiles_reject_invalid_ids_without_network() {
        let source = SerenityProfiles::new("test-token");
        assert!(matches!(
            source.member_profile("abc", "2").await,
            Err(ProfileError::InvalidId)
        ));
        assert!(matches!(
            source.member_profile("1", "").await,
            Err(ProfileError::InvalidId)
        ));
    }
}
