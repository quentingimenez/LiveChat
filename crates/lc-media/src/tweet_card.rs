//! Rendu d'une « carte tweet » PNG : la publication sans photo ni vidéo
//! devient elle-même le média.
//!
//! La carte reprend l'aspect sombre de l'overlay : fond arrondi transparent
//! autour de `#171b25`, avatar circulaire, nom en gras, identifiant et date en
//! couleur secondaire, puis le texte du tweet. La mise en forme (césure,
//! bidirectionnel, repli de police, emoji couleur) est confiée à cosmic-text ;
//! tiny-skia ne sert qu'au fond, aux arrondis et à l'encodage PNG.

use std::sync::{Mutex, OnceLock};

use cosmic_text::{
    Align, Attrs, Buffer, Color, Ellipsize, EllipsizeHeightLimit, FontSystem, Metrics, Shaping,
    SwashCache, SwashContent, Weight, Wrap,
};
use tiny_skia::{FillRule, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Transform};

use crate::error::{MediaError, MediaErrorCode, MediaResult};

/// Largeur fixe d'une carte tweet.
pub const CARD_WIDTH: u32 = 1080;
/// Hauteur minimale d'une carte tweet.
pub const CARD_MIN_HEIGHT: u32 = 360;
/// Hauteur maximale d'une carte tweet (le texte est tronqué au-delà).
pub const CARD_MAX_HEIGHT: u32 = 1920;

/// Marge intérieure de la carte.
const PADDING: f32 = 48.0;
/// Rayon des coins arrondis.
const CORNER_RADIUS: f32 = 32.0;
/// Diamètre de l'avatar.
const AVATAR_SIZE: f32 = 96.0;
/// Espace entre l'en-tête et le texte.
const HEADER_GAP: f32 = 32.0;
/// Espace entre le texte et la date.
const FOOTER_GAP: f32 = 24.0;
/// Taille du nom de l'auteur.
const NAME_SIZE: f32 = 34.0;
const NAME_LINE_HEIGHT: f32 = 44.0;
/// Taille de l'identifiant et de la date.
const SECONDARY_SIZE: f32 = 30.0;
const SECONDARY_LINE_HEIGHT: f32 = 40.0;
/// Taille du texte du tweet (interligne 1,35).
const TEXT_SIZE: f32 = 40.0;
const TEXT_LINE_HEIGHT: f32 = TEXT_SIZE * 1.35;
/// Taille de l'initiale affichée à la place d'un avatar absent.
const INITIAL_SIZE: f32 = 48.0;
const INITIAL_LINE_HEIGHT: f32 = 56.0;

/// Fond de la carte.
const BACKGROUND: (u8, u8, u8) = (0x17, 0x1b, 0x25);
/// Couleur du texte principal.
const TEXT_COLOR: (u8, u8, u8) = (0xe8, 0xeb, 0xf2);
/// Couleur secondaire (identifiant, date).
const SECONDARY_COLOR: (u8, u8, u8) = (0x9a, 0xa3, 0xb5);

/// Couleurs des cercles d'initiale quand aucun avatar n'est disponible.
const INITIAL_COLORS: [(u8, u8, u8); 6] = [
    (0x3b, 0x82, 0xf6),
    (0x8b, 0x5c, 0xf6),
    (0xec, 0x48, 0x99),
    (0xf5, 0x9e, 0x0b),
    (0x10, 0xb9, 0x81),
    (0x06, 0xb6, 0xd4),
];

/// Mois français utilisés par la date de la carte.
const FRENCH_MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];

/// Contenu textuel d'une carte tweet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TweetCardContent {
    /// Nom d'affichage de l'auteur (« jack »).
    pub author_name: String,
    /// Identifiant sans `@` (« jack »).
    pub author_handle: String,
    /// Texte complet de la publication.
    pub text: String,
    /// Date ISO 8601 de publication, si connue.
    pub created_at: Option<String>,
    /// Octets bruts d'un avatar PNG 192×192, si disponible.
    pub avatar: Option<Vec<u8>>,
}

