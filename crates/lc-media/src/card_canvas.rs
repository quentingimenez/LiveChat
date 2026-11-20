//! Primitives de rendu communes aux cartes PNG (`tweet_card`, `tiktok_comment`).
//!
//! parley met en forme le texte (découverte des polices, césure, bidi), swash
//! rastérise les glyphes (emoji couleur compris) et tiny-skia ne sert qu'aux
//! fonds arrondis, aux masques et à l'encodage PNG. Ces primitives ne
//! connaissent aucun modèle de publication : les cartes leur passent le texte,
//! les styles et les images déjà décodées.

use std::sync::{Mutex, OnceLock};

use parley::{
    Alignment, AlignmentOptions, FontContext, FontWeight, Layout, LayoutContext, LineHeight,
    PositionedLayoutItem, StyleProperty,
};
use swash::scale::image::{Content as SwashContent, Image as SwashImage};
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Vector};
use swash::FontRef;
use tiny_skia::{FillRule, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Transform};

/// Couleurs des cercles d'initiale quand aucun avatar n'est disponible.
const INITIAL_COLORS: [(u8, u8, u8); 6] = [
    (0x3b, 0x82, 0xf6),
    (0x8b, 0x5c, 0xf6),
    (0xec, 0x48, 0x99),
    (0xf5, 0x9e, 0x0b),
    (0x10, 0xb9, 0x81),
    (0x06, 0xb6, 0xd4),
];

/// Contexte de mise en forme et de rastérisation partagé.
///
/// `FontContext` découvre les polices système et `LayoutContext` réutilise ses
/// allocations ; les construire à chaque carte coûterait un scan complet des
/// polices. `ScaleContext` garde les caches de swash.
pub(crate) struct TextEngine {
    font_cx: FontContext,
    layout_cx: LayoutContext<[u8; 4]>,
    scale_cx: ScaleContext,
}

static TEXT_ENGINE: OnceLock<Mutex<TextEngine>> = OnceLock::new();

/// Exécute `render` avec le moteur de texte global, verrouillé le temps du rendu.
pub(crate) fn with_text_engine<T>(render: impl FnOnce(&mut TextEngine) -> T) -> T {
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
    render(&mut engine)
}

/// Type de disposition parley d'une carte (pinceau par défaut `[u8; 4]`).
pub(crate) type CardLayout = Layout<[u8; 4]>;

/// Style d'un bloc de texte de carte.
#[derive(Clone, Copy)]
pub(crate) struct TextStyleSpec {
    /// Taille de police en pixels.
    pub size: f32,
    /// Interligne absolu en pixels.
    pub line_height: f32,
    /// Graisse forte (700).
    pub bold: bool,
    /// Alignement du paragraphe.
    pub align: Alignment,
}

/// Normalise les fins de ligne du texte.
pub(crate) fn normalized_text(raw: &str) -> String {
    raw.trim().replace("\r\n", "\n").replace('\r', "\n")
}

