// Drawing tools of the video editor: the same objects as the picture editor
// (strokes, shapes, texts, emojis, masks) plus the colour adjustments, laid
// out in the pixels of the source frame.
//
// The server burns the masks, then the drawing, then the colour adjustments,
// before the crop and the rotation (`crates/lc-media/src/edit.rs`). The stage
// follows that order: masks and drawing sit inside `.av-inner`, so the CSS
// crop, rotation and flips carry them exactly like the video, and the colour
// filter applies to that whole clip, retouches included, like the server.

import { h, icon } from '../../core/dom.js';
import {
  EMOJIS,
  FONT_LABELS,
  annotationStyle,
  annotationsToDataUrl,
  boundsOf,
  colorRow,
  ensureSize,
  makeCanvas,
  paintAnnotationsOn,
  paintObject,
  scaled,
  translated,
} from './annotations.js';
import {
  FILTER_PRESETS,
  NEUTRAL_ADJUST,
  clamp,
  cssFilter,
} from '../../core/editor-helpers.js';
import { choices, section, slider, toggle } from './frame.js';

export const ANNOTATION_TOOLS = [
  ['select', 'Sélection', 'cursor'],
  ['draw', 'Dessiner', 'edit'],
  ['shape', 'Formes', 'shapes'],
  ['text', 'Texte', 'text'],
  ['emoji', 'Emoji', 'smile'],
  ['adjust', 'Réglages', 'sliders'],
  ['blur', 'Masquer une zone', 'pixels'],
];

// Sélection stands alone; the drawing tools share a group; masking closes it.
const TOOL_GROUPS = [['select'], ['draw', 'shape', 'text', 'emoji', 'adjust'], ['blur']];

const TOOL_HINTS = {
  select: 'Sélectionne un élément pour le déplacer, le redimensionner ou le modifier.',
  draw: 'Dessine directement sur la vidéo.',
  shape: 'Trace la forme en glissant sur la vidéo.',
  text: 'Clique sur la vidéo pour y poser un texte.',
  emoji: 'Choisis un emoji\u00A0: il se pose au centre, déplace-le ensuite.',
  adjust: 'Les réglages s’appliquent à toute la vidéo.',
  blur: 'Trace un rectangle sur la zone à cacher.',
};

