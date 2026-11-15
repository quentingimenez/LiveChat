//! How a Discord member is named and pictured, shared by the interaction glue
//! (command author) and the server's profile refresh (Discord REST): both must
//! give a linked LiveChat member the same name and avatar.

/// Name Discord shows for a member: server nickname, else global name, else
/// username (`resolveDiscordAuthorName` legacy).
#[must_use]
pub fn display_name(nickname: Option<&str>, global_name: Option<&str>, username: &str) -> String {
    [nickname, global_name]
        .into_iter()
        .flatten()
        .find(|name| !name.trim().is_empty())
        .unwrap_or(username)
        .to_owned()
}

/// Avatar URL: the server avatar, else the account avatar, else none
/// (`resolveDiscordAuthorImage` legacy). Animated hashes (`a_…`) keep their
/// `.gif`.
#[must_use]
pub fn avatar_url(
    guild_id: u64,
    user_id: u64,
    guild_avatar: Option<&str>,
    user_avatar: Option<&str>,
) -> Option<String> {
    if let Some(hash) = guild_avatar {
        return Some(format!(
            "https://cdn.discordapp.com/guilds/{guild_id}/users/{user_id}/avatars/{hash}.{}?size=256",
            extension(hash)
        ));
    }
    user_avatar.map(|hash| {
        format!(
            "https://cdn.discordapp.com/avatars/{user_id}/{hash}.{}?size=256",
            extension(hash)
        )
    })
}

/// Longest member name LiveChat stores (the overlay API's limit).
pub const MEMBER_NAME_MAX_CHARS: usize = 40;

/// Member name taken from a Discord name: whitespace compacted, at most
/// [`MEMBER_NAME_MAX_CHARS`] characters, `None` when empty.
#[must_use]
pub fn member_name(raw: &str) -> Option<String> {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let name: String = compact.chars().take(MEMBER_NAME_MAX_CHARS).collect();
    let name = name.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

fn extension(hash: &str) -> &'static str {
    if hash.starts_with("a_") {
        "gif"
    } else {
        "png"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_prefers_nickname_then_global_name() {
        assert_eq!(
            display_name(Some("Surnom"), Some("Global"), "user"),
            "Surnom"
        );
        assert_eq!(display_name(Some("  "), Some("Global"), "user"), "Global");
        assert_eq!(display_name(None, None, "user"), "user");
    }

    #[test]
    fn member_name_is_compacted_and_bounded() {
        assert_eq!(
            member_name("  Léa   la  Grande ").as_deref(),
            Some("Léa la Grande")
        );
        assert_eq!(member_name("   "), None);
        assert_eq!(
            member_name(&"x".repeat(60)).map(|name| name.chars().count()),
            Some(40)
        );
    }

    #[test]
    fn avatar_url_prefers_the_server_avatar() {
        assert_eq!(
            avatar_url(1, 2, Some("abc"), Some("def")).as_deref(),
            Some("https://cdn.discordapp.com/guilds/1/users/2/avatars/abc.png?size=256")
        );
        assert_eq!(
            avatar_url(1, 2, None, Some("a_def")).as_deref(),
            Some("https://cdn.discordapp.com/avatars/2/a_def.gif?size=256")
        );
        assert_eq!(avatar_url(1, 2, None, None), None);
    }
}
