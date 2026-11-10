//! `GET /overlay/media/{assetId}` — stream d'un média protégé par URL signée HMAC.
//!
//! Remplace `server/src/components/overlay/overlayRoutes.ts:317-374`, mais la
//! vérification n'est plus un couple Bearer + contrôle de playback job :
//! le protocole v2 authentifie l'URL elle-même (cf. `docs/rust/protocol-v2.md`
//! §8 et `apps/lc-server/src/media_url.rs`) :
//!
//! ```text
//! /overlay/media/{assetId}?clientId={clientId}&exp={unixSecs}&sig={hex}
//! sig = HMAC-SHA256(MEDIA_URL_SECRET, "assetId|clientId|exp")
//! ```
//!
//! `clientId` est signé : une URL volée ne sert à rien sans être réutilisée
//! avec le même identifiant client, lui-même contrôlé actif en base. Le TTL est
//! calculé par le hub (Phase 4b) et n'est jamais prolongé ici.
//!
//! ## Ordre des contrôles
//!
//! 1. paramètres présents et bien formés → sinon `403 media_url_invalid` ;
//! 2. signature HMAC (constant-time) → sinon `403 media_url_invalid` ;
//! 3. expiration (`now >= exp`) → `403 media_url_expired` ;
//! 4. client existant et non révoqué → sinon `403 media_url_invalid` ;
//! 5. asset existant, `READY` et non expiré → sinon `404 media_not_found` ;
//! 6. fichier présent et confiné dans `MEDIA_STORAGE_DIR` → sinon
//!    `404 media_not_found_on_disk` ;
//! 7. stream avec support `HEAD`/`Range` (206, 416) via
//!    `tower_http::services::ServeFile`, en-têtes `Cache-Control: no-store`,
//!    `Accept-Ranges: bytes` et `Content-Type: <mime>`.
//!
//! ## Écarts assumés avec le legacy
//!
//! - plus de contrôle « un playback job existe pour ce couple
//!   (guilde, media) » : `lc-db` n'expose pas cette requête et le hub est la
//!   seule source d'URL signées ; l'écart est documenté dans
//!   `docs/rust/server-overlay.md` ;
//! - le fichier est cherché sous `MEDIA_STORAGE_DIR` (canonicalisation +
//!   confinement) et non plus via un `storagePath` brut.
//!
//! ## Compression
//!
//! `tower-http` ne compresse **jamais** une réponse portant `Content-Range` :
//! les réponses `206` (Range) sont donc sûres par construction. En revanche le
//! prédicat par défaut de `CompressionLayer` n'exclut pas `video/*` ni
//! `audio/*` : une réponse `200` complète (média > 32 octets, requête avec
//! `Accept-Encoding`) pourrait être compressée par le layer global de
//! `crate::app`, ce qui casserait le streaming et le `Accept-Ranges`. La
//! correction relève de `crate::app` (prédicat excluant `/overlay/media` ou les
//! types vidéo/audio) et est signalée au Lead ; ce module ne peut pas
//! l'empêcher seul sans modifier le routeur parent.

use std::path::PathBuf;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use lc_domain::{GuildId, MediaAsset, MediaAssetId, OverlayClientId};
use serde::Deserialize;
use tower::ServiceExt as _;
use tower_http::services::ServeFile;
use tracing::warn;

use crate::app::AppState;
use crate::media_url;

use super::{db, db_failure, error, internal_error, now};

/// Paramètres de l'URL signée (les autres — `startOffsetSec`, `#t=` — sont
/// ignorés : ils ne sont pas signés et ne changent que le lecteur).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SignedMediaQuery {
    /// Identifiant du client overlay auquel l'URL est liée.
    #[serde(default)]
    client_id: Option<String>,
    /// Expiration en secondes UNIX.
    #[serde(default)]
    exp: Option<String>,
    /// Signature HMAC-SHA256 hexadécimale minuscule.
    #[serde(default)]
    sig: Option<String>,
}

