//! Carte PNG d'un commentaire TikTok partagé depuis l'application mobile.
//!
//! L'application ne partage pas le commentaire lui-même : elle partage l'URL de
//! la vidéo avec `share_comment_id`, le commentaire ciblé. TikTok ne rend ce
//! commentaire ni dans le HTML de la page ni dans son API publique non signée ;
//! le service tikwm (`/api/comment/list`) est la source interrogée, à défaut de
//! signature mobile. Une fois le commentaire retrouvé, son auteur, son texte,
//! ses likes, son avatar et ses éventuelles images sont rendus en PNG, puis
//! téléchargés comme n'importe quel autre média par le pipeline.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parley::Alignment;
use serde::Deserialize;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
use url::form_urlencoded;

use crate::card_canvas::{
    build_layout, decode_png, draw_avatar, draw_body, draw_image_cover, draw_rounded_rect,
    draw_text, layout_body, normalized_text, single_line_layout, with_text_engine, TextEngine,
    TextStyleSpec,
};
use crate::config::MediaConfig;
use crate::download::{download_card_image_png, DownloadResult};
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::http;
use crate::resolve::tiktok;
use crate::{source, MediaKind};

/// Largeur fixe de la carte, alignée sur la carte de tweet.
pub const CARD_WIDTH: u32 = 1080;
/// Hauteur minimale d'une carte (un commentaire court garde une carte pleine).
pub const CARD_MIN_HEIGHT: u32 = 420;
/// Hauteur maximale : le texte est tronqué avant, les images peuvent être hautes.
pub const CARD_MAX_HEIGHT: u32 = 2800;

const PADDING: f32 = 48.0;
const CONTENT_WIDTH: f32 = CARD_WIDTH as f32 - 2.0 * PADDING;
const CORNER_RADIUS: f32 = 32.0;
const AVATAR_SIZE: f32 = 104.0;
const BRAND_TOP: f32 = 44.0;
const BRAND_HEIGHT: f32 = 32.0;
const HEADER_TOP: f32 = BRAND_TOP + BRAND_HEIGHT + 28.0;
const COMMENT_TOP: f32 = HEADER_TOP + AVATAR_SIZE + 36.0;
const COMMENT_TEXT_SIZE: f32 = 46.0;
const COMMENT_TEXT_LINE_HEIGHT: f32 = 62.0;
const COMMENT_TEXT_MAX_HEIGHT: f32 = 900.0;
const IMAGE_GAP: f32 = 16.0;
const IMAGE_RADIUS: f32 = 20.0;
const ENGAGEMENT_HEIGHT: f32 = 44.0;
const FOOTER_HEIGHT: f32 = 34.0;

const BACKGROUND: (u8, u8, u8) = (0x17, 0x1b, 0x25);
const TEXT_COLOR: (u8, u8, u8) = (0xe8, 0xeb, 0xf2);
const SECONDARY_COLOR: (u8, u8, u8) = (0x9a, 0xa3, 0xb5);
const SEPARATOR_COLOR: (u8, u8, u8) = (0x2b, 0x31, 0x40);
const PINK: (u8, u8, u8) = (0xfe, 0x2c, 0x55);
const CYAN: (u8, u8, u8) = (0x25, 0xf4, 0xee);

/// Nombre de pages de commentaires parcourues avant d'abandonner la recherche.
const MAX_COMMENT_PAGES: usize = 4;
/// Taille de page demandée à tikwm (le maximum accepté).
const COMMENT_PAGE_SIZE: u32 = 50;
/// Délai entre deux appels tikwm (son offre gratuite limite à 1 requête/s).
const COMMENT_REQUEST_INTERVAL: Duration = Duration::from_millis(1_200);
/// Agent envoyé à tikwm : l'API refuse les requêtes sans agent.
const TIKWM_USER_AGENT: &str = concat!("LiveChat/", env!("CARGO_PKG_VERSION"));
/// Nombre maximal d'images conservées dans la carte.
const MAX_CARD_IMAGES: usize = 4;

/// Contenu prêt à rendre d'une carte de commentaire.
#[derive(Clone, Debug, Default)]
pub struct TikTokCommentCard {
    /// Nom affiché de l'auteur (surnom TikTok).
    pub author_name: String,
    /// Pseudo TikTok, sans `@`.
    pub author_handle: String,
    /// Texte du commentaire.
    pub text: String,
    /// Horodatage Unix de publication, si TikTok le fournit.
    pub created_at: Option<i64>,
    /// Nombre de likes.
    pub likes: u64,
    /// Nombre de réponses.
    pub replies: u64,
    /// Compte vérifié.
    pub verified: bool,
    /// Avatar converti en PNG.
    pub avatar_png: Option<Vec<u8>>,
    /// Images du commentaire converties en PNG (quatre au plus).
    pub images_png: Vec<Vec<u8>>,
    /// Auteur de la vidéo sous laquelle le commentaire a été publié.
    pub video_author: Option<String>,
}

