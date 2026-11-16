//! `GET` / `PUT /admin/api/rights` — configurable rights per surface.
//!
//! Contract: section 5 of `docs/rust/admin-console-api.md`.
//!
//! The response is the complete effective table: every surface, column and
//! action, each cell carrying its effective `allowed` value, whether it is
//! `fixed` by the contract and its built-in `default`. `PUT` upserts the cells
//! it contains in one transaction (the console sends only the cells changed by
//! the operator), reloads [`crate::rights::RightsStore`] and broadcasts
//! `rights.changed` to every connected overlay.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use lc_domain::rights::{Action, RightsRole, RightsTable, Surface};
use lc_protocol::{RightsChangedPayload, ServerMessage};
use serde_json::{json, Map, Value};

use crate::app::AppState;
use crate::http::admin::auth::AdminAccess;
use crate::http::admin::{require_db, AdminError};
use crate::rights::{save_table, RightsCell};

/// Routes of this module, merged into the secured admin API router.
pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/admin/api/rights", get(get_rights).put(put_rights))
}

/// Handler `GET /admin/api/rights`.
pub(super) async fn get_rights(
    State(state): State<AppState>,
    _access: AdminAccess,
) -> Result<Response, AdminError> {
    Ok(Json(rights_payload(&state)).into_response())
}

/// Handler `PUT /admin/api/rights`.
///
/// `cells` is required by the contract and is applied as an upsert: the cells
/// of the body are written, the other stored overrides are kept.
/// `discordAdminIsAdmin` is optional and keeps the current switch when absent.
pub(super) async fn put_rights(
    State(state): State<AppState>,
    _access: AdminAccess,
    payload: Json<Value>,
) -> Result<Response, AdminError> {
    let cells = match payload.get("cells") {
        Some(value) => value
            .as_array()
            .ok_or_else(|| AdminError::bad_request("invalid_payload"))?,
        None => return Err(AdminError::bad_request("invalid_payload")),
    };

    let mut parsed = Vec::with_capacity(cells.len());
    let mut seen = std::collections::BTreeSet::new();
    for cell in cells {
        let surface = cell
            .get("surface")
            .and_then(Value::as_str)
            .and_then(Surface::from_id)
            .ok_or_else(|| AdminError::bad_request("invalid_right"))?;
        let action = cell
            .get("action")
            .and_then(Value::as_str)
            .and_then(Action::from_id)
            .ok_or_else(|| AdminError::bad_request("invalid_right"))?;
        let role = cell
            .get("role")
            .and_then(Value::as_str)
            .and_then(RightsRole::from_id)
            .ok_or_else(|| AdminError::bad_request("invalid_right"))?;

        // The action must belong to the requested surface (`send` on `overlay`,
        // `msg` on `extension`… are invalid rights, not fixed cells).
        if !surface.actions().contains(&action) || !surface.columns().contains(&role) {
            return Err(AdminError::bad_request("invalid_right"));
        }
        if RightsTable::is_fixed(surface, action, role) {
            return Err(AdminError::bad_request("fixed_right"));
        }

        let allowed = cell
            .get("allowed")
            .and_then(Value::as_bool)
            .ok_or_else(|| AdminError::bad_request("invalid_right"))?;
        if !seen.insert((surface, action, role)) {
            // A duplicate would violate the primary key mid-transaction.
            return Err(AdminError::bad_request("invalid_payload"));
        }
        parsed.push(RightsCell {
            surface,
            action,
            role,
            allowed,
        });
    }

    let flag = match payload.get("discordAdminIsAdmin") {
        Some(value) => Some(
            value
                .as_bool()
                .ok_or_else(|| AdminError::bad_request("invalid_payload"))?,
        ),
        None => None,
    };

    let db = require_db(&state)?;
    save_table(db, &parsed, flag)
        .await
        .map_err(|error| AdminError::db("configurable rights write", error))?;
    if let Err(error) = state.rights.reload(db).await {
        return Err(AdminError::db("configurable rights reload", error));
    }

    state
        .admin_hub
        .broadcast_all(ServerMessage::RightsChanged(RightsChangedPayload {}));

    Ok(Json(rights_payload(&state)).into_response())
}

/// Complete effective rights table, as required by section 5.
fn rights_payload(state: &AppState) -> Value {
    let table = state.rights.snapshot();
    let surfaces: Vec<Value> = Surface::ALL
        .iter()
        .map(|surface| surface_payload(&table, *surface))
        .collect();
    json!({
        "surfaces": surfaces,
        "discordAdminIsAdmin": table.discord_admin_is_admin(),
    })
}

/// One surface entry (`id`, `columns`, `actions`).
fn surface_payload(table: &RightsTable, surface: Surface) -> Value {
    let columns: Vec<&str> = surface.columns().iter().map(|role| role.id()).collect();
    let actions: Vec<Value> = surface
        .actions()
        .iter()
        .map(|action| {
            let mut cells = Map::new();
            for role in surface.columns() {
                cells.insert(
                    role.id().to_owned(),
                    json!({
                        "allowed": table.allowed(surface, *action, *role),
                        "fixed": RightsTable::is_fixed(surface, *action, *role),
                        "default": RightsTable::default_allowed(surface, *action, *role),
                    }),
                );
            }
            json!({ "id": action.id(), "cells": cells })
        })
        .collect();
    json!({
        "id": surface.id(),
        "columns": columns,
        "actions": actions,
    })
}
