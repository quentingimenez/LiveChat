//! Actions dangereuses d'une guilde depuis l'overlay (rôle admin).
//!
//! | Méthode | Chemin | Droit |
//! |---------|--------|-------|
//! | `POST` | `/overlay/admin/purge` | `PurgeGuild` |
//! | `POST` | `/overlay/admin/discord-leave` | `PurgeGuild` |
//!
//! Les deux routes exigent `{"confirmGuildId":"<guilde du jeton>"}` : la
//! confirmation forte est la même que dans la console, et une guilde ne peut
//! agir que sur elle-même. Elles partagent l'implémentation du panneau
//! ([`crate::http::admin::purge_guild`], [`crate::http::admin::leave_discord_guild`])
//! pour que les deux surfaces produisent exactement la même réponse.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use lc_domain::Permission;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::http::admin::{leave_discord_guild, purge_guild, require_guild_confirmation};

use super::auth::AuthedOverlayClient;
use super::guild::require;
use super::{error, json_response};

/// Handler `POST /overlay/admin/purge`.
///
/// Supprime définitivement la guilde du jeton : membres, overlays, codes, file
/// d'attente, board et réglages. Comme la console, la purge retire les médias
/// orphelins et ne fait pas quitter le serveur Discord par défaut (l'action
/// séparée ci-dessous le fait).
pub async fn purge(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::PurgeGuild) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    if let Err(admin_error) = require_guild_confirmation(&payload, auth.guild_id()) {
        return admin_error.into_response();
    }

    match purge_guild(&state, auth.guild_id(), true, false).await {
        Ok(report) => json_response(StatusCode::OK, report.payload()),
        Err(admin_error) => admin_error.into_response(),
    }
}

/// Handler `POST /overlay/admin/discord-leave`.
///
/// Fait quitter le serveur Discord à la guilde du jeton, sans la supprimer ni
/// retirer sa liaison : les commandes du bot n'y fonctionnent plus, les données
/// restent sur le serveur LiveChat. Le départ n'échoue jamais la requête.
pub async fn discord_leave(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::PurgeGuild) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    if let Err(admin_error) = require_guild_confirmation(&payload, auth.guild_id()) {
        return admin_error.into_response();
    }

    match leave_discord_guild(&state, auth.guild_id()).await {
        Ok(discord) => json_response(StatusCode::OK, json!({ "discordLeave": discord.payload() })),
        Err(admin_error) => admin_error.into_response(),
    }
}