/// Rendu synchrone d'une carte tweet en PNG.
///
/// La hauteur s'adapte au texte entre [`CARD_MIN_HEIGHT`] et
/// [`CARD_MAX_HEIGHT`] ; au-delà, le texte est tronqué avec une ellipsis.
pub fn render_tweet_card(content: &TweetCardContent) -> MediaResult<Vec<u8>> {
    let mutex = FONT_SYSTEM.get_or_init(|| Mutex::new(FontSystem::new()));
    let mut font_system = mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    render_with_font_system(content, &mut font_system)
}

/// `FontSystem` partagé : le chargement des polices système est coûteux et la
/// carte peut être rendue plusieurs fois par processus.
static FONT_SYSTEM: OnceLock<Mutex<FontSystem>> = OnceLock::new();

/// Date française « 21 mars 2006 » d'un horodatage ISO 8601.
///
/// Seule la date UTC (les 10 premiers caractères) est utilisée ; une valeur
/// absente ou invalide donne `None`.
pub fn format_tweet_date(created_at: &str) -> Option<String> {
    let date = created_at.trim().get(..10)?;
    let mut parts = date.split('-');
    let year = parts.next()?;
    let month = parts.next()?;
    let day = parts.next()?;
    if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    let year: u32 = year.parse().ok()?;
    let month: usize = month.parse().ok()?;
    let day: u32 = day.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let name = FRENCH_MONTHS.get(month - 1)?;
    Some(format!("{day} {name} {year}"))
}

fn render_with_font_system(
    content: &TweetCardContent,
    font_system: &mut FontSystem,
) -> MediaResult<Vec<u8>> {
    let date = content.created_at.as_deref().and_then(format_tweet_date);
    let footer_height = if date.is_some() {
        FOOTER_GAP + SECONDARY_LINE_HEIGHT
    } else {
        0.0
    };
    let body_top = PADDING + AVATAR_SIZE + HEADER_GAP;
    let text_width = CARD_WIDTH as f32 - 2.0 * PADDING;
    let max_text_height = CARD_MAX_HEIGHT as f32 - body_top - footer_height - PADDING;
    let text = normalized_text(&content.text);
    let mut body = shape_buffer(
        font_system,
        &text,
        TEXT_SIZE,
        TEXT_LINE_HEIGHT,
        text_width,
        None,
        false,
        false,
        None,
    );
    let text_height = layout_height(&body);
    let content_height = body_top + text_height + footer_height + PADDING;
    let height = (content_height.ceil() as u32).clamp(CARD_MIN_HEIGHT, CARD_MAX_HEIGHT);
    if content_height > CARD_MAX_HEIGHT as f32 {
        body = shape_buffer(
            font_system,
            &text,
            TEXT_SIZE,
            TEXT_LINE_HEIGHT,
            text_width,
            Some(max_text_height),
            false,
            true,
            None,
        );
    }
    let mut pixmap = Pixmap::new(CARD_WIDTH, height)
        .ok_or_else(|| card_error("allocation du calque impossible"))?;
    draw_background(&mut pixmap, height)?;

    let name = display_name(content);
    let handle = display_handle(content);
    let name_width = text_width - AVATAR_SIZE - 24.0;
    let name_buffer = shape_buffer(
        font_system,
        &name,
        NAME_SIZE,
        NAME_LINE_HEIGHT,
        name_width,
        Some(NAME_LINE_HEIGHT),
        true,
        true,
        // Un nom arabe est un paragraphe RTL : sans alignement forcé,
        // cosmic-text le colle au bord droit de son tampon, loin de l'avatar.
        Some(Align::Left),
    );
    let handle_buffer = shape_buffer(
        font_system,
        &handle,
        SECONDARY_SIZE,
        SECONDARY_LINE_HEIGHT,
        name_width,
        Some(SECONDARY_LINE_HEIGHT),
        false,
        true,
        Some(Align::Left),
    );
    let header_height = AVATAR_SIZE.max(NAME_LINE_HEIGHT + SECONDARY_LINE_HEIGHT);
    let header_text_top =
        PADDING + (header_height - NAME_LINE_HEIGHT - SECONDARY_LINE_HEIGHT) / 2.0;
    let header_x = PADDING + AVATAR_SIZE + 24.0;

    let mut cache = SwashCache::new();
    draw_avatar(&mut pixmap, content, font_system, &mut cache);
    draw_text(
        &mut pixmap,
        &name_buffer,
        font_system,
        &mut cache,
        (header_x, header_text_top),
        color(TEXT_COLOR),
    );
    draw_text(
        &mut pixmap,
        &handle_buffer,
        font_system,
        &mut cache,
        (header_x, header_text_top + NAME_LINE_HEIGHT),
        color(SECONDARY_COLOR),
    );
    draw_text(
        &mut pixmap,
        &body,
        font_system,
        &mut cache,
        (PADDING, body_top),
        color(TEXT_COLOR),
    );
    if let Some(date) = date {
        let date_buffer = shape_buffer(
            font_system,
            &date,
            SECONDARY_SIZE,
            SECONDARY_LINE_HEIGHT,
            text_width,
            Some(SECONDARY_LINE_HEIGHT),
            false,
            true,
            Some(Align::Left),
        );
        draw_text(
            &mut pixmap,
            &date_buffer,
            font_system,
            &mut cache,
            (PADDING, height as f32 - PADDING - SECONDARY_LINE_HEIGHT),
            color(SECONDARY_COLOR),
        );
    }
    pixmap
        .encode_png()
        .map_err(|error| card_error(&format!("encodage PNG impossible : {error}")))
}

