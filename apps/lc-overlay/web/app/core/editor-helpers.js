// Pure helpers for the media editor (no DOM, no state): kept ranges of an
// audio or a video, the edit recipe sent to the server, crop geometry and
// file naming. This module must also load under Node for its unit tests.

// Shortest range worth keeping, in seconds (the server refuses less).
export const MIN_KEEP_SEC = 0.1;
export const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];
// Speed range the server accepts.
export const MIN_SPEED = 0.25;
export const MAX_SPEED = 4;
export const MAX_VOLUME = 3;
export const MAX_FADE_SEC = 10;

export function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function roundMs(value) {
  return Math.round(value * 1000) / 1000;
}

// --- molette -------------------------------------------------------------
// Ctrl + molette zoome ; sans Ctrl, ni la frise ni l'aperçu ne changent de
// zoom (la molette garde son rôle de défilement).
export const MIN_STAGE_ZOOM = 1;
export const MAX_STAGE_ZOOM = 8;

// Un facteur continu : la molette et le pincement d'un trackpad avancent du
// même pas. deltaMode 0 = pixels, 1 = lignes, 2 = pages.
export function wheelZoomFactor(deltaY, deltaMode = 0) {
  const unit = deltaMode === 1 ? 16 : deltaMode === 2 ? 400 : 1;
  return Math.exp(-clamp(Number(deltaY) || 0, -4000, 4000) * unit * 0.0022);
}

// Le zoom demandé par un événement de molette, ou null quand ce n'est pas un
// zoom (sans Ctrl) ou qu'il est déjà à sa borne.
export function zoomedByWheel(zoom, event, { min = MIN_STAGE_ZOOM, max = MAX_STAGE_ZOOM } = {}) {
  if (!event || !(event.ctrlKey || event.metaKey)) {
    return null;
  }
  const factor = wheelZoomFactor(event.deltaY, event.deltaMode);
  const next = clamp(zoom * factor, min, max);
  return Math.abs(next - zoom) < 1e-4 ? null : next;
}

// Déplacement du défilement qui remet sous le pointeur le point du contenu
// qu'il survolait avant le redimensionnement.
export function anchorScroll({ scrollLeft = 0, scrollTop = 0, pointer, before, after }) {
  const fraction = (position, start, size) => (size > 0 ? clamp((position - start) / size, 0, 1) : 0.5);
  const x = fraction(pointer.x, before.left, before.width);
  const y = fraction(pointer.y, before.top, before.height);
  return {
    left: scrollLeft + (after.left + x * after.width - pointer.x),
    top: scrollTop + (after.top + y * after.height - pointer.y),
  };
}

// --- zoomed stage ----------------------------------------------------------
// The editor's stage scrolls virtually (see views/editor/stage-zoom.js). One
// axis of it: `full` is the zoomed content's size, `room` the frame's. Content
// that fits is centred and never scrolls; content that overflows shows a
// room-wide window from `scroll`, clamped to the content. `start` and `length`
// are that window in content pixels, `offset` where the content's own origin
// sits in the frame. Half a pixel of overflow is not worth a scrollbar.
export function stageAxis({ full, room, scroll = 0 }) {
  const size = Math.max(0, Number(full) || 0);
  const space = Math.max(0, Number(room) || 0);
  if (size <= space + 0.5) {
    return { overflow: false, scroll: 0, max: 0, start: 0, length: size, offset: (space - size) / 2 };
  }
  const max = size - space;
  const at = clamp(Number(scroll) || 0, 0, max);
  return { overflow: true, scroll: at, max, start: at, length: space, offset: -at };
}

// The scroll that keeps the content point under the pointer (in frame pixels)
// still while the content goes from `before.full` to `after` pixels. A
// pointer beside the content needs no clamp of its own: the scroll is
// clamped to the content anyway.
export function anchoredScroll({ pointer, room, before, after }) {
  const was = stageAxis({ full: before.full, room, scroll: before.scroll });
  const fraction = before.full > 0 ? (pointer - was.offset) / before.full : 0.5;
  return stageAxis({ full: after, room, scroll: fraction * after - pointer }).scroll;
}

// The scroll a plain wheel asks for, in pixels. Shift turns a vertical wheel
// into a horizontal scroll, like a native scroller.
export function wheelScrollDelta(event, room) {
  const unit = event.deltaMode === 1 ? { x: 40, y: 40 } : event.deltaMode === 2 ? { x: room.width, y: room.height } : { x: 1, y: 1 };
  let x = (Number(event.deltaX) || 0) * unit.x;
  let y = (Number(event.deltaY) || 0) * unit.y;
  if (event.shiftKey && x === 0) {
    x = y;
    y = 0;
  }
  return { x, y };
}

