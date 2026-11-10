//! Routes `/overlay/meme-board/*` (Bearer obligatoire).
//!
//! Parité REST avec `server/src/components/overlay/overlayRoutes.ts:376-567` et
//! `server/src/services/memeBoard.ts`. Le meme board est une liste ordonnée de
//! médias épinglés appartenant à la guilde du client overlay authentifié.
//!
//! ## Routes
//!
//! | Méthode | Chemin | Réponse |
//! |---------|--------|---------|
//! | `GET` | `/overlay/meme-board/items?q=&limit=&offset=` | `{total, limit, offset, items}` |
//! | `POST` | `/overlay/meme-board/items` | `{created, item}` |
//! | `GET` | `/overlay/meme-board/media/{assetId}` | flux binaire (`Range` supporté) |
//! | `PATCH` | `/overlay/meme-board/items/{itemId}` | `{updated:true, item:{id,title,message}}` |
//! | `DELETE` | `/overlay/meme-board/items/{itemId}` | `{deleted}` |
//!
//! ## Réponses exactes
//!
//! - item : `{id, guildId, mediaAssetId, title, message, createdByName, createdAt,
//!   media:{id,kind,mime,durationSec,width,height,isVertical,sizeBytes,sourceUrl}}`
//!   (`createdAt` en ISO-8601 UTC millisecondes, comme le JSON du legacy) ;
//! - erreurs : `400 invalid_payload` (+ `message`), `401 unauthorized`,
//!   `403 read_only_session` (session invitée en écriture), `404 item_not_found`
//!   / `media_not_found` / `media_not_found_on_disk`,
//!   `422 media_ingestion_failed` (+ `code`, `message`), `500 internal_error`.
//!
//! ## Sessions en lecture seule (`INVITE_READ_ONLY`)
//!
//! Le protocole v2 (§9) réserve la session `invite_read_only` à la lecture. Les
//! mutations du meme board (`POST items`, `PATCH item`, `DELETE item`) sont donc
//! refusées avec `403 {"error":"read_only_session"}` **avant toute validation de
//! charge utile**, tandis que les `GET` (`items`, `media`) restent autorisés.
//! Le mode provient de l'extracteur [`AuthedOverlayClient::is_read_only`]
//! (colonne `overlay_clients.session_mode`, chargée à l'authentification) ; il
//! est identique que le jeton arrive par `Authorization: Bearer` ou par le repli
//! `?token=` du board, et ne peut pas être influencé par le corps de requête.
//!
//! ## Écarts assumés (documentés dans `docs/rust/server-overlay.md`)
//!
//! - l'ingestion média est faite **en ligne** via `lc_media::MediaPipeline`
//!   (Phase 5b la déplacera dans un worker) : le handler bloque pendant le
//!   téléchargement/transcodage ;
//! - la recherche `q` est filtrée en Rust (le repository n'expose pas de
//!   `search`) : coût acceptable pour une board ≤ 200 items ;
//! - la libération de l'expiration épinglée après suppression compte les
//!   références via `GuildsRepository::list` (pas de `count` dédié dans
//!   `lc-db`) ; un item orphelin de guilde inconnue n'est pas compté (écart
//!   mineur, Phase 5b) ;
//! - `guildId` en query (legacy admin) est accepté uniquement s'il correspond à
//!   la guilde du Bearer, sinon `403 forbidden` (défense en profondeur).

use std::time::Duration as StdDuration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{DateTime, Utc};
use lc_domain::{MediaAsset, MediaAssetId, MediaStatus, MemeBoardItem, MemeBoardItemId};
use lc_media::{MediaConfig, MediaPipeline};
use serde::Deserialize;

use crate::app::AppState;

use super::auth::AuthedOverlayClient;
use super::{db, db_failure, error, internal_error, json_response, now};

/// Alias local pour garder les signatures courtes.
type Db = lc_db::Db;

