//! Routes `/overlay/media-board/*`.
//!
//! Parité REST avec `server/src/components/overlay/overlayRoutes.ts:376-567` et
//! `server/src/services/mediaBoard.ts`. Le board média est une liste ordonnée de
//! médias épinglés appartenant à la guilde du client overlay authentifié.
//!
//! ## Routes
//!
//! | Méthode | Chemin | Authentification | Réponse |
//! |---------|--------|------------------|---------|
//! | `GET` | `/overlay/media-board/items?q=&limit=&offset=` | Bearer | `{total, limit, offset, items}` |
//! | `POST` | `/overlay/media-board/items` | Bearer | `202 {ingestionId}` |
//! | `POST` | `/overlay/media-board/uploads` (+ `PUT …/{uploadId}/parts`, `POST …/{uploadId}/complete`) | Bearer | `202 {ingestionId}` on completion (see `media_upload`) |
//! | `POST` | `/overlay/media-board/items/{itemId}/edit` | Bearer | `202 {ingestionId}` (see [`super::media_edit`]) |
//! | `POST` | `/overlay/media-board/import` | Bearer + `X-LiveChat-Source-Token` | `{results:[...]}` (see [`super::media_import`]) |
//! | `GET` | `/overlay/media-board/ingestions` | Bearer | `{items:[...]}` |
//! | `GET` | `/overlay/media-board/media/{assetId}?clientId=&exp=&sig=` | URL signée | flux binaire (`Range` supporté) |
//! | `PATCH` | `/overlay/media-board/items/{itemId}` | Bearer | `{updated:true, item:{id,title,message}}` |
//! | `DELETE` | `/overlay/media-board/items/{itemId}` | Bearer | `{deleted}` |
//!
//! ## Asynchronous ingestion
//!
//! `POST items`, `POST upload` and `POST items/{itemId}/edit` validate the
//! request, register an ingestion in [`crate::ingestions::IngestionRegistry`]
//! and answer `202 {ingestionId}`. The download/transcode/edit then runs in a
//! detached Tokio task, so a client disconnection (Cloudflare's 100 s limit,
//! Nginx's 90 s limit) can no longer cancel it. When the task finishes it
//! updates the registry, sends `media.ingestion` to every socket of the
//! requesting overlay client and, for a new item, broadcasts `board.changed
//! {change:"added"}` to the guild.
//!
//! An upload with `replaceItemId`, or an edit with `replace:true`, replaces the
//! media of an existing item instead of creating one: the item keeps its id,
//! title, message, author, date, favorites and picture. Its result is
//! `media.ingestion {ok:true, created:false, itemId}` and `board.changed
//! {change:"updated"}`.
//!
//! ## Réponses exactes
//!
//! - item : `{id, guildId, mediaAssetId, title, message, createdByName, createdAt,
//!   mediaUrl, mediaUrlExpiresAt,
//!   media:{id,kind,mime,durationSec,width,height,isVertical,sizeBytes,sourceUrl}}`
//!   (`createdAt` en ISO-8601 UTC millisecondes, comme le JSON du legacy).
//!   `mediaUrl` est une URL absolue signée une heure pour le client demandeur
//!   (`board:{assetId}`), utilisable verbatim par `<img>`/`<video>` ;
//! - erreurs synchrones : `400 invalid_payload` (+ `message`), `401 unauthorized`,
//!   `403 read_only_session` (session invitée en écriture), `404 item_not_found`
//!   / `media_not_found` / `media_not_found_on_disk`, `413 payload_too_large`,
//!   `429 too_many_ingestions`, `500 internal_error` ;
//! - les échecs d'ingestion (`media_ingestion_failed`, `unsupported_media`,
//!   `internal_error`) sont envoyés après coup par `media.ingestion`, jamais
//!   comme réponse de la requête `202`.
//!
//! ## Sessions en lecture seule (`INVITE_READ_ONLY`)
//!
//! Le protocole v2 (§9) réserve la session `invite_read_only` à la lecture. Les
//! mutations du board média (`POST items`, `POST upload`, `POST item/edit`,
//! `PATCH item`, `DELETE item`) sont donc refusées avec `403 {"error":"read_only_session"}`
//! **avant toute validation de charge utile**, tandis que les `GET` (`items`,
//! `ingestions`, `media`) restent autorisés.
//! Le mode provient de l'extracteur [`AuthedOverlayClient::is_read_only`]
//! (colonne `overlay_clients.session_mode`, chargée à l'authentification) et ne
//! peut pas être influencé par le corps de requête. Le flux média signé applique
//! la même règle de visibilité à partir du rôle du client.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use lc_domain::rights::{Action, Surface};
use lc_domain::{
    GuildId, GuildRole, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId, MediaKind,
    OverlayClientId, Permission,
};
use lc_protocol::{BoardChange, BoardChangedPayload, ServerMessage};
use serde::Deserialize;
use serde_json::Value;
use time::UtcDateTime;

use crate::app::AppState;
use crate::ingestions::{IngestionEntry, StartIngestionError};
use crate::media::assets;
use crate::media::errors::MediaServiceError;

use super::auth::AuthedOverlayClient;
use super::guild::require;
use super::media_ingestion::{spawn_ingestion, BoardTarget, IngestionInput};
use super::{
    audit as overlay_audit, db, db_failure, error, is_conflict, json_response, now, ActorInfo,
};

/// Alias local pour garder les signatures courtes.
type Db = lc_db::Db;

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
pub(crate) struct ListQuery {
    /// Recherche plein texte sur titre/message/auteur/URL source.
    #[serde(default)]
    pub(crate) q: Option<String>,
    /// Taille de page demandée (défaut 50, clamp 1..=200).
    #[serde(default)]
    pub(crate) limit: Option<String>,
    /// Décalage (défaut 0).
    #[serde(default)]
    pub(crate) offset: Option<String>,
    /// Ancien filtre admin : doit correspondre à la guilde du Bearer.
    #[serde(default)]
    pub(crate) guild_id: Option<String>,
}

/// Paramètres de l'URL signée d'un média du board.
///
/// La route n'accepte ni `Authorization: Bearer` ni `?token=` : les tags
/// `<img>`/`<video>` ne peuvent pas porter d'en-tête, seule la signature HMAC
/// authentifie la requête.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoardMediaQuery {
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

/// Corps `POST /media-board/items`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateItemRequest {
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
    /// `true` : le média est joué une fois sur les overlays de la guilde sans
    /// créer d'item de board (droit `TriggerPlayback` au lieu d'`AddMedia`).
    #[serde(default)]
    play_once: bool,
}

