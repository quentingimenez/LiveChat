//! Authentification du panneau d'administration.
//!
//! Reproduit fidèlement `assertAdminAccess` du legacy
//! (`server/src/components/admin/adminRoutes.ts:197-225`) :
//!
//! 1. `ADMIN_UI_TOKEN` vide ⇒ `503 admin_ui_disabled` (message exact du legacy) ;
//! 2. `ADMIN_UI_LOCAL_ONLY` vrai et client non loopback ⇒ `403 admin_local_only` ;
//! 3. token absent/différent ⇒ `401 unauthorized`, comparaison à temps constant.
//!
//! Le token est accepté **uniquement** en `Authorization: Bearer <token>`.
//! L'acceptation historique de `?token=<token>` (parité legacy) a été retirée
//! (AUD-COR-05) : un jeton en query string finit dans les journaux d'accès, les
//! proxies et l'historique navigateur. Le panneau lit le token depuis son champ
//! puis `localStorage` et l'envoie en en-tête (voir `static/admin/app.js`).
//!
//! L'authentification est un extracteur ([`AdminAccess`]) plutôt qu'un
//! middleware : le routeur admin est construit sans l'état (`Router<AppState>`)
//! et un middleware `from_fn` ne peut pas extraire `State<AppState>` sans que
//! l'état soit fourni à la construction. Chaque handler JSON reçoit donc
//! `_: AdminAccess`, ce qui garantit qu'aucune route `/admin/api/**` n'oublie le
//! contrôle. Les assets statiques restent publics (page sans donnée, décision
//! E9 de `docs/rust/admin-panel.md`).
//!
//! L'ordre de lecture du token est volontaire : l'en-tête `Authorization` est la
//! seule voie depuis AUD-COR-05 (les requêtes portant un `?token=` sont
//! traitées comme non authentifiées, le jeton de query étant ignoré). Une
//! différence assumée : `Ipv4Addr::is_loopback()` couvre tout
//! `127.0.0.0/8` alors que le legacy ne testait que `127.0.0.1` — c'est plus
//! permissif mais reste strictement local.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::{header, request::Parts, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use subtle::ConstantTimeEq as _;

use crate::app::AppState;

/// Preuve que la requête a passé le contrôle d'accès admin.
///
/// Extrait par chaque handler `/admin/api/**` : si l'extraction échoue, axum
/// renvoie directement la réponse 503/403/401 correspondante, le handler n'est
/// jamais exécuté.
#[derive(Debug, Clone, Copy)]
pub struct AdminAccess;

impl FromRequestParts<AppState> for AdminAccess {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let expected = state.config.admin_ui_token.expose_secret().trim();

        if expected.is_empty() {
            return Err(disabled_response());
        }

        if state.config.admin_ui_local_only && !is_loopback_parts(parts) {
            return Err(json_status(
                StatusCode::FORBIDDEN,
                json!({ "error": "admin_local_only" }),
            ));
        }

        let Some(candidate) = request_token(parts) else {
            return Err(unauthorized_response());
        };

        if !constant_time_eq(expected, &candidate) {
            return Err(unauthorized_response());
        }

        Ok(Self)
    }
}

fn disabled_response() -> Response {
    json_status(
        StatusCode::SERVICE_UNAVAILABLE,
        json!({
            "error": "admin_ui_disabled",
            "message": "ADMIN_UI_TOKEN is not configured",
        }),
    )
}

fn unauthorized_response() -> Response {
    json_status(StatusCode::UNAUTHORIZED, json!({ "error": "unauthorized" }))
}

fn json_status(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}

/// Adresse loopback ? Vrai pour `127.0.0.0/8`, `::1` et `::ffff:127.0.0.1`.
fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(address) => address.is_loopback(),
        IpAddr::V6(address) => {
            address.is_loopback()
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|mapped: Ipv4Addr| mapped.is_loopback())
        }
    }
}

fn is_loopback_parts(parts: &Parts) -> bool {
    let Some(ConnectInfo(address)) = parts.extensions.get::<ConnectInfo<SocketAddr>>() else {
        // Aucune adresse connue (test mal câblé, proxy exotique) : on refuse,
        // comme un client externe. Le serveur réel branche toujours
        // `into_make_service_with_connect_info`.
        return false;
    };

    is_loopback(address.ip())
}

/// Token de la requête : `Authorization: Bearer` uniquement (AUD-COR-05).
///
/// Un éventuel `?token=` est volontairement ignoré : accepter un secret en
/// query string le ferait apparaître dans les journaux d'accès, les proxies et
/// l'historique du navigateur.
fn request_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(header::AUTHORIZATION)?;
    let value = value.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;

    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }

    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    Some(token.to_owned())
}

/// Comparaison à temps constant (longueur d'abord, comme le legacy).
fn constant_time_eq(expected: &str, candidate: &str) -> bool {
    let expected = expected.as_bytes();
    let candidate = candidate.as_bytes();

    if expected.len() != candidate.len() {
        return false;
    }

    expected.ct_eq(candidate).into()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn token_lu_uniquement_dans_authorization_bearer() {
        let (parts, _) = axum::http::Request::builder()
            .uri("/admin/api/overview")
            .header(header::AUTHORIZATION, "Bearer secret")
            .body(())
            .expect("requête valide")
            .into_parts();
        assert_eq!(request_token(&parts), Some("secret".to_owned()));

        let (parts, _) = axum::http::Request::builder()
            .uri("/admin/api/overview")
            .header(header::AUTHORIZATION, "Basic secret")
            .body(())
            .expect("requête valide")
            .into_parts();
        assert_eq!(request_token(&parts), None);
    }

    #[test]
    fn query_token_ignoree() {
        let (parts, _) = axum::http::Request::builder()
            .uri("/admin/api/overview?token=secret")
            .body(())
            .expect("requête valide")
            .into_parts();
        assert_eq!(request_token(&parts), None);
    }

    #[test]
    fn comparaison_temps_constant() {
        assert!(constant_time_eq("secret", "secret"));
        assert!(!constant_time_eq("secret", "secreT"));
        assert!(!constant_time_eq("secret", "secret "));
        assert!(!constant_time_eq("secret", ""));
    }

    #[test]
    fn loopback_accepte_les_formes_attendues() {
        assert!(is_loopback(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_loopback(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 5))));
        assert!(is_loopback("::1".parse().expect("IPv6 valide")));
        assert!(is_loopback("::ffff:127.0.0.1".parse().expect("IPv6 mappé")));
        assert!(!is_loopback(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))));
        assert!(!is_loopback("fe80::1".parse().expect("IPv6 valide")));
    }
}