/// Durée d'épinglage d'un média de board (legacy : `now + 100 ans`).
const PINNED_YEARS: i64 = 100;
/// Limite legacy de la longueur du titre.
const TITLE_MAX_CHARS: usize = 240;
/// Limite legacy de la longueur du message.
const MESSAGE_MAX_CHARS: usize = 500;
/// Limite legacy du nom d'auteur.
const AUTHOR_NAME_MAX_CHARS: usize = 120;
/// Taille d'un mébioctet.
const MIB: u64 = 1024 * 1024;

/// Query de liste (tout optionnel, parité legacy `q/limit/offset`).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListQuery {
    /// Recherche plein texte sur titre/message/auteur/URL source.
    #[serde(default)]
    q: Option<String>,
    /// Taille de page demandée (défaut 50, clamp 1..=200).
    #[serde(default)]
    limit: Option<String>,
    /// Décalage (défaut 0).
    #[serde(default)]
    offset: Option<String>,
    /// Ancien filtre admin : doit correspondre à la guilde du Bearer.
    #[serde(default)]
    guild_id: Option<String>,
}

/// Corps `POST /meme-board/items`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateItemRequest {
    /// URL source à ingérer.
    #[serde(default)]
    url: Option<String>,
    /// Titre affiché (≤ 240 caractères).
    #[serde(default)]
    title: Option<String>,
    /// Message affiché (≤ 500 caractères).
    #[serde(default)]
    message: Option<String>,
    /// Force une nouvelle ingestion même si le média est déjà en base.
    #[serde(default)]
    force_refresh: Option<serde_json::Value>,
}

/// Handler `GET /overlay/meme-board/items`.
pub async fn list_items(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Query(query): Query<ListQuery>,
) -> Response {
    if let Some(requested) = query
        .guild_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if requested != auth.guild_id().as_str() {
            return error(StatusCode::FORBIDDEN, "forbidden");
        }
    }
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let limit = normalize_limit(query.limit.as_deref());
    let offset = normalize_offset(query.offset.as_deref());
    let search = query
        .q
        .as_deref()
        .map(|value| truncate_chars(value.trim(), TITLE_MAX_CHARS).to_lowercase())
        .filter(|value| !value.is_empty());

    let items = match db.meme_board().list_by_guild(auth.guild_id()).await {
        Ok(items) => items,
        Err(db_error) => return db_failure("overlay_meme_board.list_items", &db_error),
    };

    let mut rows: Vec<(MemeBoardItem, MediaAsset)> = Vec::with_capacity(items.len());
    for item in items {
        let asset = match db.media_assets().get(item.media_asset_id).await {
            Ok(Some(asset)) => asset,
            Ok(None) => continue,
            Err(db_error) => {
                return db_failure("overlay_meme_board.list_item_asset", &db_error);
            }
        };
        if let Some(search) = search.as_deref() {
            if !item_matches(&item, &asset, search) {
                continue;
            }
        }
        rows.push((item, asset));
    }

    let total = rows.len();
    let page = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(item, asset)| item_payload(&item, &asset))
        .collect::<Vec<_>>();

    json_response(
        StatusCode::OK,
        serde_json::json!({
            "total": total,
            "limit": limit,
            "offset": offset,
            "items": page,
        }),
    )
}