/// Board list shared by `GET /overlay/media-board/items` and the admin console.
///
/// Signed media URLs are bound to the actor's overlay client; an admin request
/// therefore uses one of the guild's active clients when it has one.
pub(crate) async fn list_items_payload(
    state: &AppState,
    actor: &ActorInfo,
    query: &ListQuery,
) -> Result<Value, Response> {
    let db = db(state)?;
    ensure_board_visible_for(state, db, actor.guild_id(), actor.role()).await?;

    let secret = state.config.media_url_secret.expose_secret();
    if secret.is_empty() {
        tracing::warn!("MEDIA_URL_SECRET absent : URLs du board non signables");
        return Err(super::internal_error());
    }
    let expires_at_sec =
        crate::media_url::board_url_expiry(u64::try_from(now(state).unix_timestamp()).unwrap_or(0));
    let client_id = actor.client.id.to_canonical();

    let limit = normalize_limit(query.limit.as_deref());
    let offset = normalize_offset(query.offset.as_deref());
    let search = query
        .q
        .as_deref()
        .map(|value| truncate_chars(value.trim(), TITLE_MAX_CHARS).to_lowercase())
        .filter(|value| !value.is_empty());

    // AUD-A2-013 : une seule requête pour lire les lignes de la board, au lieu
    // de `list_by_guild` suivi d'un `media_assets().get` par item (N+1 non
    // borné). La recherche et la pagination restent en Rust (board de quelques
    // centaines d'items) ; seuls les assets de la page (≤ 200) sont rechargés.
    let rows = load_board_rows(db, actor.guild_id())
        .await
        .map_err(|db_error| db_failure("overlay_media_board.list_items", &db_error))?;
    let filtered = rows
        .into_iter()
        .filter(|row| {
            search
                .as_deref()
                .map(|needle| row_matches(row, needle))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    let total = filtered.len();
    let favorites: std::collections::HashSet<MediaBoardItemId> = match actor.member_id() {
        Some(member_id) => db
            .media_board()
            .favorite_item_ids(member_id)
            .await
            .map_err(|db_error| db_failure("overlay_media_board.favorites", &db_error))?
            .into_iter()
            .collect(),
        None => std::collections::HashSet::new(),
    };
    let sign = |asset_id: &str| {
        crate::media_url::sign_board_url(
            secret.as_bytes(),
            &state.config.api_origin(),
            asset_id,
            &client_id,
            expires_at_sec,
        )
    };
    let mut page = Vec::new();
    for row in filtered.into_iter().skip(offset).take(limit) {
        let asset = match db.media_assets().get(row.media_asset_id()).await {
            Ok(Some(asset)) => asset,
            Ok(None) => continue,
            Err(db_error) => {
                return Err(db_failure("overlay_media_board.list_item_asset", &db_error));
            }
        };
        let asset_id = asset.id.to_canonical();
        let media_url = match sign(&asset_id) {
            Ok(url) => url,
            Err(error) => {
                tracing::warn!(%error, "signature d'une URL du board impossible");
                return Err(super::internal_error());
            }
        };
        let artwork = match row.item.artwork_asset_id {
            Some(artwork_id) => match db.media_assets().get(artwork_id).await {
                Ok(Some(artwork)) if artwork.is_ready() => match sign(&artwork.id.to_canonical()) {
                    Ok(url) => Some(serde_json::json!({
                        "url": url,
                        "width": artwork.width,
                        "height": artwork.height,
                    })),
                    Err(error) => {
                        tracing::warn!(%error, "signature de l'image d'un média impossible");
                        None
                    }
                },
                Ok(_) => None,
                Err(db_error) => {
                    return Err(db_failure(
                        "overlay_media_board.list_item_artwork",
                        &db_error,
                    ));
                }
            },
            None => None,
        };
        // A video's thumbnail: the platform's picture of a link, fetched in
        // the background after the import (see `media::thumbnails`).
        let thumbnail_id = if asset.kind == lc_domain::MediaKind::Video {
            db.media_assets()
                .cover_of(asset.id)
                .await
                .map_err(|db_error| {
                    db_failure("overlay_media_board.list_item_thumbnail", &db_error)
                })?
        } else {
            None
        };
        let thumbnail = match thumbnail_id {
            Some(thumbnail_id) => match db.media_assets().get(thumbnail_id).await {
                Ok(Some(picture)) if picture.is_ready() => match sign(&picture.id.to_canonical()) {
                    Ok(url) => Some(serde_json::json!({
                        "url": url,
                        "width": picture.width,
                        "height": picture.height,
                    })),
                    Err(error) => {
                        tracing::warn!(%error, "signature de la miniature d'une vidéo impossible");
                        None
                    }
                },
                Ok(_) => None,
                Err(db_error) => {
                    return Err(db_failure(
                        "overlay_media_board.list_item_thumbnail",
                        &db_error,
                    ));
                }
            },
            None => None,
        };
        let mut payload = item_payload(&row.item, &asset);
        if let Some(object) = payload.as_object_mut() {
            object.insert("mediaUrl".to_owned(), serde_json::json!(media_url));
            object.insert(
                "mediaUrlExpiresAt".to_owned(),
                serde_json::json!(expires_at_sec.saturating_mul(1000)),
            );
            object.insert(
                "createdByMemberId".to_owned(),
                serde_json::json!(row.created_by_member_id),
            );
            object.insert(
                "canEdit".to_owned(),
                serde_json::json!(actor.can_edit_item(row.created_by_member_id.as_deref())),
            );
            object.insert("artwork".to_owned(), serde_json::json!(artwork));
            object.insert("thumbnail".to_owned(), serde_json::json!(thumbnail));
            object.insert(
                "artworkOnOverlay".to_owned(),
                serde_json::json!(row.item.artwork_on_overlay),
            );
            object.insert(
                "favorite".to_owned(),
                serde_json::json!(favorites.contains(&row.item.id)),
            );
        }
        page.push(payload);
    }

    Ok(serde_json::json!({
        "total": total,
        "limit": limit,
        "offset": offset,
        "items": page,
    }))
}

/// Handler `GET /overlay/media-board/items`.
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
    let actor = ActorInfo::from_auth(&state, &auth);
    match list_items_payload(&state, &actor, &query).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Handler `POST /overlay/media-board/items`.
///
/// Validates the payload, registers the ingestion and returns `202
/// {ingestionId}` immediately; the download and board insertion run in a
/// detached task (see [`super::media_ingestion`]).
pub async fn create_item(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<CreateItemRequest>, JsonRejection>,
) -> Response {
    // Garde de session : refus avant validation/ingestion (403 read_only_session).
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    // AUD-A2-009 : un corps JSON malformé est une erreur de validation dédiée
    // (`400 invalid_payload`), et non la réécriture générique `request_error`
    // appliquée par le normaliseur aux réponses non JSON d'Axum.
    let payload = match payload {
        Ok(Json(payload)) => payload,
        Err(rejection) => {
            tracing::debug!(
                error = %rejection.body_text(),
                "corps JSON invalide sur POST /overlay/media-board/items"
            );
            return json_rejection_error(rejection);
        }
    };
    // « Jouer une fois » déclenche une lecture au lieu d'ajouter au board :
    // c'est le droit d'un déclenchement manuel (`TriggerPlayback`), pas
    // `AddMedia` (403 comme les autres refus).
    if payload.play_once {
        if let Err(response) = require(&state, &auth, Permission::TriggerPlayback) {
            return response;
        }
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

    // A missing database is still reported synchronously (503): the background
    // task cannot persist the media without it.
    if let Err(response) = db(&state) {
        return response;
    }

    let ingestion_id = match state.ingestions.start(
        auth.guild_id().clone(),
        auth.client.id.as_uuid(),
        now(&state),
    ) {
        Ok(ingestion_id) => ingestion_id,
        Err(StartIngestionError::TooManyProcessing) => return too_many_ingestions(),
    };

    let target = if payload.play_once {
        BoardTarget::PlayOnce
    } else {
        BoardTarget::NewItem
    };
    spawn_ingestion(
        state,
        auth,
        ingestion_id.clone(),
        IngestionInput::Link { url, force_refresh },
        target,
        payload.title,
        payload.message,
    );

    json_response(
        StatusCode::ACCEPTED,
        serde_json::json!({ "ingestionId": ingestion_id }),
    )
}

/// `429 {"error":"too_many_ingestions"}` (20 concurrent ingestions per client).
pub(super) fn too_many_ingestions() -> Response {
    json_response(
        StatusCode::TOO_MANY_REQUESTS,
        serde_json::json!({ "error": "too_many_ingestions" }),
    )
}

/// Handler `GET /overlay/media-board/ingestions`.
///
/// Returns the requesting client's ingestions only (processing or finished less
/// than one hour ago), so a board can recover a `media.ingestion` it missed
/// while disconnected.
pub async fn list_ingestions(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    let entries = state
        .ingestions
        .list_for_client(auth.client.id.as_uuid(), now(&state));
    let items: Vec<serde_json::Value> = entries.iter().map(IngestionEntry::payload).collect();
    json_response(StatusCode::OK, serde_json::json!({ "items": items }))
}

/// Business outcome of a completed board ingestion.
#[derive(Debug)]
pub(crate) struct CompletedBoardItem {
    /// `true` when a new board row was created.
    pub created: bool,
    /// `true` when the media of an existing item was replaced.
    pub replaced: bool,
    /// Board item holding the media (new or already pinned).
    pub item: MediaBoardItem,
}

/// Titre demandé pour le nouvel item d'un board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ItemTitle {
    /// Titre saisi par le membre (lien, téléversement, édition) : laissé vide,
    /// il est remplacé par le titre source de l'asset — le vrai titre du lien.
    Requested(Option<String>),
    /// Titre d'un item copié depuis le board d'une autre guilde : conservé
    /// tel quel, même vide (l'import doit garder le titre de l'item source).
    Copied(Option<String>),
}

impl ItemTitle {
    /// Titre final de l'item, avec le repli éventuel sur le titre source.
    fn resolve(self, source_title: Option<&str>) -> Option<String> {
        let (requested, use_source_title) = match self {
            Self::Requested(title) => (title, true),
            Self::Copied(title) => (title, false),
        };
        let typed = requested
            .as_deref()
            .map(|value| truncate_chars(value.trim(), TITLE_MAX_CHARS))
            .filter(|value| !value.is_empty());
        match typed {
            Some(title) => Some(title),
            // Empty title: the link's real title, already normalized by the
            // pipeline (trimmed, whitespace collapsed, at most 240 characters).
            None if use_source_title => source_title.map(str::to_owned),
            None => None,
        }
    }
}

/// Business failure of an asynchronous ingestion (protocol vocabulary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IngestionFailure {
    /// REST/protocol machine code (`media_ingestion_failed`,
    /// `unsupported_media`, `internal_error`, ...).
    pub code: &'static str,
    /// Readable detail, when available.
    pub message: Option<String>,
}

/// Common board insertion steps after media ingestion.
///
/// Handles `(guild, asset)` deduplication, storage quota, 100-year pinning,
/// item insertion and author recording. Returns a business result instead of an
/// HTTP response: the same function serves the asynchronous task (which maps it
/// to `media.ingestion`) and keeps the error vocabulary shared with the REST
/// helpers below.
pub(crate) async fn complete_board_item(
    state: &AppState,
    auth: &AuthedOverlayClient,
    db: &Db,
    asset: MediaAsset,
    title: ItemTitle,
    message: Option<String>,
    now: UtcDateTime,
) -> Result<CompletedBoardItem, IngestionFailure> {
    // Dédoublonnage board : (guilde, asset) déjà épinglé → `created:false`.
    match db
        .media_board()
        .get_by_guild_and_asset(auth.guild_id(), asset.id)
        .await
    {
        Ok(Some(existing_item)) => {
            return Ok(CompletedBoardItem {
                created: false,
                replaced: false,
                item: existing_item,
            });
        }
        Ok(None) => {}
        Err(db_error) => {
            return Err(completion_db_failure(
                "overlay_media_board.board_lookup",
                &db_error,
            ));
        }
    }

    // Quota de stockage de la board (legacy `ensureGuildBoardHasCapacity`).
    let used = match board_total_bytes(db, auth.guild_id()).await {
        Ok(used) => used,
        Err(db_error) => {
            return Err(completion_db_failure(
                "overlay_media_board.board_usage",
                &db_error,
            ));
        }
    };
    let limit_bytes = match board_limit_bytes(state, db, auth.guild_id()).await {
        Ok(limit) => limit,
        Err(db_error) => {
            return Err(completion_db_failure(
                "overlay_media_board.board_limit",
                &db_error,
            ));
        }
    };
    if used.saturating_add(asset.size_bytes) > limit_bytes {
        return Err(board_full_failure(limit_bytes));
    }

    // AUD-A2-007 : le service a persisté l'asset avec le TTL de cache ; le
    // board l'épingle pour 100 ans (`ensurePinnedExpiry` legacy), après
    // validation de la capacité et pour les deux cas d'usage (asset neuf ou
    // déjà en cache).
    let asset = match assets::pin(db, asset.id, now).await {
        Ok(pinned) => pinned,
        Err(MediaServiceError::Database(db_error)) => {
            return Err(completion_db_failure(
                "overlay_media_board.pin_board_asset",
                &db_error,
            ));
        }
        Err(service_error) => return Err(media_service_failure(&service_error)),
    };

    let item = MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: auth.guild_id().clone(),
        media_asset_id: asset.id,
        // Empty title: the asset's source title (see [`ItemTitle::resolve`]).
        title: title.resolve(asset.source_title.as_deref()),
        message: message
            .as_deref()
            .map(|value| truncate_chars(value.trim(), MESSAGE_MAX_CHARS))
            .filter(|value| !value.is_empty()),
        created_by_discord_user_id: auth.client.created_by_discord_user_id.clone(),
        created_by_name: board_author_name(auth),
        created_at: now,
        artwork_asset_id: None,
        artwork_on_overlay: false,
        display_duration_sec: None,
        display_scale_percent: None,
    };
    match db.media_board().create(&item).await {
        Ok(()) => {
            // L'auteur sert au droit « modifier ses propres médias ». The item is
            // already visible, so an author-recording failure is logged but does
            // not turn a successful ingestion into a failure.
            if let Some(member_id) = auth.member_id() {
                if let Err(db_error) = db
                    .media_board()
                    .set_creator_member(item.id, member_id)
                    .await
                {
                    tracing::warn!(
                        error = %db_error,
                        item_id = %item.id,
                        "enregistrement de l'auteur d'un média de board impossible"
                    );
                }
            }
            Ok(CompletedBoardItem {
                created: true,
                replaced: false,
                item,
            })
        }
        Err(db_error) if is_conflict(&db_error) => match db
            .media_board()
            .get_by_guild_and_asset(auth.guild_id(), asset.id)
            .await
        {
            Ok(Some(existing_item)) => Ok(CompletedBoardItem {
                created: false,
                replaced: false,
                item: existing_item,
            }),
            Ok(None) => {
                tracing::error!("item du board introuvable après un conflit d'insertion");
                Err(internal_failure())
            }
            Err(db_error) => Err(completion_db_failure(
                "overlay_media_board.reload_item",
                &db_error,
            )),
        },
        Err(db_error) => Err(completion_db_failure(
            "overlay_media_board.insert_item",
            &db_error,
        )),
    }
}

