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

/// Middleware ajoutant les en-têtes de sécurité à chaque réponse.
pub async fn security_headers(request: Request, next: Next) -> Response {
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
    }

    #[test]
    fn csp_forbids_inline_scripts_and_framing() {
        assert!(CONTENT_SECURITY_POLICY.contains("script-src 'self';"));
        assert!(!CONTENT_SECURITY_POLICY.contains("script-src 'self' 'unsafe-inline'"));
        assert!(CONTENT_SECURITY_POLICY.contains("frame-ancestors 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("object-src 'none'"));
    }
}