/// Normalise les fins de ligne du texte (cosmic-text attend des `\n`).
fn normalized_text(raw: &str) -> String {
    raw.trim().replace("\r\n", "\n").replace('\r', "\n")
}

/// Nom affiché dans l'en-tête (repli « Post » quand tout est vide).
fn display_name(content: &TweetCardContent) -> String {
    let name = content.author_name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    let handle = content.author_handle.trim().trim_start_matches('@');
    if handle.is_empty() {
        "Post".to_string()
    } else {
        handle.to_string()
    }
}

/// Identifiant affiché sous le nom, avec son `@` (vide s'il est inconnu).
fn display_handle(content: &TweetCardContent) -> String {
    let handle = content.author_handle.trim().trim_start_matches('@');
    if handle.is_empty() {
        String::new()
    } else {
        format!("@{handle}")
    }
}

/// Met en forme un texte et force le calcul de toutes ses lignes visibles.
#[allow(clippy::too_many_arguments)]
fn shape_buffer(
    font_system: &mut FontSystem,
    text: &str,
    size: f32,
    line_height: f32,
    width: f32,
    max_height: Option<f32>,
    bold: bool,
    ellipsize: bool,
    align: Option<Align>,
) -> Buffer {
    let metrics = Metrics::new(size, line_height);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_wrap(Wrap::WordOrGlyph);
    buffer.set_size(Some(width), max_height);
    if ellipsize {
        if let Some(max_height) = max_height {
            buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Height(max_height)));
        }
    }
    let attrs = if bold {
        Attrs::new().weight(Weight::BOLD)
    } else {
        Attrs::new()
    };
    buffer.set_text(text, &attrs, Shaping::Advanced, align);
    {
        let mut borrowed = buffer.borrow_with(font_system);
        let _ = borrowed.layout_runs().last();
    }
    buffer
}

/// Hauteur occupée par les lignes visibles d'un tampon déjà mis en forme.
fn layout_height(buffer: &Buffer) -> f32 {
    buffer.layout_runs().fold(0.0, |height, run| {
        height.max(run.line_top + run.line_height)
    })
}

/// Peint le fond arrondi de la carte.
fn draw_background(pixmap: &mut Pixmap, height: u32) -> MediaResult<()> {
    let rect = Rect::from_xywh(0.0, 0.0, CARD_WIDTH as f32, height as f32)
        .ok_or_else(|| card_error("dimensions de carte invalides"))?;
    let path = rounded_rect_path(rect, CORNER_RADIUS)
        .ok_or_else(|| card_error("tracé du fond invalide"))?;
    let mut paint = Paint::default();
    paint.set_color_rgba8(BACKGROUND.0, BACKGROUND.1, BACKGROUND.2, 255);
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    Ok(())
}

