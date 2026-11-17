// Image editor: crop, rotate, flip, draw, shapes, text, emoji, masking and
// colour adjustments, all on a canvas inside the board.
//
// The document is a base picture plus a list of objects (strokes, shapes,
// texts, masks) in the base picture's pixel space, and a colour filter
// applied to the whole result. Crop, rotation and flips bake the objects into
// a new base picture. Every committed action is a snapshot: undo and redo
// walk through them.

import { h, icon, registerIcons } from '../../core/dom.js';
import { CROP_RATIOS, FILTER_PRESETS, NEUTRAL_ADJUST, centeredCrop, clamp, cssFilter, fitScale } from '../../core/editor-helpers.js';
import { createCropOverlay } from './crop-overlay.js';
import { choices, section, slider, toggle } from './frame.js';

registerIcons({
  cursor: 'M6 4l12 8-5 1.5L10.5 19z',
  shapes: 'M4 4h8v8H4zM20 16a4 4 0 11-8 0 4 4 0 018 0z',
  text: 'M5 7V5h14v2M12 5v14M9 19h6',
  smile: 'M21 12a9 9 0 11-18 0 9 9 0 0118 0zM8.5 14.5a4.5 4.5 0 007 0M9 9.5v.5M15 9.5v.5',
  pixels: 'M4 4h5v5H4zM14 4h6v5h-6zM9 9h5v6H9zM4 15h5v5H4zM14 15h6v5h-6z',
  sliders: 'M4 7h10M18 7h2M4 17h2M10 17h10M16 5v4M8 15v4',
  eraser: 'M4 16l9-9 6 6-6 6H8zM9 11l6 6',
  marker: 'M5 20h14M8 16l8-10 3 2.5-8 10H8z',
  arrow: 'M5 19L19 5M10 5h9v9',
  line: 'M5 19L19 5',
  rect: 'M4 6h16v12H4z',
  ellipse: 'M21 12a9 6 0 11-18 0 9 6 0 0118 0z',
});

// Largest side kept while editing: a bigger picture is scaled down once.
const MAX_SIDE = 4096;
const HISTORY_LIMIT = 40;
const COLORS = ['#ffffff', '#000000', '#ec6a70', '#f2b544', '#f7e04b', '#5fc9a3', '#7fa8f5', '#c58af9'];
const EMOJIS = ['😂', '🤣', '😭', '💀', '😎', '🥶', '🤡', '😱', '🤔', '😡', '🥵', '🤯', '👀', '🔥', '💯', '❤️', '👍', '👎', '🙏', '💩', '🎉', '⭐', '❌', '✅', '⚠️', '➡️', '⬅️', '⬆️', '⬇️', '❓', '❗', '💬'];
const FONTS = {
  meme: 'Impact, "Arial Black", sans-serif',
  sans: '"Segoe UI", system-ui, sans-serif',
  serif: 'Georgia, "Times New Roman", serif',
  hand: '"Segoe Print", "Comic Sans MS", cursive',
  emoji: '"Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif',
};
const FONT_LABELS = [['meme', 'Mème'], ['sans', 'Simple'], ['serif', 'Classique'], ['hand', 'Manuscrite']];
const TOOLS = [
  ['select', 'Sélection', 'cursor'],
  ['crop', 'Recadrer', 'crop'],
  ['draw', 'Dessiner', 'edit'],
  ['shape', 'Formes', 'shapes'],
  ['text', 'Texte', 'text'],
  ['emoji', 'Emoji', 'smile'],
  ['blur', 'Masquer', 'pixels'],
  ['adjust', 'Réglages', 'sliders'],
];
const TOOL_HINTS = {
  select: 'Clique un élément pour le déplacer, le redimensionner ou le supprimer.',
  crop: 'Déplace les poignées, puis applique. Entrée valide.',
  draw: 'Dessine directement sur l’image.',
  shape: 'Trace la forme en glissant sur l’image.',
  text: 'Clique sur l’image pour y poser un texte.',
  emoji: 'Choisis un emoji : il se pose au centre, déplace-le ensuite.',
  blur: 'Trace un rectangle sur la zone à cacher.',
  adjust: 'Les réglages s’appliquent à toute l’image.',
};

function makeCanvas(width, height) {
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(width));
  canvas.height = Math.max(1, Math.round(height));
  return canvas;
}

function ensureSize(canvas, width, height) {
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
}

function isDark(color) {
  const match = /^#([0-9a-f]{6})$/i.exec(color);
  if (!match) {
    return false;
  }
  const value = parseInt(match[1], 16);
  const luma = 0.299 * (value >> 16) + 0.587 * ((value >> 8) & 255) + 0.114 * (value & 255);
  return luma < 110;
}

