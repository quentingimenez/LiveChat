//! Rendu d'une « carte tweet » : la publication devient elle-même le média.
//!
//! La carte reprend l'aspect sombre de l'overlay : fond arrondi transparent
//! autour de `#171b25`, avatar circulaire, nom en gras, identifiant et date en
//! couleur secondaire, puis le texte du tweet et ses photos (grille façon X)
//! ou sa vidéo (zone réservée, composée ensuite en MP4). Un tweet cité ajoute
//! un bloc encadré (nom, identifiant, texte, photos et vidéo de la citation)
//! sous le média du tweet parent.
//!
//! La découverte et le repli des polices, la mise en forme, la césure et le
//! bidirectionnel sont confiés à parley (fontique) ; swash rastérise les
//! glyphes (dont les emoji couleur) et tiny-skia ne sert qu'au fond, aux
//! arrondis et à l'encodage PNG.

use std::sync::{Mutex, OnceLock};

use parley::{
    Alignment, AlignmentOptions, FontContext, FontWeight, Layout, LayoutContext, LineHeight,
    PositionedLayoutItem, StyleProperty,
};
use swash::scale::image::{Content as SwashContent, Image as SwashImage};
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Vector};
use swash::FontRef;
use tiny_skia::{
    FillRule, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform,
};

use crate::error::{MediaError, MediaErrorCode, MediaResult};

/// Largeur fixe d'une carte tweet.
pub const CARD_WIDTH: u32 = 1080;
/// Hauteur minimale d'une carte tweet.
pub const CARD_MIN_HEIGHT: u32 = 360;
/// Hauteur maximale d'une carte tweet (le texte est tronqué au-delà).
pub const CARD_MAX_HEIGHT: u32 = 1920;
/// Hauteur maximale d'une carte tweet qui porte un média : une vidéo verticale
/// 9:16 occupe presque toute la largeur, la carte dépasse donc 1920 px.
pub const CARD_MEDIA_MAX_HEIGHT: u32 = 3200;

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

/// Marge intérieure du bloc de citation.
const QUOTE_PADDING: f32 = 26.0;
/// Rayon des coins du bloc de citation.
const QUOTE_CORNER_RADIUS: f32 = 20.0;
/// Épaisseur de la bordure du bloc de citation.
const QUOTE_BORDER_WIDTH: f32 = 2.0;
/// Espace entre le texte du tweet et le bloc de citation.
const QUOTE_GAP: f32 = 28.0;
/// Espace vertical entre les éléments du bloc de citation.
const QUOTE_INNER_GAP: f32 = 16.0;
/// Espace entre le nom et l'identifiant de l'auteur cité.
const QUOTE_HANDLE_GAP: f32 = 10.0;
/// Taille du nom de l'auteur cité.
const QUOTE_NAME_SIZE: f32 = 30.0;
const QUOTE_NAME_LINE_HEIGHT: f32 = 40.0;
/// Taille de l'identifiant de l'auteur cité.
const QUOTE_HANDLE_SIZE: f32 = 28.0;
const QUOTE_HANDLE_LINE_HEIGHT: f32 = 38.0;
/// Taille du texte cité (interligne 1,35).
const QUOTE_TEXT_SIZE: f32 = 34.0;
const QUOTE_TEXT_LINE_HEIGHT: f32 = QUOTE_TEXT_SIZE * 1.35;
/// Nombre maximal de lignes du texte cité.
const QUOTE_MAX_TEXT_LINES: usize = 6;
/// Rapport largeur/hauteur du cadre d'une image citée seule (façon X).
const QUOTE_IMAGE_ASPECT: f32 = 16.0 / 9.0;
/// Bornes du rapport d'une image citée dans une paire : une photo portrait
/// n'est pas étirée au-delà, une photo paysage n'est pas écrasée en dessous.
const QUOTE_IMAGE_MIN_ASPECT: f32 = 0.7;
const QUOTE_IMAGE_MAX_ASPECT: f32 = 1.6;
/// Rayon des coins de l'image citée.
const QUOTE_IMAGE_RADIUS: f32 = 14.0;

/// Espace entre le texte du tweet et ses photos ou sa vidéo.
const MEDIA_GAP: f32 = 28.0;
/// Espace entre deux photos d'une même ligne.
const MEDIA_IMAGE_GAP: f32 = 12.0;
/// Rayon des coins des photos et des vidéos du tweet parent.
const MEDIA_IMAGE_RADIUS: f32 = 18.0;
/// Bornes du rapport largeur/hauteur d'une vidéo : 9:16 (vertical) à 16:9.
const MEDIA_VIDEO_MIN_ASPECT: f32 = 9.0 / 16.0;
const MEDIA_VIDEO_MAX_ASPECT: f32 = 16.0 / 9.0;
/// Fond de la zone réservée à la vidéo, avant composition.
const MEDIA_VIDEO_BACKGROUND: (u8, u8, u8) = (0x0b, 0x0e, 0x14);

/// Fond du bloc de citation, un peu plus clair que la carte.
const QUOTE_BACKGROUND: (u8, u8, u8) = (0x20, 0x25, 0x32);
/// Bordure du bloc de citation.
const QUOTE_BORDER_COLOR: (u8, u8, u8) = (0x35, 0x3d, 0x4f);

/// Espace entre l'avatar et le bloc nom/identifiant.
const HEADER_TEXT_GAP: f32 = 24.0;

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

/// Rectangle de recadrage suggéré par X pour une image citée.
///
/// Coordonnées relatives à l'image source (0.0 à 1.0) : le rendu les applique
/// à la taille réelle du PNG reçu, quel que soit le redimensionnement opéré
/// au téléchargement. X publie plusieurs rectangles (un par format courant) ;
/// le rendu retient celui qui ressemble le plus au cadre du bloc de citation
/// pour ne pas couper le sujet de la photo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QuoteImageFocus {
    /// Abscisse relative du coin supérieur gauche.
    pub x: f32,
    /// Ordonnée relative du coin supérieur gauche.
    pub y: f32,
    /// Largeur relative.
    pub width: f32,
    /// Hauteur relative.
    pub height: f32,
}

/// Photo jointe à un tweet (ou au tweet cité), avec son recadrage suggéré par X.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TweetPhoto {
    /// URL source de la photo, consommée par le téléchargement.
    pub source_url: String,
    /// Octets PNG de la photo, `None` tant qu'elle n'a pas été téléchargée.
    pub data: Option<Vec<u8>>,
    /// Recadrages suggérés par X pour cette photo, vides s'il n'en fournit pas.
    pub focus: Vec<QuoteImageFocus>,
}

/// Vidéo jointe à un tweet, incrustée dans la carte au téléchargement.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TweetVideo {
    /// Rapport largeur/hauteur de la vidéo (0,5625 pour du 9:16 vertical).
    pub aspect: f32,
}

/// Contenu d'un tweet cité, rendu dans un bloc encadré sous le texte.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuotedTweetContent {
    /// Nom d'affichage de l'auteur cité.
    pub author_name: String,
    /// Identifiant sans `@` de l'auteur cité.
    pub author_handle: String,
    /// Texte du tweet cité.
    pub text: String,
    /// Photos jointes au tweet cité (ou poster d'une vidéo citée), déjà
    /// téléchargées et converties en PNG par l'appelant.
    pub images: Vec<TweetPhoto>,
    /// Vidéo du tweet cité, jouée dans le bloc quand elle a pu être téléchargée.
    pub video: Option<TweetVideo>,
}

/// Contenu textuel d'une carte tweet.
#[derive(Clone, Debug, Default, PartialEq)]
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
    /// Photos jointes au tweet, déjà téléchargées et converties en PNG.
    pub photos: Vec<TweetPhoto>,
    /// Vidéo jointe au tweet, incrustée dans la carte au téléchargement.
    pub video: Option<TweetVideo>,
    /// Tweet cité par la publication, si elle en cite un.
    pub quoted: Option<QuotedTweetContent>,
}