/// Handler `GET /overlay/media/{assetId}`.
pub async fn get_media(
    State(state): State<AppState>,
    Path(asset_id_raw): Path<String>,
    Query(query): Query<SignedMediaQuery>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let (client_id_raw, exp_raw, sig) = match (query.client_id, query.exp, query.sig) {
        (Some(client_id), Some(exp), Some(sig)) => (client_id, exp, sig),
        _ => return media_url_invalid(),
    };

    let Ok(asset_id) = MediaAssetId::parse(asset_id_raw.trim()) else {
        return media_not_found();
    };
    let Ok(client_id) = OverlayClientId::parse(client_id_raw.trim()) else {
        return media_url_invalid();
    };
    let Ok(exp_sec) = exp_raw.trim().parse::<u64>() else {
        return media_url_invalid();
    };
    if sig.len() != 64 || !sig.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return media_url_invalid();
    }

    let secret = state.config.media_url_secret.expose_secret();
    if secret.is_empty() {
        warn!("MEDIA_URL_SECRET absent : média signé refusé");
        return media_url_invalid();
    }
    let asset_key = asset_id.to_canonical();
    let client_key = client_id.to_canonical();
    if !media_url::verify(secret.as_bytes(), &asset_key, &client_key, exp_sec, &sig) {
        return media_url_invalid();
    }

    let timestamp = now(&state);
    let now_sec = u64::try_from(timestamp.timestamp()).unwrap_or(0);
    // AUD-A2-002 : une URL signée ne peut pas porter une expiration au-delà de
    // la politique serveur (défense contre un `exp` forgé/prolongé).
    if !media_url::is_expiry_within_max(exp_sec, now_sec) {
        warn!(
            asset_id = %asset_key,
            exp_sec,
            now_sec,
            max_ttl_sec = lc_protocol::MEDIA_URL_TTL_MAX_SEC,
            "URL média signée au-delà du plafond d'expiration : refus"
        );
        return media_url_invalid();
    }
    if media_url::is_expired(exp_sec, now_sec) {
        return media_url_expired();
    }

    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let client = match db.overlay_clients().get(client_id).await {
        Ok(Some(client)) => client,
        Ok(None) => return media_url_invalid(),
        Err(db_error) => return db_failure("overlay_media.client_lookup", &db_error),
    };
    if !client.is_active() {
        return media_url_invalid();
    }

    // AUD-A2-004 : parité legacy — un média signé n'est servi que si un job de
    // lecture de la guilde du client y fait référence. Sans ce contrôle, une URL
    // signée (client valide d'une guilde) permettait de lire le média d'une
    // autre guilde ; le legacy répond `403 forbidden` dans ce cas.
    let asset = match db.media_assets().get(asset_id).await {
        Ok(Some(asset)) => asset,
        Ok(None) => return media_not_found(),
        Err(db_error) => return db_failure("overlay_media.asset_lookup", &db_error),
    };
    if !asset.is_ready() || asset.is_expired_at(timestamp) {
        return media_not_found();
    }

    match playback_access_exists(db, asset_id, &client.guild_id).await {
        Ok(true) => {}
        Ok(false) => return forbidden(),
        Err(db_error) => return db_failure("overlay_media.playback_binding", &db_error),
    }

    let file_path = match resolve_media_file(&state, &asset).await {
        Ok(path) => path,
        Err(response) => return response,
    };

    touch_asset(&state, &asset, timestamp).await;

    let range = headers.get(header::RANGE).cloned();
    serve_file(&asset, &file_path, method, range.as_ref()).await
}

/// Vérifie qu'un job de lecture de la guilde référence cet asset.
///
/// AUD-A2-004 : c'est la liaison manquante entre l'URL signée (asset + client)
/// et la lecture réelle (job de la guilde). Le legacy refuse avec `403
/// forbidden` quand aucun job de la guilde ne porte sur le média.
pub(super) async fn playback_access_exists(
    db: &lc_db::Db,
    asset_id: MediaAssetId,
    guild_id: &GuildId,
) -> lc_db::DbResult<bool> {
    let found = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(\
             SELECT 1 FROM playback_jobs \
              WHERE media_asset_id = ? AND guild_id = ?\
         )",
    )
    .bind(asset_id.to_canonical())
    .bind(guild_id.as_str())
    .fetch_one(db.pool())
    .await
    .map_err(lc_db::DbError::from)?;
    Ok(found != 0)
}

/// Réponse `403 {"error":"forbidden"}` (accès média hors job de la guilde).
pub(super) fn forbidden() -> Response {
    error(StatusCode::FORBIDDEN, "forbidden")
}

/// Réponse `403 {"error":"media_url_invalid"}`.
pub(super) fn media_url_invalid() -> Response {
    error(StatusCode::FORBIDDEN, "media_url_invalid")
}

/// Réponse `403 {"error":"media_url_expired"}`.
pub(super) fn media_url_expired() -> Response {
    error(StatusCode::FORBIDDEN, "media_url_expired")
}

/// Réponse `404 {"error":"media_not_found"}`.
pub(super) fn media_not_found() -> Response {
    error(StatusCode::NOT_FOUND, "media_not_found")
}

/// Réponse `404 {"error":"media_not_found_on_disk"}`.
pub(super) fn media_not_found_on_disk() -> Response {
    error(StatusCode::NOT_FOUND, "media_not_found_on_disk")
}

