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
//! | `POST` | `/overlay/media-board/items/{itemId}/cover` | Bearer | `200 {cover:{width,height,mime}}` (see [`super::media_cover`]) |
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

use std::collections::HashMap;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use lc_domain::rights::{Action, Surface};
use lc_domain::{
    GuildId, GuildMember, GuildRole, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId,
    MediaKind, OverlayClientId, Permission,
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
    /// Text shown for a play-once only (≤ 500 characters). It is never stored
    /// on the board: an add-to-board ignores it.
    #[serde(default)]
    message: Option<String>,
    /// Force une nouvelle ingestion même si le média est déjà en base.
    #[serde(default)]
    force_refresh: Option<serde_json::Value>,
    /// `true` : le média est joué une fois sur les overlays de la guilde sans
    /// créer d'item de board (droit `TriggerPlayback` au lieu d'`AddMedia`).
    #[serde(default)]
    play_once: bool,
    /// Durée d'affichage demandée pour le « Jouer une fois » (1 à 600 s).
    /// Absente ou `null` : durée par défaut de la guilde. Le genre du média
    /// n'est connu qu'après l'ingestion : la valeur n'est appliquée qu'à une
    /// image (une vidéo ou un son garde sa propre durée).
    #[serde(default)]
    display_duration_sec: Option<serde_json::Value>,
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

    // B-03 : filtre, tri, comptage et pagination poussés en SQL. Une requête
    // pour la page (avec toutes ses jointures), une pour le total, une pour
    // les membres actifs, une pour les favoris : nombre constant, indépendant
    // de la taille de la board. `instr(lower(...), ?)` garde la sémantique Rust
    // d'avant : sous-chaîne insensible à la casse, `%` et `_` littéraux.
    let page = load_board_page(db, actor.guild_id(), search.as_deref(), limit, offset)
        .await
        .map_err(|db_error| db_failure("overlay_media_board.list_items", &db_error))?;
    let total = count_board_rows(db, actor.guild_id(), search.as_deref())
        .await
        .map_err(|db_error| db_failure("overlay_media_board.list_total", &db_error))?;
    // L'auteur est résolu une fois pour toute la page : le filtre « Ajouté par »
    // regroupe les médias d'un même membre, même renommé ou lié à Discord.
    let members = db
        .guild_members()
        .list_by_guild(actor.guild_id())
        .await
        .map_err(|db_error| db_failure("overlay_media_board.list_members", &db_error))?;
    let authors = BoardAuthors::new(&members);
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
    let mut items = Vec::with_capacity(page.len());
    for mut row in page {
        row.author = authors.resolve(&row);
        let asset = match row.asset.take() {
            Some(asset) => asset,
            None => continue,
        };
        let asset_id = asset.id.to_canonical();
        let media_url = match sign(&asset_id) {
            Ok(url) => url,
            Err(error) => {
                tracing::warn!(%error, "signature d'une URL du board impossible");
                return Err(super::internal_error());
            }
        };
        let artwork = match (&row.item.artwork_asset_id, &row.artwork) {
            (Some(_), Some(artwork)) if artwork.is_ready() => {
                match sign(&artwork.id.to_canonical()) {
                    Ok(url) => Some(serde_json::json!({
                        "url": url,
                        "width": artwork.width,
                        "height": artwork.height,
                    })),
                    Err(error) => {
                        tracing::warn!(%error, "signature de l'image d'un média impossible");
                        None
                    }
                }
            }
            _ => None,
        };
        // A video's thumbnail: the platform's picture of a link, fetched in
        // the background after the import (see `media::thumbnails`).
        let thumbnail = match (asset.kind == lc_domain::MediaKind::Video, &row.thumbnail) {
            (true, Some(picture)) if picture.is_ready() => match sign(&picture.id.to_canonical()) {
                Ok(url) => Some(serde_json::json!({
                    "url": url,
                    "width": picture.width,
                    "height": picture.height,
                    // A video's picture keeps its URL when it is replaced
                    // (the editor's frame rewrites the same cover): the
                    // size of its bytes tells the client when to fetch it
                    // again instead of showing the cached one.
                    "version": picture.size_bytes,
                })),
                Err(error) => {
                    tracing::warn!(%error, "signature de la miniature d'une vidéo impossible");
                    None
                }
            },
            _ => None,
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
            // Identité stable de l'auteur (membre, compte Discord, ou nom
            // historique) : le filtre du board s'appuie dessus pour ne pas
            // séparer un membre de son compte Discord lié.
            object.insert(
                "createdByAuthorId".to_owned(),
                serde_json::json!(row.author.as_ref().map(|author| author.key.as_str())),
            );
            if let Some(author) = &row.author {
                object.insert(
                    "createdByName".to_owned(),
                    serde_json::json!(author.name.as_str()),
                );
            }
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
        items.push(payload);
    }

    Ok(serde_json::json!({
        "total": total,
        "limit": limit,
        "offset": offset,
        "items": items,
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
    // The display time only belongs to a play once; its media kind is unknown
    // until the ingestion, so only the bounds are checked here.
    if !payload.play_once && payload.display_duration_sec.is_some() {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({
                "error": "invalid_payload",
                "message": "displayDurationSec is only allowed for a play once",
            }),
        );
    }
    let display_duration_sec =
        match play_once_display_duration(payload.display_duration_sec.as_ref()) {
            Ok(seconds) => seconds,
            Err(response) => return response,
        };
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
        BoardTarget::PlayOnce {
            display_duration_sec,
        }
    } else {
        BoardTarget::NewItem
    };
    // The text is the caption of that play only: an add-to-board never
    // receives it, so nothing sent with a media can reach the item.
    let play_text = payload.play_once.then_some(payload.message).flatten();
    spawn_ingestion(
        state,
        auth,
        ingestion_id.clone(),
        IngestionInput::Link { url, force_refresh },
        target,
        payload.title,
        play_text,
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
///
/// The quota is read and enforced inside the same `BEGIN IMMEDIATE` transaction
/// as the insertion and the pin, so two concurrent ingestions cannot both pass
/// the limit check.
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
    // Le média est tout de même ré-épinglé : une ré-ingestion concurrente a pu
    // écraser son échéance avec le TTL de cache entre-temps, et le média doit
    // rester lisible tant que la carte est affichée.
    match db
        .media_board()
        .get_by_guild_and_asset(auth.guild_id(), asset.id)
        .await
    {
        Ok(Some(_)) => {
            return repin_existing_board_item(db, auth.guild_id(), &asset, now).await;
        }
        Ok(None) => {}
        Err(db_error) => {
            return Err(completion_db_failure(
                "overlay_media_board.board_lookup",
                &db_error,
            ));
        }
    }

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

    let mut transaction = begin_board_write_transaction(db).await?;
    match create_board_item_locked(
        state,
        db,
        &mut transaction,
        auth.guild_id(),
        &item,
        asset.size_bytes,
        now,
    )
    .await
    {
        Ok(LockedCreate::Created) => {
            commit_board_write_transaction(transaction).await?;
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
        Ok(LockedCreate::Existing) => {
            rollback_board_write_transaction(transaction).await;
            repin_existing_board_item(db, auth.guild_id(), &asset, now).await
        }
        Err(failure) => {
            rollback_board_write_transaction(transaction).await;
            Err(failure)
        }
    }
}

/// Ré-épingle et relit l'item d'un asset déjà présent sur la board.
async fn repin_existing_board_item(
    db: &Db,
    guild_id: &GuildId,
    asset: &MediaAsset,
    now: UtcDateTime,
) -> Result<CompletedBoardItem, IngestionFailure> {
    if let Err(service_error) = assets::pin(db, asset.id, now).await {
        return Err(match service_error {
            MediaServiceError::Database(db_error) => {
                completion_db_failure("overlay_media_board.repin_board_asset", &db_error)
            }
            other => media_service_failure(&other),
        });
    }
    match db
        .media_board()
        .get_by_guild_and_asset(guild_id, asset.id)
        .await
    {
        Ok(Some(existing_item)) => Ok(CompletedBoardItem {
            created: false,
            replaced: false,
            item: existing_item,
        }),
        Ok(None) => {
            tracing::error!("item du board introuvable après une ingestion concurrente");
            Err(internal_failure())
        }
        Err(db_error) => Err(completion_db_failure(
            "overlay_media_board.reload_item",
            &db_error,
        )),
    }
}

/// Résultat de l'insertion tentée dans la transaction d'écriture du board.
enum LockedCreate {
    /// Item inséré (et asset épinglé) dans la transaction.
    Created,
    /// L'asset a été inséré par une ingestion concurrente : l'appelant relit
    /// l'item après le rollback et le ré-épingle.
    Existing,
}

/// Insère l'item dans la transaction, après avoir relu le quota sur la même
/// connexion : la vérification et l'écriture ne peuvent plus s'intercaler.
async fn create_board_item_locked(
    state: &AppState,
    db: &Db,
    connection: &mut sqlx::SqliteConnection,
    guild_id: &GuildId,
    item: &MediaBoardItem,
    requested_bytes: i64,
    now: UtcDateTime,
) -> Result<LockedCreate, IngestionFailure> {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM media_board_items WHERE guild_id = ? AND media_asset_id = ?",
    )
    .bind(guild_id.as_str())
    .bind(item.media_asset_id.to_canonical())
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| {
        completion_db_failure(
            "overlay_media_board.board_lookup_tx",
            &lc_db::DbError::from(error),
        )
    })?;
    if existing.is_some() {
        return Ok(LockedCreate::Existing);
    }
    ensure_board_capacity_locked(state, connection, guild_id, 0, requested_bytes).await?;
    match db.media_board().create_on(connection, item).await {
        Ok(()) => {}
        Err(db_error) if is_conflict(&db_error) => return Ok(LockedCreate::Existing),
        Err(db_error) => {
            return Err(completion_db_failure(
                "overlay_media_board.insert_item",
                &db_error,
            ));
        }
    }
    assets::pin_on(connection, item.media_asset_id, now)
        .await
        .map_err(|db_error| {
            completion_db_failure("overlay_media_board.pin_board_asset", &db_error)
        })?;
    Ok(LockedCreate::Created)
}

/// Replaces the media of an existing board item with an ingested asset.
///
/// The item keeps its id, title, message, author, date, favorites and
/// picture. The new asset must be of the same kind as the previous one and
/// must not already be on the guild's board. The storage quota is checked on
/// the size difference inside the same transaction as the replacement, so a
/// concurrent replacement cannot exceed the limit; the new asset is pinned and
/// the previous one is released when no item shows it any more.
pub(crate) async fn replace_board_item_media(
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

    let release_seconds = i64::try_from(state.runtime().media_cache_ttl_hours())
        .unwrap_or(i64::MAX / 3_600)
        .saturating_mul(3_600);
    let released_expiry = now + time::Duration::seconds(release_seconds);

    let mut transaction = begin_board_write_transaction(db).await?;
    match replace_board_item_media_locked(
        state,
        db,
        &mut transaction,
        auth.guild_id(),
        item_id,
        &asset,
        now,
        released_expiry,
    )
    .await
    {
        Ok(previous_media_asset_id) => {
            commit_board_write_transaction(transaction).await?;
            Ok(CompletedBoardItem {
                created: false,
                replaced: previous_media_asset_id != asset.id,
                item: MediaBoardItem {
                    media_asset_id: asset.id,
                    ..item
                },
            })
        }
        Err(failure) => {
            rollback_board_write_transaction(transaction).await;
            Err(failure)
        }
    }
}

/// Replacement performed inside the board write transaction: re-reads the
/// item's previous size, checks the quota on the same connection, replaces and
/// pins.
async fn replace_board_item_media_locked(
    state: &AppState,
    db: &Db,
    connection: &mut sqlx::SqliteConnection,
    guild_id: &GuildId,
    item_id: MediaBoardItemId,
    asset: &MediaAsset,
    now: UtcDateTime,
    released_expiry: UtcDateTime,
) -> Result<MediaAssetId, IngestionFailure> {
    let previous_bytes: Option<i64> = sqlx::query_scalar(
        "SELECT a.size_bytes FROM media_board_items b \
           JOIN media_assets a ON a.id = b.media_asset_id \
          WHERE b.id = ? AND b.guild_id = ?",
    )
    .bind(item_id.to_canonical())
    .bind(guild_id.as_str())
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| {
        completion_db_failure(
            "overlay_media_board.replace_board_usage",
            &lc_db::DbError::from(error),
        )
    })?;
    let Some(previous_bytes) = previous_bytes else {
        return Err(replace_failure(
            "le média à remplacer n'est plus sur le board",
        ));
    };
    let taken: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM media_board_items \
          WHERE guild_id = ? AND media_asset_id = ? AND id <> ?",
    )
    .bind(guild_id.as_str())
    .bind(asset.id.to_canonical())
    .bind(item_id.to_canonical())
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| {
        completion_db_failure(
            "overlay_media_board.replace_board_lookup",
            &lc_db::DbError::from(error),
        )
    })?;
    if taken.is_some() {
        return Err(replace_failure(ALREADY_ON_BOARD));
    }
    ensure_board_capacity_locked(
        state,
        connection,
        guild_id,
        previous_bytes,
        asset.size_bytes,
    )
    .await?;
    let previous = db
        .media_board()
        .replace_media_on(
            connection,
            item_id,
            guild_id,
            asset.id,
            now,
            released_expiry,
        )
        .await
        .map_err(|db_error| {
            if is_conflict(&db_error) {
                replace_failure(ALREADY_ON_BOARD)
            } else {
                completion_db_failure("overlay_media_board.replace_media", &db_error)
            }
        })?;
    let Some(previous) = previous else {
        return Err(replace_failure(
            "le média à remplacer n'est plus sur le board",
        ));
    };
    assets::pin_on(connection, asset.id, now)
        .await
        .map_err(|db_error| {
            completion_db_failure("overlay_media_board.replace_pin_asset", &db_error)
        })?;
    Ok(previous)
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