/// Contexte de mise en forme et de rastérisation partagé.
///
/// `FontContext` découvre les polices système et `LayoutContext` réutilise ses
/// allocations ; les construire à chaque carte coûterait un scan complet des
/// polices. `ScaleContext` garde les caches de swash.
struct TextEngine {
    font_cx: FontContext,
    layout_cx: LayoutContext<[u8; 4]>,
    scale_cx: ScaleContext,
}

static TEXT_ENGINE: OnceLock<Mutex<TextEngine>> = OnceLock::new();

/// Type de disposition parley de la carte (pinceau par défaut `[u8; 4]`).
type CardLayout = Layout<[u8; 4]>;

/// Style d'un bloc de texte de la carte.
#[derive(Clone, Copy)]
struct TextStyleSpec {
    size: f32,
    line_height: f32,
    bold: bool,
    align: Alignment,
}

/// En-tête « nom @identifiant » de l'auteur cité.
struct QuoteHeader {
    /// Nom de l'auteur cité.
    name: CardLayout,
    /// Largeur du nom, pour placer l'identifiant à sa suite.
    name_width: f32,
    /// Identifiant (`@handle`) de l'auteur cité, absent quand le nom prend
    /// toute la largeur.
    handle: Option<CardLayout>,
    /// Hauteur de la ligne d'en-tête.
    height: f32,
}

/// Image du tweet cité, déjà mise à l'échelle pour la carte.
struct QuoteImage {
    pixmap: Pixmap,
    /// Largeur du cadre affiché.
    width: u32,
    /// Hauteur du cadre affiché.
    height: u32,
    /// Échelle appliquée à l'image source.
    scale_x: f32,
    scale_y: f32,
    /// Décalage de l'image dans le cadre (recadrage centré).
    offset_x: f32,
    offset_y: f32,
    /// Rayon des coins du cadre.
    radius: f32,
}

/// Ligne d'images du bloc de citation : une image pleine largeur ou une paire.
struct QuoteImageRow {
    /// Images de la ligne, de gauche à droite.
    images: Vec<QuoteImage>,
    /// Hauteur de la ligne, commune à ses images.
    height: f32,
}

/// Zone vidéo du bloc de citation, position relative au bloc.
struct QuoteVideoArea {
    /// Ordonnée du haut de la zone, depuis le haut du bloc.
    top: f32,
    /// Largeur de la zone.
    width: u32,
    /// Hauteur de la zone.
    height: u32,
}

/// Disposition du bloc de citation, calculée avant le corps pour réserver la
/// hauteur nécessaire.
struct QuoteLayout {
    /// En-tête de l'auteur cité, absent si le payload n'a ni nom ni handle.
    header: Option<QuoteHeader>,
    /// Texte cité, vide si la citation n'a pas de texte.
    text: BodyLayout,
    /// Photos citées, disposées en lignes façon X.
    rows: Vec<QuoteImageRow>,
    /// Vidéo citée, absente si la citation n'en porte pas ou si elle a échoué.
    video: Option<QuoteVideoArea>,
    /// Hauteur totale du bloc, marges et bordure comprises.
    height: f32,
    /// Largeur intérieure disponible pour le contenu.
    inner_width: f32,
}

/// Vidéo du tweet parent, disposée sous son texte.
struct MediaVideo {
    /// Largeur du cadre, pleine largeur intérieure de la carte.
    width: u32,
    /// Hauteur du cadre, déduite du rapport de la vidéo.
    height: u32,
    /// Rayon des coins du cadre.
    radius: f32,
}

/// Média du tweet parent : ses photos en lignes façon X ou sa vidéo.
struct CardMediaLayout {
    /// Photos disposées en lignes.
    rows: Vec<QuoteImageRow>,
    /// Zone réservée à la vidéo, absente si le tweet n'en porte pas.
    video: Option<MediaVideo>,
    /// Hauteur totale du média, espaces entre lignes compris.
    height: f32,
}

/// Cible d'une zone vidéo composée dans la carte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardVideoTarget {
    /// Vidéo du tweet lui-même.
    Parent,
    /// Vidéo du tweet cité.
    Quoted,
}

/// Zone vidéo de la carte, en pixels de l'image rendue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardVideoSlot {
    /// Abscisse du coin supérieur gauche.
    pub x: u32,
    /// Ordonnée du coin supérieur gauche.
    pub y: u32,
    /// Largeur de la zone.
    pub width: u32,
    /// Hauteur de la zone.
    pub height: u32,
    /// Rayon des coins de la zone.
    pub radius: u32,
    /// Média dont la vidéo doit être incrustée dans cette zone.
    pub target: CardVideoTarget,
}

/// Carte rendue : le PNG de fond et les zones vidéo à composer par-dessus.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderedCard {
    /// Image PNG de la carte, zones vidéo remplies d'un fond sombre.
    pub png: Vec<u8>,
    /// Zones où incruster une vidéo, dans l'ordre de composition.
    pub video_slots: Vec<CardVideoSlot>,
}

/// Rendu synchrone d'une carte tweet en PNG.
///
/// La hauteur s'adapte au contenu entre [`CARD_MIN_HEIGHT`] et
/// [`CARD_MAX_HEIGHT`] — jusqu'à [`CARD_MEDIA_MAX_HEIGHT`] quand la carte porte
/// un média ; au-delà, le texte est tronqué avec une ellipsis.
pub fn render_tweet_card(content: &TweetCardContent) -> MediaResult<Vec<u8>> {
    render_tweet_card_with_slots(content).map(|card| card.png)
}

/// Rendu synchrone d'une carte tweet, avec les zones vidéo à composer.
///
/// Les zones vidéo sont peintes en sombre dans le PNG : l'appelant y incruste
/// ensuite la vidéo du tweet ou de la citation pour produire un MP4.
pub fn render_tweet_card_with_slots(content: &TweetCardContent) -> MediaResult<RenderedCard> {
    let mutex = TEXT_ENGINE.get_or_init(|| {
        Mutex::new(TextEngine {
            font_cx: FontContext::new(),
            layout_cx: LayoutContext::new(),
            scale_cx: ScaleContext::new(),
        })
    });
    let mut engine = mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    render_with_engine(content, &mut engine)
}

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