// Builds the drawing layer over the video. `bounds()` gives the source frame
// size, `source()` the video element, `view()` the mapping from the stage to
// the frame (`x`, `y` of its top-left corner in source pixels and `scale`, the
// CSS pixels per source pixel).
export function createVideoAnnotations({ frame, host, bounds, source, view, objects, adjust, onCommit, onPanel }) {
  const style = annotationStyle();
  const styleDefaults = { ...style };
  let tool = 'select';
  let selectedId = null;
  let drag = null;
  let nextId = 1;
  let overlayCache = null;
  let paintQueued = false;

  const tiny = makeCanvas(1, 1);
  const drawn = h('canvas', { class: 'annot-drawing' });
  const masks = h('div', { class: 'annot-masks' });
  const box = h('div', { class: 'annot-box' }, masks, drawn);
  const selBox = h('div', { class: 'sel-box', hidden: true }, h('span', { class: 'sel-handle' }));
  const surface = h('div', { class: 'annot-surface' });
  host.append(box, surface, selBox);
  box.append(selBox);

  const list = () => objects() || [];
  const colorState = () => adjust() || { ...NEUTRAL_ADJUST };
  const selected = () => list().find((object) => object.id === selectedId) || null;
  // One length unit follows the frame, like the picture editor: a stroke looks
  // the same on a small clip and on a 4K one.
  const unit = () => Math.max(1, bounds().width, bounds().height) / 400;

  function size() {
    const { width, height } = bounds();
    return { width: Math.max(2, Math.round(width)), height: Math.max(2, Math.round(height)) };
  }

  // --- painting -----------------------------------------------------------
  function paint() {
    if (paintQueued) {
      return;
    }
    paintQueued = true;
    requestAnimationFrame(() => {
      paintQueued = false;
      paintNow();
    });
  }

  function paintNow() {
    const { width, height } = size();
    ensureSize(drawn, width, height);
    const context = drawn.getContext('2d');
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, width, height);
    for (const object of list()) {
      if (object.type !== 'mask') {
        paintObject(context, object);
      }
    }
    placeSelection();
  }

  // Pixel masks follow the video: they are redrawn for every frame it shows.
  // Blur masks are plain DOM: `backdrop-filter` blurs the video behind them.
  function paintMasks() {
    const video = source();
    if (!video || video.readyState < 2) {
      return;
    }
    for (const view of maskViews) {
      if (view.object.mode === 'blur') {
        continue;
      }
      const { x, y, w, h } = view.rect;
      const block = Math.max(2, view.object.strength);
      const columns = Math.max(1, Math.round(w / block));
      const rows = Math.max(1, Math.round(h / block));
      ensureSize(tiny, columns, rows);
      const small = tiny.getContext('2d');
      small.clearRect(0, 0, columns, rows);
      small.drawImage(video, x, y, w, h, 0, 0, columns, rows);
      const context = view.context;
      context.clearRect(0, 0, view.canvas.width, view.canvas.height);
      context.imageSmoothingEnabled = false;
      context.drawImage(tiny, 0, 0, columns, rows, 0, 0, view.canvas.width, view.canvas.height);
    }
  }

  // Draws the masks and the drawing into `context`, which must map one unit to
  // one source pixel with the frame's top-left corner at the origin. The
  // preview of the display size reuses it, so both surfaces show the same
  // retouches in the order the server burns them.
  function paintInto(context) {
    const video = source();
    const { width, height } = size();
    if (!video || video.readyState < 2) {
      return;
    }
    context.save();
    for (const object of list()) {
      if (object.type !== 'mask') {
        continue;
      }
      const rect = maskRect(object);
      if (rect.w < 1 || rect.h < 1) {
        continue;
      }
      if (object.mode === 'blur') {
        context.save();
        context.beginPath();
        context.rect(rect.x, rect.y, rect.w, rect.h);
        context.clip();
        context.filter = `blur(${Math.max(1, object.strength).toFixed(1)}px)`;
        context.drawImage(video, 0, 0, width, height);
        context.restore();
        continue;
      }
      const block = Math.max(2, object.strength);
      const columns = Math.max(1, Math.round(rect.w / block));
      const rows = Math.max(1, Math.round(rect.h / block));
      ensureSize(tiny, columns, rows);
      const small = tiny.getContext('2d');
      small.clearRect(0, 0, columns, rows);
      small.drawImage(video, rect.x, rect.y, rect.w, rect.h, 0, 0, columns, rows);
      context.imageSmoothingEnabled = false;
      context.drawImage(tiny, 0, 0, columns, rows, rect.x, rect.y, rect.w, rect.h);
      context.imageSmoothingEnabled = true;
    }
    context.restore();
    paintAnnotationsOn(context, list());
  }


  // --- layout -------------------------------------------------------------
  // `view()` describes where the frame is on the stage: the masks and the
  // drawing are laid over the whole frame, and `.av-inner` clips them to the
  // crop, like the video.
  let maskViews = [];

  function layout() {
    const { width, height } = size();
    const mapping = view();
    const cssWidth = width * mapping.scale;
    const cssHeight = height * mapping.scale;
    // The layer covers the whole frame: `.av-inner` clips it to the crop, like
    // the video, so an annotation laid outside the crop is not shown.
    box.style.left = `${-mapping.x * mapping.scale}px`;
    box.style.top = `${-mapping.y * mapping.scale}px`;
    box.style.width = `${cssWidth}px`;
    box.style.height = `${cssHeight}px`;
    drawn.style.width = `${cssWidth}px`;
    drawn.style.height = `${cssHeight}px`;
    syncMasks(mapping);
    placeSelection();
  }

  function maskRect(object) {
    const { width, height } = size();
    const x = clamp(Math.min(object.x1, object.x2), 0, width);
    const y = clamp(Math.min(object.y1, object.y2), 0, height);
    const right = clamp(Math.max(object.x1, object.x2), 0, width);
    const bottom = clamp(Math.max(object.y1, object.y2), 0, height);
    return { x, y, w: right - x, h: bottom - y };
  }

  // Rebuilds the mask elements: a blurred rectangle, or a canvas repainted from
  // the video for every frame.
  function syncMasks(mapping) {
    maskViews = [];
    const { width, height } = size();
    const wanted = list().filter((object) => object.type === 'mask');
    masks.replaceChildren();
    for (const object of wanted) {
      const rect = maskRect(object);
      if (rect.w < 1 || rect.h < 1) {
        continue;
      }
      const element = h('div', { class: `annot-mask is-${object.mode === 'blur' ? 'blur' : 'pixel'}` });
      element.style.left = `${(rect.x / width) * 100}%`;
      element.style.top = `${(rect.y / height) * 100}%`;
      element.style.width = `${(rect.w / width) * 100}%`;
      element.style.height = `${(rect.h / height) * 100}%`;
      let canvas = null;
      let context = null;
      if (object.mode === 'blur') {
        // The server blurs with a gaussian of `strength` source pixels: the
        // preview uses the same radius, scaled to the stage.
        element.style.setProperty('--mask-blur', `${Math.max(1, object.strength * mapping.scale).toFixed(1)}px`);
      } else {
        canvas = makeCanvas(rect.w, rect.h);
        context = canvas.getContext('2d');
        canvas.className = 'annot-mask-canvas';
        element.append(canvas);
      }
      masks.append(element);
      maskViews.push({ object, element, canvas, context, rect });
    }
    paintMasks();
  }

  function placeSelection() {
    const object = selected();
    if (!object || tool === 'draw' || tool === 'shape' || tool === 'blur' || tool === 'text') {
      selBox.hidden = true;
      return;
    }
    const { width, height } = size();
    const boundsBox = boundsOf(object);
    selBox.hidden = false;
    selBox.style.left = `${(boundsBox.x / width) * 100}%`;
    selBox.style.top = `${(boundsBox.y / height) * 100}%`;
    selBox.style.width = `${(boundsBox.w / width) * 100}%`;
    selBox.style.height = `${(boundsBox.h / height) * 100}%`;
  }

  // --- coordinates --------------------------------------------------------
  // `offsetX`/`offsetY` are read in the element's own space, so the CSS
  // rotation and flips of the stage do not move them.
  function toSource(event) {
    const mapping = view();
    return {
      x: mapping.x + event.offsetX / mapping.scale,
      y: mapping.y + event.offsetY / mapping.scale,
      per: 1 / mapping.scale,
    };
  }

  function hitTest(point) {
    const pad = 6 * point.per;
    const all = list();
    for (let index = all.length - 1; index >= 0; index -= 1) {
      const object = all[index];
      if (object.type === 'stroke' && object.mode === 'eraser') {
        continue;
      }
      const box = boundsOf(object);
      if (point.x >= box.x - pad && point.x <= box.x + box.w + pad && point.y >= box.y - pad && point.y <= box.y + box.h + pad) {
        return object;
      }
    }
    return null;
  }

  // --- objects ------------------------------------------------------------
  function addObject(object) {
    const created = { id: nextId, ...object };
    nextId += 1;
    list().push(created);
    return created;
  }

  function addText(text, x, y, overrides) {
    const object = addObject({
      type: 'text',
      text,
      x,
      y,
      size: Math.round(style.textSize * unit()),
      color: style.textColor,
      font: style.font,
      outline: style.outline,
      ...overrides,
    });
    commit();
    selectedId = object.id;
    return object;
  }

  function select(id) {
    if (selectedId !== id) {
      selectedId = id;
      if (onPanel) {
        onPanel();
      }
    }
    placeSelection();
  }

  function replaceObject(next) {
    const current = list().find((object) => object.id === next.id);
    if (current) {
      Object.assign(current, next);
    }
  }

  function commit() {
    overlayCache = null;
    if (onCommit) {
      onCommit();
    }
    layout();
    paint();
  }

  function focusTextField() {
    const field = frame.panel.querySelector('textarea');
    if (field) {
      field.focus();
      field.select();
    }
  }

  // --- pointer ------------------------------------------------------------
  surface.addEventListener('pointerdown', (event) => {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    frame.node.focus();
    const point = toSource(event);
    if (tool === 'text') {
      const { width, height } = size();
      addText('Ton texte', clamp(point.x, 0, width), clamp(point.y, 0, height));
      setTool('select');
      focusTextField();
      return;
    }
    surface.setPointerCapture(event.pointerId);
    if (tool === 'draw') {
      selectedId = null;
      const object = addObject({ type: 'stroke', mode: style.drawMode, color: style.color, width: style.stroke * unit() * (style.drawMode === 'pen' ? 1 : 3), points: [point.x, point.y] });
      drag = { kind: 'create', id: object.id };
    } else if (tool === 'shape') {
      selectedId = null;
      const object = addObject({ type: 'shape', shape: style.shape, color: style.color, width: style.stroke * unit(), fill: style.fill, x1: point.x, y1: point.y, x2: point.x, y2: point.y });
      drag = { kind: 'create', id: object.id };
    } else if (tool === 'blur') {
      selectedId = null;
      const object = addObject({ type: 'mask', mode: style.maskMode, strength: style.maskStrength * unit(), x1: point.x, y1: point.y, x2: point.x, y2: point.y });
      drag = { kind: 'create', id: object.id };
    } else {
      const current = selected();
      const currentBox = current ? boundsOf(current) : null;
      if (currentBox && Math.hypot(point.x - (currentBox.x + currentBox.w), point.y - (currentBox.y + currentBox.h)) <= 12 * point.per) {
        drag = { kind: 'resize', id: current.id, origin: structuredClone(current), anchor: { x: currentBox.x, y: currentBox.y }, reach: Math.hypot(currentBox.w, currentBox.h), moved: false };
      } else {
        const hit = hitTest(point);
        select(hit ? hit.id : null);
        drag = hit ? { kind: 'move', id: hit.id, origin: structuredClone(hit), start: point, moved: false } : null;
      }
    }
    paint();
  });

  surface.addEventListener('pointermove', (event) => {
    if (!drag) {
      return;
    }
    const point = toSource(event);
    const object = list().find((candidate) => candidate.id === drag.id);
    if (!object) {
      return;
    }
    if (drag.kind === 'create') {
      if (object.type === 'stroke') {
        const points = object.points;
        if (Math.hypot(point.x - points[points.length - 2], point.y - points[points.length - 1]) >= point.per) {
          points.push(point.x, point.y);
        }
      } else {
        object.x2 = point.x;
        object.y2 = point.y;
      }
      syncMasks(view());
      paint();
    } else if (drag.kind === 'move') {
      drag.moved = true;
      replaceObject(translated(drag.origin, point.x - drag.start.x, point.y - drag.start.y));
      syncMasks(view());
      paint();
    } else {
      drag.moved = true;
      const factor = clamp(Math.hypot(point.x - drag.anchor.x, point.y - drag.anchor.y) / Math.max(1, drag.reach), 0.05, 40);
      replaceObject(scaled(drag.origin, drag.anchor, factor));
      syncMasks(view());
      paint();
    }
  });

  function endDrag(event) {
    if (!drag) {
      return;
    }
    const finished = drag;
    drag = null;
    if (finished.kind === 'create') {
      const object = list().find((candidate) => candidate.id === finished.id);
      const point = toSource(event);
      const tinyShape = object && object.type !== 'stroke' && Math.hypot(object.x2 - object.x1, object.y2 - object.y1) < 4 * point.per;
      if (tinyShape) {
        removeObject(finished.id);
        paint();
        return;
      }
      commit();
      if (onPanel) {
        onPanel();
      }
    } else if (finished.moved) {
      commit();
      if (finished.kind === 'resize' && onPanel) {
        onPanel();
      }
    }
  }
  surface.addEventListener('pointerup', endDrag);
  surface.addEventListener('pointercancel', endDrag);

  surface.addEventListener('dblclick', (event) => {
    const hit = hitTest(toSource(event));
    if (hit && hit.type === 'text') {
      if (tool !== 'select') {
        setTool('select');
      }
      select(hit.id);
      focusTextField();
    }
  });

  function removeObject(id) {
    const all = list();
    const index = all.findIndex((object) => object.id === id);
    if (index >= 0) {
      all.splice(index, 1);
    }
    if (selectedId === id) {
      selectedId = null;
    }
  }

  // --- panels -------------------------------------------------------------
  function applyToSelection(patch) {
    const object = selected();
    if (object) {
      Object.assign(object, patch);
      commit();
      paint();
    }
  }

  function removeSelection() {
    if (!selected()) {
      return;
    }
    removeObject(selectedId);
    commit();
    if (onPanel) {
      onPanel();
    }
    paint();
  }

  function duplicateSelection() {
    const object = selected();
    if (!object) {
      return;
    }
    const offset = 12 * unit();
    const { id: _sourceId, ...clone } = translated(object, offset, offset);
    const copy = addObject(clone);
    commit();
    select(copy.id);
    paint();
  }

  function bringToFront() {
    const object = selected();
    if (!object) {
      return;
    }
    const all = list();
    const index = all.indexOf(object);
    if (index >= 0) {
      all.splice(index, 1);
      all.push(object);
    }
    commit();
    paint();
  }

  function selectionPanel(object) {
    const rows = [];
    if (object.type === 'text' && object.font !== 'emoji') {
      const field = h('textarea', {
        class: 'field',
        rows: '3',
        maxlength: '400',
        value: object.text,
        'aria-label': 'Texte',
        onInput: () => {
          object.text = field.value;
          paint();
        },
        onChange: commit,
      });
      rows.push(
        field,
        choices(FONT_LABELS, object.font, (font) => { style.font = font; applyToSelection({ font }); if (onPanel) onPanel(); }, 'Police'),
        colorRow(object.color, (color) => { style.textColor = color; applyToSelection({ color }); if (onPanel) onPanel(); }),
        toggle('Contour', object.outline, (outline) => { style.outline = outline; applyToSelection({ outline }); }),
      );
    }
    if (object.type === 'text') {
      rows.push(
        slider({
          label: 'Taille',
          min: 4,
          max: 160,
          value: clamp(Math.round(object.size / unit()), 4, 160),
          defaultValue: styleDefaults.textSize,
          onInput: (value) => { object.size = value * unit(); paint(); },
          onCommit: commit,
        }),
      );
    }
    if (object.type === 'shape' || object.type === 'stroke') {
      rows.push(
        colorRow(object.color, (color) => { style.color = color; applyToSelection({ color }); if (onPanel) onPanel(); }),
        slider({
          label: 'Épaisseur',
          min: 1,
          max: 40,
          value: clamp(Math.round(object.width / unit()), 1, 40),
          defaultValue: styleDefaults.stroke,
          onInput: (value) => { object.width = value * unit(); paint(); },
          onCommit: commit,
        }),
      );
    }
    if (object.type === 'mask') {
      rows.push(
        choices([['pixel', 'Pixels'], ['blur', 'Flou']], object.mode, (mode) => { applyToSelection({ mode }); if (onPanel) onPanel(); }, 'Type de masque'),
        slider({
          label: 'Intensité',
          min: 2,
          max: 20,
          value: clamp(Math.round(object.strength / unit()), 2, 20),
          defaultValue: styleDefaults.maskStrength,
          onInput: (value) => { object.strength = value * unit(); syncMasks(view()); paint(); },
          onCommit: commit,
        }),
      );
    }
    rows.push(
      h(
        'div',
        { class: 'row is-wrap' },
        h('button', { class: 'btn', type: 'button', onClick: duplicateSelection }, icon('copy'), 'Dupliquer'),
        h('button', { class: 'btn', type: 'button', title: 'Passer devant les autres éléments', onClick: bringToFront }, 'Premier plan'),
        h('button', { class: 'btn is-danger', type: 'button', onClick: removeSelection }, icon('remove'), 'Supprimer'),
      ),
    );
    return section('Élément sélectionné', ...rows);
  }

  function toolPanel() {
    const hint = h('p', { class: 'meta' }, TOOL_HINTS[tool] || '');
    if (tool === 'draw') {
      return section(
        'Dessiner',
        hint,
        choices([['pen', 'Pinceau', 'edit'], ['marker', 'Surligneur', 'marker'], ['eraser', 'Gomme', 'eraser']], style.drawMode, (mode) => { style.drawMode = mode; if (onPanel) onPanel(); }, 'Outil de dessin'),
        style.drawMode === 'eraser' ? h('p', { class: 'meta' }, 'La gomme efface ce que tu as ajouté, jamais la vidéo.') : colorRow(style.color, (color) => { style.color = color; if (onPanel) onPanel(); }),
        slider({ label: 'Épaisseur', min: 1, max: 40, value: style.stroke, defaultValue: styleDefaults.stroke, onInput: (value) => { style.stroke = value; } }),
      );
    }
    if (tool === 'shape') {
      return section(
        'Formes',
        hint,
        choices([['arrow', 'Flèche', 'arrow'], ['rect', 'Rectangle', 'rect'], ['ellipse', 'Ellipse', 'ellipse'], ['line', 'Trait', 'line']], style.shape, (shape) => { style.shape = shape; if (onPanel) onPanel(); }, 'Forme', 'is-grid-2'),
        colorRow(style.color, (color) => { style.color = color; if (onPanel) onPanel(); }),
        slider({ label: 'Épaisseur', min: 1, max: 40, value: style.stroke, defaultValue: styleDefaults.stroke, onInput: (value) => { style.stroke = value; } }),
        style.shape === 'rect' || style.shape === 'ellipse' ? toggle('Forme pleine', style.fill, (fill) => { style.fill = fill; }) : null,
      );
    }
    if (tool === 'text') {
      const { width, height } = size();
      const meme = (text, top) => {
        const textSize = Math.round(width / 9);
        addText(text, width / 2, top ? textSize * 0.85 : height - textSize * 0.85, { font: 'meme', color: '#ffffff', outline: true, size: textSize });
        setTool('select');
        focusTextField();
      };
      return section(
        'Texte',
        hint,
        h(
          'div',
          { class: 'editor-pair' },
          h('button', { class: 'btn', type: 'button', onClick: () => meme('Texte du haut', true) }, 'Texte en haut'),
          h('button', { class: 'btn', type: 'button', onClick: () => meme('Texte du bas', false) }, 'Texte en bas'),
        ),
        choices(FONT_LABELS, style.font, (font) => { style.font = font; if (onPanel) onPanel(); }, 'Police', 'is-grid-2'),
        colorRow(style.textColor, (color) => { style.textColor = color; if (onPanel) onPanel(); }),
        slider({ label: 'Taille', min: 4, max: 160, value: style.textSize, defaultValue: styleDefaults.textSize, onInput: (value) => { style.textSize = value; } }),
        toggle('Contour', style.outline, (outline) => { style.outline = outline; }),
      );
    }
    if (tool === 'emoji') {
      return section(
        'Emoji',
        hint,
        h(
          'div',
          { class: 'emoji-grid' },
          EMOJIS.map((emoji) =>
            h('button', {
              class: 'emoji',
              type: 'button',
              'aria-label': `Ajouter ${emoji}`,
              onClick: () => {
                const { width, height } = size();
                const object = addText(emoji, width / 2, height / 2, { font: 'emoji', outline: false, color: '#ffffff', size: Math.round(60 * unit()) });
                if (onPanel) onPanel();
                select(object.id);
                paint();
              },
            }, emoji),
          ),
        ),
      );
    }
    if (tool === 'blur') {
      return section(
        'Masquer une zone',
        hint,
        choices([['pixel', 'Pixels'], ['blur', 'Flou']], style.maskMode, (mode) => { style.maskMode = mode; if (onPanel) onPanel(); }, 'Type de masque', 'is-grid-2'),
        slider({ label: 'Intensité', min: 2, max: 20, value: style.maskStrength, defaultValue: styleDefaults.maskStrength, onInput: (value) => { style.maskStrength = value; } }),
      );
    }
    if (tool === 'adjust') {
      const current = colorState();
      const adjustSlider = (label, key, min, max) =>
        slider({
          label,
          min,
          max,
          value: current[key],
          defaultValue: NEUTRAL_ADJUST[key],
          format: key === 'blur' ? (value) => String(value) : (value) => `${value}\u00A0%`,
          onInput: (value) => { colorState()[key] = value; applyPreviewFilter(); },
          onCommit: commit,
        });
      return section(
        'Réglages',
        hint,
        choices(FILTER_PRESETS.map((preset) => [preset.id, preset.label]), current.preset, (preset) => { colorState().preset = preset; commit(); applyPreviewFilter(); if (onPanel) onPanel(); }, 'Filtre', 'is-grid-2'),
        adjustSlider('Luminosité', 'brightness', 40, 160),
        adjustSlider('Contraste', 'contrast', 40, 160),
        adjustSlider('Saturation', 'saturate', 0, 200),
        adjustSlider('Flou', 'blur', 0, 10),
        h('button', { class: 'btn', type: 'button', onClick: () => { const state = colorState(); Object.assign(state, NEUTRAL_ADJUST); commit(); applyPreviewFilter(); if (onPanel) onPanel(); } }, 'Réinitialiser les réglages'),
      );
    }
    if (selected()) {
      return null;
    }
    return section('Sélection', h('p', { class: 'meta' }, 'Aucun élément sélectionné. Dessine, pose un texte ou trace un masque avec les outils de gauche.'));
  }

  // CSS filter that renders the colour adjustments at `cssScale` pixels per
  // source pixel. The server adjusts the picture at the frame's own size, so
  // the blur radius follows that unit too.
  function filterFor(cssScale) {
    const current = colorState();
    const blur = Math.round(current.blur * unit() * cssScale * 10) / 10;
    return cssFilter({ ...current, blur });
  }

  // Applies the colour adjustments to the clip that holds the video and the
  // retouches, in stage pixels, so the preview follows the recipe: the server
  // burns the masks and the drawing, then adjusts the picture they produced.
  function applyPreviewFilter() {
    const media = source();
    const layer = box.parentElement;
    if (!media || !layer) {
      return;
    }
    media.style.filter = '';
    layer.style.filter = filterFor(view().scale);
  }

  function panelSections() {
    if (tool !== 'select') {
      return [toolPanel()].filter(Boolean);
    }
    const object = selected();
    return [object ? selectionPanel(object) : toolPanel()].filter(Boolean);
  }

  // --- rail ---------------------------------------------------------------
  function renderRail() {
    const rail = [];
    for (const group of TOOL_GROUPS) {
      const entries = group.map((id) => ANNOTATION_TOOLS.find((entry) => entry[0] === id));
      if (rail.length) {
        rail.push(h('div', { class: 'tool-sep', role: 'separator' }));
      }
      rail.push(
        ...entries.map(([id, label, glyph]) =>
          h('button', { class: `tool${tool === id ? ' is-active' : ''}`, type: 'button', 'aria-pressed': String(tool === id), onClick: () => setTool(id) }, icon(glyph), h('span', null, label)),
        ),
      );
    }
    frame.rail.replaceChildren(...rail);
    frame.rail.hidden = false;
  }

  function setTool(next) {
    if (tool === next) {
      return;
    }
    tool = next;
    if (['draw', 'shape', 'blur', 'text'].includes(tool)) {
      selectedId = null;
    }
    surface.dataset.tool = tool;
    renderRail();
    placeSelection();
    paint();
    if (onPanel) {
      onPanel();
    }
  }

  // Hides the whole layer while the crop box is open: the two cannot be moved
  // at the same time, and the crop previews the frame without them.
  function setEnabled(enabled) {
    box.hidden = !enabled;
    surface.hidden = !enabled;
    selBox.hidden = !enabled;
    if (!enabled) {
      selectedId = null;
    }
  }

  // Called after an undo, a redo or a restore: the history replaced the object
  // list, so everything is rebuilt from it.
  function sync() {
    selectedId = null;
    overlayCache = null;
    layout();
    paint();
    applyPreviewFilter();
  }

  // The drawing as a PNG of the frame, or null: the recipe carries it as the
  // annotation overlay, and the server burns it before the crop.
  function overlay() {
    if (overlayCache !== null) {
      return overlayCache || null;
    }
    const { width, height } = size();
    const dataUrl = annotationsToDataUrl(list(), width, height);
    overlayCache = dataUrl ? dataUrl.slice(dataUrl.indexOf(',') + 1) : '';
    return overlayCache || null;
  }

  function handleKey(event) {
    if (event.key === 'Delete' || event.key === 'Backspace') {
      if (selected()) {
        removeSelection();
        return true;
      }
      return false;
    }
    if (event.key === 'Escape') {
      if (selected()) {
        select(null);
        return true;
      }
      return false;
    }
    if (event.key === 'Enter' && selected() && !(event.target instanceof HTMLButtonElement)) {
      const object = selected();
      if (object.type === 'text') {
        focusTextField();
        return true;
      }
    }
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      const object = selected();
      if (!object) {
        return false;
      }
      const step = (event.shiftKey ? 10 : 1) * unit();
      const dx = event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0;
      const dy = event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0;
      replaceObject(translated(object, dx, dy));
      commit();
      return true;
    }
    return false;
  }

  function destroy() {
    box.remove();
    surface.remove();
    selBox.remove();
  }

  renderRail();
  layout();
  paint();
  applyPreviewFilter();

  return {
    style,
    setTool,
    tool: () => tool,
    renderRail,
    panelSections,
    layout,
    paint,
    paintMasks,
    paintInto,
    sync,
    overlay,
    applyPreviewFilter,
    filterFor,
    setEnabled,
    handleKey,
    hasSelection: () => Boolean(selected()),
    destroy,
  };
}