/// Construit une URL média signée au format protocole v2.
///
/// Fonction pure réutilisable par le hub WebSocket (Phase 4b, protocole v2 §8)
/// pour ne pas dupliquer la logique de signature : l'appelant fournit le secret,
/// la base d'API et l'instant d'expiration (calculé via
/// `crate::media_url::expiry_for`). Le format produit est exactement :
///
/// ```text
/// {apiUrl}/overlay/media/{assetId}?clientId={clientId}&exp={expSecs}&sig={hex}
/// ```
///
/// Cette même forme est celle vérifiée par [`get_media`], ce qui garantit un
/// aller-retour signé/vérifié identique côté hub et côté REST.
pub fn sign_media_url(
    secret: &[u8],
    api_url: &str,
    asset_id: &MediaAssetId,
    client_id: &OverlayClientId,
    exp_sec: u64,
) -> Result<String, crate::media_url::MediaUrlError> {
    let asset_key = asset_id.to_canonical();
    let client_key = client_id.to_canonical();
    let signature = media_url::sign(secret, &asset_key, &client_key, exp_sec)?;
    Ok(format!(
        "{}/overlay/media/{}?clientId={}&exp={}&sig={}",
        api_url.trim_end_matches('/'),
        asset_key,
        client_key,
        exp_sec,
        signature
    ))
}

/// Résout le fichier d'un asset sous `MEDIA_STORAGE_DIR`.
///
/// Le chemin stocké peut être absolu (pipeline média) ou relatif au répertoire
/// courant du serveur ; dans les deux cas on canonicalise puis on vérifie le
/// confinement sous `MEDIA_STORAGE_DIR` pour empêcher toute traversée.
pub(super) async fn resolve_media_file(
    state: &AppState,
    asset: &MediaAsset,
) -> Result<PathBuf, Response> {
    if asset.storage_path.trim().is_empty() {
        return Err(media_not_found_on_disk());
    }
    let raw = PathBuf::from(&asset.storage_path);
    let candidate = if raw.is_absolute() {
        raw
    } else {
        let Ok(cwd) = std::env::current_dir() else {
            return Err(media_not_found_on_disk());
        };
        cwd.join(raw)
    };
    let canonical = match tokio::fs::canonicalize(&candidate).await {
        Ok(path) => path,
        Err(error) => {
            warn!(path = %candidate.display(), error = %error, "fichier média introuvable");
            return Err(media_not_found_on_disk());
        }
    };
    match tokio::fs::canonicalize(&state.config.media_storage_dir).await {
        Ok(base) => {
            if !canonical.starts_with(&base) {
                warn!(
                    path = %canonical.display(),
                    base = %base.display(),
                    "fichier média hors du répertoire autorisé"
                );
                return Err(media_not_found_on_disk());
            }
        }
        Err(error) => {
            warn!(
                base = %state.config.media_storage_dir.display(),
                error = %error,
                "racine média non canonicalisable : confinement non vérifiable, accès refusé"
            );
            return Err(media_not_found_on_disk());
        }
    }
    Ok(canonical)
}

/// Sert un fichier média avec `Range`/`HEAD` (mise en forme legacy conservée).
///
/// Utilisé par la route média signée et par le média du meme board (protégé
/// Bearer). Le corps de `ServeFile` est converti en `axum::body::Body`.
pub(super) async fn serve_file(
    asset: &MediaAsset,
    file_path: &std::path::Path,
    method: Method,
    range: Option<&HeaderValue>,
) -> Response {
    let mut builder = axum::http::Request::builder().method(method).uri("/");
    if let Some(value) = range {
        builder = builder.header(header::RANGE, value.clone());
    }
    let serve_request = match builder.body(Body::empty()) {
        Ok(request) => request,
        Err(_) => return internal_error(),
    };

    let response = match ServeFile::new(file_path).oneshot(serve_request).await {
        Ok(response) => response,
        Err(_) => return internal_error(),
    };
    let (parts, body) = response.into_parts();
    let mut response = Response::from_parts(parts, Body::new(body));

    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&asset.mime)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response
}

/// Rafraîchit `last_accessed_at` **sans** prolonger `expires_at`.
///
/// AUD-A2-005 : le legacy ne prolonge pas la rétention cache à chaque lecture
/// d'un média servi ; prolonger l'échéance à chaque GET permettait à un client
/// de garder un asset indéfiniment (et cassait l'épinglage board, un asset
/// épinglé ne devant jamais expirer). Best-effort, jamais bloquant.
pub(super) async fn touch_asset(
    state: &AppState,
    asset: &MediaAsset,
    timestamp: chrono::DateTime<chrono::Utc>,
) {
    let Some(db) = state.db.as_ref() else {
        return;
    };
    if let Err(db_error) = sqlx::query("UPDATE media_assets SET last_accessed_at = ? WHERE id = ?")
        .bind(lc_domain::time::datetime_to_ms(timestamp))
        .bind(asset.id.to_canonical())
        .execute(db.pool())
        .await
        .map_err(lc_db::DbError::from)
    {
        warn!(asset_id = %asset.id, error = %db_error, "touch média impossible");
    }
}