/// Réponse de tikwm pour une page de commentaires.
#[derive(Debug, Deserialize)]
struct TikwmCommentResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: Option<TikwmCommentData>,
}

/// Page de commentaires tikwm.
#[derive(Debug, Default, Deserialize)]
struct TikwmCommentData {
    #[serde(default)]
    comments: Vec<TikwmCommentItem>,
    #[serde(default, rename = "hasMore")]
    has_more: bool,
    #[serde(default)]
    cursor: u64,
}

/// Un commentaire tel que le renvoie tikwm.
#[derive(Debug, Deserialize)]
struct TikwmCommentItem {
    #[serde(default)]
    id: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    create_time: Option<i64>,
    #[serde(default)]
    digg_count: Option<i64>,
    #[serde(default)]
    reply_total: Option<i64>,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    user: Option<TikwmCommentUser>,
}

/// Auteur d'un commentaire tikwm.
#[derive(Debug, Deserialize)]
struct TikwmCommentUser {
    #[serde(default)]
    unique_id: String,
    #[serde(default)]
    nickname: String,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    verified: Option<bool>,
}

/// Rend la carte PNG d'un commentaire.
pub fn render_tiktok_comment_card(comment: &TikTokCommentCard) -> MediaResult<Vec<u8>> {
    render_at(comment, unix_now())
}

/// Télécharge le commentaire partagé et rend sa carte comme média.
///
/// Échoue si le commentaire est introuvable dans les premières pages de la
/// vidéo ; l'appelant (`download_tiktok`) retombe alors sur la vidéo, pour ne
/// jamais casser un lien que TikTok ouvre lui-même.
pub async fn download_tiktok_comment(
    media_source: &source::MediaSource,
    comment_id: &str,
    config: &MediaConfig,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    let item = fetch_shared_comment(config, &media_source.canonical_url, comment_id)
        .await?
        .ok_or_else(|| {
            MediaError::new(
                MediaErrorCode::MediaNotFound,
                "le commentaire partagé est introuvable dans la liste TikTok",
            )
        })?;
    let mut card = TikTokCommentCard::from_item(&item);
    card.video_author = source::tiktok_author(&media_source.canonical_url);

    let headers = vec![
        ("User-Agent", config.download.user_agent.as_str()),
        ("Referer", tiktok::TIKTOK_REFERER),
    ];
    if let Some(avatar_url) = item.user.as_ref().and_then(|user| user.avatar.as_deref()) {
        card.avatar_png = download_card_image_png(
            config,
            avatar_url,
            temp_dir,
            "tiktok-comment-avatar",
            &headers,
        )
        .await;
    }
    for (index, image_url) in item.images.iter().take(MAX_CARD_IMAGES).enumerate() {
        if let Some(png) = download_card_image_png(
            config,
            image_url,
            temp_dir,
            &format!("tiktok-comment-image-{index}"),
            &headers,
        )
        .await
        {
            card.images_png.push(png);
        }
    }

    let png = render_tiktok_comment_card(&card)?;
    let path = temp_dir.join("tiktok-comment.png");
    std::fs::write(&path, &png).map_err(|error| {
        MediaError::with_raw(
            MediaErrorCode::TranscodeFailed,
            "écriture de la carte de commentaire impossible",
            error.to_string(),
        )
    })?;
    let size_bytes = png.len() as u64;
    let title = if !card.text.trim().is_empty() {
        card.text.trim().to_string()
    } else if card.author_handle.is_empty() {
        "Commentaire TikTok".to_string()
    } else {
        format!("Commentaire de @{}", card.author_handle)
    };
    Ok(DownloadResult {
        path,
        kind: Some(MediaKind::Image),
        mime: Some("image/png".to_string()),
        size_bytes,
        source_url: media_source.canonical_url.clone(),
        title: Some(title),
    })
}