// The part of the source frame a window of the zoomed video preview shows.
// The displayed picture is `crop` (source pixels) turned by `turns` quarter
// turns clockwise, then mirrored, the order the server applies them, at
// `scale` CSS pixels per source pixel. `window` is a rectangle of that
// displayed picture from its top-left corner, in CSS pixels. The result is in
// source pixels, before the turn and the mirrors: what `.av-inner` holds.
export function videoViewBox({ crop, turns = 0, flipH = false, flipV = false, scale, window: view }) {
  const quarter = ((Math.round(Number(turns) || 0) % 4) + 4) % 4;
  const shown = rotatedSize(crop.width, crop.height, quarter);
  // The window around the centre of the displayed picture, in source pixels.
  let x0 = view.x / scale - shown.width / 2;
  let x1 = (view.x + view.width) / scale - shown.width / 2;
  let y0 = view.y / scale - shown.height / 2;
  let y1 = (view.y + view.height) / scale - shown.height / 2;
  // Undo the mirrors, then the turn: CSS turns a point (x, y) clockwise to
  // (-y, x), so each quarter back maps it to (y, -x).
  if (flipH) {
    [x0, x1] = [-x1, -x0];
  }
  if (flipV) {
    [y0, y1] = [-y1, -y0];
  }
  for (let step = 0; step < quarter; step += 1) {
    [x0, x1, y0, y1] = [y0, y1, -x1, -x0];
  }
  return {
    x: crop.x + crop.width / 2 + x0,
    y: crop.y + crop.height / 2 + y0,
    width: x1 - x0,
    height: y1 - y0,
  };
}

// --- kept ranges ---------------------------------------------------------
// The edit of an audio or a video is a list of kept ranges `[start, end]` in
// source seconds, ascending and never overlapping.

export function wholeSegments(duration) {
  return duration > 0 ? [[0, duration]] : [];
}

// Sorts, clamps to the media, merges touching ranges and drops empty ones.
export function normalizeSegments(segments, duration) {
  const total = Number(duration) > 0 ? Number(duration) : 0;
  const ranges = [];
  for (const segment of segments || []) {
    const start = clamp(Number(segment[0]), 0, total);
    const end = clamp(Number(segment[1]), 0, total);
    if (Number.isFinite(start) && Number.isFinite(end) && end - start > 0.001) {
      ranges.push([start, end]);
    }
  }
  ranges.sort((a, b) => a[0] - b[0]);
  const merged = [];
  for (const range of ranges) {
    const last = merged[merged.length - 1];
    if (last && range[0] <= last[1] + 0.001) {
      last[1] = Math.max(last[1], range[1]);
    } else {
      merged.push([range[0], range[1]]);
    }
  }
  return merged;
}

// Keeps only what lies inside `[start, end]`.
export function keepRange(segments, start, end) {
  const from = Math.min(start, end);
  const to = Math.max(start, end);
  const kept = [];
  for (const [a, b] of segments) {
    const left = Math.max(a, from);
    const right = Math.min(b, to);
    if (right - left > 0.001) {
      kept.push([left, right]);
    }
  }
  return kept;
}

// Removes `[start, end]` from the kept ranges.
export function cutRange(segments, start, end) {
  const from = Math.min(start, end);
  const to = Math.max(start, end);
  const kept = [];
  for (const [a, b] of segments) {
    if (to <= a || from >= b) {
      kept.push([a, b]);
      continue;
    }
    if (from - a > 0.001) {
      kept.push([a, from]);
    }
    if (b - to > 0.001) {
      kept.push([to, b]);
    }
  }
  return kept;
}

export function keptDuration(segments) {
  return segments.reduce((total, [start, end]) => total + (end - start), 0);
}

export function isWhole(segments, duration) {
  return segments.length === 1 && segments[0][0] <= 0.001 && segments[0][1] >= duration - 0.001;
}

// Ranges removed from the media, for the timeline.
export function removedRanges(segments, duration) {
  const removed = [];
  let cursor = 0;
  for (const [start, end] of segments) {
    if (start - cursor > 0.001) {
      removed.push([cursor, start]);
    }
    cursor = end;
  }
  if (duration - cursor > 0.001) {
    removed.push([cursor, duration]);
  }
  return removed;
}

// --- fenêtre visible de la frise ---------------------------------------
// La frise montre toujours une fenêtre [start, end] du média ; au repos elle
// couvre tout. Le zoom est borné : 0,2 s minimum, et cent fois au plus.
export const MIN_TIMELINE_SPAN_SEC = 0.2;
export const MAX_TIMELINE_ZOOM = 100;

export function defaultTimelineView(duration) {
  const total = Number.isFinite(duration) && duration > 0 ? duration : 0;
  return { start: 0, end: roundMs(total) };
}

