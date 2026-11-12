//! Panneau d'administration du serveur LiveChat (Phase 7b).
//!
//! Sert la page statique livrée en Phase 7a (`static/admin/`) et expose les
//! 13 routes JSON du contrat legacy, sous `/admin/api/**`. Les contrats sont
//! décrits dans `docs/rust/admin-panel.md` (SSOT) et `docs/rust/server-admin.md`
//! documente l'implémentation Rust.
//!
//! # Structure
//!
//! - [`static_files`] : `GET /admin` (308) et `GET /admin/*` (assets) ;
//! - [`auth`] : extracteur des routes API (503/403/401, `Authorization: Bearer`
//!   uniquement — le repli `?token=` a été retiré, cf. AUD-COR-05) ;
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

mod auth;
mod clients;
mod error;
mod guilds;
mod ingest_authors;
mod overview;
mod pairings;
mod queries;
mod runtime_settings;
mod static_files;
#[cfg(test)]
mod tests;

pub mod hub;

pub use error::AdminError;
pub use hub::{AdminHub, NoopAdminHub};

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

/// CSP du panneau : aucune ressource distante hors images Discord, styles
/// inline autorisés (deux barres de progression du JS utilisent `style="width"`),
/// `connect-src 'self'` pour les `fetch` de `app.js`.
pub(crate) const ADMIN_CSP: &str = "default-src 'none'; script-src 'self'; \
style-src 'self' 'unsafe-inline'; \
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
            "/admin/api/guilds/{guild_id}/meme-board/clear",
            post(guilds::clear_meme_board),
        )
        .route(
            "/admin/api/guilds/{guild_id}/discord-unlink",
            post(guilds::discord_unlink),
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
        );

    secure(api).merge(static_files::router())
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

    let mut response = next.run(request).await;

    if mutating {
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

    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
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