impl TikTokCommentCard {
    /// Convertit un commentaire tikwm en contenu de carte.
    fn from_item(item: &TikwmCommentItem) -> Self {
        let user = item.user.as_ref();
        let author_name = user
            .map(|user| user.nickname.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_default();
        let author_handle = user
            .map(|user| user.unique_id.trim().to_string())
            .filter(|handle| !handle.is_empty())
            .unwrap_or_default();
        Self {
            author_name,
            author_handle,
            text: item.text.clone(),
            created_at: item.create_time,
            likes: item.digg_count.unwrap_or(0).max(0) as u64,
            replies: item.reply_total.unwrap_or(0).max(0) as u64,
            verified: user.and_then(|user| user.verified).unwrap_or(false),
            avatar_png: None,
            images_png: Vec::new(),
            video_author: None,
        }
    }
}

/// Cherche le commentaire `comment_id` dans les pages de commentaires de la vidéo.
///
/// tikwm trie les commentaires par likes : un commentaire peu liké peut être
/// loin, d'où plusieurs pages. Au-delà, mieux vaut rendre la vidéo que faire
/// attendre indéfiniment.
async fn fetch_shared_comment(
    config: &MediaConfig,
    video_url: &str,
    comment_id: &str,
) -> MediaResult<Option<TikwmCommentItem>> {
    let mut cursor = 0_u64;
    for page in 0..MAX_COMMENT_PAGES {
        if page > 0 {
            tokio::time::sleep(COMMENT_REQUEST_INTERVAL).await;
        }
        let url = comment_list_url(
            &config.resolve.tiktok_comment_endpoint,
            video_url,
            COMMENT_PAGE_SIZE,
            cursor,
        );
        let mut response = fetch_comment_page(config, &url).await?;
        if is_rate_limited(&response) {
            tokio::time::sleep(COMMENT_REQUEST_INTERVAL).await;
            response = fetch_comment_page(config, &url).await?;
        }
        if response.code != 0 {
            return Err(MediaError::with_raw(
                MediaErrorCode::DownloadFailed,
                "l'API de commentaires TikTok a refusé la requête",
                format!("code {} : {}", response.code, response.msg),
            ));
        }
        let Some(data) = response.data else {
            break;
        };
        let TikwmCommentData {
            comments,
            has_more,
            cursor: next_cursor,
        } = data;
        if let Some(item) = comments.into_iter().find(|item| item.id == comment_id) {
            return Ok(Some(item));
        }
        if !has_more || next_cursor <= cursor {
            break;
        }
        cursor = next_cursor;
    }
    Ok(None)
}

async fn fetch_comment_page(config: &MediaConfig, url: &str) -> MediaResult<TikwmCommentResponse> {
    let headers = [
        ("User-Agent", TIKWM_USER_AGENT),
        ("Accept", "application/json"),
    ];
    let text = http::get_text(
        &config.download,
        url,
        config.resolve.oembed_timeout,
        &headers,
    )
    .await?;
    serde_json::from_str(&text.body).map_err(|error| {
        MediaError::with_raw(
            MediaErrorCode::InvalidMedia,
            "réponse de commentaires TikTok invalide",
            error.to_string(),
        )
    })
}

fn is_rate_limited(response: &TikwmCommentResponse) -> bool {
    response.code == -1 && response.msg.to_ascii_lowercase().contains("limit")
}

/// Construit l'URL d'une page de commentaires (base sans `/list`).
fn comment_list_url(endpoint: &str, video_url: &str, count: u32, cursor: u64) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("url", video_url);
    query.append_pair("count", &count.to_string());
    query.append_pair("cursor", &cursor.to_string());
    format!("{}/list?{}", endpoint.trim_end_matches('/'), query.finish())
}