/// Opens the board write transaction.
///
/// `BEGIN IMMEDIATE` takes the SQLite write lock before the quota is read, so
/// two concurrent additions or replacements are serialized: the second one
/// re-reads the committed total and cannot both pass the limit check.
///
/// This uses sqlx's tracked [`Transaction`](sqlx::Transaction) rather than raw
/// SQL: sqlx starts a rollback when the guard is dropped without a commit
/// (including during a panic), so a failed path cannot leave the connection
/// holding the SQLite write lock.
async fn begin_board_write_transaction(
    db: &Db,
) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, IngestionFailure> {
    db.pool()
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| {
            completion_db_failure("overlay_media_board.begin", &lc_db::DbError::from(error))
        })
}

/// Commits the board write transaction; sqlx starts a rollback by itself when
/// the commit fails.
async fn commit_board_write_transaction(
    transaction: sqlx::Transaction<'static, sqlx::Sqlite>,
) -> Result<(), IngestionFailure> {
    transaction.commit().await.map_err(|error| {
        completion_db_failure("overlay_media_board.commit", &lc_db::DbError::from(error))
    })
}

/// Rolls back the board write transaction; the caller is already on a failure
/// path, so a rollback error is only logged.
async fn rollback_board_write_transaction(transaction: sqlx::Transaction<'static, sqlx::Sqlite>) {
    if let Err(error) = transaction.rollback().await {
        tracing::warn!(error = %error, "rollback de la transaction board impossible");
    }
}

/// Storage limit of a guild's board, in bytes, read on the transaction's
/// connection.
///
/// The per-guild `board_quota_mb` column (migration 0011) overrides
/// `MEDIA_BOARD_MAX_TOTAL_MB` when it is set. A guild without a settings row
/// keeps the server-wide limit. The query mirrors
/// `GuildsRepository::board_quota_mb` but runs on the transaction's connection:
/// reading it through the pool would deadlock a one-connection test pool and,
/// more importantly, would not be part of the write transaction.
async fn board_limit_bytes(
    state: &AppState,
    connection: &mut sqlx::SqliteConnection,
    guild_id: &GuildId,
) -> Result<i64, IngestionFailure> {
    let quota: Option<i64> = sqlx::query_scalar("SELECT board_quota_mb FROM guilds WHERE id = ?")
        .bind(guild_id.as_str())
        .fetch_optional(&mut *connection)
        .await
        .map_err(|error| {
            completion_db_failure(
                "overlay_media_board.board_limit",
                &lc_db::DbError::from(error),
            )
        })?
        .flatten();
    let megabytes = match quota {
        Some(quota) => u64::try_from(quota).unwrap_or(1).max(1),
        None => state.runtime().media_board_max_total_mb().max(1),
    };
    Ok(i64::try_from(megabytes.saturating_mul(MIB)).unwrap_or(i64::MAX))
}

