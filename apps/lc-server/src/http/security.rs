//! En-têtes de sécurité appliqués à toutes les réponses.
//!
//! ## Décisions vs legacy
//!
//! Le legacy utilisait Helmet avec une CSP permissive (`script-src 'self'
//! 'unsafe-inline'`) et `Cross-Origin-Embedder-Policy` désactivé
//! (`server/src/server.ts`, cf. `docs/rust/audit-dynamique.md`). La cible v3
//! corrige les points S4 de l'annexe de sécurité du plan de migration :
//!
//! - `script-src 'self'` **sans** `'unsafe-inline'` : aucun script inline n'est
//!   nécessaire dans le client v2, et l'admin UI sera buildée avec des fichiers
//!   séparés (Phase 7) ;
//! - `style-src 'self' 'unsafe-inline'` est conservé : les overrides de thème de
//!   l'overlay injectent des styles inline ;
//! - `frame-ancestors 'none'` + `X-Frame-Options: DENY` : l'overlay n'est jamais
//!   destiné à être embarqué ; c'est le legacy qui n'avait que `frame-src 'none'` ;
//! - `Cross-Origin-Resource-Policy: cross-origin` est conservé pour que les
//!   médias servis par le serveur restent chargeables par l'overlay Tauri ;
//! - COEP reste désactivé (compatibilité des médias tiers).
//!
//! Les en-têtes sont ajoutés par un middleware appliqué suffisamment à
//! l'extérieur pour couvrir aussi les réponses d'erreur (404, 429, 413).

use axum::extract::Request;
use axum::http::{header, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

/// Politique de sécurité du contenu (CSP) cible.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
     script-src 'self'; \
     style-src 'self' 'unsafe-inline'; \
     img-src 'self' data: https:; \
     media-src 'self'; \
     font-src 'self'; \
     connect-src 'self'; \
     object-src 'none'; \
     frame-src 'none'; \
     base-uri 'self'; \
     form-action 'self'; \
     frame-ancestors 'none'";

/// Politique de permissions (fonctionnalités navigateur inutiles au produit).
pub const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), payment=(), usb=()";

/// HSTS (AUD-COR-18/A7-020). Appliqué uniquement quand la requête arrive en
/// HTTPS (directement ou via un reverse proxy qui pose `X-Forwarded-Proto`).
pub const STRICT_TRANSPORT_SECURITY: &str = "max-age=31536000; includeSubDomains";

/// Isolation d'ouverture inter-origines (COOP).
pub const CROSS_ORIGIN_OPENER_POLICY: &str = "same-origin";

/// La requête est-elle servie en HTTPS aux yeux du navigateur ?
///
/// Le serveur écoute en HTTP : en production, c'est le reverse proxy TLS qui
/// termine HTTPS et transmet `X-Forwarded-Proto: https`. On ne fait confiance
/// qu'à cette information pour HSTS (un HSTS posé en HTTP est de toute façon
/// ignoré par les navigateurs, mais ne pas l'émettre en développement évite de
/// verrouiller `localhost`).
fn request_is_secure(request: &Request) -> bool {
    if request.uri().scheme_str() == Some("https") {
        return true;
    }
    request
        .headers()
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .next()
                .is_some_and(|scheme| scheme.trim().eq_ignore_ascii_case("https"))
        })
}

/// Middleware ajoutant les en-têtes de sécurité à chaque réponse.
pub async fn security_headers(request: Request, next: Next) -> Response {
    let secure = request_is_secure(&request);
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    // `entry().or_insert()` : la couche étant externe, un sous-routeur (page
    // admin) peut avoir posé sa propre CSP plus stricte ; la valeur globale ne
    // doit s'appliquer qu'en l'absence de valeur (sinon l'admin ne pourrait pas
    // restreindre script/style/connect pour sa page).
    headers
        .entry(HeaderName::from_static("content-security-policy"))
        .or_insert(HeaderValue::from_static(CONTENT_SECURITY_POLICY));
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
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static(CROSS_ORIGIN_OPENER_POLICY),
    );
    // HSTS uniquement si le navigateur voit du HTTPS (délégué au reverse proxy).
    if secure {
        headers.insert(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static(STRICT_TRANSPORT_SECURITY),
        );
    }

    response
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn security_headers_are_added_to_responses() {
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn(security_headers));

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let headers = response.headers();
        assert_eq!(
            headers.get("content-security-policy").unwrap(),
            CONTENT_SECURITY_POLICY
        );
        assert_eq!(
            headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
        assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
        assert_eq!(headers.get(header::REFERRER_POLICY).unwrap(), "no-referrer");
        assert_eq!(
            headers.get("cross-origin-resource-policy").unwrap(),
            "cross-origin"
        );
        assert_eq!(
            headers.get("permissions-policy").unwrap(),
            PERMISSIONS_POLICY
        );
        assert_eq!(
            headers.get("cross-origin-opener-policy").unwrap(),
            CROSS_ORIGIN_OPENER_POLICY
        );
        // HTTP simple (développement, tests) : pas de HSTS.
        assert!(headers.get("strict-transport-security").is_none());
    }

    #[tokio::test]
    async fn hsts_is_emitted_only_over_tls() {
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn(security_headers));

        // Derrière un reverse proxy TLS (`X-Forwarded-Proto: https`).
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("x-forwarded-proto", "https")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers().get("strict-transport-security").unwrap(),
            STRICT_TRANSPORT_SECURITY
        );

        // HTTP direct : jamais de HSTS (ne pas verrouiller le développement).
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(response
            .headers()
            .get("strict-transport-security")
            .is_none());
    }

    #[test]
    fn csp_forbids_inline_scripts_and_framing() {
        assert!(CONTENT_SECURITY_POLICY.contains("script-src 'self';"));
        assert!(!CONTENT_SECURITY_POLICY.contains("script-src 'self' 'unsafe-inline'"));
        assert!(CONTENT_SECURITY_POLICY.contains("frame-ancestors 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("object-src 'none'"));
    }
}