export function minTimelineSpan(duration) {
  const total = Number.isFinite(duration) && duration > 0 ? duration : 0;
  return roundMs(Math.min(total, Math.max(MIN_TIMELINE_SPAN_SEC, total / MAX_TIMELINE_ZOOM)));
}

export function clampTimelineView(view, duration) {
  const total = Number.isFinite(duration) && duration > 0 ? duration : 0;
  if (!(total > 0)) {
    return { start: 0, end: 0 };
  }
  const min = minTimelineSpan(total);
  const startValue = Number.isFinite(view && view.start) ? view.start : 0;
  const endValue = Number.isFinite(view && view.end) ? view.end : total;
  const width = clamp(endValue - startValue, min, total);
  const start = roundMs(clamp(startValue, 0, total - width));
  return { start, end: roundMs(start + width) };
}

// Zoome d'un facteur `factor` (> 1 rapproche) en gardant `anchorTime` sous le
// pointeur ; la fenêtre reste dans le média.
export function zoomTimelineView(view, duration, anchorTime, factor) {
  const current = clampTimelineView(view, duration);
  const total = current.end - current.start > 0 ? current.end - current.start : 0;
  const full = Number.isFinite(duration) && duration > 0 ? duration : 0;
  if (!(total > 0) || !(full > 0)) {
    return current;
  }
  const rate = Number.isFinite(factor) && factor > 0 ? factor : 1;
  const width = clamp(total / rate, minTimelineSpan(full), full);
  const anchor = Number.isFinite(anchorTime) ? clamp(anchorTime, current.start, current.end) : current.start + total / 2;
  const ratio = clamp((anchor - current.start) / total, 0, 1);
  const start = clamp(anchor - ratio * width, 0, full - width);
  return { start: roundMs(start), end: roundMs(start + width) };
}

// Déplace la fenêtre de `seconds` (positif = vers la fin) sans sortir du média.
export function panTimelineView(view, duration, seconds) {
  const current = clampTimelineView(view, duration);
  const total = Number.isFinite(duration) && duration > 0 ? duration : 0;
  const width = current.end - current.start;
  if (!(total > 0) || width >= total - 0.001) {
    return current;
  }
  const delta = Number.isFinite(seconds) ? seconds : 0;
  const start = clamp(current.start + delta, 0, total - width);
  return { start: roundMs(start), end: roundMs(start + width) };
}

// Déplacement de la frise à la souris : le contenu suit le pointeur, donc un
// glissement vers la droite (dx > 0) remonte le temps. `width` est la largeur
// écran de la frise.
export function panTimelineByDrag(view, duration, dx, width) {
  const current = clampTimelineView(view, duration);
  const span = current.end - current.start;
  const pixels = Number.isFinite(width) && width > 0 ? width : 1;
  const move = Number.isFinite(dx) ? dx : 0;
  return panTimelineView(current, duration, -(move / pixels) * span);
}

// Ramène la fenêtre sur `time` quand la tête de lecture en sort. En avançant
// elle réapparaît à `lead` de la largeur (25 % par défaut), en reculant à
// droite : la lecture ne saute donc pas à chaque image.
export function revealTimelineTime(view, duration, time, lead = 0.25) {
  const current = clampTimelineView(view, duration);
  const total = Number.isFinite(duration) && duration > 0 ? duration : 0;
  const width = current.end - current.start;
  if (!(total > 0) || !(width > 0)) {
    return current;
  }
  const at = Number.isFinite(time) ? clamp(time, 0, total) : current.start;
  if (at >= current.start && at <= current.end) {
    return current;
  }
  const fraction = at > current.end ? clamp(lead, 0, 1) : 1 - clamp(lead, 0, 1);
  const start = clamp(at - width * fraction, 0, total - width);
  return { start: roundMs(start), end: roundMs(start + width) };
}

export function timelineZoomFactor(view, duration) {
  const raw = view && Number.isFinite(view.start) && Number.isFinite(view.end) ? view.end - view.start : 0;
  if (!(raw > 0)) {
    return 1;
  }
  const current = clampTimelineView(view, duration);
  const width = current.end - current.start;
  const total = Number.isFinite(duration) && duration > 0 ? duration : 0;
  return width > 0 ? total / width : 1;
}

// Where playback continues from `time`: `time` itself inside a kept range,
// the start of the next kept range inside a removed one, `null` after the
// last kept range.
export function nextKeptTime(segments, time) {
  for (const [start, end] of segments) {
    if (time < start) {
      return start;
    }
    if (time < end - 0.01) {
      return time;
    }
  }
  return null;
}

// Position in the result (seconds) of a source time.
export function sourceToOutputTime(segments, speed, time) {
  let elapsed = 0;
  for (const [start, end] of segments) {
    if (time <= start) {
      break;
    }
    elapsed += Math.min(time, end) - start;
  }
  return elapsed / (speed > 0 ? speed : 1);
}