/// Handler `POST /overlay/meme-board/items`.
pub async fn create_item(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Json(payload): Json<CreateItemRequest>,
) -> Response {
    // Garde de session : refus avant validation/ingestion (403 read_only_session).
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let url = payload
        .url
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    if url.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "invalid_payload", "message": "url is required"}),
        );
    }
    let force_refresh = coerce_flag(payload.force_refresh.as_ref());

    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now = now(&state);

    let pipeline = match MediaPipeline::new(media_config(&state)) {
        Ok(pipeline) => pipeline,
        Err(media_error) => return media_ingestion_error(&media_error),
    };
    let ingested = match pipeline.ingest_url(&url).await {
        Ok(ingested) => ingested,
        Err(media_error) => return media_ingestion_error(&media_error),
    };

    let pinned_expiry = pinned_expiry(now);
    let asset = match db
        .media_assets()
        .get_by_source_hash(&ingested.source.source_hash)
        .await
    {
        Ok(Some(mut existing)) if !force_refresh && existing.is_ready() => {
            if existing.expires_at < pinned_expiry {
                if let Err(db_error) = db
                    .media_assets()
                    .touch_last_accessed(existing.id, now, pinned_expiry)
                    .await
                {
                    return db_failure("overlay_meme_board.pin_existing_asset", &db_error);
                }
                existing.expires_at = pinned_expiry;
                existing.last_accessed_at = now;
            }
            existing
        }
        Ok(Some(mut existing)) => {
            // Cache existant mais non prêt : on le repasse READY (les
            // métadonnées seront re-validées par la Phase 5b).
            if let Err(db_error) = db
                .media_assets()
                .update_status(existing.id, MediaStatus::Ready, None)
                .await
            {
                return db_failure("overlay_meme_board.mark_asset_ready", &db_error);
            }
            existing.status = MediaStatus::Ready;
            existing.error = None;
            existing.expires_at = pinned_expiry;
            existing.last_accessed_at = now;
            existing
        }
        Ok(None) => {
            let asset = MediaAsset {
                id: MediaAssetId::new_v4(),
                source_hash: ingested.source.source_hash.clone(),
                source_url: ingested.source.canonical_url.clone(),
                kind: ingested.normalized.kind,
                mime: ingested.normalized.mime.clone(),
                duration_sec: ingested.normalized.duration_sec,
                width: ingested.normalized.width.map(i64::from),
                height: ingested.normalized.height.map(i64::from),
                is_vertical: ingested.normalized.is_vertical,
                storage_path: ingested
                    .normalized
                    .storage_path
                    .to_string_lossy()
                    .into_owned(),
                size_bytes: i64::try_from(ingested.normalized.size_bytes).unwrap_or(i64::MAX),
                status: MediaStatus::Ready,
                error: None,
                last_accessed_at: now,
                expires_at: pinned_expiry,
                created_at: now,
            };
            match db.media_assets().insert(&asset).await {
                Ok(()) => asset,
                Err(db_error) if db_error.is_unique_violation() => match db
                    .media_assets()
                    .get_by_source_hash(&ingested.source.source_hash)
                    .await
                {
                    Ok(Some(existing)) => existing,
                    Ok(None) => return internal_error(),
                    Err(db_error) => {
                        return db_failure("overlay_meme_board.reload_asset", &db_error);
                    }
                },
                Err(db_error) => return db_failure("overlay_meme_board.insert_asset", &db_error),
            }
        }
        Err(db_error) => return db_failure("overlay_meme_board.asset_by_source_hash", &db_error),
    };

    // Dédoublonnage board : (guilde, asset) déjà épinglé → `created:false`.
    match db
        .meme_board()
        .get_by_guild_and_asset(auth.guild_id(), asset.id)
        .await
    {
        Ok(Some(existing_item)) => {
            return json_response(
                StatusCode::OK,
                serde_json::json!({"created": false, "item": item_payload(&existing_item, &asset)}),
            );
        }
        Ok(None) => {}
        Err(db_error) => return db_failure("overlay_meme_board.board_lookup", &db_error),
    }

    // Quota de stockage de la board (legacy `ensureGuildBoardHasCapacity`).
    let used = match board_total_bytes(db, auth.guild_id()).await {
        Ok(used) => used,
        Err(db_error) => return db_failure("overlay_meme_board.board_usage", &db_error),
    };
    let limit_bytes = i64::try_from(
        state
            .config
            .media_board_max_total_mb
            .saturating_mul(MIB)
            .max(MIB),
    )
    .unwrap_or(i64::MAX);
    if used.saturating_add(asset.size_bytes) > limit_bytes {
        return json_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            serde_json::json!({
                "error": "media_ingestion_failed",
                "code": "BOARD_STORAGE_LIMIT_REACHED",
                "message": format!(
                    "la board de la guilde dépasserait la limite de {} Mo",
                    state.config.media_board_max_total_mb
                ),
            }),
        );
    }

    let item = MemeBoardItem {
        id: MemeBoardItemId::new_v4(),
        guild_id: auth.guild_id().clone(),
        media_asset_id: asset.id,
        title: payload
            .title
            .as_deref()
            .map(|value| truncate_chars(value.trim(), TITLE_MAX_CHARS))
            .filter(|value| !value.is_empty()),
        message: payload
            .message
            .as_deref()
            .map(|value| truncate_chars(value.trim(), MESSAGE_MAX_CHARS))
            .filter(|value| !value.is_empty()),
        created_by_discord_user_id: auth.client.created_by_discord_user_id.clone(),
        created_by_name: board_author_name(&auth),
        created_at: now,
    };
    match db.meme_board().create(&item).await {
        Ok(()) => json_response(
            StatusCode::OK,
            serde_json::json!({"created": true, "item": item_payload(&item, &asset)}),
        ),
        Err(db_error) if db_error.is_unique_violation() => match db
            .meme_board()
            .get_by_guild_and_asset(auth.guild_id(), asset.id)
            .await
        {
            Ok(Some(existing_item)) => json_response(
                StatusCode::OK,
                serde_json::json!({"created": false, "item": item_payload(&existing_item, &asset)}),
            ),
            Ok(None) => internal_error(),
            Err(db_error) => db_failure("overlay_meme_board.reload_item", &db_error),
        },
        Err(db_error) => db_failure("overlay_meme_board.insert_item", &db_error),
    }
}

