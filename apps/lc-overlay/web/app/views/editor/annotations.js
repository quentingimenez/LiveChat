// Drawing model shared by the picture editor and the video editor: strokes,
// shapes, texts, emojis and masks live in the pixel space of the source frame,
// so the same objects can be painted on a canvas (a picture) or on top of a
// playing video.
//
// The server burns the masks first, then the drawing, then the colour
// adjustments, before the crop and the rotation: the editors paint in that
// order too.

import { h, registerIcons } from '../../core/dom.js';
import { NEUTRAL_ADJUST } from '../../core/editor-helpers.js';

registerIcons({
  cursor: 'M6 4l12 8-5 1.5L10.5 19z',
  shapes: 'M4 4h8v8H4zM20 16a4 4 0 11-8 0 4 4 0 018 0z',
  text: 'M5 7V5h14v2M12 5v14M9 19h6',
  smile: 'M21 12a9 9 0 11-18 0 9 9 0 0118 0zM8.5 14.5a4.5 4.5 0 007 0M9 9.5v.5M15 9.5v.5',
  pixels: 'M4 4h5v5H4zM14 4h6v5h-6zM9 9h5v6H9zM4 15h5v5H4zM14 15h6v5h-6z',
  sliders: 'M4 7h10M18 7h2M4 17h2M10 17h10M16 5v4M8 15v4',
  display: 'M4 9V4h5M20 15v5h-5M4 15v5h5M16 4h4v5',
  eraser: 'M4 16l9-9 6 6-6 6H8zM9 11l6 6',
  marker: 'M5 20h14M8 16l8-10 3 2.5-8 10H8z',
  arrow: 'M5 19L19 5M10 5h9v9',
  line: 'M5 19L19 5',
  rect: 'M4 6h16v12H4z',
  ellipse: 'M21 12a9 6 0 11-18 0 9 6 0 0118 0z',
});

export const COLORS = ['#ffffff', '#000000', '#ec6a70', '#f2b544', '#f7e04b', '#5fc9a3', '#7fa8f5', '#c58af9'];
export const EMOJIS = ['😂', '🤣', '😭', '💀', '😎', '🥶', '🤡', '😱', '🤔', '😡', '🥵', '🤯', '👀', '🔥', '💯', '❤️', '👍', '👎', '🙏', '💩', '🎉', '⭐', '❌', '✅', '⚠️', '➡️', '⬅️', '⬆️', '⬇️', '❓', '❗', '💬'];
export const FONTS = {
  meme: 'Impact, "Arial Black", sans-serif',
  sans: '"Segoe UI", system-ui, sans-serif',
  serif: 'Georgia, "Times New Roman", serif',
  hand: '"Segoe Print", "Comic Sans MS", cursive',
  emoji: '"Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif',
};
export const FONT_LABELS = [['meme', 'Mème'], ['sans', 'Simple'], ['serif', 'Classique'], ['hand', 'Manuscrite']];

// Shipped default of every tool setting: a double click on a slider puts it
// back there.
export function annotationStyle() {
  return {
    color: '#ec6a70',
    stroke: 4,
    drawMode: 'pen',
    shape: 'arrow',
    fill: false,
    font: 'meme',
    textSize: 30,
    outline: true,
    textColor: '#ffffff',
    maskMode: 'pixel',
    maskStrength: 6,
  };
}

// Default state of the colour adjustments of a video edit.
export function defaultAdjust() {
  return { ...NEUTRAL_ADJUST };
}

export function makeCanvas(width, height) {
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(width));
  canvas.height = Math.max(1, Math.round(height));
  return canvas;
}

export function ensureSize(canvas, width, height) {
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
}

export function isDark(color) {
  const match = /^#([0-9a-f]{6})$/i.exec(color);
  if (!match) {
    return false;
  }
  const value = parseInt(match[1], 16);
  const luma = 0.299 * (value >> 16) + 0.587 * ((value >> 8) & 255) + 0.114 * (value & 255);
  return luma < 110;
}