export function outputDuration(segments, speed) {
  return keptDuration(segments) / (speed > 0 ? speed : 1);
}

// Volume factor of the fades at `time` of a result lasting `duration`.
export function fadeGain(time, duration, fadeIn, fadeOut) {
  let gain = 1;
  if (fadeIn > 0 && time < fadeIn) {
    gain = Math.min(gain, Math.max(0, time / fadeIn));
  }
  if (fadeOut > 0 && time > duration - fadeOut) {
    gain = Math.min(gain, Math.max(0, (duration - time) / fadeOut));
  }
  return gain;
}

// --- recipe --------------------------------------------------------------
export function defaultAvEdit(duration) {
  return {
    segments: wholeSegments(duration),
    speed: 1,
    volume: 1,
    mute: false,
    fadeIn: 0,
    fadeOut: 0,
    reverse: false,
    crop: null, // { x, y, width, height } as fractions of the frame
    rotate: 0,
    flipH: false,
    flipV: false,
    output: 'same',
  };
}

function cropIsFull(crop) {
  return !crop || (crop.x <= 0.001 && crop.y <= 0.001 && crop.width >= 0.999 && crop.height >= 0.999);
}

// --- annotations ---------------------------------------------------------
// Server limits of the annotation recipe (`crates/lc-media/src/edit.rs`): the
// client clamps to them instead of letting an edit fail.
export const MAX_RECIPE_MASKS = 20;
export const MAX_MASK_STRENGTH = 0.25;
export const MIN_MASK_FRACTION = 0.005;

function roundFraction(value) {
  return Math.round(value * 10000) / 10000;
}

// Masks of the drawing, as fractions of the source frame: the server turns
// them back into pixels once it knows the frame size.
export function masksForRecipe(objects, width, height) {
  if (!(width > 0) || !(height > 0)) {
    return [];
  }
  const masks = [];
  for (const object of objects || []) {
    if (object.type !== 'mask') {
      continue;
    }
    const left = clamp(Math.min(object.x1, object.x2), 0, width);
    const top = clamp(Math.min(object.y1, object.y2), 0, height);
    const right = clamp(Math.max(object.x1, object.x2), 0, width);
    const bottom = clamp(Math.max(object.y1, object.y2), 0, height);
    const x = roundFraction(left / width);
    const y = roundFraction(top / height);
    const w = roundFraction((right - left) / width);
    const h = roundFraction((bottom - top) / height);
    if (w < MIN_MASK_FRACTION || h < MIN_MASK_FRACTION) {
      continue;
    }
    masks.push({
      x,
      y,
      width: w,
      height: h,
      mode: object.mode === 'blur' ? 'blur' : 'pixel',
      // Strength is a length in source pixels: a pixel block or a blur radius.
      strength: roundFraction(clamp(object.strength / width, MIN_MASK_FRACTION, MAX_MASK_STRENGTH)),
    });
    if (masks.length >= MAX_RECIPE_MASKS) {
      break;
    }
  }
  return masks;
}

// The colour adjustments of the recipe, or null when they change nothing.
export function adjustForRecipe(adjust) {
  if (!adjust || adjustIsNeutral(adjust)) {
    return null;
  }
  return {
    brightness: Math.round(adjust.brightness),
    contrast: Math.round(adjust.contrast),
    saturate: Math.round(adjust.saturate),
    blur: Math.round(adjust.blur * 10) / 10,
    preset: adjust.preset || 'none',
  };
}

// Recipe sent to `POST /overlay/media-board/items/{id}/edit`: only what
// differs from the source. `extras` carries the drawing: `{ width, height }`
// of the source frame and `overlay`, the PNG of the drawing as a data URL.
export function buildRecipe(edit, duration, extras = {}) {
  const recipe = {};
  const segments = normalizeSegments(edit.segments, duration);
  if (!isWhole(segments, duration)) {
    recipe.segments = segments.map(([start, end]) => [roundMs(start), roundMs(end)]);
  }
  if (Math.abs(edit.speed - 1) > 0.001) {
    recipe.speed = edit.speed;
  }
  if (edit.mute) {
    recipe.mute = true;
  } else if (Math.abs(edit.volume - 1) > 0.001) {
    recipe.volume = roundMs(clamp(edit.volume, 0, MAX_VOLUME));
  }
  if (!edit.mute && edit.fadeIn > 0) {
    recipe.fadeInSec = roundMs(clamp(edit.fadeIn, 0, MAX_FADE_SEC));
  }
  if (!edit.mute && edit.fadeOut > 0) {
    recipe.fadeOutSec = roundMs(clamp(edit.fadeOut, 0, MAX_FADE_SEC));
  }
  if (edit.reverse) {
    recipe.reverse = true;
  }
  if (!cropIsFull(edit.crop)) {
    recipe.crop = {
      x: roundMs(edit.crop.x),
      y: roundMs(edit.crop.y),
      width: roundMs(edit.crop.width),
      height: roundMs(edit.crop.height),
    };
  }
  if (edit.rotate) {
    recipe.rotate = ((edit.rotate % 360) + 360) % 360;
  }
  if (edit.flipH) {
    recipe.flipH = true;
  }
  if (edit.flipV) {
    recipe.flipV = true;
  }
  if (edit.output && edit.output !== 'same') {
    recipe.output = edit.output;
  }
  const masks = masksForRecipe(edit.objects, extras.width, extras.height);
  if (masks.length > 0) {
    recipe.masks = masks;
  }
  if (extras.overlay && extras.width > 0 && extras.height > 0) {
    recipe.overlay = { png: extras.overlay, width: extras.width, height: extras.height };
  }
  const adjust = adjustForRecipe(edit.adjust);
  if (adjust) {
    recipe.adjust = adjust;
  }
  return recipe;
}

