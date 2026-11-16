//! Discord OAuth2 login for the admin console.
//!
//! The flow is described in section 1 of `docs/rust/admin-console-api.md`:
//!
//! 1. `GET /admin/auth/discord` redirects to Discord with the `identify` scope
//!    and a random `state` stored in a short-lived cookie;
//! 2. `GET /admin/auth/discord/callback` exchanges the authorization code for
//!    an access token, reads `GET /users/@me`, and refuses user ids outside
//!    `ADMIN_DISCORD_USER_IDS`.
//!
//! The external calls sit behind [`AdminOAuth`] so handlers are testable without
//! network access. [`DiscordOAuth`] is the production implementation;
//! [`DisabledOAuth`] is the default when Discord login is not configured.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::config::Secret;

/// Discord authorization endpoint (browser redirect).
pub const AUTHORIZE_URL: &str = "https://discord.com/oauth2/authorize";
/// Discord token exchange endpoint.
const TOKEN_URL: &str = "https://discord.com/api/oauth2/token";
/// Discord current-user endpoint.
const USERS_ME_URL: &str = "https://discord.com/api/users/@me";

/// Identity returned by `GET /users/@me`, limited to what the panel needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordIdentity {
    /// Discord user id (snowflake as a string).
    pub id: String,
    /// Discord global name when set, otherwise the username.
    pub name: String,
    /// Avatar CDN URL, `None` when the user has no custom avatar.
    pub avatar_url: Option<String>,
}

/// Failure of an OAuth2 exchange or profile lookup.
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    /// No Discord OAuth2 client is configured.
    #[error("Discord login is not configured")]
    Disabled,
    /// The token endpoint failed or returned an unusable payload.
    #[error("Discord token exchange failed: {0}")]
    Exchange(String),
    /// The user profile endpoint failed or returned an unusable payload.
    #[error("Discord profile request failed: {0}")]
    Profile(String),
}

/// Injectable Discord OAuth2 client.
#[async_trait]
pub trait AdminOAuth: Send + Sync {
    /// Exchanges an authorization code and returns the Discord identity.
    async fn exchange(&self, code: &str, redirect_uri: &str)
        -> Result<DiscordIdentity, OAuthError>;
}

/// Default implementation when Discord login is disabled.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisabledOAuth;

#[async_trait]
impl AdminOAuth for DisabledOAuth {
    async fn exchange(
        &self,
        _code: &str,
        _redirect_uri: &str,
    ) -> Result<DiscordIdentity, OAuthError> {
        Err(OAuthError::Disabled)
    }
}

/// Production implementation calling the Discord HTTP API with `reqwest`.
#[derive(Debug, Clone)]
pub struct DiscordOAuth {
    client_id: String,
    client_secret: Secret,
    http: reqwest::Client,
}

impl DiscordOAuth {
    /// Builds the client. The secret stays wrapped in [`Secret`] so `Debug`
    /// output never exposes it.
    pub fn new(client_id: impl Into<String>, client_secret: Secret) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client_id: client_id.into(),
            client_secret,
            http,
        }
    }

    async fn access_token(&self, code: &str, redirect_uri: &str) -> Result<String, OAuthError> {
        let response = self
            .http
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("client_id", self.client_id.trim()),
                ("client_secret", self.client_secret.expose_secret()),
            ])
            .send()
            .await
            .map_err(|error| OAuthError::Exchange(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(OAuthError::Exchange(format!(
                "token endpoint returned HTTP {status}"
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|error| OAuthError::Exchange(format!("invalid token payload: {error}")))?;
        let token = payload
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| OAuthError::Exchange("missing access_token".to_owned()))?;

        Ok(token.to_owned())
    }

    async fn identity(&self, access_token: &str) -> Result<DiscordIdentity, OAuthError> {
        let response = self
            .http
            .get(USERS_ME_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|error| OAuthError::Profile(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(OAuthError::Profile(format!(
                "users/@me returned HTTP {status}"
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|error| OAuthError::Profile(format!("invalid profile payload: {error}")))?;

        let id = payload
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| OAuthError::Profile("missing id".to_owned()))?;
        let username = payload
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let display_name = payload
            .get("global_name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(username)
            .trim();
        if display_name.is_empty() {
            return Err(OAuthError::Profile("missing display name".to_owned()));
        }
        let avatar = payload
            .get("avatar")
            .and_then(Value::as_str)
            .filter(|hash| !hash.is_empty());
        let avatar_url =
            avatar.map(|hash| format!("https://cdn.discordapp.com/avatars/{id}/{hash}.png"));

        Ok(DiscordIdentity {
            id: id.to_owned(),
            name: display_name.to_owned(),
            avatar_url,
        })
    }
}

#[async_trait]
impl AdminOAuth for DiscordOAuth {
    async fn exchange(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<DiscordIdentity, OAuthError> {
        let access_token = self.access_token(code, redirect_uri).await?;
        self.identity(&access_token).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[tokio::test]
    async fn disabled_oauth_refuses_every_exchange() {
        let result = DisabledOAuth
            .exchange("code", "https://example.test/callback")
            .await;

        assert!(matches!(result, Err(OAuthError::Disabled)));
    }

    #[test]
    fn debug_output_never_exposes_the_client_secret() {
        let oauth = DiscordOAuth::new("123456789012345678", Secret::new("super-secret-value"));
        let debug = format!("{oauth:?}");

        assert!(!debug.contains("super-secret-value"));
        assert!(debug.contains("Secret(<redacted>)"));
    }
}