/// Re-checks the guild's board capacity on the transaction's connection.
///
/// `freed_bytes` is the size the write removes (0 for an addition, the replaced
/// asset's size for a replacement).
async fn ensure_board_capacity_locked(
    state: &AppState,
    connection: &mut sqlx::SqliteConnection,
    guild_id: &GuildId,
    freed_bytes: i64,
    requested_bytes: i64,
) -> Result<(), IngestionFailure> {
    let used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(a.size_bytes), 0) \
           FROM media_board_items b \
           JOIN media_assets a ON a.id = b.media_asset_id \
          WHERE b.guild_id = ?",
    )
    .bind(guild_id.as_str())
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| {
        completion_db_failure(
            "overlay_media_board.board_usage",
            &lc_db::DbError::from(error),
        )
    })?;
    let limit_bytes = board_limit_bytes(state, connection, guild_id).await?;
    if used
        .saturating_sub(freed_bytes.max(0))
        .saturating_add(requested_bytes)
        > limit_bytes
    {
        return Err(board_full_failure(limit_bytes));
    }
    Ok(())
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
    // Mêmes avertissements que la route de lecture : un refus ici laisse une
    // carte de board sans image, sans que le client puisse le distinguer d'un
    // média encore en cours de chargement.
    let (client_id_raw, exp_raw, sig) = match (query.client_id, query.exp, query.sig) {
        (Some(client_id), Some(exp), Some(sig)) => (client_id, exp, sig),
        _ => {
            tracing::warn!(
                asset_id = asset_id_raw,
                "média du board demandé sans signature : refus"
            );
            return super::media::media_url_invalid();
        }
    };
    let Ok(asset_id) = MediaAssetId::parse(asset_id_raw.trim()) else {
        tracing::warn!(
            asset_id = asset_id_raw,
            "identifiant de média du board illisible : refus"
        );
        return super::media::media_not_found();
    };
    let Ok(client_id) = OverlayClientId::parse(client_id_raw.trim()) else {
        tracing::warn!(
            client_id = client_id_raw,
            "client overlay illisible : média du board refusé"
        );
        return super::media::media_url_invalid();
    };
    let Ok(exp_sec) = exp_raw.trim().parse::<u64>() else {
        tracing::warn!(
            exp = exp_raw,
            "expiration illisible : média du board refusé"
        );
        return super::media::media_url_invalid();
    };
    if sig.len() != 64 || !sig.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        tracing::warn!(
            asset_id = %asset_id.to_canonical(),
            sig_len = sig.len(),
            "signature mal formée : média du board refusé"
        );
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
        tracing::warn!(
            asset_id = %asset_key,
            client_id = %client_key,
            exp_sec,
            "signature d'URL de média du board invalide : refus"
        );
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
        tracing::warn!(
            asset_id = %asset_key,
            client_id = %client_key,
            exp_sec,
            now_sec,
            "URL de média du board expirée : refus"
        );
        return super::media::media_url_expired();
    }

    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let client = match db.overlay_clients().get(client_id).await {
        Ok(Some(client)) => client,
        Ok(None) => {
            tracing::warn!(client_id = %client_key, "client overlay inconnu : média du board refusé");
            return super::media::media_url_invalid();
        }
        Err(db_error) => return db_failure("overlay_media_board.media_client", &db_error),
    };
    if !client.is_active() {
        tracing::warn!(
            client_id = %client_key,
            "client overlay inactif : média du board refusé"
        );
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
        Ok(false) => {
            tracing::warn!(
                asset_id = %asset_key,
                guild_id = client.guild_id.as_str(),
                "média absent du board de la guilde : refus"
            );
            return super::media::media_not_found();
        }
        Err(db_error) => return db_failure("overlay_media_board.media_binding", &db_error),
    }
    let asset = match db.media_assets().get(asset_id).await {
        Ok(Some(asset)) => asset,
        Ok(None) => {
            tracing::warn!(asset_id = %asset_key, "média du board inconnu : refus");
            return super::media::media_not_found();
        }
        Err(db_error) => return db_failure("overlay_media_board.media_asset", &db_error),
    };
    if !asset.is_ready() || asset.is_expired_at(timestamp) {
        tracing::warn!(
            asset_id = %asset_key,
            status = ?asset.status,
            expires_at = ?asset.expires_at,
            "média du board indisponible ou expiré : refus"
        );
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

    // Only an image takes a chosen display time. A scale applies to every
    // kind: the overlay sizes the picture of a video, and the artwork of a
    // sound, exactly like a picture.
    if matches!(display_duration, Some(Some(_))) {
        match db.media_assets().get(existing.media_asset_id).await {
            Ok(Some(asset)) if asset.kind == MediaKind::Image => {}
            Ok(_) => return Err(invalid_display_duration()),
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

/// Reads the optional `displayDurationSec` sent with a play once.
///
/// Absent or `null` → `None` (the guild default). The media kind is unknown
/// before the ingestion, so only the bounds are checked here: the value is
/// applied to the playback job when the ready asset is an image (see
/// [`super::media_ingestion`]).
pub(super) fn play_once_display_duration(
    raw: Option<&serde_json::Value>,
) -> Result<Option<i64>, Response> {
    match raw {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => match value.as_i64() {
            Some(seconds) if lc_domain::entities::is_valid_display_duration(seconds) => {
                Ok(Some(seconds))
            }
            _ => Err(invalid_play_once_duration()),
        },
    }
}

/// `400 invalid_payload` for a play-once display time out of bounds or not an
/// integer. The media kind is not known yet, so this message never mentions
/// the kind.
pub(super) fn invalid_play_once_duration() -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        serde_json::json!({
            "error": "invalid_payload",
            "message": "displayDurationSec must be an integer between 1 and 600, or null",
        }),
    )
}

/// `400 invalid_payload` for a display scale out of bounds, or not an integer.
fn invalid_display_scale() -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        serde_json::json!({
            "error": "invalid_payload",
            "message": "displayScalePercent must be an integer between 10 and 1000, or null",
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

/// B-03: map a joined `media_assets` row to a domain asset.
fn joined_asset(
    raw: &sqlx::sqlite::SqliteRow,
    prefix: &str,
) -> lc_db::DbResult<Option<MediaAsset>> {
    use sqlx::Row as _;

    let id_raw: Option<String> =
        raw.try_get::<Option<String>, _>(format!("{prefix}_id").as_str())?;
    let Some(id_raw) = id_raw else {
        return Ok(None);
    };
    let created_at_ms: i64 = raw.try_get::<i64, _>(format!("{prefix}_created_at").as_str())?;
    let expires_at_ms: i64 = raw.try_get::<i64, _>(format!("{prefix}_expires_at").as_str())?;
    let last_accessed_at_ms: i64 =
        raw.try_get::<i64, _>(format!("{prefix}_last_accessed_at").as_str())?;
    let duration_sec: Option<i64> = raw.try_get(format!("{prefix}_duration_sec").as_str())?;
    let width: Option<i64> = raw.try_get(format!("{prefix}_width").as_str())?;
    let height: Option<i64> = raw.try_get(format!("{prefix}_height").as_str())?;
    let size_bytes: i64 = raw.try_get::<i64, _>(format!("{prefix}_size_bytes").as_str())?;
    let is_vertical: i64 = raw.try_get::<i64, _>(format!("{prefix}_is_vertical").as_str())?;
    let kind_raw: String = raw.try_get(format!("{prefix}_kind").as_str())?;
    let status_raw: String = raw.try_get(format!("{prefix}_status").as_str())?;
    let parse_error = |detail: String| corrupt_row(detail);

    let id = MediaAssetId::parse(&id_raw)
        .map_err(|error| parse_error(format!("{prefix}_id invalide ({id_raw}): {error}")))?;
    let kind = lc_domain::enums::MediaKind::from_db_str(&kind_raw)
        .map_err(|_| parse_error(format!("{prefix}_kind invalide ({kind_raw})")))?;
    let status = lc_domain::enums::MediaStatus::from_db_str(&status_raw)
        .map_err(|_| parse_error(format!("{prefix}_status invalide ({status_raw})")))?;
    let created_at = lc_domain::time::ms_to_datetime(created_at_ms)
        .ok_or_else(|| parse_error(format!("{prefix}_created_at invalide ({created_at_ms})")))?;
    let expires_at = lc_domain::time::ms_to_datetime(expires_at_ms)
        .ok_or_else(|| parse_error(format!("{prefix}_expires_at invalide ({expires_at_ms})")))?;
    let last_accessed_at =
        lc_domain::time::ms_to_datetime(last_accessed_at_ms).ok_or_else(|| {
            parse_error(format!(
                "{prefix}_last_accessed_at invalide ({last_accessed_at_ms})"
            ))
        })?;

    Ok(Some(MediaAsset {
        id,
        source_hash: raw.try_get(format!("{prefix}_source_hash").as_str())?,
        source_url: raw.try_get(format!("{prefix}_source_url").as_str())?,
        source_title: raw.try_get(format!("{prefix}_source_title").as_str())?,
        kind,
        mime: raw.try_get(format!("{prefix}_mime").as_str())?,
        duration_sec,
        width,
        height,
        is_vertical: is_vertical != 0,
        storage_path: raw.try_get(format!("{prefix}_storage_path").as_str())?,
        size_bytes,
        status,
        error: raw.try_get(format!("{prefix}_error").as_str())?,
        last_accessed_at,
        expires_at,
        created_at,
    }))
}

/// B-03 (chef review) : `lower()` de SQLite ne replie que l'ASCII, alors que le
/// needle est déjà passé par `to_lowercase()` (Unicode) côté Rust. Quand le
/// needle contient des non-ASCII, on replie explicitement les majuscules
/// correspondantes avec `replace()` ; sinon on garde `lower(col)` tel quel.
fn folded_search_expr(column: &str, folds: &[(char, char)]) -> String {
    let mut expr = format!("lower({column})");
    for &(upper, lower) in folds {
        // Les littéraux insérés dans le SQL échappent `'` en le doublant.
        let upper_literal = upper.to_string().replace('\'', "''");
        let lower_literal = lower.to_string().replace('\'', "''");
        expr = format!("replace({expr}, '{upper_literal}', '{lower_literal}')");
    }
    expr
}

/// B-03: build the shared `WHERE` for the filter. `q` uses `instr(folded(col), ?)`
/// to keep the exact legacy substring semantics (`%`/`_` are literals), with
/// Unicode case folding matching Rust's `to_lowercase()` on the stored text.
fn board_filter_clause(search: Option<&str>) -> (String, Vec<String>) {
    let mut clause = String::from("b.guild_id = ?1");
    let mut binds: Vec<String> = Vec::new();
    if let Some(needle) = search {
        // Une seule passe sur le needle, bornée par ses caractères non-ASCII
        // distincts (pas par l'alphabet). Les uppercases multi-char sont
        // ignorés : c'est le seul cas exotique toléré par la spec B-03.
        let mut folds: Vec<(char, char)> = Vec::new();
        if !needle.is_ascii() {
            for lower in needle.chars().filter(|c| !c.is_ascii()) {
                let mut uppercase = lower.to_uppercase();
                let upper = match (uppercase.next(), uppercase.next()) {
                    (Some(upper), None) if upper != lower => upper,
                    _ => continue,
                };
                if !folds.iter().any(|(known, _)| *known == upper) {
                    folds.push((upper, lower));
                }
            }
        }
        let folded = |column: &str| folded_search_expr(column, &folds);
        // `created_by_name` is the name copied at creation time; the active
        // member display name is matched through the two author paths used
        // by `BoardAuthors::resolve` (legacy `row_matches` matches the
        // resolved author name).
        clause.push_str(&format!(
            " AND (instr({title}, ?2) > 0 \
                  OR instr({message}, ?2) > 0 \
                  OR instr({by_name}, ?2) > 0 \
                  OR instr({author}, ?2) > 0 \
                  OR instr({url}, ?2) > 0)",
            title = folded("b.title"),
            message = folded("b.message"),
            by_name = folded("b.created_by_name"),
            author = folded(
                "COALESCE(author_member.display_name, \
                           author_discord.display_name, b.created_by_name)"
            ),
            url = folded("a.source_url"),
        ));
        binds.push(needle.to_owned());
    }
    (clause, binds)
}

/// B-03: all page rows, with every joined asset in the same query.
///
/// `LEFT JOIN` on artwork/cover keeps rows whose extra assets are missing or
/// not ready — the payload then omits that field, exactly as before. The main
/// asset uses an inner join (legacy skipped rows with no asset), and `total`
/// is computed by [`count_board_rows`] with the same filter.
async fn load_board_page(
    db: &Db,
    guild_id: &GuildId,
    search: Option<&str>,
    limit: usize,
    offset: usize,
) -> lc_db::DbResult<Vec<BoardRow>> {
    use sqlx::Row as _;

    let (filter, binds) = board_filter_clause(search);
    let (limit_ph, offset_ph) = if search.is_some() {
        ("?3", "?4")
    } else {
        ("?2", "?3")
    };
    let sql = format!(
        "SELECT b.id, b.guild_id, b.media_asset_id, b.title, b.message, \
                b.created_by_discord_user_id, b.created_by_name, b.created_at, b.created_by_member_id, \
                b.artwork_asset_id, b.artwork_on_overlay, b.display_duration_sec, b.display_scale_percent, \
                a.id AS asset_id, a.source_hash AS asset_source_hash, a.source_url AS asset_source_url, \
                a.source_title AS asset_source_title, a.kind AS asset_kind, a.mime AS asset_mime, \
                a.duration_sec AS asset_duration_sec, a.width AS asset_width, a.height AS asset_height, \
                a.is_vertical AS asset_is_vertical, a.storage_path AS asset_storage_path, \
                a.size_bytes AS asset_size_bytes, a.status AS asset_status, a.error AS asset_error, \
                a.last_accessed_at AS asset_last_accessed_at, a.expires_at AS asset_expires_at, \
                a.created_at AS asset_created_at, \
                art.id AS art_id, art.source_hash AS art_source_hash, art.source_url AS art_source_url, \
                art.source_title AS art_source_title, art.kind AS art_kind, art.mime AS art_mime, \
                art.duration_sec AS art_duration_sec, art.width AS art_width, art.height AS art_height, \
                art.is_vertical AS art_is_vertical, art.storage_path AS art_storage_path, \
                art.size_bytes AS art_size_bytes, art.status AS art_status, art.error AS art_error, \
                art.last_accessed_at AS art_last_accessed_at, art.expires_at AS art_expires_at, \
                art.created_at AS art_created_at, \
                thumb.id AS thumb_id, thumb.source_hash AS thumb_source_hash, \
                thumb.source_url AS thumb_source_url, thumb.source_title AS thumb_source_title, \
                thumb.kind AS thumb_kind, thumb.mime AS thumb_mime, \
                thumb.duration_sec AS thumb_duration_sec, thumb.width AS thumb_width, \
                thumb.height AS thumb_height, thumb.is_vertical AS thumb_is_vertical, \
                thumb.storage_path AS thumb_storage_path, thumb.size_bytes AS thumb_size_bytes, \
                thumb.status AS thumb_status, thumb.error AS thumb_error, \
                thumb.last_accessed_at AS thumb_last_accessed_at, thumb.expires_at AS thumb_expires_at, \
                thumb.created_at AS thumb_created_at \
           FROM media_board_items b \
           JOIN media_assets a ON a.id = b.media_asset_id \
           LEFT JOIN guild_members author_member \
                  ON author_member.id = b.created_by_member_id \
                 AND author_member.removed_at IS NULL \
           LEFT JOIN guild_members author_discord \
                  ON author_discord.discord_user_id = b.created_by_discord_user_id \
                 AND author_discord.removed_at IS NULL \
           LEFT JOIN media_assets art \
                  ON art.id = b.artwork_asset_id AND art.status = 'READY' \
           LEFT JOIN media_assets cover ON cover.id = a.cover_asset_id \
           LEFT JOIN media_assets thumb \
                  ON thumb.id = cover.id AND thumb.status = 'READY' AND thumb.kind = 'IMAGE' \
          WHERE {filter} \
          ORDER BY b.created_at DESC, b.id ASC \
          LIMIT {limit_ph} OFFSET {offset_ph}"
    );
    let mut query = sqlx::query(&sql).bind(guild_id.as_str());
    for value in &binds {
        query = query.bind(value.as_str());
    }
    let raw_rows = query
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .bind(i64::try_from(offset).unwrap_or(i64::MAX))
        .fetch_all(db.pool())
        .await
        .map_err(lc_db::DbError::from)?;

    let mut rows = Vec::with_capacity(raw_rows.len());
    for raw in raw_rows {
        let item_id_raw: String = raw.try_get("id")?;
        let guild_raw: String = raw.try_get("guild_id")?;
        let asset_id_raw: String = raw.try_get("media_asset_id")?;
        let created_at_ms: i64 = raw.try_get("created_at")?;
        let artwork_raw: Option<String> = raw.try_get("artwork_asset_id")?;
        let artwork_on_overlay: i64 = raw.try_get("artwork_on_overlay")?;
        let item = MediaBoardItem {
            id: MediaBoardItemId::parse(&item_id_raw)
                .map_err(|error| corrupt_row(format!("id invalide ({item_id_raw}): {error}")))?,
            guild_id: GuildId::parse(&guild_raw).map_err(|error| {
                corrupt_row(format!("guild_id invalide ({guild_raw}): {error}"))
            })?,
            media_asset_id: MediaAssetId::parse(&asset_id_raw).map_err(|error| {
                corrupt_row(format!("media_asset_id invalide ({asset_id_raw}): {error}"))
            })?,
            title: raw.try_get("title")?,
            message: raw.try_get("message")?,
            created_by_discord_user_id: raw.try_get("created_by_discord_user_id")?,
            created_by_name: raw.try_get("created_by_name")?,
            created_at: lc_domain::time::ms_to_datetime(created_at_ms)
                .ok_or_else(|| corrupt_row(format!("created_at invalide ({created_at_ms})")))?,
            artwork_asset_id: artwork_raw
                .as_deref()
                .map(MediaAssetId::parse)
                .transpose()
                .map_err(|error| corrupt_row(format!("artwork_asset_id invalide: {error}")))?,
            artwork_on_overlay: artwork_on_overlay != 0,
            display_duration_sec: raw.try_get("display_duration_sec")?,
            display_scale_percent: raw.try_get("display_scale_percent")?,
        };
        rows.push(BoardRow {
            item,
            asset: joined_asset(&raw, "asset")?,
            artwork: joined_asset(&raw, "art")?,
            thumbnail: joined_asset(&raw, "thumb")?,
            created_by_member_id: raw.try_get("created_by_member_id")?,
            author: None,
        });
    }
    Ok(rows)
}

/// B-03: count matching rows with the exact same filter as the page query.
async fn count_board_rows(
    db: &Db,
    guild_id: &GuildId,
    search: Option<&str>,
) -> lc_db::DbResult<usize> {
    use sqlx::Row as _;

    let (filter, binds) = board_filter_clause(search);
    let sql = format!(
        "SELECT COUNT(*) AS total FROM media_board_items b \
                JOIN media_assets a ON a.id = b.media_asset_id \
                LEFT JOIN guild_members author_member \
                       ON author_member.id = b.created_by_member_id \
                      AND author_member.removed_at IS NULL \
                LEFT JOIN guild_members author_discord \
                       ON author_discord.discord_user_id = b.created_by_discord_user_id \
                      AND author_discord.removed_at IS NULL \
          WHERE {filter}"
    );
    let mut query = sqlx::query(&sql).bind(guild_id.as_str());
    for value in &binds {
        query = query.bind(value.as_str());
    }
    let total: i64 = query
        .fetch_one(db.pool())
        .await
        .map_err(lc_db::DbError::from)?
        .try_get("total")?;
    Ok(usize::try_from(total).unwrap_or(0))
}
/// Ligne de board paginée en SQL : item et assets joints en une seule requête.
struct BoardRow {
    item: MediaBoardItem,
    /// Asset principal (joint en SQL), absent si la ligne est orpheline.
    asset: Option<MediaAsset>,
    /// Illustration prête, si elle existe.
    artwork: Option<MediaAsset>,
    /// Miniature de la vidéo (couverture prête), si elle existe.
    thumbnail: Option<MediaAsset>,
    /// Membre auteur (`media_board_items.created_by_member_id`), s'il est connu.
    created_by_member_id: Option<String>,
    /// Auteur résolu pour l'affichage (voir [`BoardAuthors`]).
    author: Option<BoardAuthor>,
}

/// Auteur canonique d'un média du board.
struct BoardAuthor {
    key: String,
    name: String,
}

/// Index des membres actifs de la guilde, pour attribuer chaque média à son auteur.
struct BoardAuthors<'a> {
    by_member_id: HashMap<String, &'a GuildMember>,
    by_discord_user_id: HashMap<String, &'a GuildMember>,
}

impl<'a> BoardAuthors<'a> {
    fn new(members: &'a [GuildMember]) -> Self {
        let mut by_member_id = HashMap::with_capacity(members.len());
        let mut by_discord_user_id = HashMap::new();
        for member in members {
            by_member_id.insert(member.id.to_canonical(), member);
            if let Some(discord_user_id) = member.discord_user_id.as_deref() {
                by_discord_user_id.insert(discord_user_id.to_owned(), member);
            }
        }
        Self {
            by_member_id,
            by_discord_user_id,
        }
    }

    fn resolve(&self, row: &BoardRow) -> Option<BoardAuthor> {
        let member = row
            .created_by_member_id
            .as_deref()
            .and_then(|member_id| self.by_member_id.get(member_id).copied())
            .or_else(|| {
                row.item
                    .created_by_discord_user_id
                    .as_deref()
                    .and_then(|discord_user_id| {
                        self.by_discord_user_id.get(discord_user_id).copied()
                    })
            });
        if let Some(member) = member {
            return Some(BoardAuthor {
                key: format!("member:{}", member.id.to_canonical()),
                name: member.display_name.clone(),
            });
        }
        let name = row.item.created_by_name.clone()?;
        let key = match (
            row.created_by_member_id.as_deref(),
            row.item.created_by_discord_user_id.as_deref(),
        ) {
            (Some(member_id), _) => format!("member:{member_id}"),
            (None, Some(discord_user_id)) => format!("discord:{discord_user_id}"),
            (None, None) => format!("name:{name}"),
        };
        Some(BoardAuthor { key, name })
    }
}
/// Erreur `Corrupt` ciblée pour une ligne de board illisible.
fn corrupt_row(detail: String) -> lc_db::DbError {
    lc_db::DbError::Corrupt {
        table: "media_board_items",
        detail,
    }
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
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::{
        atomic::{self, AtomicBool},
        Arc, Mutex, OnceLock,
    };

    use lc_domain::{
        FixedClock, Guild, GuildId, GuildMember, GuildMemberId, GuildRole, MediaAsset,
        MediaAssetId, MediaBoardItem, MediaBoardItemId, MediaKind, MediaStatus, OverlayClient,
        OverlayClientId, OverlaySessionMode,
    };
    use serde_json::json;
    use tempfile::TempDir;
    use time::{Duration as TimeDuration, UtcDateTime};
    use tracing::dispatcher::Dispatch;
    use tracing::Level;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::Layer;

    /// sqlx-sqlite émet ses événements SQL sur les threads de ses connexions.
    /// On compte les événements dont le champ `db.statement` est précisément
    /// `SELECT * FROM media_assets WHERE id = ?` : c'est la requête du N+1
    /// legacy (2 fois par item), absente de la liste paginée. Le filtrage par
    /// requête rend le test insensible au SQL des autres tests parallèles.
    struct AssetGetCounter {
        count: Arc<Mutex<usize>>,
        total: Arc<Mutex<usize>>,
        active: Arc<AtomicBool>,
    }

    impl<S: tracing::Subscriber> Layer<S> for AssetGetCounter {
        fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
            if !self.active.load(atomic::Ordering::Acquire)
                || event.metadata().target() != "sqlx::query"
                || *event.metadata().level() != Level::DEBUG
            {
                return;
            }
            // Total brut : prouve que l'instrumentation capte bien le SQL
            // pendant la fenêtre armée (un compteur à zéro est suspect).
            *self.total.lock().expect("compteur SQL total") += 1;
            struct StatementVisitor(bool);
            impl tracing::field::Visit for StatementVisitor {
                fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                    if field.name() == "db.statement" {
                        self.0 = value.trim().contains("FROM media_assets WHERE id = ?");
                    }
                }
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if field.name() == "db.statement" {
                        let text = format!("{value:?}");
                        self.0 = text.contains("FROM media_assets WHERE id = ?");
                    }
                }
            }
            let mut visitor = StatementVisitor(false);
            event.record(&mut visitor);
            if visitor.0 {
                *self.count.lock().expect("compteur SQL") += 1;
            }
        }
    }

    /// Compteur global ciblé : (lookups asset, total SQL, fenêtre armée).
    type AssetCounter = (Arc<Mutex<usize>>, Arc<Mutex<usize>>, Arc<AtomicBool>);

    /// Installé une seule fois avec un filtre DEBUG pour rendre les événements
    /// sqlx visibles.
    fn asset_get_counter() -> AssetCounter {
        static COUNTER: OnceLock<AssetCounter> = OnceLock::new();
        COUNTER
            .get_or_init(|| {
                let count = Arc::new(Mutex::new(0usize));
                let total = Arc::new(Mutex::new(0usize));
                let active = Arc::new(AtomicBool::new(false));
                let subscriber = tracing_subscriber::registry()
                    .with(AssetGetCounter {
                        count: count.clone(),
                        total: total.clone(),
                        active: active.clone(),
                    })
                    .with(tracing_subscriber::filter::LevelFilter::DEBUG);
                tracing::dispatcher::set_global_default(Dispatch::new(subscriber))
                    .expect("subscriber SQL global");
                (count, total, active)
            })
            .clone()
    }

    use super::super::auth::{hash_token, AuthedOverlayClient};
    use super::{
        coerce_flag, list_items_payload, normalize_limit, normalize_offset, ItemTitle, ListQuery,
    };
    use crate::app::AppState;
    use crate::config::Config;
    use crate::http::overlay::ActorInfo;
    use crate::shutdown::Shutdown;

    fn fixed_now() -> UtcDateTime {
        time::macros::utc_datetime!(2026-11-10 12:00:00)
    }

    fn guild() -> GuildId {
        GuildId::parse("123456789012345678").expect("guild id valide")
    }

    fn test_config(media_dir: &std::path::Path) -> Config {
        let pairs: Vec<(String, String)> = vec![
            ("API_URL".to_owned(), "http://127.0.0.1:8099".to_owned()),
            (
                "MEDIA_URL_SECRET".to_owned(),
                "secret-de-test-overlay".to_owned(),
            ),
            (
                "MEDIA_STORAGE_DIR".to_owned(),
                media_dir.to_string_lossy().into_owned(),
            ),
        ];
        Config::from_pairs(pairs).expect("configuration de test valide")
    }

    fn overlay_client(token: &str) -> OverlayClient {
        OverlayClient {
            id: OverlayClientId::new_v4(),
            guild_id: guild(),
            label: "Poste".to_owned(),
            token_hash: hash_token(token),
            session_mode: OverlaySessionMode::Normal,
            default_author_name: None,
            default_author_image: None,
            created_by_discord_user_id: None,
            device_hash: None,
            last_seen_at: None,
            created_at: fixed_now(),
            revoked_at: None,
        }
    }

    fn member(discord_user_id: Option<&str>) -> GuildMember {
        GuildMember {
            id: GuildMemberId::new_v4(),
            guild_id: guild(),
            display_name: "Alice".to_owned(),
            avatar: None,
            role: GuildRole::Member,
            created_at: fixed_now() - TimeDuration::minutes(1),
            removed_at: None,
            discord_user_id: discord_user_id.map(str::to_owned),
            profile_custom: false,
            discord_synced_at: None,
        }
    }

    async fn insert_asset(
        db: &lc_db::Db,
        media_dir: &std::path::Path,
        name: &str,
        kind: MediaKind,
    ) -> MediaAsset {
        let path = media_dir.join(name);
        tokio::fs::write(&path, b"asset")
            .await
            .expect("écriture média");
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: format!("hash-{name}"),
            source_url: format!("https://example.test/{name}"),
            source_title: Some(format!("Source {name}")),
            kind,
            mime: "video/mp4".to_owned(),
            duration_sec: Some(30),
            width: None,
            height: None,
            is_vertical: false,
            storage_path: path.to_string_lossy().into_owned(),
            size_bytes: 5,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: fixed_now(),
            expires_at: fixed_now() + TimeDuration::hours(1),
            created_at: fixed_now(),
        };
        db.media_assets()
            .insert(&asset)
            .await
            .expect("insertion média");
        asset
    }

    async fn insert_item(
        db: &lc_db::Db,
        asset: &MediaAsset,
        title: Option<&str>,
        message: Option<&str>,
        author_name: Option<&str>,
        created_at: UtcDateTime,
    ) -> MediaBoardItemId {
        let item = MediaBoardItem {
            id: MediaBoardItemId::new_v4(),
            guild_id: guild(),
            media_asset_id: asset.id,
            title: title.map(str::to_owned),
            message: message.map(str::to_owned),
            created_by_discord_user_id: None,
            created_by_name: author_name.map(str::to_owned),
            created_at,
            artwork_asset_id: None,
            artwork_on_overlay: false,
            display_duration_sec: None,
            display_scale_percent: None,
        };
        db.media_board()
            .create(&item)
            .await
            .expect("insertion item");
        item.id
    }

    async fn harness(media_dir: &std::path::Path) -> (AppState, lc_db::Db, OverlayClient) {
        let db = lc_db::Db::open_in_memory(&lc_db::DbConfig::default())
            .await
            .expect("base en mémoire");
        db.guilds()
            .upsert(&Guild {
                id: guild(),
                name: Some("Test".to_owned()),
                busy_until: None,
                default_media_time: None,
                max_media_time: None,
                display_media_full: false,
            })
            .await
            .expect("guild");
        let client = overlay_client("token-de-test");
        db.overlay_clients().create(&client).await.expect("client");
        let clock: Arc<dyn lc_domain::Clock> = Arc::new(FixedClock::new(fixed_now()));
        let state = AppState::new(
            Arc::new(test_config(media_dir)),
            Some(db.clone()),
            Shutdown::detached(),
        )
        .with_clock(clock);
        (state, db, client)
    }

    fn auth(client: &OverlayClient, member: Option<GuildMember>) -> AuthedOverlayClient {
        AuthedOverlayClient {
            client: client.clone(),
            member,
            authenticated_at: fixed_now(),
        }
    }

    async fn list(
        state: &AppState,
        client: &OverlayClient,
        member: Option<GuildMember>,
        query: ListQuery,
    ) -> serde_json::Value {
        let actor = ActorInfo::from_auth(state, &auth(client, member));
        list_items_payload(state, &actor, &query)
            .await
            .expect("liste du board")
    }

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

    async fn seeded_board(
        media_dir: &std::path::Path,
    ) -> (AppState, lc_db::Db, OverlayClient, Vec<MediaBoardItemId>) {
        let (state, db, client) = harness(media_dir).await;
        let a = insert_asset(&db, media_dir, "alpha.mp4", MediaKind::Video).await;
        let b = insert_asset(&db, media_dir, "beta.mp4", MediaKind::Video).await;
        let c = insert_asset(&db, media_dir, "gamma.mp4", MediaKind::Video).await;
        let t0 = fixed_now() - TimeDuration::minutes(3);
        let mut ids = Vec::new();
        ids.push(
            insert_item(
                &db,
                &a,
                Some("100% Alpha"),
                Some("note _ brute"),
                Some("Alice"),
                t0,
            )
            .await,
        );
        ids.push(
            insert_item(
                &db,
                &b,
                Some("Beta"),
                Some("note normale"),
                Some("Bob"),
                t0 + TimeDuration::minutes(1),
            )
            .await,
        );
        ids.push(
            insert_item(
                &db,
                &c,
                Some("Gamma"),
                None,
                Some("Alice"),
                t0 + TimeDuration::minutes(2),
            )
            .await,
        );
        (state, db, client, ids)
    }

    fn query(q: Option<&str>, limit: Option<&str>, offset: Option<&str>) -> ListQuery {
        ListQuery {
            q: q.map(str::to_owned),
            limit: limit.map(str::to_owned),
            offset: offset.map(str::to_owned),
            guild_id: None,
        }
    }

    fn titles(body: &serde_json::Value) -> Vec<String> {
        body["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["title"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// `q` / `limit` / `offset` must keep the exact legacy semantics: substring
    /// matching (with literal `%`/`_` and case insensitivity), stable order,
    /// clamped limits, and empty pages past the end of the board.
    #[tokio::test]
    async fn board_list_keeps_legacy_filter_pagination_and_order() {
        let dir = TempDir::new().expect("tempdir");
        let (state, _db, client, ids) = seeded_board(dir.path()).await;
        let expected_all = vec![
            "Gamma".to_owned(),
            "Beta".to_owned(),
            "100% Alpha".to_owned(),
        ];

        for (label, q, limit, offset, want_total, want_limit, want_offset, want_titles) in [
            ("défaut", None, None, None, 3, 50, 0, expected_all.clone()),
            (
                "q vide",
                Some(""),
                None,
                None,
                3,
                50,
                0,
                expected_all.clone(),
            ),
            ("q inconnu", Some("zzz"), None, None, 0, 50, 0, Vec::new()),
            (
                "q insensible à la casse",
                Some("alPhA"),
                None,
                None,
                1,
                50,
                0,
                vec!["100% Alpha".to_owned()],
            ),
            (
                "q avec % littéral",
                Some("100%"),
                None,
                None,
                1,
                50,
                0,
                vec!["100% Alpha".to_owned()],
            ),
            (
                "q avec _ littéral",
                Some("note _ brute"),
                None,
                None,
                1,
                50,
                0,
                vec!["100% Alpha".to_owned()],
            ),
            (
                "q sur message",
                Some("note normale"),
                None,
                None,
                1,
                50,
                0,
                vec!["Beta".to_owned()],
            ),
            (
                "q sur auteur",
                Some("ALICE"),
                None,
                None,
                2,
                50,
                0,
                vec!["Gamma".to_owned(), "100% Alpha".to_owned()],
            ),
            (
                "q sur URL source",
                Some("beta.mp4"),
                None,
                None,
                1,
                50,
                0,
                vec!["Beta".to_owned()],
            ),
            (
                "limite legacy 0",
                None,
                Some("0"),
                None,
                3,
                1,
                0,
                vec!["Gamma".to_owned()],
            ),
            (
                "limite legacy 999",
                None,
                Some("999"),
                None,
                3,
                200,
                0,
                expected_all.clone(),
            ),
            (
                "page",
                None,
                Some("2"),
                Some("1"),
                3,
                2,
                1,
                vec!["Beta".to_owned(), "100% Alpha".to_owned()],
            ),
            (
                "offset > total",
                None,
                Some("2"),
                Some("99"),
                3,
                2,
                99,
                Vec::new(),
            ),
            (
                "offset négatif",
                None,
                Some("2"),
                Some("-4"),
                3,
                2,
                0,
                vec!["Gamma".to_owned(), "Beta".to_owned()],
            ),
        ] {
            let body = list(&state, &client, None, query(q, limit, offset)).await;
            assert_eq!(body["total"], want_total, "{label}");
            assert_eq!(body["limit"], want_limit, "{label}");
            assert_eq!(body["offset"], want_offset, "{label}");
            assert_eq!(titles(&body), want_titles, "{label}");
        }

        // Les identifiants et l'ordre restent stables pour la page par défaut.
        let body = list(&state, &client, None, query(None, None, None)).await;
        let got_ids: Vec<String> = body["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["id"].as_str().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(
            got_ids,
            vec![
                ids[2].to_canonical(),
                ids[1].to_canonical(),
                ids[0].to_canonical()
            ]
        );
    }

    /// B-03 (chef review) : `lower()` de SQLite ne replie que l'ASCII. Le
    /// legacy comparait `to_lowercase().contains()` (Unicode) : une majuscule
    /// accentuée stockée doit donc être repliée quand le needle est accentué.
    #[tokio::test]
    async fn board_list_folds_accented_case_in_sql_filter() {
        let dir = TempDir::new().expect("tempdir");
        let (state, db, client) = harness(dir.path()).await;
        let a = insert_asset(&db, dir.path(), "acc-a.mp4", MediaKind::Video).await;
        let b = insert_asset(&db, dir.path(), "acc-b.mp4", MediaKind::Video).await;
        let c = insert_asset(&db, dir.path(), "acc-c.mp4", MediaKind::Video).await;
        let t0 = fixed_now() - TimeDuration::minutes(3);
        insert_item(
            &db,
            &a,
            Some("ÉLODIE"),
            Some("100% CAFÉ"),
            Some("Alpha"),
            t0,
        )
        .await;
        insert_item(
            &db,
            &b,
            Some("élodie"),
            Some("autre message"),
            Some("Beta"),
            t0 + TimeDuration::minutes(1),
        )
        .await;
        insert_item(
            &db,
            &c,
            Some("CAFÉ 100%"),
            Some("gamma"),
            Some("Gamma"),
            t0 + TimeDuration::minutes(2),
        )
        .await;

        // Needle accentué : la majuscule accentuée stockée doit être repliée.
        let body = list(&state, &client, None, query(Some("élodie"), None, None)).await;
        assert_eq!(body["total"], 2, "élodie doit trouver ÉLODIE et élodie");
        assert_eq!(
            titles(&body),
            vec!["élodie".to_owned(), "ÉLODIE".to_owned()],
            "le titre ÉLODIE doit correspondre à élodie"
        );

        // Le needle est lowercasé côté Rust avant le SQL : la même recherche
        // écrite en majuscules accentuées doit donner exactement le même lot.
        let body = list(&state, &client, None, query(Some("ÉLODIE"), None, None)).await;
        assert_eq!(
            titles(&body),
            vec!["élodie".to_owned(), "ÉLODIE".to_owned()],
            "ÉLODIE doit être replié en élodie côté Rust"
        );

        // CAFÉ dans un message et dans un titre, avec un needle accentué.
        let body = list(&state, &client, None, query(Some("café"), None, None)).await;
        assert_eq!(body["total"], 2, "message CAFÉ et titre CAFÉ 100%");
        assert_eq!(
            titles(&body),
            vec!["CAFÉ 100%".to_owned(), "ÉLODIE".to_owned()]
        );

        // Parité ASCII conservée : 'elodie'/'cafe' ne doivent PAS trouver les
        // formes accentuées (le legacy to_lowercase().contains() non plus).
        for needle in ["elodie", "cafe"] {
            let body = list(&state, &client, None, query(Some(needle), None, None)).await;
            assert_eq!(body["total"], 0, "{needle} (ASCII) ne doit pas replier É");
            assert!(titles(&body).is_empty(), "{needle} (ASCII)");
        }

        // `%` et `_` restent littéraux dans le chemin replié (pas de LIKE).
        let body = list(&state, &client, None, query(Some("café 100%"), None, None)).await;
        assert_eq!(titles(&body), vec!["CAFÉ 100%".to_owned()], "% littéral");
        let body = list(&state, &client, None, query(Some("café _00%"), None, None)).await;
        assert!(titles(&body).is_empty(), "_ littéral dans le chemin replié");
    }

    /// Le filtre replié doit garder la pagination SQL (limit/offset/total).
    #[tokio::test]
    async fn board_list_unicode_filter_keeps_pagination() {
        let dir = TempDir::new().expect("tempdir");
        let (state, db, client) = harness(dir.path()).await;
        for (index, title) in ["ÉLODIE", "élodie", "CAFÉ"].iter().enumerate() {
            let asset = insert_asset(
                &db,
                dir.path(),
                &format!("page-{index}.mp4"),
                MediaKind::Video,
            )
            .await;
            insert_item(
                &db,
                &asset,
                Some(title),
                None,
                Some("ÉCRIVAIN"),
                fixed_now() - TimeDuration::minutes(3 - i64::try_from(index).expect("index i64")),
            )
            .await;
        }

        let body = list(
            &state,
            &client,
            None,
            query(Some("écrivain"), Some("1"), Some("1")),
        )
        .await;
        assert_eq!(body["total"], 3, "total du filtre replié");
        assert_eq!(body["limit"], 1);
        assert_eq!(body["offset"], 1);
        assert_eq!(titles(&body), vec!["élodie".to_owned()], "2e item accentué");
    }

    /// Enrichissements batch (auteur membre, favori, artwork, miniature) =
    /// mêmes champs que l'ancien chemin N+1, pour chaque item de la page.
    #[tokio::test]
    async fn board_list_keeps_author_favorite_artwork_and_thumbnail_fields() {
        let dir = TempDir::new().expect("tempdir");
        let (state, db, client, ids) = seeded_board(dir.path()).await;
        let author = member(Some("4242"));
        let member_id = author.id;
        db.guild_members().create(&author).await.expect("membre");
        db.media_board()
            .set_creator_member(ids[0], member_id)
            .await
            .expect("auteur");
        db.media_board()
            .set_favorite(member_id, ids[0], true, fixed_now())
            .await
            .expect("favori");

        let cover = insert_asset(&db, dir.path(), "cover.png", MediaKind::Image).await;
        db.media_assets()
            .set_cover(
                /* vidéo alpha = ids[0] */
                {
                    let body = list(&state, &client, None, query(None, None, None)).await;
                    let asset = body["items"][2]["media"]["id"].as_str().expect("media id");
                    MediaAssetId::parse(asset).expect("asset id")
                },
                Some(cover.id),
            )
            .await
            .expect("pochette");
        // L'artwork doit être prêt ; la pochette non prête doit être ignorée.
        db.media_assets()
            .update_status(cover.id, MediaStatus::Ready, None)
            .await
            .expect("cover ready");

        let body = list(
            &state,
            &client,
            Some(author.clone()),
            query(None, None, None),
        )
        .await;
        let alpha = &body["items"][2];
        assert_eq!(alpha["createdByName"], "Alice");
        assert_eq!(alpha["createdByMemberId"], json!(member_id.to_canonical()));
        assert_eq!(
            alpha["createdByAuthorId"],
            json!(format!("member:{}", member_id.to_canonical()))
        );
        assert_eq!(alpha["canEdit"], json!(true));
        assert_eq!(alpha["favorite"], json!(true));
        assert!(alpha["thumbnail"].is_object(), "{alpha}");
        assert_eq!(alpha["thumbnail"]["width"], json!(cover.width));
        assert_eq!(body["items"][1]["favorite"], json!(false));
        assert!(body["items"][1]["thumbnail"].is_null());
    }

    /// Insert 200 assets/items en une seule écriture groupée : le montage ne
    /// doit pas dominer les mesures de perf ou de nombre de requêtes.
    async fn seed_200(db: &lc_db::Db, _media_dir: &std::path::Path) {
        const ITEMS: usize = 200;
        let mut assets = String::from("INSERT INTO media_assets (id, source_hash, source_url, source_title, kind, mime, duration_sec, width, height, is_vertical, storage_path, size_bytes, status, error, last_accessed_at, expires_at, created_at) VALUES ");
        let mut items = String::from("INSERT INTO media_board_items (id, guild_id, media_asset_id, title, message, created_by_discord_user_id, created_by_name, created_at, created_by_member_id, artwork_asset_id, artwork_on_overlay, display_duration_sec, display_scale_percent) VALUES ");
        let now_ms = lc_domain::time::datetime_to_ms(fixed_now());
        for index in 0..ITEMS {
            if index > 0 {
                assets.push_str(", ");
                items.push_str(", ");
            }
            let asset_id = MediaAssetId::new_v4().to_canonical();
            assets.push_str(&format!(
                "('{asset_id}', 'hash-{index}', 'https://example.test/media-{index}', 'Source {index}', 'VIDEO', 'video/mp4', 30, 64, 64, 0, 'media-{index}', 5, 'READY', NULL, {now_ms}, {now_ms}, {now_ms})"
            ));
            items.push_str(&format!(
                "('{}', '{}', '{asset_id}', 'Item {index}', 'message {index}', NULL, 'Auteur', {}, NULL, NULL, 0, NULL, NULL)",
                MediaBoardItemId::new_v4().to_canonical(),
                guild().as_str(),
                now_ms - i64::try_from(index).expect("index i64"),
            ));
        }
        let mut connection = db.pool().acquire().await.expect("connexion");
        sqlx::query(&assets)
            .execute(&mut *connection)
            .await
            .expect("insertion assets");
        sqlx::query(&items)
            .execute(&mut *connection)
            .await
            .expect("insertion items");
    }

    /// B-03 : une page de 200 items ne doit déclencher aucune recherche
    /// d'asset par item (le N+1 du code legacy : ~2 `get(asset_id)`/item).
    #[tokio::test]
    async fn board_list_page_of_200_uses_a_constant_number_of_queries() {
        // Une page de 2 items et une page de 200 items doivent provoquer le
        // même nombre de `get(asset_id)` : le legacy en faisait ~2 par item,
        // soit +396 lookups entre les deux pages. Mesurer le DELTA annule le
        // bruit des autres tests parallèles, qui émettent du SQL dans la même
        // fenêtre (le compteur sqlx est global par construction).
        const MAX_DELTA: usize = 16;
        const ITEMS: usize = 200;
        let (counter, total, active) = asset_get_counter();

        let dir = TempDir::new().expect("tempdir");
        let (state, db, client) = harness(dir.path()).await;
        seed_200(&db, dir.path()).await;

        *counter.lock().expect("compteur SQL") = 0;
        *total.lock().expect("compteur SQL total") = 0;
        active.store(true, atomic::Ordering::Release);
        let small = list(&state, &client, None, query(None, Some("2"), None)).await;
        active.store(false, atomic::Ordering::Release);
        let small_lookups = *counter.lock().expect("compteur SQL");

        *counter.lock().expect("compteur SQL") = 0;
        active.store(true, atomic::Ordering::Release);
        let big = list(&state, &client, None, query(None, Some("200"), None)).await;
        active.store(false, atomic::Ordering::Release);
        let big_lookups = *counter.lock().expect("compteur SQL");
        let total_events = *total.lock().expect("compteur SQL total");
        let delta = big_lookups.saturating_sub(small_lookups);
        eprintln!(
            "board asset lookups: 2 items = {small_lookups}, 200 items = {big_lookups}, delta = {delta} (SQL events = {total_events})"
        );
        assert!(
            total_events > 0,
            "le compteur SQL n'a rien compté (instrumentation cassée)"
        );
        assert_eq!(small["total"], ITEMS);
        assert_eq!(small["items"].as_array().expect("items").len(), 2);
        assert_eq!(big["total"], ITEMS);
        assert_eq!(big["items"].as_array().expect("items").len(), ITEMS);
        assert!(
            delta <= MAX_DELTA,
            "board page200 added {delta} per-item asset lookups vs 2 items (> {MAX_DELTA}: N+1)"
        );
    }

    /// Budget B-03 : p95 ≤ 250 ms sur une page de 200 items. Le test échoue
    /// dès que le budget est dépassé (assert explicite, pas un simple log).
    #[tokio::test]
    async fn board_list_page_of_200_stays_under_250ms_p95() {
        const ITEMS: usize = 200;
        const RUNS: usize = 20;
        const BUDGET_MS: u128 = 250;
        let dir = TempDir::new().expect("tempdir");
        let (state, db, client) = harness(dir.path()).await;

        seed_200(&db, dir.path()).await;

        // Échauffement : la première requête paie la préparation du plan.
        drop(list(&state, &client, None, query(None, Some("200"), None)).await);

        let mut samples = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let started = std::time::Instant::now();
            let body = list(&state, &client, None, query(None, Some("200"), None)).await;
            samples.push(started.elapsed().as_millis());
            assert_eq!(body["total"], ITEMS);
            assert_eq!(body["items"].as_array().expect("items").len(), ITEMS);
        }
        samples.sort_unstable();
        // nearest-rank, même convention que `xtask/src/loadtest.rs:percentile`.
        let rank = (samples.len() * 95).div_ceil(100);
        let p95 = samples[rank - 1];
        eprintln!("board page200 p95 = {p95} ms (samples: {samples:?})");
        assert!(
            p95 <= BUDGET_MS,
            "board p95 {p95} ms > {BUDGET_MS} ms (samples: {samples:?})"
        );
    }

    #[test]
    fn page_limits_are_normalized_like_legacy() {
        assert_eq!(normalize_limit(None), 50);
        assert_eq!(normalize_limit(Some("0")), 1);
        assert_eq!(normalize_limit(Some("999")), 200);
        assert_eq!(normalize_limit(Some("abc")), 50);
        assert_eq!(normalize_offset(None), 0);
        assert_eq!(normalize_offset(Some("-4")), 0);
    }
}