/// Tracé d'un rectangle à coins arrondis (approximation cubique des arcs).
fn rounded_rect_path(rect: Rect, radius: f32) -> Option<Path> {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    if radius <= 0.0 {
        return Some(PathBuilder::from_rect(rect));
    }
    // Constante de rapprochement d'un quart de cercle par une cubique.
    let k = radius * 0.552_284_8;
    let (left, top, right, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let mut builder = PathBuilder::new();
    builder.move_to(left + radius, top);
    builder.line_to(right - radius, top);
    builder.cubic_to(
        right - radius + k,
        top,
        right,
        top + radius - k,
        right,
        top + radius,
    );
    builder.line_to(right, bottom - radius);
    builder.cubic_to(
        right,
        bottom - radius + k,
        right - radius + k,
        bottom,
        right - radius,
        bottom,
    );
    builder.line_to(left + radius, bottom);
    builder.cubic_to(
        left + radius - k,
        bottom,
        left,
        bottom - radius + k,
        left,
        bottom - radius,
    );
    builder.line_to(left, top + radius);
    builder.cubic_to(
        left,
        top + radius - k,
        left + radius - k,
        top,
        left + radius,
        top,
    );
    builder.close();
    builder.finish()
}

/// Peint l'avatar circulaire, ou l'initiale sur un cercle coloré.
fn draw_avatar(
    pixmap: &mut Pixmap,
    content: &TweetCardContent,
    font_system: &mut FontSystem,
    cache: &mut SwashCache,
) {
    let center = PADDING + AVATAR_SIZE / 2.0;
    let Some(circle) = PathBuilder::from_circle(center, center, AVATAR_SIZE / 2.0) else {
        return;
    };
    if let Some(avatar) = content.avatar.as_deref().and_then(decode_avatar) {
        let Some(mut layer) = Pixmap::new(AVATAR_SIZE as u32, AVATAR_SIZE as u32) else {
            return;
        };
        let paint = PixmapPaint {
            quality: tiny_skia::FilterQuality::Bicubic,
            ..PixmapPaint::default()
        };
        let scale_x = AVATAR_SIZE / avatar.width() as f32;
        let scale_y = AVATAR_SIZE / avatar.height() as f32;
        let scale = scale_x.max(scale_y);
        let offset_x = (AVATAR_SIZE - avatar.width() as f32 * scale) / 2.0;
        let offset_y = (AVATAR_SIZE - avatar.height() as f32 * scale) / 2.0;
        layer.draw_pixmap(
            0,
            0,
            avatar.as_ref(),
            &paint,
            Transform::from_row(scale, 0.0, 0.0, scale, offset_x, offset_y),
            None,
        );
        if let Some(mut mask) = Mask::new(AVATAR_SIZE as u32, AVATAR_SIZE as u32) {
            if let Some(local_circle) =
                PathBuilder::from_circle(AVATAR_SIZE / 2.0, AVATAR_SIZE / 2.0, AVATAR_SIZE / 2.0)
            {
                mask.fill_path(
                    &local_circle,
                    FillRule::Winding,
                    true,
                    Transform::identity(),
                );
                layer.apply_mask(&mask);
            }
        }
        pixmap.draw_pixmap(
            PADDING as i32,
            PADDING as i32,
            layer.as_ref(),
            &paint,
            Transform::identity(),
            None,
        );
        return;
    }
    let hashed = content
        .author_handle
        .bytes()
        .chain(content.author_name.bytes())
        .fold(0_u32, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u32::from(byte))
        });
    let (red, green, blue) = INITIAL_COLORS[(hashed as usize) % INITIAL_COLORS.len()];
    let mut paint = Paint::default();
    paint.set_color_rgba8(red, green, blue, 255);
    paint.anti_alias = true;
    pixmap.fill_path(
        &circle,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    let initial = display_name(content)
        .chars()
        .next()
        .map(|letter| letter.to_uppercase().collect::<String>())
        .unwrap_or_default();
    if initial.is_empty() {
        return;
    }
    let buffer = shape_buffer(
        font_system,
        &initial,
        INITIAL_SIZE,
        INITIAL_LINE_HEIGHT,
        AVATAR_SIZE,
        Some(INITIAL_LINE_HEIGHT),
        true,
        true,
        // Comme le nom : une initiale arabe est un paragraphe RTL, à forcer à
        // gauche avant de centrer ses glyphes.
        Some(Align::Left),
    );
    let origin = ink_bounds(&buffer, font_system, cache)
        .map(|(left, top, right, bottom)| {
            (center - (left + right) / 2.0, center - (top + bottom) / 2.0)
        })
        .unwrap_or((
            center - AVATAR_SIZE / 2.0,
            center - INITIAL_LINE_HEIGHT / 2.0,
        ));
    draw_text(
        pixmap,
        &buffer,
        font_system,
        cache,
        origin,
        color((0xff, 0xff, 0xff)),
    );
}