/// Replaces the media of an existing board item with an ingested asset.
///
/// The item keeps its id, title, message, author, date, favorites and
/// picture. The new asset must be of the same kind as the previous one and
/// must not already be on the guild's board. The storage quota is checked on
/// the size difference; the new asset is pinned and the previous one is
/// released when no item shows it any more.
pub(super) async fn replace_board_item_media(
    state: &AppState,
    auth: &AuthedOverlayClient,
    db: &Db,
    item_id: MediaBoardItemId,
    asset: MediaAsset,
    now: UtcDateTime,
) -> Result<CompletedBoardItem, IngestionFailure> {
    let item = match db.media_board().get(item_id).await {
        Ok(Some(item)) if &item.guild_id == auth.guild_id() => item,
        Ok(_) => {
            return Err(replace_failure(
                "le média à remplacer n'est plus sur le board",
            ));
        }
        Err(db_error) => {
            return Err(completion_db_failure(
                "overlay_media_board.replace_lookup",
                &db_error,
            ));
        }
    };
    if item.media_asset_id == asset.id {
        // The edit produced the very same file: nothing to replace.
        return Ok(CompletedBoardItem {
            created: false,
            replaced: false,
            item,
        });
    }
    let previous = match db.media_assets().get(item.media_asset_id).await {
        Ok(previous) => previous,
        Err(db_error) => {
            return Err(completion_db_failure(
                "overlay_media_board.replace_previous_asset",
                &db_error,
            ));
        }
    };
    if let Some(previous) = &previous {
        if previous.kind != asset.kind {
            return Err(replace_failure(
                "le média de remplacement doit être du même type que l'original (vidéo, image ou son)",
            ));
        }
    }
    match db
        .media_board()
        .get_by_guild_and_asset(auth.guild_id(), asset.id)
        .await
    {
        Ok(Some(_)) => return Err(replace_failure(ALREADY_ON_BOARD)),
        Ok(None) => {}
        Err(db_error) => {
            return Err(completion_db_failure(
                "overlay_media_board.replace_board_lookup",
                &db_error,
            ));
        }
    }

    let used = match board_total_bytes(db, auth.guild_id()).await {
        Ok(used) => used,
        Err(db_error) => {
            return Err(completion_db_failure(
                "overlay_media_board.replace_board_usage",
                &db_error,
            ));
        }
    };
    let previous_bytes = previous.as_ref().map_or(0, |previous| previous.size_bytes);
    let limit_bytes = match board_limit_bytes(state, db, auth.guild_id()).await {
        Ok(limit) => limit,
        Err(db_error) => {
            return Err(completion_db_failure(
                "overlay_media_board.replace_board_limit",
                &db_error,
            ));
        }
    };
    if used
        .saturating_sub(previous_bytes)
        .saturating_add(asset.size_bytes)
        > limit_bytes
    {
        return Err(board_full_failure(limit_bytes));
    }

    let asset = match assets::pin(db, asset.id, now).await {
        Ok(pinned) => pinned,
        Err(MediaServiceError::Database(db_error)) => {
            return Err(completion_db_failure(
                "overlay_media_board.replace_pin_asset",
                &db_error,
            ));
        }
        Err(service_error) => return Err(media_service_failure(&service_error)),
    };

    let release_seconds = i64::try_from(state.runtime().media_cache_ttl_hours())
        .unwrap_or(i64::MAX / 3_600)
        .saturating_mul(3_600);
    let released_expiry = now + time::Duration::seconds(release_seconds);
    match db
        .media_board()
        .replace_media(item_id, auth.guild_id(), asset.id, now, released_expiry)
        .await
    {
        Ok(Some(_previous)) => Ok(CompletedBoardItem {
            created: false,
            replaced: true,
            item: MediaBoardItem {
                media_asset_id: asset.id,
                ..item
            },
        }),
        Ok(None) => Err(replace_failure(
            "le média à remplacer n'est plus sur le board",
        )),
        Err(db_error) if is_conflict(&db_error) => Err(replace_failure(ALREADY_ON_BOARD)),
        Err(db_error) => Err(completion_db_failure(
            "overlay_media_board.replace_media",
            &db_error,
        )),
    }
}