fn render_with_engine(
    content: &TweetCardContent,
    engine: &mut TextEngine,
) -> MediaResult<RenderedCard> {
    let date = content.created_at.as_deref().and_then(format_tweet_date);
    let footer_height = if date.is_some() {
        FOOTER_GAP + SECONDARY_LINE_HEIGHT
    } else {
        0.0
    };
    let body_top = PADDING + AVATAR_SIZE + HEADER_GAP;
    let text_width = CARD_WIDTH as f32 - 2.0 * PADDING;
    // Le média et le bloc de citation sont disposés avant le corps : leur
    // hauteur réduit celle qui reste au texte du tweet parent.
    let media = layout_card_media(&content.photos, content.video, text_width);
    let quote = content
        .quoted
        .as_ref()
        .map(|quoted| layout_quote(engine, quoted, text_width));
    let text = normalized_text(&content.text);
    let media_gap = if media.height > 0.0 && !text.is_empty() {
        MEDIA_GAP
    } else {
        0.0
    };
    let quote_height = quote.as_ref().map_or(0.0, |quote| QUOTE_GAP + quote.height);
    // Une carte qui porte un média peut dépasser la hauteur d'une carte de
    // texte : une vidéo verticale occupe presque toute la largeur.
    let max_card_height = if media.height > 0.0 {
        CARD_MEDIA_MAX_HEIGHT
    } else {
        CARD_MAX_HEIGHT
    };
    let reserved_height = media_gap + media.height + quote_height;
    let max_text_height =
        (max_card_height as f32 - body_top - footer_height - PADDING - reserved_height).max(0.0);
    let body_style = TextStyleSpec {
        size: TEXT_SIZE,
        line_height: TEXT_LINE_HEIGHT,
        bold: false,
        // Le corps garde l'alignement naturel : arabe à droite, latin à gauche.
        align: Alignment::Start,
    };
    let body = layout_body(engine, &text, body_style, text_width, max_text_height);
    let media_top = body_top + body.height + media_gap;
    let quote_top = media_top + media.height + QUOTE_GAP;
    let content_height = body_top + body.height + reserved_height + footer_height + PADDING;
    let height = if body.clamped {
        max_card_height
    } else {
        (content_height.ceil() as u32).clamp(CARD_MIN_HEIGHT, max_card_height)
    };
    let mut pixmap = Pixmap::new(CARD_WIDTH, height)
        .ok_or_else(|| card_error("allocation du calque impossible"))?;
    draw_background(&mut pixmap, height)?;

    let name = display_name(content);
    let handle = display_handle(content);
    let name_width = text_width - AVATAR_SIZE - HEADER_TEXT_GAP;
    let name_layout = single_line_layout(
        engine,
        &name,
        TextStyleSpec {
            size: NAME_SIZE,
            line_height: NAME_LINE_HEIGHT,
            bold: true,
            // Un nom arabe est un paragraphe RTL : sans alignement forcé, il
            // se collerait au bord droit de son bloc, loin de l'avatar.
            align: Alignment::Left,
        },
        name_width,
    );
    let handle_layout = single_line_layout(
        engine,
        &handle,
        TextStyleSpec {
            size: SECONDARY_SIZE,
            line_height: SECONDARY_LINE_HEIGHT,
            bold: false,
            align: Alignment::Left,
        },
        name_width,
    );
    let header_height = AVATAR_SIZE.max(NAME_LINE_HEIGHT + SECONDARY_LINE_HEIGHT);
    let header_text_top =
        PADDING + (header_height - NAME_LINE_HEIGHT - SECONDARY_LINE_HEIGHT) / 2.0;
    let header_x = PADDING + AVATAR_SIZE + HEADER_TEXT_GAP;

    draw_avatar(&mut pixmap, content, engine);
    draw_text(
        &mut pixmap,
        &name_layout,
        engine,
        (header_x, header_text_top),
        TEXT_COLOR,
    );
    draw_text(
        &mut pixmap,
        &handle_layout,
        engine,
        (header_x, header_text_top + NAME_LINE_HEIGHT),
        SECONDARY_COLOR,
    );
    draw_body(&mut pixmap, &body, engine, (PADDING, body_top), TEXT_COLOR);
    if media.height > 0.0 {
        draw_card_media(&mut pixmap, &media, (PADDING, media_top));
    }
    if let Some(quote) = quote.as_ref() {
        draw_quote(&mut pixmap, quote, engine, (PADDING, quote_top), text_width);
    }
    if let Some(date) = date {
        let date_layout = single_line_layout(
            engine,
            &date,
            TextStyleSpec {
                size: SECONDARY_SIZE,
                line_height: SECONDARY_LINE_HEIGHT,
                bold: false,
                align: Alignment::Left,
            },
            text_width,
        );
        draw_text(
            &mut pixmap,
            &date_layout,
            engine,
            (PADDING, height as f32 - PADDING - SECONDARY_LINE_HEIGHT),
            SECONDARY_COLOR,
        );
    }
    let mut video_slots = Vec::new();
    if let Some(video) = &media.video {
        video_slots.push(CardVideoSlot {
            x: PADDING.round() as u32,
            y: media_top.round() as u32,
            width: video.width,
            height: video.height,
            radius: video.radius.round() as u32,
            target: CardVideoTarget::Parent,
        });
    }
    if let Some(video) = quote.as_ref().and_then(|quote| quote.video.as_ref()) {
        video_slots.push(CardVideoSlot {
            x: (PADDING + QUOTE_BORDER_WIDTH + QUOTE_PADDING).round() as u32,
            y: (quote_top + video.top).round() as u32,
            width: video.width,
            height: video.height,
            radius: QUOTE_IMAGE_RADIUS.round() as u32,
            target: CardVideoTarget::Quoted,
        });
    }
    let png = pixmap
        .encode_png()
        .map_err(|error| card_error(&format!("encodage PNG impossible : {error}")))?;
    Ok(RenderedCard { png, video_slots })
}

/// Normalise les fins de ligne du texte.
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

/// Construit et casse un paragraphe avec le style demandé.
fn build_layout(
    engine: &mut TextEngine,
    text: &str,
    style: TextStyleSpec,
    max_width: Option<f32>,
) -> CardLayout {
    let mut builder = engine
        .layout_cx
        .ranged_builder(&mut engine.font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(style.size));
    builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(
        style.line_height,
    )));
    if style.bold {
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(700.0)));
    }
    let mut layout = builder.build(text);
    layout.set_alignment(style.align, AlignmentOptions::default());
    layout.break_all_lines(max_width);
    layout
}