export function fontOf(object) {
  const weight = object.font === 'meme' || object.font === 'emoji' ? '' : '600 ';
  return `${weight}${object.size}px ${FONTS[object.font] || FONTS.sans}`;
}

export function linesOf(object) {
  const text = String(object.text || '');
  return (object.font === 'meme' ? text.toLocaleUpperCase('fr-FR') : text).split('\n');
}

// Measuring needs a canvas, which a headless test cannot build: it is made on
// first use, not at import time.
let measure = null;
function measureContext() {
  if (!measure) {
    measure = makeCanvas(1, 1).getContext('2d');
  }
  return measure;
}

// Bounding box of an object, in source pixels.
export function boundsOf(object) {
  if (object.type === 'text') {
    const context = measureContext();
    context.font = fontOf(object);
    const lines = linesOf(object);
    const width = Math.max(object.size * 0.6, ...lines.map((line) => context.measureText(line).width));
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

export function translated(object, dx, dy) {
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
export function scaled(object, anchor, factor) {
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

// Moves the bottom-right corner of a mask without touching the top-left one: a
// mask is a rectangle, both sides are free. `anchor` is its top-left corner in
// source pixels, `min` the smallest side kept (source pixels).
export function resizedMask(object, anchor, point, min = 0) {
  const copy = structuredClone(object);
  copy.x1 = anchor.x;
  copy.y1 = anchor.y;
  copy.x2 = Math.max(anchor.x + min, point.x);
  copy.y2 = Math.max(anchor.y + min, point.y);
  return copy;
}

// Resizes a selected object from its bottom-right corner: a mask follows the
// pointer on both axes, everything else keeps its own ratio (`scaled`).
export function resized(object, anchor, point, factor, min = 0) {
  return object.type === 'mask' ? resizedMask(object, anchor, point, min) : scaled(object, anchor, factor);
}

// Keeps both corners of a mask inside the frame. The video preview is clipped,
// so a corner dragged past the picture would hide the handle that resizes it.
export function clampedMask(object, width, height) {
  const copy = structuredClone(object);
  copy.x1 = Math.min(Math.max(copy.x1, 0), width);
  copy.y1 = Math.min(Math.max(copy.y1, 0), height);
  copy.x2 = Math.min(Math.max(copy.x2, 0), width);
  copy.y2 = Math.min(Math.max(copy.y2, 0), height);
  return copy;
}

// Translation that keeps a whole box inside the frame: a mask dragged against
// an edge stops there instead of sliding past it.
export function clampedDelta(box, width, height, dx, dy) {
  const minX = Math.min(0, -box.x);
  const maxX = Math.max(minX, width - box.x - box.w);
  const minY = Math.min(0, -box.y);
  const maxY = Math.max(minY, height - box.y - box.h);
  return { dx: Math.min(Math.max(dx, minX), maxX) || 0, dy: Math.min(Math.max(dy, minY), maxY) || 0 };
}

export function paintObject(context, object) {
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

// Paints every drawn object (masks excluded: they are not drawing) in source
// pixels, with a transparent background. This is the picture sent to the
// server as the annotation overlay.
export function paintAnnotations(target, objects) {
  const context = target.getContext('2d');
  context.setTransform(1, 0, 0, 1, 0, 0);
  context.clearRect(0, 0, target.width, target.height);
  paintAnnotationsOn(context, objects);
}

// The same drawing in an existing context, already transformed into the
// source frame's pixels: the video preview composes it with the video.
export function paintAnnotationsOn(context, objects) {
  for (const object of objects) {
    if (object.type !== 'mask') {
      paintObject(context, object);
    }
  }
}

// The drawing as a PNG data URL, or null when there is nothing to send.
export function annotationsToDataUrl(objects, width, height) {
  if (!objects.some((object) => object.type !== 'mask')) {
    return null;
  }
  const canvas = makeCanvas(width, height);
  paintAnnotations(canvas, objects);
  return canvas.toDataURL('image/png');
}

// A row of colour swatches plus the native picker, for the object panels.
export function colorRow(current, onPick) {
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