function fontOf(object) {
  const weight = object.font === 'meme' || object.font === 'emoji' ? '' : '600 ';
  return `${weight}${object.size}px ${FONTS[object.font] || FONTS.sans}`;
}

function linesOf(object) {
  const text = String(object.text || '');
  return (object.font === 'meme' ? text.toLocaleUpperCase('fr-FR') : text).split('\n');
}

const measureContext = makeCanvas(1, 1).getContext('2d');

// Bounding box of an object, in base pixels.
function boundsOf(object) {
  if (object.type === 'text') {
    measureContext.font = fontOf(object);
    const lines = linesOf(object);
    const width = Math.max(object.size * 0.6, ...lines.map((line) => measureContext.measureText(line).width));
    const height = lines.length * object.size * 1.18;
    const pad = object.size * 0.12;
    return { x: object.x - width / 2 - pad, y: object.y - height / 2 - pad, w: width + pad * 2, h: height + pad * 2 };
  }
  if (object.type === 'stroke') {
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (let index = 0; index < object.points.length; index += 2) {
      minX = Math.min(minX, object.points[index]);
      maxX = Math.max(maxX, object.points[index]);
      minY = Math.min(minY, object.points[index + 1]);
      maxY = Math.max(maxY, object.points[index + 1]);
    }
    const pad = object.width / 2;
    return { x: minX - pad, y: minY - pad, w: maxX - minX + pad * 2, h: maxY - minY + pad * 2 };
  }
  const pad = object.type === 'shape' ? object.width / 2 : 0;
  const x = Math.min(object.x1, object.x2);
  const y = Math.min(object.y1, object.y2);
  return { x: x - pad, y: y - pad, w: Math.abs(object.x2 - object.x1) + pad * 2, h: Math.abs(object.y2 - object.y1) + pad * 2 };
}

function translated(object, dx, dy) {
  const copy = structuredClone(object);
  if (copy.type === 'text') {
    copy.x += dx;
    copy.y += dy;
  } else if (copy.type === 'stroke') {
    for (let index = 0; index < copy.points.length; index += 2) {
      copy.points[index] += dx;
      copy.points[index + 1] += dy;
    }
  } else {
    copy.x1 += dx;
    copy.x2 += dx;
    copy.y1 += dy;
    copy.y2 += dy;
  }
  return copy;
}

// Scales an object around `anchor` (the top-left corner of its box).
function scaled(object, anchor, factor) {
  const copy = structuredClone(object);
  const mapX = (x) => anchor.x + (x - anchor.x) * factor;
  const mapY = (y) => anchor.y + (y - anchor.y) * factor;
  if (copy.type === 'text') {
    copy.x = mapX(copy.x);
    copy.y = mapY(copy.y);
    copy.size = Math.max(4, copy.size * factor);
  } else if (copy.type === 'stroke') {
    for (let index = 0; index < copy.points.length; index += 2) {
      copy.points[index] = mapX(copy.points[index]);
      copy.points[index + 1] = mapY(copy.points[index + 1]);
    }
    copy.width = Math.max(1, copy.width * factor);
  } else {
    copy.x1 = mapX(copy.x1);
    copy.x2 = mapX(copy.x2);
    copy.y1 = mapY(copy.y1);
    copy.y2 = mapY(copy.y2);
    if (copy.type === 'shape') {
      copy.width = Math.max(1, copy.width * factor);
    }
  }
  return copy;
}