fn render_at(comment: &TikTokCommentCard, now: i64) -> MediaResult<Vec<u8>> {
    with_text_engine(|engine| {
        let raw_text = normalized_text(&comment.text);
        let text = if raw_text.is_empty() {
            String::from("(commentaire vide)")
        } else {
            raw_text
        };
        let body = layout_body(
            engine,
            &text,
            TextStyleSpec {
                size: COMMENT_TEXT_SIZE,
                line_height: COMMENT_TEXT_LINE_HEIGHT,
                bold: false,
                align: Alignment::Left,
            },
            CONTENT_WIDTH,
            COMMENT_TEXT_MAX_HEIGHT,
        );
        let images: Vec<&[u8]> = comment
            .images_png
            .iter()
            .filter(|png| decode_png(png).is_some())
            .map(Vec::as_slice)
            .take(MAX_CARD_IMAGES)
            .collect();
        let (boxes, grid_height) = image_grid(images.len());

        let mut bottom = COMMENT_TOP + body.height + 34.0;
        if !boxes.is_empty() {
            bottom += grid_height + 30.0;
        }
        let engagement_top = bottom;
        let separator_y = engagement_top + ENGAGEMENT_HEIGHT + 22.0;
        let footer_top = separator_y + 20.0;
        let height = (footer_top + FOOTER_HEIGHT + 44.0)
            .ceil()
            .clamp(CARD_MIN_HEIGHT as f32, CARD_MAX_HEIGHT as f32) as u32;

        let mut pixmap = Pixmap::new(CARD_WIDTH, height).ok_or_else(|| {
            MediaError::new(
                MediaErrorCode::TranscodeFailed,
                "allocation de la carte de commentaire impossible",
            )
        })?;
        draw_rounded_rect(
            &mut pixmap,
            rect(0.0, 0.0, CARD_WIDTH as f32, height as f32),
            CORNER_RADIUS,
            BACKGROUND,
        )
        .ok_or_else(|| card_error("fond de la carte de commentaire impossible"))?;

        draw_brand(&mut pixmap, engine);
        draw_header(&mut pixmap, engine, comment, now);
        draw_body(
            &mut pixmap,
            &body,
            engine,
            (PADDING, COMMENT_TOP),
            TEXT_COLOR,
        );
        if !boxes.is_empty() {
            let image_top = COMMENT_TOP + body.height + 34.0;
            for (png, box_rect) in images.iter().zip(boxes.iter()) {
                let image_rect = rect(
                    PADDING + box_rect.left(),
                    image_top + box_rect.top(),
                    box_rect.width(),
                    box_rect.height(),
                );
                let _ = draw_image_cover(&mut pixmap, png, image_rect, IMAGE_RADIUS);
            }
        }
        draw_engagement(&mut pixmap, engine, comment, engagement_top);
        let _ = draw_rounded_rect(
            &mut pixmap,
            rect(PADDING, separator_y, CONTENT_WIDTH, 1.0),
            0.0,
            SEPARATOR_COLOR,
        );
        draw_footer(&mut pixmap, engine, comment, footer_top);

        pixmap.encode_png().map_err(|error| {
            MediaError::with_raw(
                MediaErrorCode::TranscodeFailed,
                "encodage de la carte de commentaire impossible",
                error.to_string(),
            )
        })
    })
}

fn draw_brand(pixmap: &mut Pixmap, engine: &mut TextEngine) {
    let _ = draw_rounded_rect(
        pixmap,
        rect(PADDING + 9.0, BRAND_TOP + 3.0, 8.0, 26.0),
        4.0,
        CYAN,
    );
    let _ = draw_rounded_rect(pixmap, rect(PADDING, BRAND_TOP + 3.0, 8.0, 26.0), 4.0, PINK);
    let label = single_line_layout(
        engine,
        "COMMENTAIRE TIKTOK",
        TextStyleSpec {
            size: 24.0,
            line_height: 30.0,
            bold: true,
            align: Alignment::Left,
        },
        480.0,
    );
    draw_text(
        pixmap,
        &label,
        engine,
        (PADDING + 34.0, BRAND_TOP),
        SECONDARY_COLOR,
    );
}

fn draw_header(
    pixmap: &mut Pixmap,
    engine: &mut TextEngine,
    comment: &TikTokCommentCard,
    now: i64,
) {
    draw_avatar(
        pixmap,
        rect(PADDING, HEADER_TOP, AVATAR_SIZE, AVATAR_SIZE),
        comment.avatar_png.as_deref(),
        &display_name(comment),
        &comment.author_handle,
        engine,
    );
    let name_x = PADDING + AVATAR_SIZE + 24.0;
    let name = single_line_layout(
        engine,
        &display_name(comment),
        TextStyleSpec {
            size: 36.0,
            line_height: 46.0,
            bold: true,
            align: Alignment::Left,
        },
        760.0,
    );
    draw_text(
        pixmap,
        &name,
        engine,
        (name_x, HEADER_TOP + 2.0),
        TEXT_COLOR,
    );
    if comment.verified {
        draw_verified_badge(
            pixmap,
            name_x + name.width() + 18.0,
            HEADER_TOP + 2.0 + 23.0,
            14.0,
        );
    }

    let mut handle_line = String::new();
    if !comment.author_handle.is_empty() {
        handle_line.push('@');
        handle_line.push_str(&comment.author_handle);
    }
    if let Some(date) = comment
        .created_at
        .map(|created_at| format_relative_date(created_at, now))
    {
        if !handle_line.is_empty() {
            handle_line.push_str(" · ");
        }
        handle_line.push_str(&date);
    }
    if !handle_line.is_empty() {
        let handle = single_line_layout(
            engine,
            &handle_line,
            TextStyleSpec {
                size: 27.0,
                line_height: 37.0,
                bold: false,
                align: Alignment::Left,
            },
            820.0,
        );
        draw_text(
            pixmap,
            &handle,
            engine,
            (name_x, HEADER_TOP + 52.0),
            SECONDARY_COLOR,
        );
    }
}