/// Handler `GET /overlay/meme-board/media/{assetId}`.
pub async fn get_media(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(asset_id_raw): Path<String>,
    method: axum::http::Method,
    headers: HeaderMap,
) -> Response {
    let Ok(asset_id) = asset_id_raw.trim().parse::<MediaAssetId>() else {
        return super::media::media_not_found();
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    match db
        .meme_board()
        .get_by_guild_and_asset(auth.guild_id(), asset_id)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return super::media::media_not_found(),
        Err(db_error) => return db_failure("overlay_meme_board.media_binding", &db_error),
    }
    let asset = match db.media_assets().get(asset_id).await {
        Ok(Some(asset)) => asset,
        Ok(None) => return super::media::media_not_found(),
        Err(db_error) => return db_failure("overlay_meme_board.media_asset", &db_error),
    };
    if !asset.is_ready() || asset.is_expired_at(now(&state)) {
        return super::media::media_not_found();
    }
    let file_path = match super::media::resolve_media_file(&state, &asset).await {
        Ok(path) => path,
        Err(response) => return response,
    };
    let range = headers.get(axum::http::header::RANGE).cloned();
    super::media::serve_file(&asset, &file_path, method, range.as_ref()).await
}

/// Handler `PATCH /overlay/meme-board/items/{itemId}`.
pub async fn update_item(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(item_id_raw): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    // Garde de session : aucune écriture en INVITE_READ_ONLY.
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let Some(object) = body.as_object() else {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "invalid_payload", "message": "title or message is required"}),
        );
    };
    let has_title = object.contains_key("title");
    let has_message = object.contains_key("message");
    if !has_title && !has_message {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "invalid_payload", "message": "title or message is required"}),
        );
    }
    let Ok(item_id) = item_id_raw.trim().parse::<MemeBoardItemId>() else {
        return error(StatusCode::NOT_FOUND, "item_not_found");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let existing = match db.meme_board().get(item_id).await {
        Ok(Some(item)) if &item.guild_id == auth.guild_id() => item,
        Ok(_) => return error(StatusCode::NOT_FOUND, "item_not_found"),
        Err(db_error) => return db_failure("overlay_meme_board.patch_lookup", &db_error),
    };

    // Valeur absente → on conserve ; null/vide → on efface (parité legacy
    // `hasOwnProperty` + `toNonEmptyString`).
    let title = if has_title {
        normalize_optional_text(object.get("title"), TITLE_MAX_CHARS)
    } else {
        existing.title.clone()
    };
    let message = if has_message {
        normalize_optional_text(object.get("message"), MESSAGE_MAX_CHARS)
    } else {
        existing.message.clone()
    };

    match db
        .meme_board()
        .update(item_id, title.as_deref(), message.as_deref())
        .await
    {
        Ok(true) => json_response(
            StatusCode::OK,
            serde_json::json!({
                "updated": true,
                "item": {
                    "id": existing.id.to_canonical(),
                    "title": title,
                    "message": message,
                },
            }),
        ),
        Ok(false) => error(StatusCode::NOT_FOUND, "item_not_found"),
        Err(db_error) => db_failure("overlay_meme_board.patch_update", &db_error),
    }
}