/// Sentence shown when the result of a replacement is already on the board.
const ALREADY_ON_BOARD: &str = "ce média est déjà sur le board de la guilde";

/// Readable failure of a replacement (`media_ingestion_failed`).
fn replace_failure(message: &str) -> IngestionFailure {
    IngestionFailure {
        code: "media_ingestion_failed",
        message: Some(message.to_owned()),
    }
}

/// Storage limit of a guild's board, in bytes.
///
/// The per-guild `board_quota_mb` column (migration 0011) overrides
/// `MEDIA_BOARD_MAX_TOTAL_MB` when it is set. A guild without a settings row
/// keeps the server-wide limit.
async fn board_limit_bytes(state: &AppState, db: &Db, guild_id: &GuildId) -> lc_db::DbResult<i64> {
    let megabytes = match db.guilds().board_quota_mb(guild_id).await? {
        Some(quota) => u64::try_from(quota).unwrap_or(1).max(1),
        None => state.runtime().media_board_max_total_mb().max(1),
    };
    Ok(i64::try_from(megabytes.saturating_mul(MIB)).unwrap_or(i64::MAX))
}

/// Failure sent when a media would not fit in the guild's board.
fn board_full_failure(limit_bytes: i64) -> IngestionFailure {
    let limit_mb = u64::try_from(limit_bytes / i64::try_from(MIB).unwrap_or(1)).unwrap_or(0);
    IngestionFailure {
        code: "media_ingestion_failed",
        message: Some(format!(
            "la board de la guilde dépasserait la limite de {limit_mb} Mo"
        )),
    }
}

/// Logs a database error and turns it into the protocol `internal_error`.
fn completion_db_failure(operation: &'static str, db_error: &lc_db::DbError) -> IngestionFailure {
    tracing::error!(operation, error = %db_error, "échec d'accès base pendant une ingestion média");
    internal_failure()
}

/// Protocol `internal_error`, without detail for the client.
fn internal_failure() -> IngestionFailure {
    IngestionFailure {
        code: "internal_error",
        message: None,
    }
}

/// Handler `GET`/`HEAD /overlay/media-board/media/{assetId}`.
///
/// Authentification **uniquement** par URL signée (`clientId`, `exp`, `sig`) :
/// ni Bearer ni `?token=`, qui ne sont plus acceptés sur cette route. La
/// signature couvre `board:{assetId}`, donc une URL de lecture ne peut pas être
/// rejouée ici (et inversement).
pub async fn get_media(
    State(state): State<AppState>,
    Path(asset_id_raw): Path<String>,
    Query(query): Query<BoardMediaQuery>,
    method: axum::http::Method,
    headers: HeaderMap,
) -> Response {
    let (client_id_raw, exp_raw, sig) = match (query.client_id, query.exp, query.sig) {
        (Some(client_id), Some(exp), Some(sig)) => (client_id, exp, sig),
        _ => return super::media::media_url_invalid(),
    };
    let Ok(asset_id) = MediaAssetId::parse(asset_id_raw.trim()) else {
        return super::media::media_not_found();
    };
    let Ok(client_id) = OverlayClientId::parse(client_id_raw.trim()) else {
        return super::media::media_url_invalid();
    };
    let Ok(exp_sec) = exp_raw.trim().parse::<u64>() else {
        return super::media::media_url_invalid();
    };
    if sig.len() != 64 || !sig.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return super::media::media_url_invalid();
    }

    let secret = state.config.media_url_secret.expose_secret();
    if secret.is_empty() {
        tracing::warn!("MEDIA_URL_SECRET absent : média du board refusé");
        return super::media::media_url_invalid();
    }
    let asset_key = asset_id.to_canonical();
    let client_key = client_id.to_canonical();
    if !crate::media_url::verify_board(secret.as_bytes(), &asset_key, &client_key, exp_sec, &sig) {
        return super::media::media_url_invalid();
    }

    let timestamp = now(&state);
    let now_sec = u64::try_from(timestamp.unix_timestamp()).unwrap_or(0);
    if !crate::media_url::is_expiry_within_max(exp_sec, now_sec) {
        tracing::warn!(
            asset_id = %asset_key,
            exp_sec,
            now_sec,
            max_ttl_sec = lc_protocol::MEDIA_URL_TTL_MAX_SEC,
            "URL de média du board au-delà du plafond d'expiration : refus"
        );
        return super::media::media_url_invalid();
    }
    if crate::media_url::is_expired(exp_sec, now_sec) {
        return super::media::media_url_expired();
    }

    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let client = match db.overlay_clients().get(client_id).await {
        Ok(Some(client)) => client,
        Ok(None) => return super::media::media_url_invalid(),
        Err(db_error) => return db_failure("overlay_media_board.media_client", &db_error),
    };
    if !client.is_active() {
        return super::media::media_url_invalid();
    }

    let role = match db.guild_members().get_for_overlay_client(client.id).await {
        Ok(Some(member)) => member.role,
        Ok(None) => match client.session_mode {
            lc_domain::OverlaySessionMode::InviteReadOnly => GuildRole::Guest,
            lc_domain::OverlaySessionMode::Normal => GuildRole::Member,
        },
        Err(db_error) => return db_failure("overlay_media_board.media_member", &db_error),
    };
    if let Err(response) = ensure_board_visible_for(&state, db, &client.guild_id, role).await {
        return response;
    }

    match db
        .media_board()
        .guild_references_asset(&client.guild_id, asset_id)
        .await
    {
        Ok(true) => {}
        Ok(false) => return super::media::media_not_found(),
        Err(db_error) => return db_failure("overlay_media_board.media_binding", &db_error),
    }
    let asset = match db.media_assets().get(asset_id).await {
        Ok(Some(asset)) => asset,
        Ok(None) => return super::media::media_not_found(),
        Err(db_error) => return db_failure("overlay_media_board.media_asset", &db_error),
    };
    if !asset.is_ready() || asset.is_expired_at(timestamp) {
        return super::media::media_not_found();
    }
    let file_path = match super::media::resolve_media_file(&state, &asset).await {
        Ok(path) => path,
        Err(response) => return response,
    };
    let range = headers.get(axum::http::header::RANGE).cloned();
    super::media::serve_file(
        &asset,
        &file_path,
        method,
        range.as_ref(),
        super::media::MediaCache::UntilExpiry {
            expires_at_sec: exp_sec,
            now_sec,
        },
    )
    .await
}

