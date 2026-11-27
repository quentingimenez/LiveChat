//! Audit of the administrative actions done from an overlay.
//!
//! The owner console journals its mutations in `admin_audit` through the
//! `admin_headers` middleware. Guild admins acting from the board app write to
//! the same journal, with the actor kind `overlay`, so both the owner console
//! and the overlay's Journal tab show one history.
//!
//! Two entry points:
//!
//! - [`overlay_admin_audit`] is the middleware layered on `/overlay/admin/**`:
//!   one row per successful mutation, none for reads, 4xx or 5xx;
//! - [`record_moderation`] is called explicitly by the board routes when a
//!   member uses an elevated right on another member's item (delete, edit,
//!   artwork). Stop-for-everyone is journalled from the WebSocket handler with
//!   the same [`record_overlay_audit`] primitive.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use lc_domain::{MediaBoardItemId, Permission};

use crate::app::AppState;
use crate::http::admin::{audit_action_for, record_overlay_audit};

use super::auth;
use super::ActorInfo;

/// Middleware of `/overlay/admin/**`: journals each successful mutation.
///
/// The Bearer token is resolved before the handler runs, both to name the actor
/// and to hand the authenticated client to the handler's extractor. A request
/// that ends in 401/403/404/409/5xx leaves no row. The journal records the
/// client resolved from a declared proxy chain, never the proxy address; an
/// unresolvable chain gives `NULL`.
pub(crate) async fn overlay_admin_audit(mut request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    if !matches!(
        method,
        Method::POST | Method::PATCH | Method::PUT | Method::DELETE
    ) {
        return next.run(request).await;
    }
    let Some(state) = request.extensions().get::<AppState>().cloned() else {
        return next.run(request).await;
    };
    let peer_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip());
    let client_ip = crate::http::proxy::audit_client_ip(
        peer_ip,
        request.headers(),
        &state.config.trusted_proxies,
    );

    let authed = match auth::bearer_token(request.headers()) {
        Some(token) => match state.db.as_ref() {
            Some(db) => auth::authenticate_token(db, &token, state.clock.now())
                .await
                .ok()
                .flatten(),
            None => None,
        },
        None => None,
    };
    if let Some(authed) = authed.clone() {
        request.extensions_mut().insert(authed);
    }

    let response = next.run(request).await;
    if !response.status().is_success() {
        return response;
    }
    let Some(authed) = authed else {
        return response;
    };
    // Keep the owner console's overview in sync, exactly like its own
    // middleware does for a successful mutation.
    state.events.overview_changed();
    let actor_name = authed
        .member
        .as_ref()
        .map(|member| member.display_name.as_str())
        .unwrap_or(authed.client.label.as_str());
    let action = audit_action_for(method.as_str(), &path);
    record_overlay_audit(
        &state,
        authed.guild_id(),
        &action,
        actor_name,
        method.as_str(),
        &path,
        response.status().as_u16(),
        client_ip,
    )
    .await;
    response
}

/// Journals an elevated action on a board item owned by another member.
///
/// `creator_member` must be read before the item is deleted. Only a real
/// moderation is recorded: the operator console passes an operator actor (its
/// own middleware already journals the route), a member editing their own item
/// is not a moderation, and an item without a recorded author has no other
/// member to moderate.
pub(super) async fn record_moderation(
    state: &AppState,
    actor: &ActorInfo,
    creator_member: Option<lc_domain::GuildMemberId>,
    action: &str,
    method: &str,
    path: &str,
) {
    if actor.operator || !actor.can(Permission::EditAnyMedia) {
        return;
    }
    let Some(member_id) = actor.member_id() else {
        return;
    };
    if !creator_member.is_some_and(|creator| creator != member_id) {
        return;
    }
    let actor_name = actor
        .member
        .as_ref()
        .map(|member| member.display_name.as_str())
        .unwrap_or(actor.client.label.as_str());
    record_overlay_audit(
        state,
        actor.guild_id(),
        action,
        actor_name,
        method,
        path,
        200,
        None,
    )
    .await;
}

/// Reads the recorded author of a board item, logging a lookup failure.
///
/// A missing author and a database error both yield `None`: the action still
/// succeeds, only the journal entry is skipped.
pub(super) async fn creator_member(
    db: &lc_db::Db,
    item_id: MediaBoardItemId,
) -> Option<lc_domain::GuildMemberId> {
    match db.media_board().creator_member(item_id).await {
        Ok(creator) => creator,
        Err(error) => {
            tracing::warn!(
                error = %error,
                item_id = %item_id,
                "lecture de l'auteur d'un média impossible pour le journal"
            );
            None
        }
    }
}

/// Journals a successful guild-wide stop requested from an overlay socket.
///
/// `stopped` is the number of jobs the playback engine actually stopped: a
/// request that changed nothing is not an administrative action.
pub(crate) async fn record_playback_stop(
    state: &AppState,
    db: &lc_db::Db,
    guild_id: &lc_domain::GuildId,
    client_id: lc_domain::OverlayClientId,
    stopped: usize,
) {
    if stopped == 0 {
        return;
    }
    let actor_name = match db.guild_members().get_for_overlay_client(client_id).await {
        Ok(Some(member)) => member.display_name,
        Ok(None) => "Overlay".to_owned(),
        Err(error) => {
            tracing::warn!(error = %error, "lecture du membre pour le journal impossible");
            "Overlay".to_owned()
        }
    };
    record_overlay_audit(
        state,
        guild_id,
        "overlay.playback_stop",
        &actor_name,
        "WS",
        "/ws",
        200,
        None,
    )
    .await;
}

/// Nom de l'acteur d'une action faite depuis un overlay : le pseudo du membre
/// rattaché au client, ou le libellé du client quand le token n'en a pas.
pub(super) fn actor_name(auth: &super::AuthedOverlayClient) -> &str {
    auth.member
        .as_ref()
        .map(|member| member.display_name.as_str())
        .unwrap_or(auth.client.label.as_str())
}

/// Journalled action of a queue moderation, if any.
pub(super) async fn record_queue_action(
    state: &AppState,
    auth: &super::AuthedOverlayClient,
    action: &str,
    path: &str,
) {
    let actor_name = actor_name(auth);
    record_overlay_audit(
        state,
        auth.guild_id(),
        action,
        actor_name,
        "DELETE",
        path,
        200,
        None,
    )
    .await;
}