export function recipeIsNoop(recipe) {
  return Object.keys(recipe).length === 0;
}

// Why an edit cannot be saved, or `null`.
export function avEditProblem(edit, duration) {
  const kept = keptDuration(normalizeSegments(edit.segments, duration));
  if (kept < MIN_KEEP_SEC) {
    return 'Il ne reste presque rien : garde au moins un dixième de seconde.';
  }
  return null;
}

// Reads a typed speed ("1,3", "0.8", "2x"): rounded to the hundredth and
// kept inside the accepted range; `null` when unreadable.
export function parseSpeed(text) {
  const raw = String(text ?? '').trim().replace(',', '.').replace(/\s*[x×]$/i, '');
  if (!/^\d*\.?\d+$/.test(raw)) {
    return null;
  }
  const value = Number(raw);
  if (!(value > 0)) {
    return null;
  }
  return Math.round(clamp(value, MIN_SPEED, MAX_SPEED) * 100) / 100;
}

// "1,25×".
export function formatSpeed(speed) {
  return `${String(Math.round(speed * 100) / 100).replace('.', ',')}×`;
}

// --- time ----------------------------------------------------------------
// "0:07,3", "12:40,0", "1:02:03,5".
export function formatClock(seconds) {
  const total = Math.max(0, Number(seconds) || 0);
  const tenths = Math.round(total * 10);
  const whole = Math.floor(tenths / 10);
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const secs = whole % 60;
  const tail = `${String(secs).padStart(2, '0')},${tenths % 10}`;
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, '0')}:${tail}` : `${minutes}:${tail}`;
}

// Reads "1:23,4", "83.4" or "1:02:03" as seconds; `null` when unreadable.
export function parseClock(text) {
  const raw = String(text ?? '').trim().replace(',', '.');
  if (!raw || !/^[0-9:.]+$/.test(raw)) {
    return null;
  }
  const parts = raw.split(':');
  if (parts.length > 3 || parts.some((part) => part === '')) {
    return null;
  }
  let total = 0;
  for (const part of parts) {
    const value = Number(part);
    if (!Number.isFinite(value)) {
      return null;
    }
    total = total * 60 + value;
  }
  return total;
}

// --- crop geometry -------------------------------------------------------
export const CROP_RATIOS = [
  { id: 'free', label: 'Libre', ratio: 0 },
  { id: 'square', label: '1:1', ratio: 1 },
  { id: 'wide', label: '16:9', ratio: 16 / 9 },
  { id: 'tall', label: '9:16', ratio: 9 / 16 },
  { id: 'classic', label: '4:3', ratio: 4 / 3 },
];

// Largest centered rectangle of `aspect` (width / height) inside the bounds;
// the whole bounds when `aspect` is 0.
export function centeredCrop(aspect, width, height) {
  if (!(aspect > 0)) {
    return { x: 0, y: 0, width, height };
  }
  let cropWidth = width;
  let cropHeight = width / aspect;
  if (cropHeight > height) {
    cropHeight = height;
    cropWidth = height * aspect;
  }
  return { x: (width - cropWidth) / 2, y: (height - cropHeight) / 2, width: cropWidth, height: cropHeight };
}

// Moves or resizes a crop rectangle by a pointer delta. `handle` is `move`
// or a compass side/corner (`n`, `se`…). With `aspect`, the shape is kept.
export function dragCropRect(rect, handle, dx, dy, bounds, aspect = 0, minSize = 8) {
  if (handle === 'move') {
    return {
      x: clamp(rect.x + dx, 0, bounds.width - rect.width),
      y: clamp(rect.y + dy, 0, bounds.height - rect.height),
      width: rect.width,
      height: rect.height,
    };
  }
  let left = rect.x;
  let top = rect.y;
  let right = rect.x + rect.width;
  let bottom = rect.y + rect.height;
  if (handle.includes('w')) {
    left = clamp(left + dx, 0, right - minSize);
  }
  if (handle.includes('e')) {
    right = clamp(right + dx, left + minSize, bounds.width);
  }
  if (handle.includes('n')) {
    top = clamp(top + dy, 0, bottom - minSize);
  }
  if (handle.includes('s')) {
    bottom = clamp(bottom + dy, top + minSize, bounds.height);
  }
  if (!(aspect > 0)) {
    return { x: left, y: top, width: right - left, height: bottom - top };
  }
  const centerX = rect.x + rect.width / 2;
  const centerY = rect.y + rect.height / 2;
  // The side opposite to the handle stays where it is; a side handle keeps
  // the other axis centered.
  const maxWidth = handle.includes('w') ? right : handle.includes('e') ? bounds.width - left : 2 * Math.min(centerX, bounds.width - centerX);
  const maxHeight = handle.includes('n') ? bottom : handle.includes('s') ? bounds.height - top : 2 * Math.min(centerY, bounds.height - centerY);
  let width = handle === 'n' || handle === 's' ? (bottom - top) * aspect : right - left;
  width = Math.min(width, maxWidth, maxHeight * aspect);
  width = Math.max(width, minSize, minSize * aspect);
  const height = width / aspect;
  const x = handle.includes('w') ? right - width : handle.includes('e') ? left : centerX - width / 2;
  const y = handle.includes('n') ? bottom - height : handle.includes('s') ? top : centerY - height / 2;
  return { x, y, width, height };
}

// Size of a picture after `quarterTurns` rotations of 90°.
export function rotatedSize(width, height, quarterTurns) {
  return quarterTurns % 2 === 0 ? { width, height } : { width: height, height: width };
}

// Size of a picture after the recipe's crop, in the whole pixels ffmpeg keeps:
// each side is truncated to an even number (`crop=w='max(2,trunc(iw*w/2)*2)'`,
// crates/lc-media/src/edit.rs), then swapped by a quarter turn. A fractional
// crop fraction must never leak as `198.53 px` into the editor's panel.
export function croppedPictureSize({ width, height, crop, quarterTurns = 0 } = {}) {
  const naturalWidth = Math.round(Number(width));
  const naturalHeight = Math.round(Number(height));
  if (!(naturalWidth > 0) || !(naturalHeight > 0)) {
    return null;
  }
  const ratio = (value) => {
    const number = Number(value);
    return Number.isFinite(number) ? Math.min(1, Math.max(0, number)) : 1;
  };
  const side = (size, fraction) => Math.max(2, 2 * Math.trunc((size * fraction) / 2));
  const croppedWidth = crop ? side(naturalWidth, ratio(crop.width)) : naturalWidth;
  const croppedHeight = crop ? side(naturalHeight, ratio(crop.height)) : naturalHeight;
  return rotatedSize(croppedWidth, croppedHeight, quarterTurns);
}

// Scale bringing a picture inside `maxSide` pixels (never above 1).
export function fitScale(width, height, maxSide) {
  const longest = Math.max(width, height);
  return longest > maxSide ? maxSide / longest : 1;
}

// --- picture adjustments -------------------------------------------------
export const NEUTRAL_ADJUST = { brightness: 100, contrast: 100, saturate: 100, blur: 0, preset: 'none' };

export const FILTER_PRESETS = [
  { id: 'none', label: 'Aucun', css: '' },
  { id: 'mono', label: 'Noir & blanc', css: 'grayscale(1)' },
  { id: 'sepia', label: 'Sépia', css: 'sepia(0.9)' },
  { id: 'vivid', label: 'Vif', css: 'saturate(1.6) contrast(1.1)' },
  { id: 'fried', label: 'Deep fried', css: 'saturate(3) contrast(1.8) brightness(1.1)' },
  { id: 'invert', label: 'Négatif', css: 'invert(1)' },
];

// Canvas `filter` string of the adjustments (`none` when neutral).
export function cssFilter(adjust) {
  const parts = [];
  if (adjust.brightness !== 100) {
    parts.push(`brightness(${adjust.brightness / 100})`);
  }
  if (adjust.contrast !== 100) {
    parts.push(`contrast(${adjust.contrast / 100})`);
  }
  if (adjust.saturate !== 100) {
    parts.push(`saturate(${adjust.saturate / 100})`);
  }
  if (adjust.blur > 0) {
    parts.push(`blur(${adjust.blur}px)`);
  }
  const preset = FILTER_PRESETS.find((candidate) => candidate.id === adjust.preset);
  if (preset && preset.css) {
    parts.push(preset.css);
  }
  return parts.length > 0 ? parts.join(' ') : 'none';
}

export function adjustIsNeutral(adjust) {
  return cssFilter(adjust) === 'none';
}

// --- display scale -------------------------------------------------------
// How big an image shows on the overlay, in percent of its natural size. The
// file is untouched: this is a display setting stored with the board item.
export const MIN_DISPLAY_SCALE_PERCENT = 10;
export const MAX_DISPLAY_SCALE_PERCENT = 1000;
export const DEFAULT_DISPLAY_SCALE_PERCENT = 100;

// Whole percent inside the accepted range; an absent or unreadable value is
// 100 %.
export function normalizeDisplayScale(percent) {
  if (percent === null || percent === undefined || percent === '') {
    return DEFAULT_DISPLAY_SCALE_PERCENT;
  }
  const value = Number(percent);
  if (!Number.isFinite(value)) {
    return DEFAULT_DISPLAY_SCALE_PERCENT;
  }
  return Math.round(clamp(value, MIN_DISPLAY_SCALE_PERCENT, MAX_DISPLAY_SCALE_PERCENT));
}

// Value sent to the server: 100 % is the natural size, stored as `null`.
export function displayScaleForStorage(percent) {
  const value = normalizeDisplayScale(percent);
  return value === DEFAULT_DISPLAY_SCALE_PERCENT ? null : value;
}

// The display size only applies when the result shows a picture: the frames of
// a video, or the artwork of a sound. An audio-only result, or a sound without
// artwork, has nothing to scale.
export function showsDisplaySize({ isVideo, output, hasArtwork }) {
  if (isVideo) {
    return output !== 'audio';
  }
  return Boolean(hasArtwork);
}

// Reads the numeric field: a whole percent from 10 to 1000.
export function parseDisplayScale(raw) {
  const text = String(raw ?? '').trim();
  if (!/^\d+$/.test(text)) {
    return { ok: false };
  }
  const value = Number(text);
  if (value < MIN_DISPLAY_SCALE_PERCENT || value > MAX_DISPLAY_SCALE_PERCENT) {
    return { ok: false };
  }
  return { ok: true, value };
}

// Box of a picture `scalePercent` of its natural size, kept inside
// `maxWidth` × `maxHeight` with its aspect ratio. `capped` tells whether the
// zone limit shrank it.
export function displayedImageSize({ width, height, scalePercent, maxWidth, maxHeight }) {
  const factor = normalizeDisplayScale(scalePercent) / 100;
  const naturalWidth = Number(width);
  const naturalHeight = Number(height);
  if (!(naturalWidth > 0) || !(naturalHeight > 0)) {
    return { width: 0, height: 0, capped: false };
  }
  let nextWidth = naturalWidth * factor;
  let nextHeight = naturalHeight * factor;
  const hasLimit = (value) =>
    value !== null && value !== undefined && value !== '' && Number.isFinite(Number(value));
  const limitWidth = hasLimit(maxWidth) ? Math.max(0, Number(maxWidth)) : Infinity;
  const limitHeight = hasLimit(maxHeight) ? Math.max(0, Number(maxHeight)) : Infinity;
  const fit = Math.min(1, limitWidth / nextWidth, limitHeight / nextHeight);
  nextWidth *= fit;
  nextHeight *= fit;
  return {
    width: Math.max(1, Math.round(nextWidth)),
    height: Math.max(1, Math.round(nextHeight)),
    capped: fit < 1,
  };
}

// Height the overlay keeps above a picture: the author chip
// (`--overlay-media-header-height: 54px`) and its gap to the picture
// (`--overlay-media-header-gap: 10px`), see styles/overlay/base.css. The
// legend under the picture is not part of this budget.
export const OVERLAY_MEDIA_HEADER_HEIGHT = 54;
export const OVERLAY_MEDIA_HEADER_GAP = 10;
export const OVERLAY_MEDIA_RESERVED_HEIGHT = OVERLAY_MEDIA_HEADER_HEIGHT + OVERLAY_MEDIA_HEADER_GAP;

// Room the overlay leaves for the picture inside its display zone, in the same
// pixels as the zone. `null` means no limit (unknown zone).
export function displayZoneLimits({
  zoneWidth,
  zoneHeight,
  reservedHeight = OVERLAY_MEDIA_RESERVED_HEIGHT,
} = {}) {
  const width = Number(zoneWidth);
  const height = Number(zoneHeight);
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    return { maxWidth: null, maxHeight: null };
  }
  const reserved = Number.isFinite(Number(reservedHeight)) ? Math.max(0, Number(reservedHeight)) : OVERLAY_MEDIA_RESERVED_HEIGHT;
  return {
    maxWidth: Math.round(width),
    maxHeight: Math.max(0, Math.round(height) - reserved),
  };
}

// --- media summary -------------------------------------------------------
// Size of a file in the units of the system (1024-based, French suffixes):
// `32 ko`, `1,5 Mo`. Empty when the size is unknown.
export function formatFileSize(bytes) {
  const value = Number(bytes);
  if (!(value > 0)) {
    return '';
  }
  const units = ['o', 'ko', 'Mo', 'Go'];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  const digits = index === 0 || size >= 10 ? 0 : 1;
  const text = size.toLocaleString('fr-FR', { minimumFractionDigits: digits, maximumFractionDigits: digits });
  return `${text} ${units[index]}`;
}

// "Vidéo · MP4" from a mime type; empty when it is unknown.
const MEDIA_TYPE_LABELS = {
  mp4: 'MP4',
  quicktime: 'MOV',
  webm: 'WebM',
  'x-matroska': 'MKV',
  ogg: 'OGG',
  gif: 'GIF',
  mpeg: 'MP3',
  'x-wav': 'WAV',
  wav: 'WAV',
  'x-m4a': 'M4A',
  aac: 'AAC',
  png: 'PNG',
  jpeg: 'JPEG',
  jpg: 'JPEG',
  webp: 'WebP',
  avif: 'AVIF',
  bmp: 'BMP',
  heic: 'HEIC',
  'svg+xml': 'SVG',
};

const MEDIA_FAMILIES = { video: 'Vidéo', audio: 'Son', image: 'Image' };

export function mediaTypeLabel(mime) {
  const [family, subtype = ''] = String(mime || '').toLowerCase().split('/');
  if (!MEDIA_FAMILIES[family]) {
    return '';
  }
  const format = MEDIA_TYPE_LABELS[subtype] || subtype.toUpperCase();
  return `${MEDIA_FAMILIES[family]}${format ? ` · ${format}` : ''}`;
}

// Facts of the editor's media summary (bottom of the panel): what the file is
// and what the recipe makes of it. Rows without a value are dropped.
export function mediaInfoFacts({
  name = '',
  mime = '',
  bytes = 0,
  kind = 'video',
  duration = 0,
  resultDuration = 0,
  width = 0,
  height = 0,
  resultWidth = 0,
  resultHeight = 0,
  displayWidth = 0,
  displayHeight = 0,
  hasArtwork = false,
  artworkWidth = 0,
  artworkHeight = 0,
} = {}) {
  const rows = [];
  const title = String(name || '').trim();
  if (title) {
    rows.push(['Fichier', title]);
  }
  const type = mediaTypeLabel(mime);
  if (type) {
    rows.push(['Type', type]);
  }
  const weight = formatFileSize(bytes);
  if (weight) {
    rows.push(['Poids', weight]);
  }
  const length = Number(duration);
  if (length > 0) {
    const result = Number(resultDuration) > 0 ? Number(resultDuration) : length;
    const source = formatClock(length);
    const shown = formatClock(result);
    rows.push(['Durée', source === shown ? source : `${source} → ${shown}`]);
  }
  const pixels = (value, other) =>
    Number(value) > 0 && Number(other) > 0 ? `${Math.round(value)} × ${Math.round(other)} px` : '';
  if (kind === 'audio') {
    const cover = hasArtwork ? pixels(artworkWidth, artworkHeight) : '';
    rows.push(['Pochette', cover || 'aucune']);
  } else {
    const source = pixels(width, height);
    const shown = pixels(resultWidth, resultHeight);
    if (source) {
      rows.push(['Image', shown && shown !== source ? `${source} → ${shown}` : source]);
    }
    // The picture's size on the overlay differs from its pixels only when the
    // display tool scales it or the zone caps it: otherwise, no row.
    const onScreen = pixels(displayWidth, displayHeight);
    if (onScreen && onScreen !== shown) {
      rows.push(['À l’écran', onScreen]);
    }
  }
  return rows;
}

// --- naming --------------------------------------------------------------
// A JPEG stays a JPEG (no transparency to keep); everything else is saved as
// a PNG.
export function exportMimeFor(sourceMime) {  return /^image\/jpe?g$/i.test(String(sourceMime || '')) ? 'image/jpeg' : 'image/png';
}

export function editedTitle(title) {
  const base = String(title || '').trim() || 'Sans titre';
  return /\(édité\)$/i.test(base) ? base : `${base} (édité)`.slice(0, 240);
}

// File name of an edited picture: no path separator, no control character.
export function editedFileName(title, mime) {
  const extension = mime === 'image/jpeg' ? 'jpg' : 'png';
  const stem = String(title || '')
    .replace(/[\\/:*?"<>|\u0000-\u001f]+/g, ' ')
    .replace(/\.+/g, '.')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 120);
  return `${stem || 'image'}.${extension}`;
}