/// Board item update shared by `PATCH /overlay/media-board/items/{itemId}` and
/// the admin console. The read-only guard runs before any payload check.
pub(crate) async fn update_item_payload(
    state: &AppState,
    actor: &ActorInfo,
    item_id_raw: &str,
    body: Value,
) -> Result<Value, Response> {
    if actor.is_read_only() {
        return Err(error(StatusCode::FORBIDDEN, "read_only_session"));
    }
    let Some(object) = body.as_object() else {
        return Err(json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "invalid_payload", "message": "title or message is required"}),
        ));
    };
    let has_title = object.contains_key("title");
    let has_message = object.contains_key("message");
    let artwork_on_overlay = match object.get("artworkOnOverlay") {
        None => None,
        Some(serde_json::Value::Bool(value)) => Some(*value),
        Some(_) => {
            return Err(json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error": "invalid_payload", "message": "artworkOnOverlay must be a boolean"}),
            ));
        }
    };
    // Absent → kept; `null` → cleared; else an integer within the bounds.
    let display_duration = match object.get("displayDurationSec") {
        None => None,
        Some(serde_json::Value::Null) => Some(None),
        Some(value) => match value.as_i64() {
            Some(seconds) if lc_domain::entities::is_valid_display_duration(seconds) => {
                Some(Some(seconds))
            }
            _ => return Err(invalid_display_duration()),
        },
    };
    // Absent → kept; `null` → cleared; else an integer within the bounds.
    let display_scale = match object.get("displayScalePercent") {
        None => None,
        Some(serde_json::Value::Null) => Some(None),
        Some(value) => match value.as_i64() {
            Some(percent) if lc_domain::entities::is_valid_display_scale(percent) => {
                Some(Some(percent))
            }
            _ => return Err(invalid_display_scale()),
        },
    };
    if !has_title
        && !has_message
        && artwork_on_overlay.is_none()
        && display_duration.is_none()
        && display_scale.is_none()
    {
        return Err(json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "invalid_payload", "message": "title or message is required"}),
        ));
    }
    let Ok(item_id) = item_id_raw.trim().parse::<MediaBoardItemId>() else {
        return Err(error(StatusCode::NOT_FOUND, "item_not_found"));
    };
    let db = db(state)?;
    let existing = match db.media_board().get(item_id).await {
        Ok(Some(item)) if &item.guild_id == actor.guild_id() => item,
        Ok(_) => return Err(error(StatusCode::NOT_FOUND, "item_not_found")),
        Err(db_error) => {
            return Err(db_failure("overlay_media_board.patch_lookup", &db_error));
        }
    };
    ensure_actor_can_edit(db, actor, item_id).await?;

    // Only an image takes a chosen display time or scale.
    let sets_duration = matches!(display_duration, Some(Some(_)));
    let sets_scale = matches!(display_scale, Some(Some(_)));
    if sets_duration || sets_scale {
        match db.media_assets().get(existing.media_asset_id).await {
            Ok(Some(asset)) if asset.kind == MediaKind::Image => {}
            Ok(_) => {
                return Err(if sets_duration {
                    invalid_display_duration()
                } else {
                    invalid_display_scale()
                });
            }
            Err(db_error) => {
                return Err(db_failure("overlay_media_board.patch_asset", &db_error));
            }
        }
    }

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

    let edit = lc_db::BoardItemEdit {
        title: title.as_deref(),
        message: message.as_deref(),
        artwork_on_overlay,
        display_duration_sec: display_duration,
        display_scale_percent: display_scale,
    };
    // Read before the update: the author is what makes this a moderation of
    // another member's media (the item itself is not removed here).
    let creator = overlay_audit::creator_member(db, item_id).await;
    match db.media_board().update(item_id, &edit).await {
        Ok(true) => {
            state.admin_hub.broadcast_to_guild(
                actor.guild_id(),
                ServerMessage::BoardChanged(BoardChangedPayload {
                    change: BoardChange::Updated,
                    item_id: item_id.to_canonical(),
                }),
            );
            overlay_audit::record_moderation(
                state,
                actor,
                creator,
                "overlay.media_update",
                "PATCH",
                &format!("/overlay/media-board/items/{}", item_id.to_canonical()),
            )
            .await;
            Ok(serde_json::json!({
                "updated": true,
                "item": {
                    "id": existing.id.to_canonical(),
                    "title": title,
                    "message": message,
                    "artworkOnOverlay": artwork_on_overlay.unwrap_or(existing.artwork_on_overlay),
                    "displayDurationSec": display_duration.unwrap_or(existing.display_duration_sec),
                    "displayScalePercent": display_scale.unwrap_or(existing.display_scale_percent),
                },
            }))
        }
        Ok(false) => Err(error(StatusCode::NOT_FOUND, "item_not_found")),
        Err(db_error) => Err(db_failure("overlay_media_board.patch_update", &db_error)),
    }
}