function paintObject(context, object) {
  context.save();
  context.lineCap = 'round';
  context.lineJoin = 'round';
  if (object.type === 'stroke') {
    context.lineWidth = object.width;
    context.strokeStyle = object.color;
    context.fillStyle = object.color;
    if (object.mode === 'eraser') {
      context.globalCompositeOperation = 'destination-out';
    } else if (object.mode === 'marker') {
      context.globalAlpha = 0.45;
    }
    const points = object.points;
    context.beginPath();
    if (points.length <= 2) {
      context.arc(points[0], points[1], object.width / 2, 0, Math.PI * 2);
      context.fill();
    } else {
      context.moveTo(points[0], points[1]);
      for (let index = 2; index < points.length; index += 2) {
        context.lineTo(points[index], points[index + 1]);
      }
      context.stroke();
    }
  } else if (object.type === 'shape') {
    context.lineWidth = object.width;
    context.strokeStyle = object.color;
    context.fillStyle = object.color;
    const { x1, y1, x2, y2 } = object;
    context.beginPath();
    if (object.shape === 'rect') {
      context.lineJoin = 'miter';
      context.rect(Math.min(x1, x2), Math.min(y1, y2), Math.abs(x2 - x1), Math.abs(y2 - y1));
    } else if (object.shape === 'ellipse') {
      context.ellipse((x1 + x2) / 2, (y1 + y2) / 2, Math.abs(x2 - x1) / 2, Math.abs(y2 - y1) / 2, 0, 0, Math.PI * 2);
    } else if (object.shape === 'arrow') {
      const angle = Math.atan2(y2 - y1, x2 - x1);
      const head = Math.min(Math.hypot(x2 - x1, y2 - y1), object.width * 4);
      const baseX = x2 - Math.cos(angle) * head;
      const baseY = y2 - Math.sin(angle) * head;
      context.moveTo(x1, y1);
      context.lineTo(baseX, baseY);
      context.stroke();
      context.beginPath();
      context.moveTo(x2, y2);
      context.lineTo(baseX - Math.sin(angle) * head * 0.55, baseY + Math.cos(angle) * head * 0.55);
      context.lineTo(baseX + Math.sin(angle) * head * 0.55, baseY - Math.cos(angle) * head * 0.55);
      context.closePath();
      context.fill();
      context.restore();
      return;
    } else {
      context.moveTo(x1, y1);
      context.lineTo(x2, y2);
    }
    if (object.fill && (object.shape === 'rect' || object.shape === 'ellipse')) {
      context.fill();
    } else {
      context.stroke();
    }
  } else if (object.type === 'text') {
    const lines = linesOf(object);
    const lineHeight = object.size * 1.18;
    const top = object.y - ((lines.length - 1) * lineHeight) / 2;
    context.font = fontOf(object);
    context.textAlign = 'center';
    context.textBaseline = 'middle';
    if (object.outline && object.font !== 'emoji') {
      context.lineWidth = Math.max(1, object.size / 7);
      context.strokeStyle = isDark(object.color) ? '#ffffff' : '#000000';
      lines.forEach((line, index) => context.strokeText(line, object.x, top + index * lineHeight));
    }
    context.fillStyle = object.color;
    lines.forEach((line, index) => context.fillText(line, object.x, top + index * lineHeight));
  }
  context.restore();
}

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
// action. Resolves to the editor's controls; rejects when the picture cannot
// be decoded.
export async function createImageEditor({ blob, frame, onChange }) {
  const doc = { base: await loadBase(blob), objects: [], adjust: { ...NEUTRAL_ADJUST } };
  let nextId = 1;
  let tool = 'select';
  let selectedId = null;
  let drag = null;
  let cropRatio = 'free';
  let paintQueued = false;
  const style = { color: '#ec6a70', stroke: 4, drawMode: 'pen', shape: 'arrow', fill: false, font: 'meme', textSize: 30, outline: true, textColor: '#ffffff', maskMode: 'pixel', maskStrength: 6 };

  const scratch = makeCanvas(1, 1);
  const layer = makeCanvas(1, 1);
  const tiny = makeCanvas(1, 1);
  const view = h('canvas', { class: 'img-canvas' });
  const selBox = h('div', { class: 'sel-box', hidden: true }, h('span', { class: 'sel-handle' }));
  const wrap = h('div', { class: 'img-wrap' }, view, selBox);
  frame.stage.replaceChildren(wrap);

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
      compose(view, doc.adjust);
      placeSelection();
    });
  }

  function layout() {
    const availableWidth = frame.stage.clientWidth - 48;
    const availableHeight = frame.stage.clientHeight - 48;
    if (availableWidth <= 0 || availableHeight <= 0) {
      return;
    }
    const scale = Math.min(availableWidth / doc.base.width, availableHeight / doc.base.height, 8);
    const cssWidth = Math.max(1, Math.round(doc.base.width * scale));
    const cssHeight = Math.max(1, Math.round(doc.base.height * scale));
    wrap.style.width = `${cssWidth}px`;
    wrap.style.height = `${cssHeight}px`;
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    ensureSize(view, Math.round(cssWidth * ratio), Math.round(cssHeight * ratio));
    frame.setStatus(`${doc.base.width} × ${doc.base.height} px`);
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
      replaceObject(scaled(drag.origin, drag.anchor, factor));
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
  function colorRow(current, onPick) {
    const custom = h('input', { type: 'color', class: 'swatch-custom', value: /^#[0-9a-f]{6}$/i.test(current) ? current : '#ffffff', title: 'Autre couleur', onChange: () => onPick(custom.value) });
    return h(
      'div',
      { class: 'swatches', role: 'group', 'aria-label': 'Couleur' },
      COLORS.map((color) =>
        h('button', { class: `swatch${color === current ? ' is-active' : ''}`, type: 'button', style: `background:${color}`, 'aria-label': `Couleur ${color}`, 'aria-pressed': String(color === current), onClick: () => onPick(color) }),
      ),
      custom,
    );
  }

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

  function toolPanel() {
    const hint = h('p', { class: 'meta' }, TOOL_HINTS[tool]);
    if (tool === 'crop') {
      return section(
        'Recadrer et pivoter',
        hint,
        choices(CROP_RATIOS.map((entry) => [entry.id, entry.label]), cropRatio, (id) => { cropRatio = id; startCrop(); renderPanel(); }, 'Format du recadrage'),
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
        slider({ label: 'Épaisseur', min: 1, max: 40, value: style.stroke, onInput: (value) => { style.stroke = value; } }),
      );
    }
    if (tool === 'shape') {
      return section(
        'Formes',
        hint,
        choices([['arrow', 'Flèche', 'arrow'], ['rect', 'Rectangle', 'rect'], ['ellipse', 'Ellipse', 'ellipse'], ['line', 'Trait', 'line']], style.shape, (shape) => { style.shape = shape; renderPanel(); }, 'Forme'),
        colorRow(style.color, (color) => { style.color = color; renderPanel(); }),
        slider({ label: 'Épaisseur', min: 1, max: 40, value: style.stroke, onInput: (value) => { style.stroke = value; } }),
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
          { class: 'row is-wrap' },
          h('button', { class: 'btn', type: 'button', onClick: () => meme('Texte du haut', true) }, 'Texte en haut'),
          h('button', { class: 'btn', type: 'button', onClick: () => meme('Texte du bas', false) }, 'Texte en bas'),
        ),
        choices(FONT_LABELS, style.font, (font) => { style.font = font; renderPanel(); }, 'Police'),
        colorRow(style.textColor, (color) => { style.textColor = color; renderPanel(); }),
        slider({ label: 'Taille', min: 4, max: 160, value: style.textSize, onInput: (value) => { style.textSize = value; } }),
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
        choices([['pixel', 'Pixels'], ['blur', 'Flou']], style.maskMode, (mode) => { style.maskMode = mode; renderPanel(); }, 'Type de masque'),
        slider({ label: 'Intensité', min: 2, max: 20, value: style.maskStrength, onInput: (value) => { style.maskStrength = value; } }),
      );
    }
    if (tool === 'adjust') {
      const adjustSlider = (label, key, min, max) =>
        slider({
          label,
          min,
          max,
          value: doc.adjust[key],
          format: key === 'blur' ? (value) => String(value) : (value) => `${value} %`,
          onInput: (value) => { doc.adjust[key] = value; repaint(); },
          onCommit: commit,
        });
      return section(
        'Réglages',
        hint,
        choices(FILTER_PRESETS.map((preset) => [preset.id, preset.label]), doc.adjust.preset, (preset) => { doc.adjust.preset = preset; commit(); repaint(); renderPanel(); }, 'Filtre'),
        adjustSlider('Luminosité', 'brightness', 40, 160),
        adjustSlider('Contraste', 'contrast', 40, 160),
        adjustSlider('Saturation', 'saturate', 0, 200),
        adjustSlider('Flou', 'blur', 0, 10),
        h('button', { class: 'btn', type: 'button', onClick: () => { doc.adjust = { ...NEUTRAL_ADJUST }; commit(); repaint(); renderPanel(); } }, 'Réinitialiser les réglages'),
      );
    }
    return section('Sélection', hint);
  }

  function renderPanel() {
    const object = selected();
    frame.panel.replaceChildren(...[toolPanel(), object ? selectionPanel(object) : null].filter(Boolean));
    frame.panel.hidden = false;
    frame.rail.replaceChildren(
      ...TOOLS.map(([id, label, glyph]) =>
        h('button', { class: `tool${tool === id ? ' is-active' : ''}`, type: 'button', 'aria-pressed': String(tool === id), onClick: () => setTool(id) }, icon(glyph), h('span', null, label)),
      ),
    );
    frame.rail.hidden = false;
  }

  function setTool(next) {
    if (tool === 'crop' && next !== 'crop') {
      stopCrop();
    }
    tool = next;
    if (['draw', 'shape', 'blur', 'text', 'crop'].includes(tool)) {
      selectedId = null;
    }
    wrap.dataset.tool = tool;
    if (tool === 'crop') {
      startCrop();
    }
    renderPanel();
    placeSelection();
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
    isDirty: () => history[historyIndex] !== initial,
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
      stopCrop();
    },
  };
}
