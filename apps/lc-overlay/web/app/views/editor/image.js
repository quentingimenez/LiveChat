// Image editor: crop, rotate, flip, draw, shapes, text, emoji, masking and
// colour adjustments, all on a canvas inside the board.
//
// The document is a base picture plus a list of objects (strokes, shapes,
// texts, masks) in the base picture's pixel space, and a colour filter
// applied to the whole result. Crop, rotation and flips bake the objects into
// a new base picture. Every committed action is a snapshot: undo and redo
// walk through them.

import { h, icon, resetOnDoubleClick } from '../../core/dom.js';
import { formatDuration } from '../../core/format.js';
import {
  CROP_RATIOS,
  DEFAULT_DISPLAY_SCALE_PERCENT,
  FILTER_PRESETS,
  MAX_DISPLAY_SCALE_PERCENT,
  MIN_DISPLAY_SCALE_PERCENT,
  NEUTRAL_ADJUST,
  OVERLAY_MEDIA_RESERVED_HEIGHT,
  centeredCrop,
  clamp,
  cssFilter,
  displayScaleForStorage,
  displayZoneLimits,
  displayedImageSize,
  fitScale,
  mediaInfoFacts,
  normalizeDisplayScale,
  parseDisplayScale,
} from '../../core/editor-helpers.js';
import { createCropOverlay } from './crop-overlay.js';
import { createDisplayPreview } from './display-preview.js';
import { createStageZoom } from './stage-zoom.js';
import {
  COLORS,
  EMOJIS,
  FONT_LABELS,
  FONTS,
  boundsOf,
  colorRow,
  ensureSize,
  fontOf,
  isDark,
  linesOf,
  makeCanvas,
  paintObject,
  resized,
  translated,
} from './annotations.js';
import { choices, facts, section, slider, toggle } from './frame.js';
import { railGroups, toolHint } from './tools.js';

// Largest side kept while editing: a bigger picture is scaled down once.
const MAX_SIDE = 4096;
const HISTORY_LIMIT = 40;

// Decodes the picture into a canvas (scaled down above MAX_SIDE).
async function loadBase(blob) {
  const url = URL.createObjectURL(blob);
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const width = image.naturalWidth || image.width;
    const height = image.naturalHeight || image.height;
    if (!width || !height) {
      throw new Error('empty picture');
    }
    const scale = fitScale(width, height, MAX_SIDE);
    const base = makeCanvas(width * scale, height * scale);
    const context = base.getContext('2d');
    context.imageSmoothingQuality = 'high';
    context.drawImage(image, 0, 0, base.width, base.height);
    return base;
  } finally {
    URL.revokeObjectURL(url);
  }
}