/// Handler `PATCH /overlay/media-board/items/{itemId}`.
pub async fn update_item(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(item_id_raw): Path<String>,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Response {
    // Garde de session : aucune écriture en INVITE_READ_ONLY.
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let body = match body {
        Ok(Json(value)) => value,
        Err(rejection) => {
            tracing::debug!(
                error = %rejection.body_text(),
                "corps JSON invalide sur PATCH /overlay/media-board/items"
            );
            return json_rejection_error(rejection);
        }
    };
    let actor = ActorInfo::from_auth(&state, &auth);
    match update_item_payload(&state, &actor, &item_id_raw, body).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Deletes a board item shared by the overlay route and the admin console.
pub(crate) async fn delete_item_payload(
    state: &AppState,
    actor: &ActorInfo,
    item_id_raw: &str,
) -> Result<Value, Response> {
    if actor.is_read_only() {
        return Err(error(StatusCode::FORBIDDEN, "read_only_session"));
    }
    let Ok(item_id) = item_id_raw.trim().parse::<MediaBoardItemId>() else {
        return Ok(serde_json::json!({"deleted": false}));
    };
    let db = db(state)?;
    let existing = match db.media_board().get(item_id).await {
        Ok(Some(item)) if &item.guild_id == actor.guild_id() => item,
        Ok(_) => return Ok(serde_json::json!({"deleted": false})),
        Err(db_error) => {
            return Err(db_failure("overlay_media_board.delete_lookup", &db_error));
        }
    };
    ensure_actor_can_edit(db, actor, item_id).await?;

    // AUD-A2-013 : suppression et libération de l'expiration épinglée dans une
    // même transaction (le comptage de références est fait côté SQL), au lieu
    // d'un DELETE puis d'un comptage applicatif non transactionnel.
    // L'auteur est lu avant la suppression : c'est lui qui distingue une
    // modération d'un simple retrait de son propre média.
    let creator = overlay_audit::creator_member(db, item_id).await;
    let now = now(state);
    let release_seconds = i64::try_from(state.runtime().media_cache_ttl_hours())
        .unwrap_or(i64::MAX / 3_600)
        .saturating_mul(3_600);
    let released_expiry = now + time::Duration::seconds(release_seconds);
    let deleted = delete_item_and_release(
        db,
        item_id,
        actor.guild_id(),
        &[Some(existing.media_asset_id), existing.artwork_asset_id],
        released_expiry,
    )
    .await
    .map_err(|db_error| db_failure("overlay_media_board.delete", &db_error))?;

    if deleted {
        state.admin_hub.broadcast_to_guild(
            actor.guild_id(),
            ServerMessage::BoardChanged(BoardChangedPayload {
                change: BoardChange::Deleted,
                item_id: item_id.to_canonical(),
            }),
        );
        overlay_audit::record_moderation(
            state,
            actor,
            creator,
            "overlay.media_remove",
            "DELETE",
            &format!("/overlay/media-board/items/{}", item_id.to_canonical()),
        )
        .await;
    }

    Ok(serde_json::json!({"deleted": deleted}))
}

/// Handler `DELETE /overlay/media-board/items/{itemId}`.
pub async fn delete_item(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(item_id_raw): Path<String>,
) -> Response {
    // Garde de session : aucune suppression en INVITE_READ_ONLY.
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let actor = ActorInfo::from_auth(&state, &auth);
    match delete_item_payload(&state, &actor, &item_id_raw).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Garde d'écriture : refuse les mutations en session `INVITE_READ_ONLY`.
///
/// Le mode est celui du client authentifié par Bearer (persisté en
/// `overlay_clients.session_mode`). Les erreurs du legacy
/// (`400`, `404`) ne sont volontairement pas évaluées avant cette garde : une
/// session invitée ne doit obtenir aucune information sur la validité d'une
/// charge utile ou d'un identifiant.
pub(super) fn ensure_writable(auth: &AuthedOverlayClient) -> Result<(), Response> {
    if auth.is_read_only() {
        return Err(error(StatusCode::FORBIDDEN, "read_only_session"));
    }
    Ok(())
}

/// Read guard: `viewBoard` comes from the rights cache; a guest must also be
/// allowed by the guild setting (`guilds.guests_can_view_board`, true by
/// default).
pub(super) async fn ensure_board_visible_for(
    state: &AppState,
    db: &Db,
    guild_id: &GuildId,
    role: GuildRole,
) -> Result<(), Response> {
    if !state
        .rights
        .allowed(Surface::Overlay, Action::ViewBoard, role.into())
    {
        return Err(error(StatusCode::FORBIDDEN, "forbidden"));
    }
    if role != GuildRole::Guest {
        return Ok(());
    }
    let guests_can_view_board = match db.guilds().access(guild_id).await {
        Ok(access) => access.is_none_or(|access| access.guests_can_view_board),
        Err(db_error) => return Err(db_failure("overlay_media_board.board_access", &db_error)),
    };
    if guests_can_view_board {
        Ok(())
    } else {
        Err(error(StatusCode::FORBIDDEN, "forbidden"))
    }
}

/// `true` si le client peut modifier ou supprimer un média dont l'auteur est
/// `creator_member_id`.
///
/// Un admin modifie tout ; un membre modifie ses propres médias. Un média sans
/// auteur enregistré (antérieur aux rôles, ou ajouté depuis Discord) reste
/// modifiable par tout membre, comme avant ; il en va de même pour un overlay
/// sans membre rattaché.
fn can_edit(state: &AppState, auth: &AuthedOverlayClient, creator_member_id: Option<&str>) -> bool {
    let rights = state.rights.snapshot();
    let role = auth.role().into();
    if rights.allowed(Surface::Overlay, Action::EditAnyMedia, role) {
        return true;
    }
    if !rights.allowed(Surface::Overlay, Action::EditOwnMedia, role) {
        return false;
    }
    match (auth.member_id(), creator_member_id) {
        (Some(member_id), Some(creator)) => member_id.to_canonical() == creator,
        _ => true,
    }
}

/// Item of the caller's guild, or `404 item_not_found`.
pub(super) async fn guild_item(
    db: &Db,
    auth: &AuthedOverlayClient,
    item_id_raw: &str,
    operation: &'static str,
) -> Result<MediaBoardItem, Response> {
    let Ok(item_id) = item_id_raw.trim().parse::<MediaBoardItemId>() else {
        return Err(error(StatusCode::NOT_FOUND, "item_not_found"));
    };
    match db.media_board().get(item_id).await {
        Ok(Some(item)) if &item.guild_id == auth.guild_id() => Ok(item),
        Ok(_) => Err(error(StatusCode::NOT_FOUND, "item_not_found")),
        Err(db_error) => Err(db_failure(operation, &db_error)),
    }
}

/// Garde de modification d'un média existant pour un [`ActorInfo`]
/// (`403 forbidden` sinon).
pub(crate) async fn ensure_actor_can_edit(
    db: &Db,
    actor: &ActorInfo,
    item_id: MediaBoardItemId,
) -> Result<(), Response> {
    if actor.operator {
        return Ok(());
    }
    let creator = match db.media_board().creator_member(item_id).await {
        Ok(creator) => creator.map(lc_domain::GuildMemberId::to_canonical),
        Err(db_error) => {
            return Err(db_failure("overlay_media_board.item_author", &db_error));
        }
    };
    if actor.can_edit_item(creator.as_deref()) {
        Ok(())
    } else {
        Err(error(StatusCode::FORBIDDEN, "forbidden"))
    }
}

/// Garde de modification d'un média existant (`403 forbidden` sinon).
pub(super) async fn ensure_can_edit(
    state: &AppState,
    auth: &AuthedOverlayClient,
    item_id: MediaBoardItemId,
) -> Result<(), Response> {
    let db = db(state)?;
    let creator = match db.media_board().creator_member(item_id).await {
        Ok(creator) => creator.map(lc_domain::GuildMemberId::to_canonical),
        Err(db_error) => return Err(db_failure("overlay_media_board.item_author", &db_error)),
    };
    if can_edit(state, auth, creator.as_deref()) {
        Ok(())
    } else {
        Err(error(StatusCode::FORBIDDEN, "forbidden"))
    }
}

/// `400 invalid_payload` for a display time out of bounds, not an integer, or
/// set on a media that is not an image.
fn invalid_display_duration() -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        serde_json::json!({
            "error": "invalid_payload",
            "message": "displayDurationSec must be an integer between 1 and 600, or null, on an image",
        }),
    )
}

/// `400 invalid_payload` for a display scale out of bounds, not an integer, or
/// set on a media that is not an image.
fn invalid_display_scale() -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        serde_json::json!({
            "error": "invalid_payload",
            "message": "displayScalePercent must be an integer between 10 and 1000, or null, on an image",
        }),
    )
}

