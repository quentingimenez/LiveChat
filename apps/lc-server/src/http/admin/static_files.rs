//! Admin console assets embedded in the server binary.
//!
//! The Preact/TypeScript console is built by Vite into
//! `apps/lc-admin-web/dist/` and embedded with [`rust_embed`]:
//!
//! - `GET /admin` ⇒ `308 /admin/` (the console uses relative asset URLs, like
//!   the former static panel);
//! - `GET /admin/` ⇒ `index.html`;
//! - `GET /admin/{*path}` ⇒ the embedded file with its MIME type.
//!
//! Hashed Vite assets (`assets/*`) are immutable and cached for a year;
//! `index.html` is never cached. Unknown paths answer `404` and **never** fall
//! back to `index.html`: the console router is hash based, so a missing file is
//! a real error.
//!
//! `build.rs` builds the front with npm before compilation when the toolchain
//! is available (`LC_ADMIN_WEB_BUILD=0` or `skip` disables it when `dist/` is
//! provided, as in the Docker image). When `dist/` is still absent, `/admin/`
//! answers a minimal "console was not built" page without inline style or
//! script (the console CSP has no `'unsafe-inline'`).
//!
//! Security headers (CSP, `nosniff`, `Referrer-Policy`, `X-Frame-Options`,
//! `Permissions-Policy`) are applied by [`super::secure`]. The page is public
//! (it contains no business data); authentication protects `/admin/api/**`.

use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::RustEmbed;

use super::secure;
use crate::app::AppState;

/// Files built by Vite and embedded at compile time.
///
/// `allow_missing` keeps `cargo build` working when the front was not built; the
/// fallback page then explains how to get a working console.
#[derive(RustEmbed)]
#[folder = "../lc-admin-web/dist/"]
#[allow_missing = true]
struct Assets;

/// Cache policy of hashed assets.
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

/// Cache policy of the HTML entry point and of the fallback page.
const NO_STORE: &str = "no-store";

/// Sub-router of the console assets (`/admin` and `/admin/*`).
pub fn router() -> Router<AppState> {
    let routes = Router::new()
        .route("/admin", get(redirect_to_trailing_slash))
        .route("/admin/", get(index))
        .route("/admin/{*path}", get(asset));

    secure(routes)
}

/// Redirects to `/admin/` (308, permanent redirect).
async fn redirect_to_trailing_slash() -> Response {
    axum::response::Redirect::permanent("/admin/").into_response()
}

/// Serves `index.html`, or the "not built" page when nothing was embedded.
async fn index() -> Response {
    match Assets::get("index.html") {
        Some(file) => file_response("index.html", file.data.into_owned()),
        None => html_response(StatusCode::OK, NOT_BUILT_PAGE.to_owned(), NO_STORE),
    }
}

/// Serves one embedded file, or `404` when it does not exist.
async fn asset(Path(path): Path<String>) -> Response {
    let path = path.trim_start_matches('/');

    // A directory-like path is never a file: keep the "no index fallback" rule.
    let Some(file) = Assets::get(path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };

    file_response(path, file.data.into_owned())
}

/// Builds the response of one embedded file with its cache policy.
fn file_response(path: &str, data: Vec<u8>) -> Response {
    let cache = if path.starts_with("assets/") {
        IMMUTABLE_CACHE
    } else {
        NO_STORE
    };
    let content_type = mime_type(path);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
        ],
        data,
    )
        .into_response()
}

/// Builds an HTML response with an explicit cache policy.
fn html_response(status: StatusCode, body: String, cache: &'static str) -> Response {
    (
        status,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
        ],
        body,
    )
        .into_response()
}

/// MIME type derived from the file extension.
fn mime_type(path: &str) -> &'static str {
    let extension = path
        .rsplit('.')
        .next()
        .filter(|extension| *extension != path)
        .map(str::to_ascii_lowercase);

    match extension.as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("json") => "application/json",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Names of the embedded files (integration tests).
#[cfg(test)]
pub(super) fn embedded_files() -> Vec<String> {
    <Assets as RustEmbed>::iter()
        .map(|path| path.into_owned())
        .collect()
}

/// Minimal page served when the console was not built.
///
/// It intentionally contains no inline style or script: the admin CSP only
/// allows `'self'` for both.
const NOT_BUILT_PAGE: &str = "<!doctype html>\
<html lang=\"en\">\
<head>\
<meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>LiveChat admin console is not built</title>\
</head>\
<body>\
<h1>Admin console not built</h1>\
<p>Build <code>apps/lc-admin-web</code> before compiling the server: \
<code>npm ci &amp;&amp; npm run build</code>.</p>\
</body>\
</html>";

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::body::to_bytes;
    use axum::http::StatusCode;

    use super::*;

    #[test]
    fn mime_types_are_derived_from_the_extension() {
        assert_eq!(mime_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            mime_type("assets/index-abc.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(mime_type("assets/index-abc.css"), "text/css; charset=utf-8");
        assert_eq!(mime_type("logo.svg"), "image/svg+xml");
        assert_eq!(mime_type("favicon.png"), "image/png");
        assert_eq!(mime_type("favicon.ico"), "image/x-icon");
        assert_eq!(mime_type("manifest.json"), "application/json");
        assert_eq!(mime_type("font.woff2"), "font/woff2");
        assert_eq!(mime_type("LICENSE"), "application/octet-stream");
    }

    #[test]
    fn fallback_page_has_no_inline_style_or_script() {
        assert!(NOT_BUILT_PAGE.contains("not built"));
        assert!(!NOT_BUILT_PAGE.contains("<script"));
        assert!(!NOT_BUILT_PAGE.contains("<style"));
        assert!(!NOT_BUILT_PAGE.contains("style="));
        assert!(!NOT_BUILT_PAGE.contains("onclick"));
    }

    #[tokio::test]
    async fn unknown_asset_is_a_404_without_index_fallback() {
        let response = asset(Path("does-not-exist.js".to_owned())).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body readable");
        assert!(!String::from_utf8_lossy(&bytes).contains("<!doctype html>"));
    }
}
