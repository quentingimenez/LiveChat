//! Panneau d'administration du serveur LiveChat (Phase 7b).
//!
//! Serves the embedded console (`apps/lc-admin-web/dist`, built by Vite and
//! embedded with `rust-embed`), exposes the contract JSON routes under
//! `/admin/api/**` and the session routes under `/admin/auth/**`. The contracts
//! are described in `docs/rust/admin-console-api.md` (and
//! `docs/rust/server-admin.md` documents the Rust implementation).
//!
//! # Structure
//!
//! - [`static_files`] : `GET /admin` (308) et `GET /admin/*` (assets) ;
//! - [`events`] : real-time `GET /admin/api/events` stream (SSE) ;
//! - [`auth`] : extracteur des routes API (503/403/401, cookie de session
//!   `lc_admin_session`) ;
//! - [`auth_routes`] : connexion secret/Discord, session courante et déconnexion ;
//! - [`session`] : sessions en SQL brut et cookies ;
//! - [`oauth`] : échange OAuth2 Discord derrière le trait [`AdminOAuth`] ;
//! - [`hub`] : trait [`AdminHub`] vers les sockets WebSocket (comptage,
//!   fermeture, diffusions d'arrêt et de réglages) ;
//! - [`overview`], [`runtime_settings`], [`ingest_authors`], [`guilds`],
//!   [`clients`], [`pairings`] : handlers ;
//! - [`queries`] : agrégats et suppressions transverses en SQL brut (ce que les
//!   repositories de `lc-db` n'exposent pas).
//!
//! # En-têtes
//!
//! Toutes les réponses du panneau passent par [`secure`] : `Cache-Control:
//! no-store`, CSP stricte ([`ADMIN_CSP`]), `X-Content-Type-Options: nosniff`,
//! `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer` et une
//! `Permissions-Policy` restrictive. La couche globale `http::security` est
//! conçue pour ne pas écraser ces en-têtes (`entry(...).or_insert(...)`, fix
//! validé par le Lead).
//!
//! Cette couche journalise aussi un **audit minimal** des mutations
//! administratives (AUD-COR-10) : méthode, chemin seul, statut HTTP et adresse
//! du client. Ni la query string ni les en-têtes ne sont journalisés (un
//! `?token=` ou un `Authorization` finirait en clair dans les journaux).

mod audit;
mod auth;
mod auth_routes;
mod clients;
mod devices;
mod error;
mod events;
mod guild_detail;
mod guilds;
pub(crate) use guilds::stop_guild_playback;
mod ingest_authors;
mod oauth;
mod ops;
mod overview;
mod pairings;
mod queries;
mod rights;
mod runtime_settings;
mod session;
mod static_files;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_devices;
#[cfg(test)]
mod tests_fixtures;
#[cfg(test)]
mod tests_guild_detail;
#[cfg(test)]
mod tests_live;
#[cfg(test)]
mod tests_ops;
#[cfg(test)]
mod tests_rights;

pub(crate) use audit::purge_old as purge_old_audit;
pub(crate) use ops::{BinaryProbe, SystemBinaryProbe};

pub mod hub;

pub use error::AdminError;
pub use events::{AdminEvent, AdminEvents};
pub use hub::{AdminHub, NoopAdminHub};
pub use oauth::{AdminOAuth, DisabledOAuth, DiscordIdentity, DiscordOAuth, OAuthError};
pub(crate) use session::purge_expired as purge_expired_sessions;

use std::net::SocketAddr;

use axum::extract::ConnectInfo;
use axum::http::{header, HeaderName, HeaderValue, Method};
use axum::middleware;
use axum::response::Response;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use chrono::{DateTime, SecondsFormat, Utc};
use lc_db::Db;
use serde_json::Value;

use crate::app::AppState;

/// CSP du panneau : aucune ressource distante hors images Discord, feuilles de
/// style servies par le serveur, `connect-src 'self'` pour les `fetch` de la
/// console.
pub(crate) const ADMIN_CSP: &str = "default-src 'none'; script-src 'self'; \
style-src 'self'; \
img-src 'self' https://cdn.discordapp.com https://media.discordapp.net; \
connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; \
frame-ancestors 'none'";

/// Routeur du panneau : assets statiques publics + API JSON protégée.
pub fn router() -> Router<AppState> {
    let api = Router::new()
        .route("/admin/api/overview", get(overview::overview))
        .route(
            "/admin/api/runtime-settings",
            get(runtime_settings::get).patch(runtime_settings::patch),
        )
        .route("/admin/api/ingest-authors", get(ingest_authors::list))
        .route(
            "/admin/api/ingest-clients",
            post(ingest_authors::create_client),
        )
        .route("/admin/api/guilds", post(guilds::create))
        .route(
            "/admin/api/guilds/{guild_id}/admin-code",
            post(guilds::admin_code),
        )
        .route(
            "/admin/api/guilds/{guild_id}/settings",
            patch(guilds::update_settings),
        )
        .route("/admin/api/guilds/{guild_id}/stop", post(guilds::stop))
        .route("/admin/api/guilds/{guild_id}/purge", post(guilds::purge))
        .route(
            "/admin/api/guilds/{guild_id}/revoke-clients",
            post(guilds::revoke_clients),
        )
        .route(
            "/admin/api/guilds/{guild_id}/media-board/clear",
            post(guilds::clear_media_board),
        )
        .route(
            "/admin/api/guilds/{guild_id}/discord-unlink",
            post(guilds::discord_unlink),
        )
        .route(
            "/admin/api/guilds/{guild_id}/discord-leave",
            post(guilds::discord_leave),
        )
        .route(
            "/admin/api/overlay-clients/{client_id}/revoke",
            post(clients::revoke_overlay),
        )
        .route(
            "/admin/api/ingest-clients/{client_id}/revoke",
            post(clients::revoke_ingest),
        )
        .route("/admin/api/pairing-codes", get(pairings::list))
        .route(
            "/admin/api/pairing-codes/expired",
            delete(pairings::delete_expired),
        )
        .route(
            "/admin/api/pairing-codes/{code}/revoke",
            post(pairings::revoke),
        )
        .merge(ops::routes())
        .merge(audit::routes())
        .merge(rights::routes())
        .merge(events::routes());

    secure(
        api.merge(guild_detail::routes())
            .merge(devices::routes())
            .merge(auth_routes::router()),
    )
    .merge(static_files::router())
}