/// Payload JSON d'un item de board (format legacy exact).
fn item_payload(item: &MediaBoardItem, asset: &MediaAsset) -> serde_json::Value {
    serde_json::json!({
        "id": item.id.to_canonical(),
        "guildId": item.guild_id.as_str(),
        "mediaAssetId": item.media_asset_id.to_canonical(),
        "title": item.title,
        "message": item.message,
        "createdByName": item.created_by_name,
        "createdAt": lc_domain::time::to_rfc3339_millis(item.created_at),
        "displayDurationSec": item.display_duration_sec,
        "displayScalePercent": item.display_scale_percent,
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
    auth.member
        .as_ref()
        .map(|member| member.display_name.as_str())
        .or(auth.client.default_author_name.as_deref())
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

/// Ligne de board lue en une seule requête (item + URL source de l'asset).
struct BoardRow {
    item: MediaBoardItem,
    source_url: String,
    /// Membre auteur (`media_board_items.created_by_member_id`), s'il est connu.
    created_by_member_id: Option<String>,
}

impl BoardRow {
    /// Identifiant de l'asset média référencé (rechargé seulement pour la page).
    fn media_asset_id(&self) -> MediaAssetId {
        self.item.media_asset_id
    }
}

/// Filtre `q` (insensible à la casse) sur les champs exposés par le legacy.
fn row_matches(row: &BoardRow, search: &str) -> bool {
    let fields = [
        row.item.title.as_deref(),
        row.item.message.as_deref(),
        row.item.created_by_name.as_deref(),
        Some(row.source_url.as_str()),
    ];
    fields
        .into_iter()
        .flatten()
        .any(|value| value.to_lowercase().contains(search))
}

/// Charge la board d'une guilde en une requête (même ordre que `list_by_guild`).
async fn load_board_rows(db: &Db, guild_id: &GuildId) -> lc_db::DbResult<Vec<BoardRow>> {
    use sqlx::Row as _;

    let raw_rows = sqlx::query(
        "SELECT b.id, b.guild_id, b.media_asset_id, b.title, b.message, \
                b.created_by_discord_user_id, b.created_by_name, b.created_at, b.created_by_member_id, \
                b.artwork_asset_id, b.artwork_on_overlay, b.display_duration_sec, b.display_scale_percent, a.source_url \
           FROM media_board_items b \
           JOIN media_assets a ON a.id = b.media_asset_id \
          WHERE b.guild_id = ? \
          ORDER BY b.created_at DESC, b.id ASC",
    )
    .bind(guild_id.as_str())
    .fetch_all(db.pool())
    .await
    .map_err(lc_db::DbError::from)?;

    let mut rows = Vec::with_capacity(raw_rows.len());
    for raw in raw_rows {
        let item_id_raw: String = raw.try_get("id").map_err(lc_db::DbError::from)?;
        let guild_raw: String = raw.try_get("guild_id").map_err(lc_db::DbError::from)?;
        let asset_id_raw: String = raw
            .try_get("media_asset_id")
            .map_err(lc_db::DbError::from)?;
        let created_at_ms: i64 = raw.try_get("created_at").map_err(lc_db::DbError::from)?;
        let title: Option<String> = raw.try_get("title").map_err(lc_db::DbError::from)?;
        let message: Option<String> = raw.try_get("message").map_err(lc_db::DbError::from)?;
        let created_by_discord_user_id: Option<String> = raw
            .try_get("created_by_discord_user_id")
            .map_err(lc_db::DbError::from)?;
        let created_by_name: Option<String> = raw
            .try_get("created_by_name")
            .map_err(lc_db::DbError::from)?;
        let source_url: String = raw.try_get("source_url").map_err(lc_db::DbError::from)?;
        let created_by_member_id: Option<String> = raw
            .try_get("created_by_member_id")
            .map_err(lc_db::DbError::from)?;
        let artwork_raw: Option<String> = raw
            .try_get("artwork_asset_id")
            .map_err(lc_db::DbError::from)?;
        let artwork_on_overlay: i64 = raw
            .try_get("artwork_on_overlay")
            .map_err(lc_db::DbError::from)?;
        let display_duration_sec: Option<i64> = raw
            .try_get("display_duration_sec")
            .map_err(lc_db::DbError::from)?;
        let display_scale_percent: Option<i64> = raw
            .try_get("display_scale_percent")
            .map_err(lc_db::DbError::from)?;
        let artwork_asset_id = artwork_raw
            .as_deref()
            .map(MediaAssetId::parse)
            .transpose()
            .map_err(|error| corrupt_row(format!("artwork_asset_id invalide: {error}")))?;

        let item_id = MediaBoardItemId::parse(&item_id_raw)
            .map_err(|error| corrupt_row(format!("id invalide ({item_id_raw}): {error}")))?;
        let item_guild = GuildId::parse(&guild_raw)
            .map_err(|error| corrupt_row(format!("guild_id invalide ({guild_raw}): {error}")))?;
        let media_asset_id = MediaAssetId::parse(&asset_id_raw).map_err(|error| {
            corrupt_row(format!("media_asset_id invalide ({asset_id_raw}): {error}"))
        })?;
        let created_at = lc_domain::time::ms_to_datetime(created_at_ms)
            .ok_or_else(|| corrupt_row(format!("created_at invalide ({created_at_ms})")))?;

        rows.push(BoardRow {
            item: MediaBoardItem {
                id: item_id,
                guild_id: item_guild,
                media_asset_id,
                title,
                message,
                created_by_discord_user_id,
                created_by_name,
                created_at,
                artwork_asset_id,
                artwork_on_overlay: artwork_on_overlay != 0,
                display_duration_sec,
                display_scale_percent,
            },
            source_url,
            created_by_member_id,
        });
    }
    Ok(rows)
}

/// Erreur `Corrupt` ciblée pour une ligne de board illisible.
fn corrupt_row(detail: String) -> lc_db::DbError {
    lc_db::DbError::Corrupt {
        table: "media_board_items",
        detail,
    }
}

/// Somme des tailles des médias actuellement référencés par une board.
async fn board_total_bytes(db: &Db, guild_id: &GuildId) -> lc_db::DbResult<i64> {
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(a.size_bytes), 0) \
           FROM media_board_items b \
           JOIN media_assets a ON a.id = b.media_asset_id \
          WHERE b.guild_id = ?",
    )
    .bind(guild_id.as_str())
    .fetch_one(db.pool())
    .await
    .map_err(lc_db::DbError::from)?;
    Ok(total)
}

/// Supprime un item de board et libère l'expiration épinglée si l'asset n'est
/// plus référencé, le tout dans une transaction (AUD-A2-013).
async fn delete_item_and_release(
    db: &Db,
    item_id: MediaBoardItemId,
    guild_id: &GuildId,
    asset_ids: &[Option<MediaAssetId>],
    released_expiry: UtcDateTime,
) -> lc_db::DbResult<bool> {
    let mut transaction = db.pool().begin().await.map_err(lc_db::DbError::from)?;

    let deleted = sqlx::query("DELETE FROM media_board_items WHERE id = ? AND guild_id = ?")
        .bind(item_id.to_canonical())
        .bind(guild_id.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(lc_db::DbError::from)?
        .rows_affected()
        > 0;

    for media_asset_id in asset_ids.iter().flatten().filter(|_| deleted) {
        let remaining = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM media_board_items \
              WHERE media_asset_id = ? OR artwork_asset_id = ?",
        )
        .bind(media_asset_id.to_canonical())
        .bind(media_asset_id.to_canonical())
        .fetch_one(&mut *transaction)
        .await
        .map_err(lc_db::DbError::from)?;
        if remaining == 0 {
            // `touch_last_accessed` ne peut pas rejoindre la transaction
            // (repository séparé) : l'UPDATE est fait directement sur la même
            // connexion pour rester atomique avec la suppression.
            sqlx::query(
                "UPDATE media_assets SET last_accessed_at = ?, expires_at = ? WHERE id = ?",
            )
            .bind(lc_domain::time::datetime_to_ms(UtcDateTime::now()))
            .bind(lc_domain::time::datetime_to_ms(released_expiry))
            .bind(media_asset_id.to_canonical())
            .execute(&mut *transaction)
            .await
            .map_err(lc_db::DbError::from)?;
            // Its cover (a video's thumbnail, an audio's album art) was
            // pinned with it, unless an item still shows it as its picture.
            sqlx::query(
                "UPDATE media_assets SET last_accessed_at = ?, expires_at = ? \
                  WHERE id = (SELECT cover_asset_id FROM media_assets WHERE id = ?) \
                    AND id NOT IN (SELECT artwork_asset_id FROM media_board_items \
                                    WHERE artwork_asset_id IS NOT NULL)",
            )
            .bind(lc_domain::time::datetime_to_ms(UtcDateTime::now()))
            .bind(lc_domain::time::datetime_to_ms(released_expiry))
            .bind(media_asset_id.to_canonical())
            .execute(&mut *transaction)
            .await
            .map_err(lc_db::DbError::from)?;
        }
    }

    transaction.commit().await.map_err(lc_db::DbError::from)?;
    Ok(deleted)
}

/// Error vocabulary shared by the REST responses and `media.ingestion`.
///
/// `rest_code` is the detailed code documented by the REST contract
/// (`UNSUPPORTED_SOURCE`, `BOARD_STORAGE_LIMIT_REACHED`, ...); `ws_code` is the
/// single machine code carried by the protocol payload.
pub(super) struct MediaErrorPayload {
    /// HTTP status of the synchronous equivalent.
    status: StatusCode,
    /// Value of the REST `error` field.
    error: &'static str,
    /// Value of the REST detailed `code` field, when the contract has one.
    rest_code: Option<&'static str>,
    /// Readable message, when available.
    message: Option<String>,
    /// Machine code used by `media.ingestion`.
    ws_code: &'static str,
}

impl MediaErrorPayload {
    /// `500 {"error":"internal_error"}` / `{code:"internal_error"}`.
    fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: "internal_error",
            rest_code: None,
            message: None,
            ws_code: "internal_error",
        }
    }

    /// Builds the synchronous REST response from the same data.
    fn response(&self) -> Response {
        let mut body = serde_json::Map::new();
        body.insert(
            "error".to_owned(),
            serde_json::Value::String(self.error.to_owned()),
        );
        if let Some(code) = self.rest_code {
            body.insert(
                "code".to_owned(),
                serde_json::Value::String(code.to_owned()),
            );
        }
        if let Some(message) = &self.message {
            body.insert(
                "message".to_owned(),
                serde_json::Value::String(message.clone()),
            );
        }
        json_response(self.status, serde_json::Value::Object(body))
    }

    /// Builds the asynchronous failure sent through `media.ingestion`.
    fn failure(&self) -> IngestionFailure {
        IngestionFailure {
            code: self.ws_code,
            message: self.message.clone(),
        }
    }
}