/// Handler `DELETE /overlay/meme-board/items/{itemId}`.
pub async fn delete_item(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(item_id_raw): Path<String>,
) -> Response {
    // Garde de session : aucune suppression en INVITE_READ_ONLY.
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let Ok(item_id) = item_id_raw.trim().parse::<MemeBoardItemId>() else {
        return json_response(StatusCode::OK, serde_json::json!({"deleted": false}));
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let existing = match db.meme_board().get(item_id).await {
        Ok(Some(item)) if &item.guild_id == auth.guild_id() => item,
        Ok(_) => return json_response(StatusCode::OK, serde_json::json!({"deleted": false})),
        Err(db_error) => return db_failure("overlay_meme_board.delete_lookup", &db_error),
    };
    let deleted = match db.meme_board().delete(item_id).await {
        Ok(deleted) => deleted,
        Err(db_error) => return db_failure("overlay_meme_board.delete", &db_error),
    };

    if deleted {
        // Libération de l'expiration épinglée si plus aucun board ne référence
        // ce média (parité legacy `releasePinnedExpiry`).
        match board_reference_count(db, &existing.media_asset_id).await {
            Ok(0) => {
                let now = now(&state);
                let released_expiry = now
                    + StdDuration::from_secs(
                        state.config.media_cache_ttl_hours.saturating_mul(3_600),
                    );
                if let Err(db_error) = db
                    .media_assets()
                    .touch_last_accessed(existing.media_asset_id, now, released_expiry)
                    .await
                {
                    tracing::warn!(error = %db_error, "libération de l'expiration épinglée impossible");
                }
            }
            Ok(_) => {}
            Err(db_error) => {
                tracing::warn!(error = %db_error, "comptage des références board impossible");
            }
        }
    }

    json_response(StatusCode::OK, serde_json::json!({"deleted": deleted}))
}

/// Garde d'écriture : refuse les mutations en session `INVITE_READ_ONLY`.
///
/// Le mode est celui du client authentifié (persisté en
/// `overlay_clients.session_mode`), quelle que soit la voie d'authentification
/// (`Authorization: Bearer` ou repli `?token=` du board). Les erreurs du legacy
/// (`400`, `404`) ne sont volontairement pas évaluées avant cette garde : une
/// session invitée ne doit obtenir aucune information sur la validité d'une
/// charge utile ou d'un identifiant.
fn ensure_writable(auth: &AuthedOverlayClient) -> Result<(), Response> {
    if auth.is_read_only() {
        return Err(error(StatusCode::FORBIDDEN, "read_only_session"));
    }
    Ok(())
}

/// Payload JSON d'un item de board (format legacy exact).
fn item_payload(item: &MemeBoardItem, asset: &MediaAsset) -> serde_json::Value {
    serde_json::json!({
        "id": item.id.to_canonical(),
        "guildId": item.guild_id.as_str(),
        "mediaAssetId": item.media_asset_id.to_canonical(),
        "title": item.title,
        "message": item.message,
        "createdByName": item.created_by_name,
        "createdAt": item.created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "media": {
            "id": asset.id.to_canonical(),
            "kind": asset.kind.as_wire_str(),
            "mime": asset.mime,
            "durationSec": asset.duration_sec,
            "width": asset.width,
            "height": asset.height,
            "isVertical": asset.is_vertical,
            "sizeBytes": asset.size_bytes,
            "sourceUrl": asset.source_url,
        },
    })
}

/// Nom d'auteur affiché d'un client overlay (repli label legacy).
fn board_author_name(auth: &AuthedOverlayClient) -> Option<String> {
    auth.client
        .default_author_name
        .as_deref()
        .or(Some(auth.client.label.as_str()))
        .map(|value| truncate_chars(value.trim(), AUTHOR_NAME_MAX_CHARS))
        .filter(|value| !value.is_empty())
}

/// Normalise `limit` (défaut 50, clamp 1..=200, parité legacy).
fn normalize_limit(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<i64>().ok())
        .map(|value| value.clamp(1, 200) as usize)
        .unwrap_or(50)
}