fn draw_engagement(
    pixmap: &mut Pixmap,
    engine: &mut TextEngine,
    comment: &TikTokCommentCard,
    top: f32,
) {
    let icon_size = 30.0;
    draw_heart(pixmap, PADDING, top + 6.0, icon_size, PINK);
    let likes = build_layout(
        engine,
        &format_count(comment.likes),
        TextStyleSpec {
            size: 29.0,
            line_height: 40.0,
            bold: true,
            align: Alignment::Left,
        },
        None,
    );
    let likes_x = PADDING + icon_size + 12.0;
    draw_text(pixmap, &likes, engine, (likes_x, top), TEXT_COLOR);
    let label = single_line_layout(
        engine,
        "j'aime",
        TextStyleSpec {
            size: 26.0,
            line_height: 36.0,
            bold: false,
            align: Alignment::Left,
        },
        200.0,
    );
    draw_text(
        pixmap,
        &label,
        engine,
        (likes_x + likes.width() + 8.0, top + 3.0),
        SECONDARY_COLOR,
    );

    if comment.replies > 0 {
        let replies_x = likes_x + likes.width() + 18.0 + label.width() + 42.0;
        draw_reply_bubble(pixmap, replies_x, top + 6.0, icon_size, SECONDARY_COLOR);
        let replies = build_layout(
            engine,
            &format_count(comment.replies),
            TextStyleSpec {
                size: 29.0,
                line_height: 40.0,
                bold: true,
                align: Alignment::Left,
            },
            None,
        );
        let text_x = replies_x + icon_size + 12.0;
        draw_text(pixmap, &replies, engine, (text_x, top), TEXT_COLOR);
        let word = single_line_layout(
            engine,
            if comment.replies == 1 {
                "réponse"
            } else {
                "réponses"
            },
            TextStyleSpec {
                size: 26.0,
                line_height: 36.0,
                bold: false,
                align: Alignment::Left,
            },
            220.0,
        );
        draw_text(
            pixmap,
            &word,
            engine,
            (text_x + replies.width() + 8.0, top + 3.0),
            SECONDARY_COLOR,
        );
    }
}

fn draw_footer(
    pixmap: &mut Pixmap,
    engine: &mut TextEngine,
    comment: &TikTokCommentCard,
    top: f32,
) {
    let mut footer = String::from("Commentaire partagé depuis TikTok");
    if let Some(author) = comment
        .video_author
        .as_deref()
        .map(str::trim)
        .filter(|author| !author.is_empty())
    {
        footer.push_str(" · sous la vidéo de @");
        footer.push_str(author);
    }
    let layout = single_line_layout(
        engine,
        &footer,
        TextStyleSpec {
            size: 25.0,
            line_height: 34.0,
            bold: false,
            align: Alignment::Left,
        },
        CONTENT_WIDTH,
    );
    draw_text(pixmap, &layout, engine, (PADDING, top), SECONDARY_COLOR);
}

fn draw_heart(pixmap: &mut Pixmap, left: f32, top: f32, size: f32, color: (u8, u8, u8)) {
    let x = |fraction: f32| left + size * fraction;
    let y = |fraction: f32| top + size * fraction;
    let mut builder = PathBuilder::new();
    builder.move_to(x(0.5), y(0.92));
    builder.cubic_to(x(0.08), y(0.62), x(0.0), y(0.28), x(0.22), y(0.12));
    builder.cubic_to(x(0.38), y(0.0), x(0.48), y(0.12), x(0.5), y(0.22));
    builder.cubic_to(x(0.52), y(0.12), x(0.62), y(0.0), x(0.78), y(0.12));
    builder.cubic_to(x(1.0), y(0.28), x(0.92), y(0.62), x(0.5), y(0.92));
    builder.close();
    if let Some(path) = builder.finish() {
        fill_path(pixmap, &path, color);
    }
}