/// Garde la première ligne d'un texte et l'ellipse si elle déborde.
///
/// Le `…` est ajouté au texte avant une nouvelle mise en forme : parley 0.13
/// n'a pas d'ellipsis intégrée. Une recherche binaire sur les frontières de
/// caractères évite de reconstruire une disposition par caractère.
fn single_line_layout(
    engine: &mut TextEngine,
    text: &str,
    style: TextStyleSpec,
    max_width: f32,
) -> CardLayout {
    let layout = build_layout(engine, text, style, Some(max_width));
    if layout.lines().len() <= 1 {
        return layout;
    }
    let end = layout
        .lines()
        .next()
        .map(|line| line.text_range().end)
        .unwrap_or(text.len());
    let boundaries: Vec<usize> = text[..end].char_indices().map(|(index, _)| index).collect();
    let mut best = build_layout(engine, "…", style, Some(max_width));
    let mut low = 0_usize;
    let mut high = boundaries.len();
    while low < high {
        let middle = (low + high) / 2;
        let cut = boundaries.get(middle).copied().unwrap_or(end);
        let candidate = format!("{}…", text[..cut].trim_end());
        let candidate = build_layout(engine, &candidate, style, Some(max_width));
        if candidate.lines().len() <= 1 {
            best = candidate;
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    best
}

/// Corps du tweet : une disposition parley par paragraphe.
///
/// Parley 0.13 ne porte qu'une direction de base par disposition. cosmic-text
/// analysait la direction de chaque ligne : un texte arabe suivi d'une ligne
/// française doit donc être découpé aux retours à la ligne pour que chaque
/// paragraphe garde sa direction et son alignement naturels.
struct BodyLayout {
    paragraphs: Vec<CardLayout>,
    height: f32,
    /// `true` quand le texte a été tronqué : la carte garde alors sa hauteur
    /// maximale au lieu de se réduire à la hauteur du texte tronqué.
    clamped: bool,
}

/// Met en forme le corps, tronqué à la hauteur maximale si besoin.
fn layout_body(
    engine: &mut TextEngine,
    text: &str,
    style: TextStyleSpec,
    max_width: f32,
    max_height: f32,
) -> BodyLayout {
    let paragraphs: Vec<&str> = text.split('\n').collect();
    let full: Vec<CardLayout> = paragraphs
        .iter()
        .map(|paragraph| build_layout(engine, paragraph, style, Some(max_width)))
        .collect();
    let height: f32 = full.iter().map(CardLayout::height).sum();
    if height <= max_height + 0.5 {
        return BodyLayout {
            paragraphs: full,
            height,
            clamped: false,
        };
    }
    let line_height = style.line_height.max(1.0);
    let max_lines = ((max_height / line_height).floor() as usize).max(1);
    let mut kept: Vec<CardLayout> = Vec::new();
    let mut remaining = max_lines;
    for (index, layout) in full.into_iter().enumerate() {
        if remaining == 0 {
            break;
        }
        let lines = layout.lines().len();
        if lines <= remaining {
            remaining -= lines;
            kept.push(layout);
        } else {
            let paragraph = paragraphs.get(index).copied().unwrap_or_default();
            kept.push(clamp_paragraph(
                engine, paragraph, style, max_width, remaining,
            ));
            remaining = 0;
        }
    }
    if kept.is_empty() {
        kept.push(clamp_paragraph(
            engine,
            paragraphs.first().copied().unwrap_or_default(),
            style,
            max_width,
            1,
        ));
    }
    let height = kept.iter().map(CardLayout::height).sum();
    BodyLayout {
        paragraphs: kept,
        height,
        clamped: true,
    }
}

/// Tronque un paragraphe à `max_lines` lignes, avec une ellipsis.
fn clamp_paragraph(
    engine: &mut TextEngine,
    text: &str,
    style: TextStyleSpec,
    max_width: f32,
    max_lines: usize,
) -> CardLayout {
    let layout = build_layout(engine, text, style, Some(max_width));
    if layout.lines().len() <= max_lines {
        return layout;
    }
    let mut lines = max_lines.max(1);
    loop {
        let cut = layout
            .lines()
            .nth(lines)
            .map(|line| line.text_range().start)
            .unwrap_or(text.len());
        let truncated = format!("{}…", text[..cut].trim_end());
        let candidate = build_layout(engine, &truncated, style, Some(max_width));
        if candidate.lines().len() <= max_lines || lines <= 1 {
            return candidate;
        }
        lines -= 1;
    }
}

/// Peint les paragraphes du corps à la suite, sans espace supplémentaire.
fn draw_body(
    pixmap: &mut Pixmap,
    body: &BodyLayout,
    engine: &mut TextEngine,
    origin: (f32, f32),
    base: (u8, u8, u8),
) {
    let mut top = origin.1;
    for paragraph in &body.paragraphs {
        draw_text(pixmap, paragraph, engine, (origin.0, top), base);
        top += paragraph.height();
    }
}

/// Dispose le bloc de citation (en-tête, texte et image) pour une largeur
/// donnée.
///
/// La hauteur du bloc est calculée avant le texte du parent : elle sert à
/// réserver la place, et le texte cité est tronqué à
/// [`QUOTE_MAX_TEXT_LINES`] lignes pour garder une carte lisible.
fn layout_quote(engine: &mut TextEngine, quoted: &QuotedTweetContent, width: f32) -> QuoteLayout {
    let inner_width = (width - 2.0 * (QUOTE_BORDER_WIDTH + QUOTE_PADDING)).max(1.0);
    let name = quoted.author_name.trim().to_string();
    let handle = {
        let handle = quoted.author_handle.trim().trim_start_matches('@');
        if handle.is_empty() {
            String::new()
        } else {
            format!("@{handle}")
        }
    };
    let name_style = TextStyleSpec {
        size: QUOTE_NAME_SIZE,
        line_height: QUOTE_NAME_LINE_HEIGHT,
        bold: true,
        align: Alignment::Left,
    };
    let header = if name.is_empty() && handle.is_empty() {
        None
    } else if name.is_empty() {
        // Sans nom, l'identifiant tient lieu d'en-tête.
        let layout = single_line_layout(engine, &handle, name_style, inner_width);
        Some(QuoteHeader {
            name_width: layout.width(),
            name: layout,
            handle: None,
            height: QUOTE_NAME_LINE_HEIGHT,
        })
    } else {
        let name_layout = single_line_layout(engine, &name, name_style, inner_width);
        let name_width = name_layout.width();
        let remaining = inner_width - name_width - QUOTE_HANDLE_GAP;
        let handle_layout = (!handle.is_empty() && remaining > 48.0).then(|| {
            single_line_layout(
                engine,
                &handle,
                TextStyleSpec {
                    size: QUOTE_HANDLE_SIZE,
                    line_height: QUOTE_HANDLE_LINE_HEIGHT,
                    bold: false,
                    align: Alignment::Left,
                },
                remaining,
            )
        });
        Some(QuoteHeader {
            name: name_layout,
            name_width,
            handle: handle_layout,
            height: QUOTE_NAME_LINE_HEIGHT,
        })
    };
    let text = normalized_text(&quoted.text);
    let text_layout = if text.is_empty() {
        BodyLayout {
            paragraphs: Vec::new(),
            height: 0.0,
            clamped: false,
        }
    } else {
        layout_body(
            engine,
            &text,
            TextStyleSpec {
                size: QUOTE_TEXT_SIZE,
                line_height: QUOTE_TEXT_LINE_HEIGHT,
                bold: false,
                align: Alignment::Start,
            },
            inner_width,
            QUOTE_MAX_TEXT_LINES as f32 * QUOTE_TEXT_LINE_HEIGHT,
        )
    };
    // Une vidéo citée remplace les photos : le poster n'est qu'un repli quand
    // la vidéo n'a pas pu être téléchargée.
    let video_dims = quoted
        .video
        .map(|video| media_video_dimensions(video, inner_width));
    let rows = if video_dims.is_some() {
        Vec::new()
    } else {
        layout_photo_rows(
            &quoted.images,
            inner_width,
            QUOTE_INNER_GAP,
            QUOTE_IMAGE_RADIUS,
        )
    };
    let mut height = 2.0 * (QUOTE_BORDER_WIDTH + QUOTE_PADDING);
    if let Some(header) = &header {
        height += header.height;
    }
    let has_text = !text_layout.paragraphs.is_empty() && text_layout.height > 0.0;
    if has_text {
        if header.is_some() {
            height += QUOTE_INNER_GAP;
        }
        height += text_layout.height;
    }
    let mut video = None;
    if let Some((video_width, video_height)) = video_dims {
        if header.is_some() || has_text {
            height += QUOTE_INNER_GAP;
        }
        video = Some(QuoteVideoArea {
            top: height,
            width: video_width,
            height: video_height,
        });
        height += video_height as f32;
    }
    if !rows.is_empty() {
        if header.is_some() || has_text {
            height += QUOTE_INNER_GAP;
        }
        height += rows_height(&rows, QUOTE_INNER_GAP);
    }
    QuoteLayout {
        header,
        text: text_layout,
        rows,
        video,
        height,
        inner_width,
    }
}

/// Image citée décodée, avant mise à l'échelle dans sa cellule.
struct DecodedQuoteImage {
    pixmap: Pixmap,
    width: f32,
    height: f32,
    focus: Vec<QuoteImageFocus>,
}

/// Dispose les photos en lignes façon X : une image seule occupe toute la
/// largeur au rapport [`QUOTE_IMAGE_ASPECT`] ; à partir de deux, elles vont par
/// paires côte à côte (trois photos : une large puis deux, quatre : deux lignes
/// de deux).
fn layout_photo_rows(
    images: &[TweetPhoto],
    inner_width: f32,
    gap: f32,
    radius: f32,
) -> Vec<QuoteImageRow> {
    let mut decoded = Vec::new();
    for image in images {
        let Some(bytes) = image.data.as_deref() else {
            continue;
        };
        let Ok(pixmap) = Pixmap::decode_png(bytes) else {
            continue;
        };
        let (width, height) = (pixmap.width() as f32, pixmap.height() as f32);
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        decoded.push(DecodedQuoteImage {
            pixmap,
            width,
            height,
            focus: image.focus.clone(),
        });
    }
    let mut rows = Vec::new();
    match decoded.len() {
        0 => {}
        1 => rows.push(full_width_row(&decoded[0], inner_width, radius)),
        2 => rows.push(paired_row(&decoded[0..2], inner_width, gap, radius)),
        3 => {
            rows.push(full_width_row(&decoded[0], inner_width, radius));
            rows.push(paired_row(&decoded[1..3], inner_width, gap, radius));
        }
        _ => {
            rows.push(paired_row(&decoded[0..2], inner_width, gap, radius));
            rows.push(paired_row(&decoded[2..4], inner_width, gap, radius));
        }
    }
    rows
}

/// Hauteur totale de lignes de photos, espaces entre lignes compris.
fn rows_height(rows: &[QuoteImageRow], gap: f32) -> f32 {
    if rows.is_empty() {
        return 0.0;
    }
    rows.iter().map(|row| row.height).sum::<f32>() + gap * rows.len().saturating_sub(1) as f32
}

/// Ligne d'une image pleine largeur, au rapport [`QUOTE_IMAGE_ASPECT`].
fn full_width_row(image: &DecodedQuoteImage, inner_width: f32, radius: f32) -> QuoteImageRow {
    let box_height = inner_width / QUOTE_IMAGE_ASPECT;
    QuoteImageRow {
        images: vec![cover_quote_image(image, inner_width, box_height, radius)],
        height: box_height,
    }
}

/// Ligne de deux images de largeur égale, à la hauteur de la première (bornée
/// par [`QUOTE_IMAGE_MIN_ASPECT`] et [`QUOTE_IMAGE_MAX_ASPECT`]).
fn paired_row(
    images: &[DecodedQuoteImage],
    inner_width: f32,
    gap: f32,
    radius: f32,
) -> QuoteImageRow {
    let cell_width = ((inner_width - gap) / 2.0).max(1.0);
    let first = &images[0];
    let aspect = (first.width / first.height).clamp(QUOTE_IMAGE_MIN_ASPECT, QUOTE_IMAGE_MAX_ASPECT);
    let cell_height = cell_width / aspect;
    QuoteImageRow {
        images: images
            .iter()
            .map(|image| cover_quote_image(image, cell_width, cell_height, radius))
            .collect(),
        height: cell_height,
    }
}

/// Met une image à l'échelle de sa cellule (couverture) et la recadre autour du
/// rectangle de `focus_rects` le plus proche du rapport de la cellule, pour ne
/// pas couper le sujet.
///
/// Une vignette plus petite que la moitié de la cellule garde sa taille (pas
/// d'agrandissement d'une image de mauvaise qualité).
fn cover_quote_image(
    image: &DecodedQuoteImage,
    box_width: f32,
    box_height: f32,
    radius: f32,
) -> QuoteImage {
    let tiny = image.width < box_width * 0.5 && image.height < box_height * 0.5;
    if tiny {
        return QuoteImage {
            pixmap: image.pixmap.clone(),
            width: (image.width.round() as u32).max(1),
            height: (image.height.round() as u32).max(1),
            scale_x: 1.0,
            scale_y: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            radius,
        };
    }
    // Couverture : l'image remplit la cellule et le surplus est recadré.
    let scale = (box_width / image.width).max(box_height / image.height);
    let scaled_width = image.width * scale;
    let scaled_height = image.height * scale;
    let mut offset_x = (box_width - scaled_width) / 2.0;
    let mut offset_y = (box_height - scaled_height) / 2.0;
    if let Some(focus) = closest_focus_rect(
        &image.focus,
        image.width,
        image.height,
        box_width / box_height,
    ) {
        let center_x = (focus.x + focus.width / 2.0) * image.width;
        let center_y = (focus.y + focus.height / 2.0) * image.height;
        // En mode couverture, `scaled_width` et `scaled_height` sont au moins
        // égaux à la cellule, mais leur arrondi f32 peut les placer un ULP
        // au-dessus : `box_width - scaled_width` vaut alors +6,1e-5 au lieu de
        // 0.0, et `f32::clamp` panique quand sa borne basse dépasse sa borne
        // haute (« min > max, or either was NaN »).
        let min_x = (box_width - scaled_width).min(0.0);
        let min_y = (box_height - scaled_height).min(0.0);
        offset_x = (box_width / 2.0 - center_x * scale).clamp(min_x, 0.0);
        offset_y = (box_height / 2.0 - center_y * scale).clamp(min_y, 0.0);
    }
    QuoteImage {
        pixmap: image.pixmap.clone(),
        width: (box_width.round() as u32).max(1),
        height: (box_height.round() as u32).max(1),
        scale_x: scale,
        scale_y: scale,
        offset_x,
        offset_y,
        radius,
    }
}

/// Retourne le recadrage suggéré dont le rapport largeur/hauteur est le plus
/// proche de celui de la cellule de citation.
fn closest_focus_rect(
    focus_rects: &[QuoteImageFocus],
    image_width: f32,
    image_height: f32,
    target_aspect: f32,
) -> Option<QuoteImageFocus> {
    let target = target_aspect.ln();
    focus_rects
        .iter()
        .filter(|focus| focus.width > 0.0 && focus.height > 0.0)
        .min_by(|left, right| {
            let ratio = |focus: &QuoteImageFocus| {
                (focus.width * image_width / (focus.height * image_height)).ln() - target
            };
            ratio(left)
                .abs()
                .partial_cmp(&ratio(right).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}

/// Dispose le média du tweet parent : ses photos en lignes façon X, ou sa
/// vidéo en pleine largeur.
fn layout_card_media(
    photos: &[TweetPhoto],
    video: Option<TweetVideo>,
    inner_width: f32,
) -> CardMediaLayout {
    if let Some(video) = video {
        let (width, height) = media_video_dimensions(video, inner_width);
        return CardMediaLayout {
            rows: Vec::new(),
            video: Some(MediaVideo {
                width,
                height,
                radius: MEDIA_IMAGE_RADIUS,
            }),
            height: height as f32,
        };
    }
    let rows = layout_photo_rows(photos, inner_width, MEDIA_IMAGE_GAP, MEDIA_IMAGE_RADIUS);
    let height = rows_height(&rows, MEDIA_IMAGE_GAP);
    CardMediaLayout {
        rows,
        video: None,
        height,
    }
}

/// Dimensions du cadre d'une vidéo : pleine largeur, hauteur déduite du rapport
/// borné entre 9:16 et 16:9, en pixels pairs pour l'encodage H.264.
fn media_video_dimensions(video: TweetVideo, inner_width: f32) -> (u32, u32) {
    let width = even_dimension(inner_width);
    let aspect = if video.aspect.is_finite() && video.aspect > 0.0 {
        video.aspect
    } else {
        MEDIA_VIDEO_MAX_ASPECT
    };
    let aspect = aspect.clamp(MEDIA_VIDEO_MIN_ASPECT, MEDIA_VIDEO_MAX_ASPECT);
    let height = even_dimension(width as f32 / aspect);
    (width, height)
}

/// Arrondit une dimension à la valeur paire la plus proche (au moins 2).
fn even_dimension(value: f32) -> u32 {
    let rounded = value.round().max(2.0) as u32;
    rounded - rounded % 2
}

/// Peint le média du tweet parent sous son texte.
fn draw_card_media(pixmap: &mut Pixmap, media: &CardMediaLayout, origin: (f32, f32)) {
    if let Some(video) = &media.video {
        draw_video_area(
            pixmap,
            origin.0,
            origin.1,
            video.width,
            video.height,
            video.radius,
        );
        return;
    }
    draw_photo_rows(
        pixmap,
        &media.rows,
        origin.0,
        origin.1,
        CARD_WIDTH as f32 - 2.0 * PADDING,
        MEDIA_IMAGE_GAP,
    );
}

/// Peint des lignes de photos centrées horizontalement, à la suite.
fn draw_photo_rows(
    pixmap: &mut Pixmap,
    rows: &[QuoteImageRow],
    left: f32,
    top: f32,
    inner_width: f32,
    gap: f32,
) {
    let mut top = top;
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            top += gap;
        }
        let gaps = gap * row.images.len().saturating_sub(1) as f32;
        let row_width: f32 = row
            .images
            .iter()
            .map(|image| image.width as f32)
            .sum::<f32>()
            + gaps;
        let mut cursor = left + (inner_width - row_width) / 2.0;
        for image in &row.images {
            draw_quote_image(
                pixmap,
                image,
                cursor,
                top + (row.height - image.height as f32) / 2.0,
            );
            cursor += image.width as f32 + gap;
        }
        top += row.height;
    }
}

/// Peint la zone réservée à une vidéo : un rectangle arrondi sombre, recouvert
/// ensuite par l'incrustation ffmpeg.
fn draw_video_area(pixmap: &mut Pixmap, left: f32, top: f32, width: u32, height: u32, radius: f32) {
    let Some(rect) = Rect::from_xywh(left, top, width as f32, height as f32) else {
        return;
    };
    let Some(path) = rounded_rect_path(rect, radius) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        MEDIA_VIDEO_BACKGROUND.0,
        MEDIA_VIDEO_BACKGROUND.1,
        MEDIA_VIDEO_BACKGROUND.2,
        255,
    );
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

/// Construit le masque PNG d'une zone vidéo : rectangle blanc arrondi sur fond
/// noir, consommé par `alphamerge` lors de la composition ffmpeg.
pub fn build_video_mask_png(width: u32, height: u32, radius: u32) -> MediaResult<Vec<u8>> {
    let width = width.max(2);
    let height = height.max(2);
    let mut pixmap = Pixmap::new(width, height)
        .ok_or_else(|| card_error("allocation du masque vidéo impossible"))?;
    pixmap.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
    let rect = Rect::from_xywh(0.0, 0.0, width as f32, height as f32)
        .ok_or_else(|| card_error("dimensions du masque vidéo invalides"))?;
    let path = rounded_rect_path(rect, radius as f32)
        .ok_or_else(|| card_error("tracé du masque vidéo invalide"))?;
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    pixmap
        .encode_png()
        .map_err(|error| card_error(&format!("encodage du masque vidéo impossible : {error}")))
}

/// Peint le bloc de citation : cadre arrondi, en-tête, texte puis image.
fn draw_quote(
    pixmap: &mut Pixmap,
    quote: &QuoteLayout,
    engine: &mut TextEngine,
    origin: (f32, f32),
    width: f32,
) {
    let Some(rect) = Rect::from_xywh(origin.0, origin.1, width, quote.height) else {
        return;
    };
    let Some(path) = rounded_rect_path(rect, QUOTE_CORNER_RADIUS) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        QUOTE_BACKGROUND.0,
        QUOTE_BACKGROUND.1,
        QUOTE_BACKGROUND.2,
        255,
    );
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    paint.set_color_rgba8(
        QUOTE_BORDER_COLOR.0,
        QUOTE_BORDER_COLOR.1,
        QUOTE_BORDER_COLOR.2,
        255,
    );
    let stroke = Stroke {
        width: QUOTE_BORDER_WIDTH,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);

    let content_x = origin.0 + QUOTE_BORDER_WIDTH + QUOTE_PADDING;
    let mut top = origin.1 + QUOTE_BORDER_WIDTH + QUOTE_PADDING;
    if let Some(header) = &quote.header {
        draw_text(pixmap, &header.name, engine, (content_x, top), TEXT_COLOR);
        if let Some(handle) = &header.handle {
            draw_text(
                pixmap,
                handle,
                engine,
                (content_x + header.name_width + QUOTE_HANDLE_GAP, top),
                SECONDARY_COLOR,
            );
        }
        top += header.height;
    }
    if !quote.text.paragraphs.is_empty() {
        if quote.header.is_some() {
            top += QUOTE_INNER_GAP;
        }
        draw_body(pixmap, &quote.text, engine, (content_x, top), TEXT_COLOR);
        top += quote.text.height;
    }
    if let Some(video) = &quote.video {
        draw_video_area(
            pixmap,
            content_x,
            origin.1 + video.top,
            video.width,
            video.height,
            QUOTE_IMAGE_RADIUS,
        );
    }
    if !quote.rows.is_empty() {
        if quote.header.is_some() || !quote.text.paragraphs.is_empty() {
            top += QUOTE_INNER_GAP;
        }
        draw_photo_rows(
            pixmap,
            &quote.rows,
            content_x,
            top,
            quote.inner_width,
            QUOTE_INNER_GAP,
        );
    }
}

/// Peint l'image citée avec des coins arrondis, centrée par l'appelant.
fn draw_quote_image(pixmap: &mut Pixmap, image: &QuoteImage, left: f32, top: f32) {
    if image.width == 0 || image.height == 0 {
        return;
    }
    let Some(mut layer) = Pixmap::new(image.width, image.height) else {
        return;
    };
    let paint = PixmapPaint {
        quality: tiny_skia::FilterQuality::Bicubic,
        ..PixmapPaint::default()
    };
    let scale_x = image.scale_x;
    let scale_y = image.scale_y;
    layer.draw_pixmap(
        0,
        0,
        image.pixmap.as_ref(),
        &paint,
        Transform::from_row(scale_x, 0.0, 0.0, scale_y, image.offset_x, image.offset_y),
        None,
    );
    if let Some(mut mask) = Mask::new(image.width, image.height) {
        if let Some(rect) = Rect::from_xywh(0.0, 0.0, image.width as f32, image.height as f32) {
            if let Some(path) = rounded_rect_path(rect, image.radius) {
                mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
                layer.apply_mask(&mask);
            }
        }
    }
    pixmap.draw_pixmap(
        left.round() as i32,
        top.round() as i32,
        layer.as_ref(),
        &paint,
        Transform::identity(),
        None,
    );
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
fn draw_avatar(pixmap: &mut Pixmap, content: &TweetCardContent, engine: &mut TextEngine) {
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
    let layout = build_layout(
        engine,
        &initial,
        TextStyleSpec {
            size: INITIAL_SIZE,
            line_height: INITIAL_LINE_HEIGHT,
            bold: true,
            // Comme le nom : une initiale arabe est un paragraphe RTL, à
            // forcer à gauche avant de centrer ses glyphes.
            align: Alignment::Left,
        },
        Some(AVATAR_SIZE),
    );
    let origin = ink_bounds(&layout, engine)
        .map(|(left, top, right, bottom)| {
            (center - (left + right) / 2.0, center - (top + bottom) / 2.0)
        })
        .unwrap_or((
            center - AVATAR_SIZE / 2.0,
            center - INITIAL_LINE_HEIGHT / 2.0,
        ));
    draw_text(pixmap, &layout, engine, origin, (0xff, 0xff, 0xff));
}

/// Décode un avatar PNG ; une image invalide (JPEG brut, données tronquées)
/// est ignorée au profit du cercle d'initiale.
fn decode_avatar(bytes: &[u8]) -> Option<Pixmap> {
    Pixmap::decode_png(bytes).ok()
}

/// Boîte englobante de l'encre des glyphes d'une disposition.
///
/// Les positions des glyphes et le placement des images swash donnent les
/// vrais bords de l'encre : l'avance logique seule ne dit pas où tombe une
/// initiale arabe (ou un emoji) dans le cercle.
fn ink_bounds(layout: &CardLayout, engine: &mut TextEngine) -> Option<(f32, f32, f32, f32)> {
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    rasterize_layout(layout, engine, (0.0, 0.0), |image, left, top| {
        if image.placement.width == 0 || image.placement.height == 0 {
            return;
        }
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
    });
    bounds
}

/// Peint une disposition parley dans le calque.
fn draw_text(
    pixmap: &mut Pixmap,
    layout: &CardLayout,
    engine: &mut TextEngine,
    origin: (f32, f32),
    base: (u8, u8, u8),
) {
    rasterize_layout(layout, engine, origin, |image, left, top| {
        draw_glyph_image(pixmap, image, left as i32, top as i32, base);
    });
}

/// Rastérise chaque glyphe d'une disposition et transmet son image placée.
///
/// Un scaler swash est construit par run (police, taille, coordonnées de
/// variation) puis réutilisé pour tous ses glyphes. `Render::offset` applique
/// la position fractionnaire, comme le faisait cosmic-text.
fn rasterize_layout(
    layout: &CardLayout,
    engine: &mut TextEngine,
    origin: (f32, f32),
    mut draw: impl FnMut(&SwashImage, f32, f32),
) {
    let sources = [
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::Outline,
    ];
    let mut render = Render::new(&sources);
    render.format(Format::Alpha);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let run = glyph_run.run();
            let font = run.font();
            let Ok(font_index) = usize::try_from(font.index) else {
                continue;
            };
            let font_data = font.data.as_ref();
            let Some(font_ref) = FontRef::from_index(font_data, font_index) else {
                continue;
            };
            let coords: Vec<i16> = run
                .normalized_coords()
                .iter()
                .map(|coord| coord.to_bits())
                .collect();
            let mut scaler = engine
                .scale_cx
                .builder(font_ref)
                .size(run.font_size())
                .hint(true)
                .normalized_coords(coords.iter().copied())
                .build();
            for glyph in glyph_run.positioned_glyphs() {
                let Ok(glyph_id) = u16::try_from(glyph.id) else {
                    continue;
                };
                let x = origin.0 + glyph.x;
                let y = origin.1 + glyph.y;
                let base_x = x.floor();
                let base_y = y.floor();
                render.offset(Vector::new(x - base_x, y - base_y));
                if let Some(image) = render.render(&mut scaler, glyph_id) {
                    let left = base_x + image.placement.left as f32;
                    let top = base_y - image.placement.top as f32;
                    draw(&image, left, top);
                }
            }
        }
    }
}

/// Compose une image de glyphe (masque teinté ou RGBA) dans le calque.
fn draw_glyph_image(
    pixmap: &mut Pixmap,
    image: &SwashImage,
    left: i32,
    top: i32,
    base: (u8, u8, u8),
) {
    let width = image.placement.width as i32;
    let height = image.placement.height as i32;
    match image.content {
        SwashContent::Mask => {
            for row in 0..height {
                for column in 0..width {
                    let coverage = image.data[(row * width + column) as usize];
                    blend_pixel(
                        pixmap,
                        left + column,
                        top + row,
                        base.0,
                        base.1,
                        base.2,
                        coverage,
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
    const QUOTE_FIXTURE: &str = include_str!("../tests/fixtures/twitter_syndication_quote.json");
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
            (CARD_MIN_HEIGHT..=CARD_MEDIA_MAX_HEIGHT).contains(&height),
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
            photos: Vec::new(),
            video: None,
            quoted: None,
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
            photos: Vec::new(),
            video: None,
            quoted: None,
        };
        let png = render_tweet_card(&content).unwrap();
        let card = Pixmap::decode_png(&png).unwrap();

        // (a) Le nom arabe doit commencer juste après l'avatar : des pixels
        // de texte existent dans la bande de la ligne du nom, avant x = 600.
        // Sans alignement forcé, le nom RTL se collerait au bord droit.
        let header_height = AVATAR_SIZE.max(NAME_LINE_HEIGHT + SECONDARY_LINE_HEIGHT);
        let name_top = PADDING + (header_height - NAME_LINE_HEIGHT - SECONDARY_LINE_HEIGHT) / 2.0;
        let name_left = (PADDING + AVATAR_SIZE + HEADER_TEXT_GAP - 4.0) as u32;
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
            photos: Vec::new(),
            video: None,
            quoted: None,
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
            photos: Vec::new(),
            video: None,
            quoted: None,
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

    /// Photo citée prête à rendre, à partir d'un PNG de test.
    fn quoted_image(png: Vec<u8>) -> TweetPhoto {
        TweetPhoto {
            source_url: "https://pbs.twimg.com/media/test.jpg".to_string(),
            data: Some(png),
            focus: Vec::new(),
        }
    }

    /// Petit PNG uni bleu, pour vérifier le rendu d'une image citée.
    fn solid_png(width: u32, height: u32) -> Vec<u8> {
        solid_color_png(width, height, (0x40, 0x80, 0xc0))
    }

    /// Petit PNG d'une couleur unie.
    fn solid_color_png(width: u32, height: u32, color: (u8, u8, u8)) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width, height).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(
            color.0, color.1, color.2, 0xff,
        ));
        pixmap.encode_png().unwrap()
    }

    /// PNG de `width`×`height` : moitié haute rouge, moitié basse verte.
    fn split_png(width: u32, height: u32) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width, height).unwrap();
        let mut paint = Paint {
            anti_alias: false,
            ..Paint::default()
        };
        paint.set_color_rgba8(0xd0, 0x30, 0x30, 0xff);
        let top = Rect::from_xywh(0.0, 0.0, width as f32, (height / 2) as f32).unwrap();
        pixmap.fill_rect(top, &paint, Transform::identity(), None);
        paint.set_color_rgba8(0x30, 0xc0, 0x50, 0xff);
        let bottom = Rect::from_xywh(
            0.0,
            (height / 2) as f32,
            width as f32,
            (height - height / 2) as f32,
        )
        .unwrap();
        pixmap.fill_rect(bottom, &paint, Transform::identity(), None);
        pixmap.encode_png().unwrap()
    }

    /// Compte les pixels franchement rouges et verts d'une carte rendue.
    fn tone_counts(png: &[u8]) -> (usize, usize) {
        let pixmap = Pixmap::decode_png(png).unwrap();
        let mut red = 0;
        let mut green = 0;
        for pixel in pixmap.pixels() {
            let (r, g) = (pixel.red(), pixel.green());
            if r > g.saturating_add(40) {
                red += 1;
            } else if g > r.saturating_add(40) {
                green += 1;
            }
        }
        (red, green)
    }

    #[test]
    fn renders_a_quoted_tweet_block_from_the_fixture() {
        let content = content_from(QUOTE_FIXTURE);
        let quoted = content.quoted.as_ref().unwrap();
        assert_eq!(quoted.author_name, "berkani");
        assert_eq!(quoted.author_handle, "inconnu95267");
        assert_eq!(quoted.text, "Allez wAllah");
        // La fixture porte deux photos, pas encore téléchargées.
        assert_eq!(quoted.images.len(), 2);
        assert!(quoted.images.iter().all(|image| image.data.is_none()));
        let with_quote = render_tweet_card(&content).unwrap();
        assert_valid_card(&with_quote);

        // Le bloc de citation ajoute de la hauteur sous le texte du parent.
        let mut bare = content.clone();
        bare.quoted = None;
        let without_quote = render_tweet_card(&bare).unwrap();
        let (_, with_height) = png_dimensions(&with_quote);
        let (_, without_height) = png_dimensions(&without_quote);
        assert!(
            with_height > without_height,
            "citation non rendue : {with_height} px contre {without_height} px sans citation"
        );
    }

    #[test]
    fn renders_a_quoted_tweet_with_an_image() {
        let mut content = content_from(QUOTE_FIXTURE);
        content.quoted.as_mut().unwrap().images = vec![quoted_image(solid_png(320, 200))];
        let png = render_tweet_card(&content).unwrap();
        let (_, with_image) = assert_valid_card(&png);

        let mut text_only = content.clone();
        text_only.quoted.as_mut().unwrap().images.clear();
        let (_, without_image) = assert_valid_card(&render_tweet_card(&text_only).unwrap());
        assert!(
            with_image > without_image,
            "image citée non rendue : {with_image} px contre {without_image} px sans image"
        );
    }

    #[test]
    fn renders_two_quoted_photos_side_by_side() {
        let mut content = content_from(QUOTE_FIXTURE);
        // Avatar neutre : ni rouge ni vert, pour ne pas fausser la détection
        // des deux photos dans les pixels.
        content.avatar = Some(solid_color_png(96, 96, (0x50, 0x50, 0x50)));
        {
            let quoted = content.quoted.as_mut().unwrap();
            quoted.text = String::new();
            quoted.images = vec![
                quoted_image(solid_color_png(400, 400, (0xd0, 0x30, 0x30))),
                quoted_image(solid_color_png(400, 400, (0x30, 0xc0, 0x50))),
            ];
        }
        let png = render_tweet_card(&content).unwrap();
        assert_valid_card(&png);
        let pixmap = Pixmap::decode_png(&png).unwrap();
        let (mut red_max_x, mut green_min_x, mut green_max_x) = (0_u32, u32::MAX, 0_u32);
        for (index, pixel) in pixmap.pixels().iter().enumerate() {
            let x = index as u32 % pixmap.width();
            let (r, g) = (pixel.red(), pixel.green());
            if r > g.saturating_add(40) {
                red_max_x = red_max_x.max(x);
            } else if g > r.saturating_add(40) {
                green_min_x = green_min_x.min(x);
                green_max_x = green_max_x.max(x);
            }
        }
        assert!(
            red_max_x > 0 && green_max_x > 0,
            "les deux photos ne sont pas rendues (rouge jusqu'à {red_max_x}, vert jusqu'à {green_max_x})"
        );
        assert!(
            red_max_x < green_min_x,
            "les photos ne sont pas côte à côte : rouge jusqu'à {red_max_x}, vert à partir de {green_min_x}"
        );
    }

    #[test]
    fn ignores_an_invalid_quoted_image() {
        let mut content = content_from(QUOTE_FIXTURE);
        content.quoted.as_mut().unwrap().images = vec![TweetPhoto {
            source_url: "https://pbs.twimg.com/media/invalide.jpg".to_string(),
            data: Some(b"pas un PNG".to_vec()),
            focus: Vec::new(),
        }];
        let png = render_tweet_card(&content).unwrap();
        assert_valid_card(&png);
    }

    #[test]
    fn renders_a_quoted_tweet_without_author_or_text() {
        let content = TweetCardContent {
            author_name: "jack".to_string(),
            author_handle: "jack".to_string(),
            text: "un tweet qui cite une image".to_string(),
            created_at: None,
            avatar: None,
            photos: Vec::new(),
            video: None,
            quoted: Some(QuotedTweetContent {
                author_name: String::new(),
                author_handle: String::new(),
                text: String::new(),
                images: vec![quoted_image(solid_png(200, 120))],
                video: None,
            }),
        };
        let png = render_tweet_card(&content).unwrap();
        assert_valid_card(&png);
    }

    #[test]
    fn crops_the_quoted_image_on_the_suggested_focus() {
        let mut content = content_from(QUOTE_FIXTURE);
        {
            let quoted = content.quoted.as_mut().unwrap();
            quoted.images = vec![TweetPhoto {
                source_url: "https://pbs.twimg.com/media/focus.jpg".to_string(),
                data: Some(split_png(400, 400)),
                // X suggère de cadrer le bas de la photo : le recadrage doit
                // suivre cette suggestion plutôt que le centre de l'image.
                focus: vec![QuoteImageFocus {
                    x: 0.0,
                    y: 0.5,
                    width: 1.0,
                    height: 0.5,
                }],
            }];
        }
        let png = render_tweet_card(&content).unwrap();
        assert_valid_card(&png);
        let (red, green) = tone_counts(&png);
        assert!(
            green > red * 3,
            "recadrage non aligné sur le focus : {green} pixels verts contre {red} rouges"
        );
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

    #[test]
    fn renders_the_tweet_photo_under_the_text() {
        let mut content = content_from(PHOTO_FIXTURE);
        content.photos = vec![TweetPhoto {
            source_url: "https://pbs.twimg.com/media/test.jpg".to_string(),
            data: Some(solid_png(400, 300)),
            focus: Vec::new(),
        }];
        let png = render_tweet_card(&content).unwrap();
        let (_, with_photo) = assert_valid_card(&png);

        let mut text_only = content;
        text_only.photos.clear();
        let (_, without_photo) = png_dimensions(&render_tweet_card(&text_only).unwrap());
        assert!(
            with_photo > without_photo + 200,
            "photo du tweet non rendue : {with_photo} px contre {without_photo} px sans photo"
        );
        // La photo occupe la largeur de la carte (moins les marges).
        let pixmap = Pixmap::decode_png(&png).unwrap();
        let center = pixmap
            .pixel(CARD_WIDTH / 2, with_photo - PADDING as u32 - 120)
            .unwrap();
        assert!(
            center.blue() > 120 && center.green() > 80,
            "zone photo vide au centre de la carte"
        );
    }

    #[test]
    fn places_a_photo_one_ulp_wider_than_its_cell_without_panicking() {
        // Régression : le filtre ffmpeg `scale=1800:1800:force_original_aspect_
        // ratio=decrease` agrandit les petites images, donc une photo 1234×1326
        // arrive à 1675×1800. Dans une cellule de 984 × 553,5 px,
        // `1675,0 * scale` vaut 983,99994 : la borne basse du recadrage valait
        // +6,1e-5 au lieu d'être négative et `f32::clamp(min, 0.0)` paniquait
        // (« min > max, or either was NaN »), faisant échouer toute l'ingestion.
        let mut content = content_from(PHOTO_FIXTURE);
        content.photos = vec![TweetPhoto {
            source_url: "https://pbs.twimg.com/media/une-pixel-de-trop.jpg".to_string(),
            data: Some(solid_png(1675, 1800)),
            focus: vec![QuoteImageFocus {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            }],
        }];
        let png = render_tweet_card(&content).unwrap();
        assert_valid_card(&png);
    }

    #[test]
    fn reserves_a_vertical_video_area_for_a_tweet_video() {
        let mut content = content_from(TEXT_FIXTURE);
        content.video = Some(TweetVideo { aspect: 9.0 / 16.0 });
        let card = render_tweet_card_with_slots(&content).unwrap();
        assert_eq!(card.video_slots.len(), 1);
        let slot = &card.video_slots[0];
        assert_eq!(slot.target, CardVideoTarget::Parent);
        assert_eq!(slot.x, PADDING as u32);
        assert_eq!(slot.width % 2, 0, "largeur impaire refusée par H.264");
        assert_eq!(slot.height % 2, 0, "hauteur impaire refusée par H.264");
        // La zone vidéo garde le rapport 9:16 sans recadrage.
        let expected = slot.width as f32 / (9.0 / 16.0);
        assert!(
            (slot.height as f32 - expected).abs() <= 2.0,
            "zone vidéo {}×{} au lieu de {} px de haut",
            slot.width,
            slot.height,
            expected
        );
        let (_, height) = assert_valid_card(&card.png);
        assert!(
            height > CARD_MAX_HEIGHT,
            "carte vidéo trop courte : {height}"
        );
        assert!(height <= CARD_MEDIA_MAX_HEIGHT);
    }

    #[test]
    fn places_the_quoted_video_inside_the_quote_block() {
        let mut content = content_from(QUOTE_FIXTURE);
        {
            let quoted = content.quoted.as_mut().unwrap();
            quoted.images.clear();
            quoted.video = Some(TweetVideo { aspect: 16.0 / 9.0 });
        }
        let card = render_tweet_card_with_slots(&content).unwrap();
        assert_eq!(card.video_slots.len(), 1);
        let slot = &card.video_slots[0];
        assert_eq!(slot.target, CardVideoTarget::Quoted);
        // Largeur utile du bloc de citation : carte moins marges et cadre.
        let inner = CARD_WIDTH as f32 - 2.0 * PADDING - 2.0 * (QUOTE_BORDER_WIDTH + QUOTE_PADDING);
        assert!(
            (slot.width as f32 - inner).abs() <= 2.0,
            "zone vidéo citée de {} px au lieu de {inner}",
            slot.width
        );
        assert!(slot.x > PADDING as u32, "zone vidéo citée hors du bloc");
        assert!(slot.y > AVATAR_SIZE as u32, "zone vidéo citée trop haute");
        let (_, with_video) = assert_valid_card(&card.png);
        let mut text_only = content.clone();
        text_only.quoted.as_mut().unwrap().video = None;
        let (_, without_video) = png_dimensions(&render_tweet_card(&text_only).unwrap());
        assert!(
            with_video > without_video,
            "vidéo citée non rendue : {with_video} px contre {without_video} px sans vidéo"
        );
    }

    #[test]
    fn builds_a_rounded_video_mask() {
        let png = build_video_mask_png(200, 100, 20).unwrap();
        let mask = Pixmap::decode_png(&png).unwrap();
        assert_eq!((mask.width(), mask.height()), (200, 100));
        // Coin hors du rayon : noir (zone masquée), centre : blanc (visible).
        let corner = mask.pixel(0, 0).unwrap();
        assert!(corner.red() < 10, "coin du masque non noir");
        assert_eq!(corner.alpha(), 255);
        let center = mask.pixel(100, 50).unwrap();
        assert!(center.red() > 240, "centre du masque non blanc");
    }
}