/// Normalise `offset` (défaut 0, jamais négatif, parité legacy).
fn normalize_offset(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<i64>().ok())
        .map(|value| value.max(0) as usize)
        .unwrap_or(0)
}

/// Filtre `q` (insensible à la casse) sur les champs exposés par le legacy.
fn item_matches(item: &MemeBoardItem, asset: &MediaAsset, search: &str) -> bool {
    let fields = [
        item.title.as_deref(),
        item.message.as_deref(),
        item.created_by_name.as_deref(),
        Some(asset.source_url.as_str()),
    ];
    fields
        .into_iter()
        .flatten()
        .any(|value| value.to_lowercase().contains(search))
}

/// Somme des tailles des médias actuellement référencés par une board.
async fn board_total_bytes(db: &Db, guild_id: &lc_domain::GuildId) -> lc_db::DbResult<i64> {
    let mut total: i64 = 0;
    for item in db.meme_board().list_by_guild(guild_id).await? {
        if let Some(asset) = db.media_assets().get(item.media_asset_id).await? {
            total = total.saturating_add(asset.size_bytes.max(0));
        }
    }
    Ok(total)
}

/// Nombre de références board à un asset (toutes guildes connues).
async fn board_reference_count(db: &Db, asset_id: &MediaAssetId) -> lc_db::DbResult<usize> {
    let mut count = 0_usize;
    for guild in db.guilds().list().await? {
        for item in db.meme_board().list_by_guild(&guild.id).await? {
            if item.media_asset_id == *asset_id {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Construit la configuration média depuis la configuration serveur.
fn media_config(state: &AppState) -> MediaConfig {
    let mut config = MediaConfig::default();
    config.storage.directory = state.config.media_storage_dir.clone();
    config.storage.cache_ttl =
        StdDuration::from_secs(state.config.media_cache_ttl_hours.saturating_mul(3_600));
    config.storage.board_max_total_bytes =
        state.config.media_board_max_total_mb.saturating_mul(MIB);
    config
}

/// Réponse `422 media_ingestion_failed` (format API legacy).
fn media_ingestion_error(media_error: &lc_media::MediaError) -> Response {
    json_response(
        StatusCode::UNPROCESSABLE_ENTITY,
        serde_json::json!({
            "error": "media_ingestion_failed",
            "code": media_error.code().as_str(),
            "message": media_error.message(),
        }),
    )
}

/// Tronque proprement (frontières UTF-8) après trim.
fn truncate_chars(raw: &str, max: usize) -> String {
    raw.trim()
        .chars()
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Normalise un champ texte optionnel : null/vide → `None`, sinon tronqué.
fn normalize_optional_text(value: Option<&serde_json::Value>, max: usize) -> Option<String> {
    match value {
        Some(serde_json::Value::String(raw)) => {
            let trimmed = truncate_chars(raw, max);
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        _ => None,
    }
}

/// Interprète `forceRefresh` (booléen JSON, `"true"`/`"1"` legacy).
fn coerce_flag(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(flag)) => *flag,
        Some(serde_json::Value::String(raw)) => {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes"
            )
        }
        Some(serde_json::Value::Number(number)) => number.as_i64().is_some_and(|value| value != 0),
        _ => false,
    }
}

/// Expiration « épinglée » d'un média de board (legacy : +100 ans).
fn pinned_expiry(now: DateTime<Utc>) -> DateTime<Utc> {
    now + chrono::Duration::days(365 * PINNED_YEARS)
}