/// Traduit une erreur du service média (`Database`/`Unavailable` → 500, comme
/// le board legacy qui ne documente que `422 media_ingestion_failed`).
fn media_service_error_payload(service_error: &MediaServiceError) -> MediaErrorPayload {
    if let Some(media_error) = service_error.media_error() {
        // The board shows a sentence for people; the technical detail (an
        // ffmpeg line, a yt-dlp error) stays in the logs.
        tracing::info!(
            code = media_error.code().as_str(),
            detail = media_error.message(),
            "média du board refusé"
        );
        return MediaErrorPayload {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            error: "media_ingestion_failed",
            rest_code: Some(media_error.code().as_str()),
            message: Some(media_error.code().user_message_fr().to_owned()),
            ws_code: "media_ingestion_failed",
        };
    }
    tracing::warn!(error = %service_error, "opération média du board impossible");
    MediaErrorPayload::internal()
}

/// Asynchronous failure for a media-service error.
pub(super) fn media_service_failure(service_error: &MediaServiceError) -> IngestionFailure {
    media_service_error_payload(service_error).failure()
}

/// Réponse REST d'une erreur du service média.
pub(super) fn media_service_error(service_error: &MediaServiceError) -> Response {
    media_service_error_payload(service_error).response()
}

/// JSON extraction error response.
///
/// Exceeding the global limit (`DefaultBodyLimit`, 1 MiB) keeps its `413`
/// status; other rejections remain `400 invalid_payload` (AUD-A2-009).
pub(super) fn json_rejection_error(rejection: JsonRejection) -> Response {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        // Keep the rejection's `413` and let `normalize_errors` produce the
        // usual `{"error":"request_error"}` body.
        return rejection.into_response();
    }
    invalid_json()
}

/// Réponse `400 invalid_payload` pour un corps JSON malformé (AUD-A2-009).
fn invalid_json() -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        serde_json::json!({"error": "invalid_payload", "message": "corps JSON invalide"}),
    )
}

/// Tronque proprement (frontières UTF-8) après trim.
pub(super) fn truncate_chars(raw: &str, max: usize) -> String {
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

/// Interprète `forceRefresh` (booléen JSON, `"true"`/`"1"`/`"on"` legacy).
fn coerce_flag(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(flag)) => *flag,
        Some(serde_json::Value::String(raw)) => matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on"
        ),
        Some(serde_json::Value::Number(number)) => number.as_i64().is_some_and(|value| value != 0),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{coerce_flag, ItemTitle};
    use serde_json::json;

    #[test]
    fn a_requested_title_falls_back_to_the_source_title_when_empty() {
        let source = Some("Titre de la vidéo");
        assert_eq!(
            ItemTitle::Requested(None).resolve(source).as_deref(),
            source
        );
        assert_eq!(
            ItemTitle::Requested(Some("   ".to_owned()))
                .resolve(source)
                .as_deref(),
            source
        );
        // A typed title always wins.
        assert_eq!(
            ItemTitle::Requested(Some("  Mon titre ".to_owned()))
                .resolve(source)
                .as_deref(),
            Some("Mon titre")
        );
        assert_eq!(ItemTitle::Requested(None).resolve(None), None);
    }

    #[test]
    fn a_copied_title_never_takes_the_source_title() {
        let source = Some("Titre de la vidéo");
        assert_eq!(ItemTitle::Copied(None).resolve(source), None);
        assert_eq!(
            ItemTitle::Copied(Some("Titre copié".to_owned()))
                .resolve(source)
                .as_deref(),
            Some("Titre copié")
        );
    }

    #[test]
    fn coerce_flag_accepts_legacy_truthy_values() {
        // AUD-A2-014 : `toBooleanFlag` legacy accepte aussi "on".
        for value in [
            json!(true),
            json!(1),
            json!("1"),
            json!("true"),
            json!("TRUE"),
            json!("yes"),
            json!("on"),
            json!(" On "),
        ] {
            assert!(coerce_flag(Some(&value)), "valeur acceptee : {value}");
        }
        for value in [json!(false), json!(0), json!("0"), json!("no"), json!("")] {
            assert!(!coerce_flag(Some(&value)), "valeur refusee : {value}");
        }
        assert!(!coerce_flag(None));
    }
}