/// Applique les en-têtes communs du panneau.
///
/// Le middleware est posé en dernier : il s'exécute après les handlers et après
/// les éventuelles couches internes, et pose les en-têtes en `insert` (la CSP
/// admin remplace alors toute valeur posée plus tôt sur ces routes). La couche
/// globale `http::security` est en `or_insert`, elle ne réécrase donc pas ces
/// valeurs (fix validé par le Lead).
pub(crate) fn secure(router: Router<AppState>) -> Router<AppState> {
    router.layer(middleware::from_fn(admin_headers))
}

/// Middleware qui pose les en-têtes de sécurité du panneau.
///
/// Il journalise également un audit minimal des mutations administratives
/// (AUD-COR-10). Le chemin seul est conservé : la query string et les en-têtes
/// ne sont jamais journalisés, pour ne pas exposer un jeton (AUD-COR-05).
async fn admin_headers(request: axum::extract::Request, next: middleware::Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    // The application state is inserted as a request extension by
    // `app::router_with_hub`: the admin router itself is built before the state
    // is applied, so this middleware cannot take a typed `State` extractor.
    let state = request.extensions().get::<AppState>().cloned();
    let client_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip());
    // `x-request-id` est posé par la couche la plus externe (AUD-COR-10/A3-012) :
    // il corrèle la ligne d'audit avec les logs HTTP et la réponse.
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mutating = matches!(
        method,
        Method::POST | Method::PATCH | Method::PUT | Method::DELETE
    );
    // Read the actor before the handler runs: a logout deletes its session.
    let actor = if mutating {
        match state.as_ref() {
            Some(state) => audit::actor_for_request(state, request.headers()).await,
            None => None,
        }
    } else {
        None
    };
    let mut response = next.run(request).await;

    if mutating {
        if let Some(state) = state.as_ref() {
            // Signal the overview before the audit row: the SSE stream then
            // emits the refreshed overview before the mutation's own `audit`
            // event (the audit insert is awaited below).
            if response.status().is_success() {
                state.events.overview_changed();
            }

            audit::record(
                state,
                method.as_str(),
                &path,
                i64::from(response.status().as_u16()),
                actor,
                client_ip,
            )
            .await;
        }
        tracing::info!(
            target: "lc_server::admin_audit",
            method = %method,
            path = %path,
            status = response.status().as_u16(),
            client_ip = ?client_ip,
            request_id = ?request_id,
            "mutation administrative"
        );
    }

    let headers = response.headers_mut();

    // Static assets set their own policy (hashed files are immutable); every
    // other admin response keeps the historical `no-store`.
    headers
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(ADMIN_CSP),
    );

    response
}

/// Base de données exigée par les handlers : sans base, 500 neutre.
pub(crate) fn require_db(state: &AppState) -> Result<&Db, AdminError> {
    state.db.as_ref().ok_or_else(|| {
        tracing::error!("le panneau admin nécessite une base de données configurée");
        AdminError::Internal
    })
}

/// Date ISO 8601 UTC avec millisecondes (`2026-11-09T22:49:44.000Z`).
pub(crate) fn to_iso(date_time: DateTime<Utc>) -> String {
    date_time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Variante `Option` de [`to_iso`] (JSON `null` si absent).
pub(crate) fn opt_iso(date_time: Option<DateTime<Utc>>) -> Option<String> {
    date_time.map(to_iso)
}

/// Convertit un entier millisecondes (stockage SQLite) en ISO 8601.
pub(crate) fn iso_from_ms(milliseconds: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(milliseconds).map(to_iso)
}

/// Pourcentage arrondi à 2 décimales et borné à `[0, 100]`, comme `toPercent`.
pub(crate) fn percent(value: i64, maximum: i64) -> f64 {
    if maximum <= 0 {
        return 0.0;
    }

    let ratio = value as f64 / maximum as f64 * 100.0;
    ((ratio * 100.0).round() / 100.0).clamp(0.0, 100.0)
}

/// Mégaoctets → octets, avec au moins 1 Mo et saturation (comme le legacy).
pub(crate) fn megabytes_to_bytes(megabytes: u64) -> i64 {
    i64::try_from(megabytes.max(1))
        .unwrap_or(i64::MAX)
        .saturating_mul(1024 * 1024)
}

/// `toNonEmptyString` legacy : chaîne trimée non vide, sinon `None`.
pub(crate) fn to_non_empty_string(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let trimmed = value.as_str()?.trim();

    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