/// Construit et casse un paragraphe avec le style demandé.
pub(crate) fn build_layout(
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
pub(crate) fn single_line_layout(
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

/// Corps d'une carte : une disposition parley par paragraphe.
///
/// Parley 0.13 ne porte qu'une direction de base par disposition. cosmic-text
/// analysait la direction de chaque ligne : un texte arabe suivi d'une ligne
/// française doit donc être découpé aux retours à la ligne pour que chaque
/// paragraphe garde sa direction et son alignement naturels.
pub(crate) struct BodyLayout {
    /// Paragraphes mis en forme, dans l'ordre de lecture.
    pub paragraphs: Vec<CardLayout>,
    /// Hauteur totale des paragraphes.
    pub height: f32,
    /// `true` quand le texte a été tronqué : la carte garde alors sa hauteur
    /// maximale au lieu de se réduire à la hauteur du texte tronqué.
    pub clamped: bool,
}

/// Met en forme le corps, tronqué à la hauteur maximale si besoin.
pub(crate) fn layout_body(
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
pub(crate) fn clamp_paragraph(
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
pub(crate) fn draw_body(
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

/// Remplit un rectangle à coins arrondis.
pub(crate) fn draw_rounded_rect(
    pixmap: &mut Pixmap,
    rect: Rect,
    radius: f32,
    color: (u8, u8, u8),
) -> Option<()> {
    let path = rounded_rect_path(rect, radius)?;
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.0, color.1, color.2, 255);
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    Some(())
}

/// Tracé d'un rectangle à coins arrondis (approximation cubique des arcs).
pub(crate) fn rounded_rect_path(rect: Rect, radius: f32) -> Option<Path> {
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
///
/// `rect` est le carré de l'avatar ; l'initiale et son interligne sont dérivés
/// de sa taille pour que la même primitive serve les différentes cartes.
pub(crate) fn draw_avatar(
    pixmap: &mut Pixmap,
    rect: Rect,
    avatar: Option<&[u8]>,
    name: &str,
    handle: &str,
    engine: &mut TextEngine,
) {
    let size = rect.width();
    let center_x = rect.left() + size / 2.0;
    let center_y = rect.top() + size / 2.0;
    let Some(circle) = PathBuilder::from_circle(center_x, center_y, size / 2.0) else {
        return;
    };
    if let Some(avatar) = avatar.and_then(decode_png) {
        let Some(mut layer) = Pixmap::new(size as u32, size as u32) else {
            return;
        };
        let paint = PixmapPaint {
            quality: tiny_skia::FilterQuality::Bicubic,
            ..PixmapPaint::default()
        };
        let scale_x = size / avatar.width() as f32;
        let scale_y = size / avatar.height() as f32;
        let scale = scale_x.max(scale_y);
        let offset_x = (size - avatar.width() as f32 * scale) / 2.0;
        let offset_y = (size - avatar.height() as f32 * scale) / 2.0;
        layer.draw_pixmap(
            0,
            0,
            avatar.as_ref(),
            &paint,
            Transform::from_row(scale, 0.0, 0.0, scale, offset_x, offset_y),
            None,
        );
        if let Some(mut mask) = Mask::new(size as u32, size as u32) {
            if let Some(local_circle) = PathBuilder::from_circle(size / 2.0, size / 2.0, size / 2.0)
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
            rect.left() as i32,
            rect.top() as i32,
            layer.as_ref(),
            &paint,
            Transform::identity(),
            None,
        );
        return;
    }
    let hashed = handle
        .bytes()
        .chain(name.bytes())
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
    let initial = name
        .chars()
        .next()
        .map(|letter| letter.to_uppercase().collect::<String>())
        .unwrap_or_default();
    if initial.is_empty() {
        return;
    }
    let initial_size = size * 0.5;
    let initial_line_height = size * 0.583_333_3;
    let layout = build_layout(
        engine,
        &initial,
        TextStyleSpec {
            size: initial_size,
            line_height: initial_line_height,
            bold: true,
            // Comme le nom : une initiale arabe est un paragraphe RTL, à
            // forcer à gauche avant de centrer ses glyphes.
            align: Alignment::Left,
        },
        Some(size),
    );
    let origin = ink_bounds(&layout, engine)
        .map(|(left, top, right, bottom)| {
            (
                center_x - (left + right) / 2.0,
                center_y - (top + bottom) / 2.0,
            )
        })
        .unwrap_or((center_x - size / 2.0, center_y - initial_line_height / 2.0));
    draw_text(pixmap, &layout, engine, origin, (0xff, 0xff, 0xff));
}

/// Peint une image en couverture (recadrée au centre) dans un rectangle arrondi.
///
/// L'image source peut avoir n'importe quel format tant qu'elle est en PNG :
/// l'échelle est calculée pour remplir `rect`, le surplus est rogné.
pub(crate) fn draw_image_cover(
    pixmap: &mut Pixmap,
    png: &[u8],
    rect: Rect,
    radius: f32,
) -> Option<()> {
    let image = decode_png(png)?;
    let width = rect.width().round().max(1.0) as u32;
    let height = rect.height().round().max(1.0) as u32;
    let mut layer = Pixmap::new(width, height)?;
    let paint = PixmapPaint {
        quality: tiny_skia::FilterQuality::Bicubic,
        ..PixmapPaint::default()
    };
    let scale = (width as f32 / image.width() as f32).max(height as f32 / image.height() as f32);
    let offset_x = (width as f32 - image.width() as f32 * scale) / 2.0;
    let offset_y = (height as f32 - image.height() as f32 * scale) / 2.0;
    layer.draw_pixmap(
        0,
        0,
        image.as_ref(),
        &paint,
        Transform::from_row(scale, 0.0, 0.0, scale, offset_x, offset_y),
        None,
    );
    if radius > 0.0 {
        if let Some(mut mask) = Mask::new(width, height) {
            if let Some(local) = rounded_rect_path(
                Rect::from_xywh(0.0, 0.0, width as f32, height as f32)?,
                radius,
            ) {
                mask.fill_path(&local, FillRule::Winding, true, Transform::identity());
                layer.apply_mask(&mask);
            }
        }
    }
    pixmap.draw_pixmap(
        rect.left() as i32,
        rect.top() as i32,
        layer.as_ref(),
        &paint,
        Transform::identity(),
        None,
    );
    Some(())
}

/// Décode un PNG ; une image invalide est ignorée au profit du repli de l'appelant.
pub(crate) fn decode_png(bytes: &[u8]) -> Option<Pixmap> {
    Pixmap::decode_png(bytes).ok()
}

/// Boîte englobante de l'encre des glyphes d'une disposition.
///
/// Les positions des glyphes et le placement des images swash donnent les
/// vrais bords de l'encre : l'avance logique seule ne dit pas où tombe une
/// initiale arabe (ou un emoji) dans le cercle.
pub(crate) fn ink_bounds(
    layout: &CardLayout,
    engine: &mut TextEngine,
) -> Option<(f32, f32, f32, f32)> {
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
pub(crate) fn draw_text(
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
pub(crate) fn rasterize_layout(
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