/// Boîte englobante de l'encre des glyphes d'un tampon déjà mis en forme.
///
/// Les positions physiques et le placement des images swash donnent les vrais
/// bords de l'encre : l'avance logique seule ne dit pas où tombe une initiale
/// arabe (ou un emoji) dans le cercle.
fn ink_bounds(
    buffer: &Buffer,
    font_system: &mut FontSystem,
    cache: &mut SwashCache,
) -> Option<(f32, f32, f32, f32)> {
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let physical = glyph.physical((0.0, run.line_y), 1.0);
            let Some(image) = cache.get_image(font_system, physical.cache_key) else {
                continue;
            };
            if image.placement.width == 0 || image.placement.height == 0 {
                continue;
            }
            let left = (physical.x + image.placement.left) as f32;
            let top = (physical.y - image.placement.top) as f32;
            let right = left + image.placement.width as f32;
            let bottom = top + image.placement.height as f32;
            bounds = Some(match bounds {
                Some((known_left, known_top, known_right, known_bottom)) => (
                    known_left.min(left),
                    known_top.min(top),
                    known_right.max(right),
                    known_bottom.max(bottom),
                ),
                None => (left, top, right, bottom),
            });
        }
    }
    bounds
}

/// Décode un avatar PNG ; une image invalide (JPEG brut, données tronquées)
/// est ignorée au profit du cercle d'initiale.
fn decode_avatar(bytes: &[u8]) -> Option<Pixmap> {
    Pixmap::decode_png(bytes).ok()
}

/// Peint un tampon cosmic-text dans le calque.
fn draw_text(
    pixmap: &mut Pixmap,
    buffer: &Buffer,
    font_system: &mut FontSystem,
    cache: &mut SwashCache,
    origin: (f32, f32),
    base: Color,
) {
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let physical = glyph.physical((origin.0, origin.1 + run.line_y), 1.0);
            let Some(image) = cache.get_image(font_system, physical.cache_key) else {
                continue;
            };
            let left = physical.x + image.placement.left;
            let top = physical.y - image.placement.top;
            let width = image.placement.width as i32;
            let height = image.placement.height as i32;
            match image.content {
                SwashContent::Mask => {
                    let base_alpha = base.a();
                    for row in 0..height {
                        for column in 0..width {
                            let coverage = image.data[(row * width + column) as usize];
                            let alpha = (u16::from(coverage) * u16::from(base_alpha) / 255) as u8;
                            blend_pixel(
                                pixmap,
                                left + column,
                                top + row,
                                base.r(),
                                base.g(),
                                base.b(),
                                alpha,
                            );
                        }
                    }
                }
                SwashContent::Color => {
                    for row in 0..height {
                        for column in 0..width {
                            let offset = ((row * width + column) * 4) as usize;
                            blend_pixel(
                                pixmap,
                                left + column,
                                top + row,
                                image.data[offset],
                                image.data[offset + 1],
                                image.data[offset + 2],
                                image.data[offset + 3],
                            );
                        }
                    }
                }
                SwashContent::SubpixelMask => {}
            }
        }
    }
}

/// Compose une source (couleur droite + alpha) sur un calque prémultiplié.
fn blend_pixel(pixmap: &mut Pixmap, x: i32, y: i32, red: u8, green: u8, blue: u8, alpha: u8) {
    if alpha == 0 {
        return;
    }
    let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
        return;
    };
    let width = pixmap.width();
    if x >= width || y >= pixmap.height() {
        return;
    }
    let index = ((y * width + x) * 4) as usize;
    let alpha = u32::from(alpha);
    let inverse = 255 - alpha;
    let data = pixmap.data_mut();
    let red = (u32::from(red) * alpha + u32::from(data[index]) * inverse) / 255;
    let green = (u32::from(green) * alpha + u32::from(data[index + 1]) * inverse) / 255;
    let blue = (u32::from(blue) * alpha + u32::from(data[index + 2]) * inverse) / 255;
    let cover = alpha + u32::from(data[index + 3]) * inverse / 255;
    data[index] = red as u8;
    data[index + 1] = green as u8;
    data[index + 2] = blue as u8;
    data[index + 3] = cover as u8;
}