// Builds the editor inside `frame`. `onChange()` fires after every committed
// action. `allowDisplayScale` adds the tool setting how big the picture shows
// on the overlay (from a board item's `displayScalePercent`); `displayZone`
// is the display zone size in CSS pixels when known. Resolves to the editor's
// controls; rejects when the picture cannot be decoded.
export async function createImageEditor({
  blob,
  frame,
  onChange,
  allowDisplayScale = false,
  displayScalePercent = null,
  displayZone = null,
  durationSec = null,
}) {
  const doc = { base: await loadBase(blob), objects: [], adjust: { ...NEUTRAL_ADJUST } };
  // Size the picture came in at: the summary shows the crop/rotation against it.
  const sourceWidth = doc.base.width;
  const sourceHeight = doc.base.height;
  let nextId = 1;
  let tool = 'select';
  let selectedId = null;
  let drag = null;
  let cropRatio = 'free';
  let paintQueued = false;
  // Display scale is a setting of the board item, not a pixel edit: it stays
  // out of the undo history.
  const initialScale = normalizeDisplayScale(displayScalePercent);
  let displayScale = initialScale;
  const style = { color: '#ec6a70', stroke: 4, drawMode: 'pen', shape: 'arrow', fill: false, font: 'meme', textSize: 30, outline: true, textColor: '#ffffff', maskMode: 'pixel', maskStrength: 6 };
  // Shipped defaults of the tools: a double click on one of their sliders
  // puts it back there.
  const styleDefaults = { ...style };

  const scratch = makeCanvas(1, 1);
  const layer = makeCanvas(1, 1);
  const tiny = makeCanvas(1, 1);
  const view = h('canvas', { class: 'img-canvas' });
  const selBox = h('div', { class: 'sel-box', hidden: true }, h('span', { class: 'sel-handle' }));
  const wrap = h('div', { class: 'img-wrap' }, view, selBox);
  const stageZoom = createStageZoom({ content: wrap, onResize: () => layout() });

  // --- live preview of the display size ---
  // The overlay draws the picture at `natural size × scale`, capped by the
  // display zone minus the author header, and the browser resamples it. The
  // preview rebuilds that frame at the stage's own scale, so a scale change is
  // seen at once instead of on the next play.
  const preview = createDisplayPreview({
    stage: frame.stage,
    displayZone,
    frame: () => ({ width: doc.base.width, height: doc.base.height, scalePercent: displayScale }),
    paint: (target) => compose(target, doc.adjust),
  });
  const zone = preview.zone;

  frame.stage.replaceChildren(stageZoom.node, stageZoom.chip, preview.node);

  // One length unit follows the picture: a stroke looks the same on a small
  // and on a large picture.
  const unit = () => Math.max(doc.base.width, doc.base.height) / 400;
  const selected = () => doc.objects.find((object) => object.id === selectedId) || null;

  // --- history ---
  const snapshot = () => ({ base: doc.base, objects: structuredClone(doc.objects), adjust: { ...doc.adjust } });
  let history = [snapshot()];
  let historyIndex = 0;
  const initial = history[0];

  function commit() {
    history = history.slice(0, historyIndex + 1);
    history.push(snapshot());
    if (history.length > HISTORY_LIMIT) {
      history.shift();
    }
    historyIndex = history.length - 1;
    frame.refreshHistory();
    syncInfoNode();
    if (onChange) {
      onChange();
    }
  }

  function restore(index) {
    const state = history[index];
    historyIndex = index;
    doc.base = state.base;
    doc.objects = structuredClone(state.objects);
    doc.adjust = { ...state.adjust };
    selectedId = null;
    layout();
    if (tool === 'crop') {
      startCrop();
    }
    renderPanel();
    frame.refreshHistory();
    if (onChange) {
      onChange();
    }
  }

  // --- painting ---
  function paintMask(context, object, scaleX, scaleY, width, height) {
    const x = clamp(Math.min(object.x1, object.x2), 0, doc.base.width);
    const y = clamp(Math.min(object.y1, object.y2), 0, doc.base.height);
    const w = clamp(Math.max(object.x1, object.x2), 0, doc.base.width) - x;
    const h2 = clamp(Math.max(object.y1, object.y2), 0, doc.base.height) - y;
    if (w < 1 || h2 < 1) {
      return;
    }
    context.save();
    if (object.mode === 'pixel') {
      const block = Math.max(2, object.strength);
      const columns = Math.max(1, Math.round(w / block));
      const rows = Math.max(1, Math.round(h2 / block));
      ensureSize(tiny, columns, rows);
      const tinyContext = tiny.getContext('2d');
      tinyContext.clearRect(0, 0, columns, rows);
      tinyContext.drawImage(doc.base, x, y, w, h2, 0, 0, columns, rows);
      context.imageSmoothingEnabled = false;
      context.drawImage(tiny, 0, 0, columns, rows, x * scaleX, y * scaleY, w * scaleX, h2 * scaleY);
    } else {
      context.beginPath();
      context.rect(x * scaleX, y * scaleY, w * scaleX, h2 * scaleY);
      context.clip();
      context.filter = `blur(${Math.max(1, object.strength * scaleX).toFixed(1)}px)`;
      context.drawImage(doc.base, 0, 0, width, height);
    }
    context.restore();
  }

  // Draws the whole document into `target`, whatever its size.
  function compose(target, adjust) {
    const width = target.width;
    const height = target.height;
    const scaleX = width / doc.base.width;
    const scaleY = height / doc.base.height;
    ensureSize(scratch, width, height);
    ensureSize(layer, width, height);
    const flat = scratch.getContext('2d');
    flat.setTransform(1, 0, 0, 1, 0, 0);
    flat.clearRect(0, 0, width, height);
    flat.imageSmoothingQuality = 'high';
    flat.drawImage(doc.base, 0, 0, width, height);
    for (const object of doc.objects) {
      if (object.type === 'mask') {
        paintMask(flat, object, scaleX, scaleY, width, height);
      }
    }
    const drawn = layer.getContext('2d');
    drawn.setTransform(1, 0, 0, 1, 0, 0);
    drawn.clearRect(0, 0, width, height);
    drawn.setTransform(scaleX, 0, 0, scaleY, 0, 0);
    for (const object of doc.objects) {
      if (object.type !== 'mask') {
        paintObject(drawn, object);
      }
    }
    flat.drawImage(layer, 0, 0);
    const context = target.getContext('2d');
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, width, height);
    context.filter = cssFilter({ ...adjust, blur: Math.round(adjust.blur * unit() * scaleX * 5) / 10 });
    context.drawImage(scratch, 0, 0);
    context.filter = 'none';
  }

  function placeSelection() {
    const object = selected();
    if (!object || tool === 'crop') {
      selBox.hidden = true;
      return;
    }
    const box = boundsOf(object);
    selBox.hidden = false;
    selBox.style.left = `${(box.x / doc.base.width) * 100}%`;
    selBox.style.top = `${(box.y / doc.base.height) * 100}%`;
    selBox.style.width = `${(box.w / doc.base.width) * 100}%`;
    selBox.style.height = `${(box.h / doc.base.height) * 100}%`;
  }

  // --- display preview ---
  function previewVisible() {
    return allowDisplayScale && tool === 'display';
  }

  // Size the picture takes on the overlay: natural size × scale, capped by the
  // display zone minus its header.
  function displayedSize() {
    return displayedImageSize({
      width: doc.base.width,
      height: doc.base.height,
      scalePercent: displayScale,
      ...displayZoneLimits({ zoneWidth: zone.width, zoneHeight: zone.height }),
    });
  }

  // Stage pixels per overlay pixel: the whole zone fits, or 1:1 for 100 %.
  // Shows the preview instead of the editing canvas while the display tool is
  // open: the picture is no longer edited there, it is rendered like the
  // overlay will render it.
  function syncStage() {
    const shown = previewVisible();
    wrap.hidden = shown;
    stageZoom.setVisible(!shown);
    if (shown) {
      preview.show();
    } else {
      preview.hide();
      repaint();
    }
  }

  function repaint() {
    if (paintQueued) {
      return;
    }
    paintQueued = true;
    requestAnimationFrame(() => {
      paintQueued = false;
      if (frame.isClosed()) {
        return;
      }
      if (previewVisible()) {
        preview.requestPaint();
        return;
      }
      compose(view, doc.adjust);
      placeSelection();
    });
  }

  function layout() {
    const area = stageZoom.available();
    if (area.width <= 0 || area.height <= 0) {
      return;
    }
    // Le zoom agrandit la taille du cadre, il ne déforme pas son contenu :
    // poignées et sélection gardent leur taille d'écran.
    const fit = Math.min(area.width / doc.base.width, area.height / doc.base.height, 8);
    const zoom = stageZoom.zoom();
    const cssWidth = Math.max(1, Math.round(doc.base.width * fit * zoom));
    const cssHeight = Math.max(1, Math.round(doc.base.height * fit * zoom));
    wrap.style.width = `${cssWidth}px`;
    wrap.style.height = `${cssHeight}px`;
    stageZoom.fit({ width: cssWidth, height: cssHeight });
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    // Le tampon suit le zoom sans dépasser les pixels de l'image : agrandir
    // ne peut pas inventer de détail, et rester au natif économise la mémoire.
    const density = Math.min(fit * zoom * ratio, 1);
    ensureSize(
      view,
      Math.max(1, Math.round(doc.base.width * density)),
      Math.max(1, Math.round(doc.base.height * density)),
    );
    frame.setStatus(`${doc.base.width}\u00A0×\u00A0${doc.base.height}\u00A0px`);
    preview.invalidate();
    repaint();
  }

  const resizeObserver = new ResizeObserver(layout);
  resizeObserver.observe(frame.stage);

  // --- pointer ---
  function toBase(event) {
    const box = wrap.getBoundingClientRect();
    return {
      x: ((event.clientX - box.left) / box.width) * doc.base.width,
      y: ((event.clientY - box.top) / box.height) * doc.base.height,
      // Base pixels per CSS pixel.
      per: doc.base.width / box.width,
    };
  }

  function hitTest(point) {
    const pad = 6 * point.per;
    for (let index = doc.objects.length - 1; index >= 0; index -= 1) {
      const object = doc.objects[index];
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

  // Updates the object in place: the selection panel holds a reference to it
  // (its text field and size slider write to that object).
  function replaceObject(next) {
    const current = doc.objects.find((object) => object.id === next.id);
    if (current) {
      Object.assign(current, next);
    }
  }

  function select(id) {
    if (selectedId !== id) {
      selectedId = id;
      renderPanel();
    }
    placeSelection();
  }

  function addObject(object) {
    const created = { id: nextId, ...object };
    nextId += 1;
    doc.objects.push(created);
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

  function focusTextField() {
    const field = frame.panel.querySelector('textarea');
    if (field) {
      field.focus();
      field.select();
    }
  }

  wrap.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || tool === 'crop') {
      return;
    }
    event.preventDefault();
    frame.node.focus();
    const point = toBase(event);
    if (tool === 'text') {
      addText('Ton texte', point.x, point.y);
      setTool('select');
      focusTextField();
      return;
    }
    wrap.setPointerCapture(event.pointerId);
    if (tool === 'draw') {
      selectedId = null;
      const object = addObject({ type: 'stroke', mode: style.drawMode, color: style.color, width: style.stroke * unit() * (style.drawMode === 'pen' ? 1 : 3), points: [point.x, point.y] });
      drag = { kind: 'create', id: object.id, start: point };
    } else if (tool === 'shape') {
      selectedId = null;
      const object = addObject({ type: 'shape', shape: style.shape, color: style.color, width: style.stroke * unit(), fill: style.fill, x1: point.x, y1: point.y, x2: point.x, y2: point.y });
      drag = { kind: 'create', id: object.id, start: point };
    } else if (tool === 'blur') {
      selectedId = null;
      const object = addObject({ type: 'mask', mode: style.maskMode, strength: style.maskStrength * unit(), x1: point.x, y1: point.y, x2: point.x, y2: point.y });
      drag = { kind: 'create', id: object.id, start: point };
    } else {
      const current = selected();
      const box = current ? boundsOf(current) : null;
      if (box && Math.hypot(point.x - (box.x + box.w), point.y - (box.y + box.h)) <= 12 * point.per) {
        drag = { kind: 'resize', id: current.id, origin: structuredClone(current), anchor: { x: box.x, y: box.y }, reach: Math.hypot(box.w, box.h), moved: false };
      } else {
        const hit = hitTest(point);
        select(hit ? hit.id : null);
        drag = hit ? { kind: 'move', id: hit.id, origin: structuredClone(hit), start: point, moved: false } : null;
      }
    }
    repaint();
  });

  wrap.addEventListener('pointermove', (event) => {
    if (!drag) {
      return;
    }
    const point = toBase(event);
    const object = doc.objects.find((candidate) => candidate.id === drag.id);
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
    } else if (drag.kind === 'move') {
      drag.moved = true;
      replaceObject(translated(drag.origin, point.x - drag.start.x, point.y - drag.start.y));
    } else {
      drag.moved = true;
      const factor = clamp(Math.hypot(point.x - drag.anchor.x, point.y - drag.anchor.y) / Math.max(1, drag.reach), 0.05, 40);
      replaceObject(resized(drag.origin, drag.anchor, point, factor, 8 * point.per));
    }
    repaint();
  });

  function endDrag(event) {
    if (!drag) {
      return;
    }
    const finished = drag;
    drag = null;
    if (finished.kind === 'create') {
      const object = doc.objects.find((candidate) => candidate.id === finished.id);
      const point = toBase(event);
      const tiny2 = object && object.type !== 'stroke' && Math.hypot(object.x2 - object.x1, object.y2 - object.y1) < 4 * point.per;
      if (tiny2) {
        doc.objects = doc.objects.filter((candidate) => candidate.id !== finished.id);
      } else {
        commit();
      }
    } else if (finished.moved) {
      commit();
      // The size shown in the panel follows a resize.
      if (finished.kind === 'resize') {
        renderPanel();
      }
    }
    repaint();
  }
  wrap.addEventListener('pointerup', endDrag);
  wrap.addEventListener('pointercancel', endDrag);

  wrap.addEventListener('dblclick', (event) => {
    const hit = hitTest(toBase(event));
    if (hit && hit.type === 'text') {
      if (tool !== 'select') {
        setTool('select');
      }
      select(hit.id);
      focusTextField();
    }
  });

  // --- crop, rotation, flips (bake the objects into a new base) ---
  let cropOverlay = null;

  function startCrop() {
    if (!cropOverlay) {
      cropOverlay = createCropOverlay({
        host: wrap,
        bounds: () => ({ width: doc.base.width, height: doc.base.height }),
        aspect: () => (CROP_RATIOS.find((entry) => entry.id === cropRatio) || CROP_RATIOS[0]).ratio,
        onChange: () => {},
        onCommit: () => {},
      });
    }
    const ratio = (CROP_RATIOS.find((entry) => entry.id === cropRatio) || CROP_RATIOS[0]).ratio;
    cropOverlay.set(centeredCrop(ratio, doc.base.width, doc.base.height));
  }

  function stopCrop() {
    if (cropOverlay) {
      cropOverlay.remove();
      cropOverlay = null;
    }
  }

  function flattened() {
    const flat = makeCanvas(doc.base.width, doc.base.height);
    compose(flat, NEUTRAL_ADJUST);
    return flat;
  }

  function setBase(next) {
    doc.base = next;
    doc.objects = [];
    selectedId = null;
    commit();
    layout();
    if (tool === 'crop') {
      startCrop();
    }
    renderPanel();
  }

  function applyCrop() {
    const rect = cropOverlay && cropOverlay.get();
    if (!rect) {
      return;
    }
    const x = clamp(Math.round(rect.x), 0, doc.base.width - 1);
    const y = clamp(Math.round(rect.y), 0, doc.base.height - 1);
    const width = clamp(Math.round(rect.width), 1, doc.base.width - x);
    const height = clamp(Math.round(rect.height), 1, doc.base.height - y);
    if (width === doc.base.width && height === doc.base.height) {
      return;
    }
    const next = makeCanvas(width, height);
    next.getContext('2d').drawImage(flattened(), x, y, width, height, 0, 0, width, height);
    setBase(next);
  }

  function rotate(direction) {
    const flat = flattened();
    const next = makeCanvas(flat.height, flat.width);
    const context = next.getContext('2d');
    context.translate(next.width / 2, next.height / 2);
    context.rotate((direction * Math.PI) / 2);
    context.drawImage(flat, -flat.width / 2, -flat.height / 2);
    setBase(next);
  }

  function flip(horizontal) {
    const flat = flattened();
    const next = makeCanvas(flat.width, flat.height);
    const context = next.getContext('2d');
    context.translate(horizontal ? flat.width : 0, horizontal ? 0 : flat.height);
    context.scale(horizontal ? -1 : 1, horizontal ? 1 : -1);
    context.drawImage(flat, 0, 0);
    setBase(next);
  }

  // --- panels ---
  // Changes a property of the selected object, or the style of the next one.
  function applyToSelection(patch) {
    const object = selected();
    if (object) {
      Object.assign(object, patch);
      commit();
      repaint();
    }
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
          repaint();
        },
        onChange: commit,
      });
      rows.push(
        field,
        choices(FONT_LABELS, object.font, (font) => { style.font = font; applyToSelection({ font }); renderPanel(); }, 'Police'),
        colorRow(object.color, (color) => { style.textColor = color; applyToSelection({ color }); renderPanel(); }),
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
          onInput: (value) => { object.size = value * unit(); repaint(); },
          onCommit: commit,
        }),
      );
    }
    if (object.type === 'shape' || object.type === 'stroke') {
      rows.push(
        colorRow(object.color, (color) => { style.color = color; applyToSelection({ color }); renderPanel(); }),
        slider({
          label: 'Épaisseur',
          min: 1,
          max: 40,
          value: clamp(Math.round(object.width / unit()), 1, 40),
          defaultValue: styleDefaults.stroke,
          onInput: (value) => { object.width = value * unit(); repaint(); },
          onCommit: commit,
        }),
      );
    }
    if (object.type === 'mask') {
      rows.push(
        choices([['pixel', 'Pixels'], ['blur', 'Flou']], object.mode, (mode) => { applyToSelection({ mode }); renderPanel(); }, 'Type de masque'),
        slider({
          label: 'Intensité',
          min: 2,
          max: 20,
          value: clamp(Math.round(object.strength / unit()), 2, 20),
          defaultValue: styleDefaults.maskStrength,
          onInput: (value) => { object.strength = value * unit(); repaint(); },
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

  function removeSelection() {
    if (!selected()) {
      return;
    }
    doc.objects = doc.objects.filter((object) => object.id !== selectedId);
    selectedId = null;
    commit();
    renderPanel();
    repaint();
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
    repaint();
  }

  function bringToFront() {
    const object = selected();
    if (!object) {
      return;
    }
    doc.objects = doc.objects.filter((candidate) => candidate !== object).concat(object);
    commit();
    repaint();
  }

  // --- display scale (board item setting, no pixel change) ---
  let syncScaleFields = null;

  function setDisplayScale(next) {
    const value = normalizeDisplayScale(next);
    if (value === displayScale) {
      return;
    }
    displayScale = value;
    if (syncScaleFields) {
      syncScaleFields();
    }
    syncInfoNode();
    // The preview is the point of this setting: it follows every move.
    preview.invalidate();
    repaint();
    if (onChange) {
      onChange();
    }
  }

  function displayScaleLine() {
    const size = displayedSize();
    // Without a known zone the numbers come from the assumed 1080p screen, so
    // they are an estimate rather than a cap the overlay will apply.
    const note = zone.known
      ? size.capped
        ? ' (plafonné par la zone, en-tête compris)'
        : ''
      : ` (estimation sur une zone ${zone.width}\u00A0×\u00A0${zone.height}\u00A0px)`;
    return `À l’écran\u00A0: ${size.width}\u00A0×\u00A0${size.height}\u00A0px${note}`;
  }

  function displayScalePanel() {
    const line = h('p', { class: 'meta' }, displayScaleLine());
    const value = h('span', { class: 'editor-value' }, `${displayScale}\u00A0%`);
    // Same span as the number field, so the presets never push the cursor out of range.
    const range = h('input', {
      type: 'range',
      min: String(MIN_DISPLAY_SCALE_PERCENT),
      max: String(MAX_DISPLAY_SCALE_PERCENT),
      step: '1',
      value: String(clamp(displayScale, MIN_DISPLAY_SCALE_PERCENT, MAX_DISPLAY_SCALE_PERCENT)),
      onInput: () => setDisplayScale(Number(range.value)),
    });
    resetOnDoubleClick(range, DEFAULT_DISPLAY_SCALE_PERCENT);
    const number = h('input', {
      class: 'field',
      type: 'number',
      min: String(MIN_DISPLAY_SCALE_PERCENT),
      max: String(MAX_DISPLAY_SCALE_PERCENT),
      step: '1',
      inputmode: 'numeric',
      value: String(displayScale),
      onInput: () => {
        const parsed = parseDisplayScale(number.value);
        if (parsed.ok) {
          setDisplayScale(parsed.value);
        }
      },
      onChange: () => {
        const parsed = parseDisplayScale(number.value);
        if (!parsed.ok) {
          number.value = String(displayScale);
        }
      },
    });
    // Both controls and the live size follow any change, whichever one moved.
    syncScaleFields = () => {
      value.textContent = `${displayScale}\u00A0%`;
      range.value = String(clamp(displayScale, MIN_DISPLAY_SCALE_PERCENT, MAX_DISPLAY_SCALE_PERCENT));
      number.value = String(displayScale);
      line.textContent = displayScaleLine();
    };
    const preset = (percent, label) =>
      h('button', { class: 'btn', type: 'button', onClick: () => setDisplayScale(percent) }, label);
    return section(
      'Taille à l’affichage',
      h('p', { class: 'meta' }, toolHint('display', 'image')),
      h(
        'p',
        { class: 'meta' },
        zone.known ? `Zone\u00A0: ${zone.width}\u00A0×\u00A0${zone.height}\u00A0px` : 'Zone d’affichage inconnue',
      ),
      h('label', { class: 'editor-slider' }, h('span', { class: 'editor-slider-head' }, 'Échelle', value), range),
      h('label', { class: 'editor-field' }, h('span', null, 'Valeur\u00A0(%)'), number),
      h(
        'div',
        { class: 'editor-presets is-three' },
        preset(150, '150\u00A0%'),
        preset(200, '200\u00A0%'),
        preset(300, '300\u00A0%'),
      ),
      h(
        'div',
        { class: 'editor-presets is-two' },
        preset(MAX_DISPLAY_SCALE_PERCENT, 'Remplir la zone'),
        preset(DEFAULT_DISPLAY_SCALE_PERCENT, 'Taille d’origine'),
      ),
      line,
      h('p', { class: 'meta' }, `L’aperçu de l’éditeur montre l’image sur l’overlay, en-tête de ${OVERLAY_MEDIA_RESERVED_HEIGHT}\u00A0px compris. Une légende sous l’image n’y est pas représentée.`),
    );
  }

  // Nothing selected: the panel tells what can be done and what the media is,
  // instead of leaving an empty column. The top bar already shows the image's
  // dimensions, so only the duration is repeated here.
  function emptySelectionPanel() {
    const duration = Number(durationSec);
    return section(
      'Aucune sélection',
      h('p', { class: 'editor-hint' }, toolHint('select', 'image')),
      Number.isFinite(duration) && duration > 0 ? facts([['Durée', formatDuration(duration)]]) : null,
    );
  }

  function toolPanel() {
    const hint = h('p', { class: 'meta' }, toolHint(tool, 'image'));
    if (tool === 'crop') {
      return section(
        'Recadrer et pivoter',
        hint,
        choices(CROP_RATIOS.map((entry) => [entry.id, entry.label]), cropRatio, (id) => { cropRatio = id; startCrop(); renderPanel(); }, 'Format du recadrage', 'is-grid-3'),
        h('button', { class: 'btn is-primary is-wide', type: 'button', onClick: applyCrop }, icon('crop'), 'Appliquer le recadrage'),
        h(
          'div',
          { class: 'row is-wrap' },
          h('button', { class: 'btn', type: 'button', title: 'Pivoter à gauche', 'aria-label': 'Pivoter à gauche', onClick: () => rotate(-1) }, icon('rotateLeft')),
          h('button', { class: 'btn', type: 'button', title: 'Pivoter à droite', 'aria-label': 'Pivoter à droite', onClick: () => rotate(1) }, icon('rotateRight')),
          h('button', { class: 'btn', type: 'button', title: 'Miroir horizontal', 'aria-label': 'Miroir horizontal', onClick: () => flip(true) }, icon('flipH')),
          h('button', { class: 'btn', type: 'button', title: 'Miroir vertical', 'aria-label': 'Miroir vertical', onClick: () => flip(false) }, icon('flipV')),
        ),
      );
    }
    if (tool === 'draw') {
      return section(
        'Dessiner',
        hint,
        choices([['pen', 'Pinceau', 'edit'], ['marker', 'Surligneur', 'marker'], ['eraser', 'Gomme', 'eraser']], style.drawMode, (mode) => { style.drawMode = mode; renderPanel(); }, 'Outil de dessin'),
        style.drawMode === 'eraser' ? h('p', { class: 'meta' }, 'La gomme efface ce que tu as ajouté, jamais l’image.') : colorRow(style.color, (color) => { style.color = color; renderPanel(); }),
        slider({ label: 'Épaisseur', min: 1, max: 40, value: style.stroke, defaultValue: styleDefaults.stroke, onInput: (value) => { style.stroke = value; } }),
      );
    }
    if (tool === 'shape') {
      return section(
        'Formes',
        hint,
        choices([['arrow', 'Flèche', 'arrow'], ['rect', 'Rectangle', 'rect'], ['ellipse', 'Ellipse', 'ellipse'], ['line', 'Trait', 'line']], style.shape, (shape) => { style.shape = shape; renderPanel(); }, 'Forme', 'is-grid-2'),
        colorRow(style.color, (color) => { style.color = color; renderPanel(); }),
        slider({ label: 'Épaisseur', min: 1, max: 40, value: style.stroke, defaultValue: styleDefaults.stroke, onInput: (value) => { style.stroke = value; } }),
        style.shape === 'rect' || style.shape === 'ellipse' ? toggle('Forme pleine', style.fill, (fill) => { style.fill = fill; }) : null,
      );
    }
    if (tool === 'text') {
      const meme = (text, top) => {
        const size = Math.round(doc.base.width / 9);
        addText(text, doc.base.width / 2, top ? size * 0.85 : doc.base.height - size * 0.85, { font: 'meme', color: '#ffffff', outline: true, size });
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
        choices(FONT_LABELS, style.font, (font) => { style.font = font; renderPanel(); }, 'Police', 'is-grid-2'),
        colorRow(style.textColor, (color) => { style.textColor = color; renderPanel(); }),
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
                const object = addText(emoji, doc.base.width / 2, doc.base.height / 2, { font: 'emoji', outline: false, color: '#ffffff', size: Math.round(60 * unit()) });
                renderPanel();
                select(object.id);
                repaint();
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
        choices([['pixel', 'Pixels'], ['blur', 'Flou']], style.maskMode, (mode) => { style.maskMode = mode; renderPanel(); }, 'Type de masque', 'is-grid-2'),
        slider({ label: 'Intensité', min: 2, max: 20, value: style.maskStrength, defaultValue: styleDefaults.maskStrength, onInput: (value) => { style.maskStrength = value; } }),
      );
    }
    if (tool === 'display') {
      return displayScalePanel();
    }
    if (tool === 'adjust') {
      const adjustSlider = (label, key, min, max) =>
        slider({
          label,
          min,
          max,
          value: doc.adjust[key],
          defaultValue: NEUTRAL_ADJUST[key],
          format: key === 'blur' ? (value) => String(value) : (value) => `${value}\u00A0%`,
          onInput: (value) => { doc.adjust[key] = value; repaint(); },
          onCommit: commit,
        });
      return section(
        'Lumière et couleurs',
        hint,
        choices(FILTER_PRESETS.map((preset) => [preset.id, preset.label]), doc.adjust.preset, (preset) => { doc.adjust.preset = preset; commit(); repaint(); renderPanel(); }, 'Filtre', 'is-grid-2'),
        adjustSlider('Luminosité', 'brightness', 40, 160),
        adjustSlider('Contraste', 'contrast', 40, 160),
        adjustSlider('Saturation', 'saturate', 0, 200),
        adjustSlider('Flou', 'blur', 0, 10),
        h('button', { class: 'btn', type: 'button', onClick: () => { doc.adjust = { ...NEUTRAL_ADJUST }; commit(); repaint(); renderPanel(); } }, 'Réinitialiser les ajustements'),
      );
    }
    return emptySelectionPanel();
  }

  // --- media summary (bottom of the panel) ---
  // A picture has no duration: the block says what the file is and what the
  // crop, the rotation and the display tool leave it at.
  function mediaFacts() {
    const onScreen = allowDisplayScale ? displayedSize() : null;
    return mediaInfoFacts({
      name: blob && blob.name ? blob.name : '',
      mime: blob ? blob.type : '',
      bytes: blob && blob.size > 0 ? blob.size : 0,
      kind: 'image',
      width: sourceWidth,
      height: sourceHeight,
      resultWidth: doc.base.width,
      resultHeight: doc.base.height,
      displayWidth: onScreen ? onScreen.width : 0,
      displayHeight: onScreen ? onScreen.height : 0,
    });
  }

  let infoNode = null;

  // The block keeps its node: the display tool and the history refresh its
  // rows without rebuilding the panel (a rebuilt panel would drop the slider).
  function syncInfoNode() {
    if (!infoNode) {
      return;
    }
    const rows = mediaFacts();
    infoNode.hidden = rows.length === 0;
    infoNode.replaceChildren(h('h3', null, 'Le média'), facts(rows));
  }

  function infoPanel() {
    if (!infoNode) {
      infoNode = h('section', { class: 'editor-info', 'aria-label': 'Le média' });
    }
    syncInfoNode();
    return infoNode;
  }

  function renderPanel() {
    const object = selected();
    const sections = [];
    if (tool !== 'select' || !object) {
      sections.push(toolPanel());
    }
    if (object) {
      sections.push(selectionPanel(object));
    }
    frame.panel.replaceChildren(...sections.filter(Boolean), infoPanel());
    frame.panel.hidden = false;

    const rail = [];
    for (const group of railGroups('image', { allowDisplayScale: Boolean(allowDisplayScale) })) {
      if (rail.length) {
        rail.push(h('div', { class: 'tool-sep', role: 'separator' }));
      }
      rail.push(
        ...group.map(({ id, label, glyph }) =>
          h('button', { class: `tool${tool === id ? ' is-active' : ''}`, type: 'button', 'aria-pressed': String(tool === id), onClick: () => setTool(id) }, icon(glyph), h('span', null, label)),
        ),
      );
    }
    frame.rail.replaceChildren(...rail);
    frame.rail.hidden = false;
  }

  function setTool(next) {
    if (tool === 'crop' && next !== 'crop') {
      stopCrop();
    }
    tool = next;
    if (['draw', 'shape', 'blur', 'text', 'crop', 'display'].includes(tool)) {
      selectedId = null;
    }
    wrap.dataset.tool = tool;
    if (tool === 'crop') {
      startCrop();
    }
    renderPanel();
    placeSelection();
    syncStage();
  }

  setTool('select');
  layout();

  return {
    canUndo: () => historyIndex > 0,
    canRedo: () => historyIndex < history.length - 1,
    undo: () => {
      if (historyIndex > 0) {
        restore(historyIndex - 1);
      }
    },
    redo: () => {
      if (historyIndex < history.length - 1) {
        restore(historyIndex + 1);
      }
    },
    isDirty: () => history[historyIndex] !== initial || displayScale !== initialScale,
    pixelsDirty: () => history[historyIndex] !== initial,
    scaleDirty: () => displayScale !== initialScale,
    displayScalePercent: () => displayScaleForStorage(displayScale),
    onKey(event, typing) {
      if (typing) {
        return false;
      }
      if ((event.key === 'Delete' || event.key === 'Backspace') && selected()) {
        removeSelection();
        return true;
      }
      if (event.key === 'Enter' && tool === 'crop') {
        applyCrop();
        return true;
      }
      if (event.key === 'Escape') {
        if (tool === 'crop') {
          setTool('select');
          return true;
        }
        if (selectedId !== null) {
          select(null);
          return true;
        }
        return false;
      }
      const arrows = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
      if (arrows[event.key] && selected()) {
        const step = (event.shiftKey ? 10 : 1) * unit();
        replaceObject(translated(selected(), arrows[event.key][0] * step, arrows[event.key][1] * step));
        commit();
        repaint();
        return true;
      }
      return false;
    },
    // Renders the result at full size. A JPEG has no transparency: it gets a
    // white background.
    exportBlob(mime) {
      const output = makeCanvas(doc.base.width, doc.base.height);
      compose(output, doc.adjust);
      let source = output;
      if (mime === 'image/jpeg') {
        source = makeCanvas(output.width, output.height);
        const context = source.getContext('2d');
        context.fillStyle = '#ffffff';
        context.fillRect(0, 0, source.width, source.height);
        context.drawImage(output, 0, 0);
      }
      repaint();
      return new Promise((resolve, reject) => {
        source.toBlob((result) => (result ? resolve(result) : reject(new Error('export failed'))), mime, 0.92);
      });
    },
    destroy() {
      resizeObserver.disconnect();
      stageZoom.destroy();
      preview.destroy();
      stopCrop();
    },
  };
}