fn draw_reply_bubble(pixmap: &mut Pixmap, left: f32, top: f32, size: f32, color: (u8, u8, u8)) {
    let _ = draw_rounded_rect(
        pixmap,
        rect(left, top, size, size * 0.72),
        size * 0.18,
        color,
    );
    let mut builder = PathBuilder::new();
    builder.move_to(left + size * 0.22, top + size * 0.66);
    builder.line_to(left + size * 0.16, top + size * 0.98);
    builder.line_to(left + size * 0.46, top + size * 0.72);
    builder.close();
    if let Some(path) = builder.finish() {
        fill_path(pixmap, &path, color);
    }
}

fn draw_verified_badge(pixmap: &mut Pixmap, center_x: f32, center_y: f32, radius: f32) {
    let Some(circle) = PathBuilder::from_circle(center_x, center_y, radius) else {
        return;
    };
    fill_path(pixmap, &circle, CYAN);
    let mut builder = PathBuilder::new();
    builder.move_to(center_x - radius * 0.42, center_y + radius * 0.02);
    builder.line_to(center_x - radius * 0.08, center_y + radius * 0.35);
    builder.line_to(center_x + radius * 0.45, center_y - radius * 0.3);
    let Some(path) = builder.finish() else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(0xff, 0xff, 0xff, 255);
    paint.anti_alias = true;
    let stroke = Stroke {
        width: radius * 0.22,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

fn fill_path(pixmap: &mut Pixmap, path: &tiny_skia::Path, color: (u8, u8, u8)) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.0, color.1, color.2, 255);
    paint.anti_alias = true;
    pixmap.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// Boîtes des images du commentaire, relatives au coin haut gauche de la grille.
///
/// Les proportions sont fixes pour que la carte reste stable quel que soit le
/// nombre d'images : une seule image large, deux côte à côte, trois avec une
/// grande à gauche, quatre en grille.
fn image_grid(count: usize) -> (Vec<Rect>, f32) {
    match count {
        0 => (Vec::new(), 0.0),
        1 => (vec![rect(0.0, 0.0, CONTENT_WIDTH, 620.0)], 620.0),
        2 => {
            let width = (CONTENT_WIDTH - IMAGE_GAP) / 2.0;
            (
                vec![
                    rect(0.0, 0.0, width, 560.0),
                    rect(width + IMAGE_GAP, 0.0, width, 560.0),
                ],
                560.0,
            )
        }
        3 => {
            let right_x = 616.0;
            let right_width = CONTENT_WIDTH - right_x;
            (
                vec![
                    rect(0.0, 0.0, 600.0, 560.0),
                    rect(right_x, 0.0, right_width, 272.0),
                    rect(right_x, 288.0, right_width, 272.0),
                ],
                560.0,
            )
        }
        _ => {
            let width = (CONTENT_WIDTH - IMAGE_GAP) / 2.0;
            (
                vec![
                    rect(0.0, 0.0, width, 420.0),
                    rect(width + IMAGE_GAP, 0.0, width, 420.0),
                    rect(0.0, 436.0, width, 420.0),
                    rect(width + IMAGE_GAP, 436.0, width, 420.0),
                ],
                856.0,
            )
        }
    }
}

fn display_name(comment: &TikTokCommentCard) -> String {
    let name = comment.author_name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    if !comment.author_handle.is_empty() {
        return format!("@{}", comment.author_handle);
    }
    "Anonyme".to_string()
}

/// Compte façon TikTok : `67,5 k`, `1,2 M`, chiffres bruts en dessous de 1000.
fn format_count(value: u64) -> String {
    if value < 1_000 {
        return value.to_string();
    }
    let (unit, divisor) = if value < 999_950 {
        ("k", 100_u64)
    } else {
        ("M", 100_000_u64)
    };
    let tenths = (value + divisor / 2) / divisor;
    if tenths.is_multiple_of(10) {
        format!("{} {unit}", tenths / 10)
    } else {
        format!("{},{:01} {unit}", tenths / 10, tenths % 10)
    }
}

/// Date relative en français, à partir d'un horodatage Unix.
fn format_relative_date(created_at: i64, now: i64) -> String {
    let seconds = (now - created_at).max(0);
    if seconds < 60 {
        return "à l'instant".to_string();
    }
    let (count, unit) = if seconds < 3_600 {
        (seconds / 60, "min")
    } else if seconds < 86_400 {
        (seconds / 3_600, "h")
    } else if seconds < 604_800 {
        (seconds / 86_400, "j")
    } else if seconds < 2_592_000 {
        (seconds / 604_800, "sem.")
    } else if seconds < 31_536_000 {
        (seconds / 2_592_000, "mois")
    } else {
        (seconds / 31_536_000, "an")
    };
    if unit == "an" && count > 1 {
        return format!("il y a {count} ans");
    }
    format!("il y a {count} {unit}")
}

// Les dimensions viennent de constantes positives de mise en page.
#[allow(clippy::expect_used)]
fn rect(left: f32, top: f32, width: f32, height: f32) -> Rect {
    Rect::from_xywh(left, top, width, height).expect("rectangle de carte valide")
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn card_error(message: &str) -> MediaError {
    MediaError::new(MediaErrorCode::TranscodeFailed, message)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use tiny_skia::Color;

    fn sample() -> TikTokCommentCard {
        TikTokCommentCard {
            author_name: "savinggrace".to_string(),
            author_handle: "x.walsh.x".to_string(),
            text: "When i accidentally step on my dogs tail at 3am".to_string(),
            created_at: Some(1_800_000_000),
            likes: 67_493,
            replies: 49,
            verified: false,
            avatar_png: None,
            images_png: Vec::new(),
            video_author: Some("bingo83865".to_string()),
        }
    }

    fn image_png(width: u32, height: u32, color: Color) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width, height).expect("allocation de l'image de test");
        pixmap.fill(color);
        pixmap.encode_png().expect("encodage de l'image de test")
    }

    #[test]
    fn formats_counts_like_tiktok() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1 k");
        assert_eq!(format_count(67_493), "67,5 k");
        assert_eq!(format_count(1_200_000), "1,2 M");
    }

    #[test]
    fn formats_relative_dates_in_french() {
        let now = 1_800_000_000;
        assert_eq!(format_relative_date(now, now), "à l'instant");
        assert_eq!(format_relative_date(now - 120, now), "il y a 2 min");
        assert_eq!(format_relative_date(now - 7_200, now), "il y a 2 h");
        assert_eq!(format_relative_date(now - 259_200, now), "il y a 3 j");
        assert_eq!(
            format_relative_date(now - 15 * 86_400, now),
            "il y a 2 sem."
        );
        assert_eq!(
            format_relative_date(now - 60 * 86_400, now),
            "il y a 2 mois"
        );
        assert_eq!(format_relative_date(now - 400 * 86_400, now), "il y a 1 an");
    }

    #[test]
    fn builds_a_comment_page_url() {
        let url = comment_list_url(
            "https://www.tikwm.com/api/comment",
            "https://www.tiktok.com/@bingo83865/video/7707202086697487646?share_comment_id=7707256588990661390",
            50,
            0,
        );
        assert!(url.starts_with("https://www.tikwm.com/api/comment/list?"));
        assert!(url.contains("count=50"));
        assert!(url.contains("cursor=0"));
        assert!(url.contains("share_comment_id"));
    }

    #[test]
    fn renders_a_text_comment_card() {
        let png = render_at(&sample(), 1_800_000_000).expect("carte rendue");
        let pixmap = Pixmap::decode_png(&png).expect("PNG décodable");
        assert_eq!(pixmap.width(), CARD_WIDTH);
        assert!(pixmap.height() >= CARD_MIN_HEIGHT);
        assert!(pixmap.height() <= CARD_MAX_HEIGHT);
    }

    #[test]
    fn renders_a_card_for_every_image_count() {
        for count in 0..=4 {
            let mut comment = sample();
            comment.text = "Trop drôle 😂".to_string();
            for index in 0..count {
                comment.images_png.push(image_png(
                    800,
                    600,
                    Color::from_rgba8(20 + index as u8 * 40, 90, 200, 255),
                ));
            }
            let png = render_at(&comment, 1_800_000_000).expect("carte rendue");
            let pixmap = Pixmap::decode_png(&png).expect("PNG décodable");
            assert_eq!(pixmap.width(), CARD_WIDTH);
        }
    }

    #[test]
    fn renders_a_long_comment_without_overflowing() {
        let mut comment = sample();
        comment.text = "Ceci est un commentaire très long. ".repeat(120);
        comment.likes = 1_200_000;
        comment.replies = 12_000;
        comment.verified = true;
        let png = render_at(&comment, 1_800_000_000).expect("carte rendue");
        let pixmap = Pixmap::decode_png(&png).expect("PNG décodable");
        assert!(pixmap.height() <= CARD_MAX_HEIGHT);
    }

    #[test]
    fn renders_rtl_and_emoji_text() {
        let mut comment = sample();
        comment.text = "شكراً 🙏🏽 جربت هذا الكومنت 👌".to_string();
        let png = render_at(&comment, 1_800_000_000).expect("carte rendue");
        assert!(!png.is_empty());
    }

    #[test]
    fn renders_an_empty_comment_with_a_placeholder() {
        let mut comment = sample();
        comment.text.clear();
        comment.likes = 0;
        comment.replies = 0;
        comment.created_at = None;
        let png = render_at(&comment, 1_800_000_000).expect("carte rendue");
        assert!(!png.is_empty());
    }

    #[test]
    #[ignore = "galerie locale pour inspection visuelle"]
    fn writes_a_comment_card_gallery() {
        let directory =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tiktok-comment-gallery");
        std::fs::create_dir_all(&directory).expect("création de la galerie");
        let mut cards: Vec<(String, TikTokCommentCard)> = Vec::new();

        cards.push(("01-texte-court".to_string(), sample()));
        let mut sans_avatar = sample();
        sans_avatar.avatar_png = None;
        sans_avatar.likes = 12;
        cards.push(("02-sans-avatar".to_string(), sans_avatar));
        let mut long = sample();
        long.text = "Quand ton chat décide de marcher sur ton clavier à 4 h du matin pendant que tu essaies de travailler tranquillement 😹".repeat(4);
        long.verified = true;
        long.likes = 987_654;
        long.replies = 1_234;
        cards.push(("03-long-et-verifie".to_string(), long));
        let mut rtl = sample();
        rtl.text = "هذا أفضل تعليق رأيته اليوم 😂😂 شكراً على المشاركة".to_string();
        rtl.author_name = "ليلى".to_string();
        rtl.author_handle = "layla".to_string();
        cards.push(("04-emoji-et-rtl".to_string(), rtl));
        let mut vide = sample();
        vide.text = String::new();
        vide.created_at = None;
        vide.likes = 0;
        vide.replies = 0;
        cards.push(("05-commentaire-vide".to_string(), vide));

        for count in 1..=4 {
            let mut card = sample();
            card.text = format!("Commentaire avec {count} image(s) 📸");
            for index in 0..count {
                card.images_png.push(image_png(
                    900,
                    700,
                    Color::from_rgba8(30 + index as u8 * 50, 60, 180, 255),
                ));
            }
            let name = format!("{:02}-images", 5 + count);
            cards.push((name, card));
        }

        for (name, comment) in cards {
            let png = render_at(&comment, 1_800_000_000).expect("carte rendue");
            let path = directory.join(format!("{name}.png"));
            std::fs::write(&path, png).expect("écriture de la carte");
            println!("{}", path.display());
        }
    }

    #[tokio::test]
    #[ignore = "réseau réel : lien de partage fourni par l'utilisateur"]
    async fn renders_the_shared_comment_from_the_user_link() {
        let config = MediaConfig::default();
        let canonical = "https://www.tiktok.com/@_/video/7707202086697487646?share_comment_id=7707256588990661390";
        let comment_id = source::tiktok_comment_id(canonical).expect("identifiant du commentaire");
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let source = source::MediaSource::from_canonical(canonical, canonical);
        let download = download_tiktok_comment(&source, &comment_id, &config, temp.path())
            .await
            .expect("carte téléchargée");
        assert_eq!(download.mime.as_deref(), Some("image/png"));
        let png = std::fs::read(&download.path).expect("carte lisible");
        let pixmap = Pixmap::decode_png(&png).expect("PNG décodable");
        assert_eq!(pixmap.width(), CARD_WIDTH);
        let directory =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tiktok-comment-gallery");
        std::fs::create_dir_all(&directory).expect("création de la galerie");
        let target = directory.join("00-lien-utilisateur.png");
        std::fs::write(&target, png).expect("écriture de la carte du lien");
        println!("{}", target.display());
    }

    #[tokio::test]
    #[ignore = "réseau réel : URL courte fournie par l'utilisateur"]
    async fn resolves_the_short_link_to_a_comment_card() {
        let config = MediaConfig::default();
        let raw = "https://vm.tiktok.com/ZG9PF1UrbgN9A-lmGlc/";
        let media_source = source::resolve_source(&config, raw)
            .await
            .expect("résolution de l'URL courte")
            .expect("source résolue");
        assert_eq!(
            source::tiktok_comment_id(&media_source.canonical_url).as_deref(),
            Some("7707256588990661390")
        );
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let download = crate::download::download_source(&media_source, None, &config, temp.path())
            .await
            .expect("téléchargement de la carte");
        assert_eq!(download.kind, Some(MediaKind::Image));
        assert_eq!(download.mime.as_deref(), Some("image/png"));
    }
}