/// Couleur cosmic-text d'un triplet RGB.
fn color((red, green, blue): (u8, u8, u8)) -> Color {
    Color::rgb(red, green, blue)
}

/// Erreur de rendu de carte.
fn card_error(message: &str) -> MediaError {
    MediaError::new(
        MediaErrorCode::TranscodeFailed,
        format!("carte tweet : {message}"),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::Value;

    const TEXT_FIXTURE: &str = include_str!("../tests/fixtures/twitter_syndication_text.json");
    const PHOTO_FIXTURE: &str = include_str!("../tests/fixtures/twitter_syndication_photo.json");
    const PREVIEW_PATH: &str = r"C:\Users\SUPERK~1\AppData\Local\Temp\claude\68a8acc7ef5015a3\60808cc6-e051-4c46-99bd-bf2036662783\scratchpad\tweet-card-preview.png";

    fn content_from(raw: &str) -> TweetCardContent {
        let value: Value = serde_json::from_str(raw).unwrap();
        crate::resolve::twitter::tweet_card_content(&value)
    }

    fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
        assert!(bytes.len() > 24, "PNG trop court");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        (width, height)
    }

    fn assert_valid_card(bytes: &[u8]) -> (u32, u32) {
        let (width, height) = png_dimensions(bytes);
        assert_eq!(width, CARD_WIDTH);
        assert!(
            (CARD_MIN_HEIGHT..=CARD_MAX_HEIGHT).contains(&height),
            "hauteur hors bornes : {height}"
        );
        (width, height)
    }

    #[test]
    fn renders_the_text_tweet_fixture() {
        let content = content_from(TEXT_FIXTURE);
        assert_eq!(content.author_name, "jack");
        assert_eq!(content.author_handle, "jack");
        assert_eq!(content.text, "just setting up my twttr");
        let png = render_tweet_card(&content).unwrap();
        let (_, height) = assert_valid_card(&png);
        assert!(height >= CARD_MIN_HEIGHT);
    }

    #[test]
    fn renders_arabic_emoji_and_newlines() {
        let content = TweetCardContent {
            author_name: "منى".to_string(),
            author_handle: "mona".to_string(),
            text: "مرحبا بالعالم 🚨🚀\nسطر جديد مع emoji 🎉".to_string(),
            created_at: Some("2026-11-17T16:16:02.000Z".to_string()),
            avatar: None,
        };
        let png = render_tweet_card(&content).unwrap();
        assert_valid_card(&png);
    }

    /// Pixel clair de texte : les trois canaux dépassent le fond sombre, et
    /// les couleurs du cercle d'initiale gardent au moins un canal bas.
    fn is_light_text(pixel: tiny_skia::PremultipliedColorU8) -> bool {
        pixel.red() > 90 && pixel.green() > 90 && pixel.blue() > 90
    }

    #[test]
    fn keeps_a_rtl_header_next_to_the_avatar_and_the_initial_inside_the_circle() {
        let content = TweetCardContent {
            author_name: "مختبر 🤖".to_string(),
            author_handle: "lab".to_string(),
            text: "🚨 قنبلة نووية في عالم الذكاء الاصطناعي: إسقاط كامل 🔥🔥\nLigne française mêlée avec des emoji 😂👍.".to_string(),
            created_at: Some("2026-11-18T10:00:00.000Z".to_string()),
            avatar: None,
        };
        let png = render_tweet_card(&content).unwrap();
        let card = Pixmap::decode_png(&png).unwrap();

        // (a) Le nom arabe doit commencer juste après l'avatar : des pixels
        // de texte existent dans la bande de la ligne du nom, avant x = 600.
        // Sans alignement forcé, le nom RTL se collerait au bord droit.
        let header_height = AVATAR_SIZE.max(NAME_LINE_HEIGHT + SECONDARY_LINE_HEIGHT);
        let name_top = PADDING + (header_height - NAME_LINE_HEIGHT - SECONDARY_LINE_HEIGHT) / 2.0;
        let name_left = (PADDING + AVATAR_SIZE + 24.0 - 4.0) as u32;
        let mut name_pixels = 0_u32;
        for y in name_top as u32..=(name_top + NAME_LINE_HEIGHT) as u32 {
            for x in name_left..=600 {
                if card.pixel(x, y).is_some_and(is_light_text) {
                    name_pixels += 1;
                }
            }
        }
        assert!(
            name_pixels > 20,
            "nom absent à gauche de x=600 dans sa bande ({name_pixels} pixels clairs) : \
             un paragraphe RTL est probablement resté aligné à droite"
        );

        // (b) L'initiale doit rester dans le cercle de l'avatar : aucun pixel
        // clair hors du cercle dans le carré de l'avatar. Tolérance de 2 px
        // pour l'anticrénelage du bord.
        let center = PADDING + AVATAR_SIZE / 2.0;
        let radius = AVATAR_SIZE / 2.0;
        for y in (PADDING as u32)..((PADDING + AVATAR_SIZE) as u32) {
            for x in (PADDING as u32)..((PADDING + AVATAR_SIZE) as u32) {
                let distance =
                    ((x as f32 + 0.5 - center).powi(2) + (y as f32 + 0.5 - center).powi(2)).sqrt();
                if distance > radius + 2.0 {
                    let pixel = card.pixel(x, y).unwrap();
                    assert!(
                        !is_light_text(pixel),
                        "pixel clair hors du cercle de l'avatar en ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn clamps_a_very_long_text_to_the_maximum_height() {
        let content = TweetCardContent {
            author_name: "jack".to_string(),
            author_handle: "jack".to_string(),
            text: "un tweet interminable ".repeat(2000),
            created_at: Some("2006-03-21T20:50:14.000Z".to_string()),
            avatar: None,
        };
        let png = render_tweet_card(&content).unwrap();
        let (_, height) = assert_valid_card(&png);
        assert_eq!(height, CARD_MAX_HEIGHT);
    }

    #[test]
    fn renders_an_invalid_avatar_as_an_initial_circle() {
        let content = TweetCardContent {
            author_name: "Mo".to_string(),
            author_handle: "EngMoElgaraihy".to_string(),
            text: "Une photo de remplacement".to_string(),
            created_at: None,
            avatar: Some(b"pas un PNG".to_vec()),
        };
        let png = render_tweet_card(&content).unwrap();
        assert_valid_card(&png);
    }

    #[test]
    fn formats_french_dates() {
        assert_eq!(
            format_tweet_date("2006-03-21T20:50:14.000Z").as_deref(),
            Some("21 mars 2006")
        );
        assert_eq!(
            format_tweet_date("2026-11-17T16:16:02.000Z").as_deref(),
            Some("17 novembre 2026")
        );
        assert_eq!(
            format_tweet_date("2024-01-05T00:00:00Z").as_deref(),
            Some("5 janvier 2024")
        );
        assert_eq!(format_tweet_date("2024-13-05T00:00:00Z"), None);
        assert_eq!(format_tweet_date("2024-00-05T00:00:00Z"), None);
        assert_eq!(format_tweet_date("pas une date"), None);
        assert_eq!(format_tweet_date(""), None);
    }

    #[test]
    fn photo_fixture_content_never_panics_on_the_card_renderer() {
        // Le rendu de carte n'est pas utilisé pour un tweet avec photo, mais
        // son contenu (arabe, liens t.co) reste un bon cas de robustesse.
        let content = content_from(PHOTO_FIXTURE);
        assert!(!content.text.is_empty());
        let png = render_tweet_card(&content).unwrap();
        assert_valid_card(&png);
    }

    #[test]
    #[ignore = "écrit un aperçu local"]
    fn writes_tweet_card_preview() {
        let content = content_from(TEXT_FIXTURE);
        let png = render_tweet_card(&content).unwrap();
        std::fs::write(PREVIEW_PATH, &png).expect("écriture de l'aperçu");
        assert_valid_card(&png);
        assert!(std::path::Path::new(PREVIEW_PATH).exists());
    }
}
