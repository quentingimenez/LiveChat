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

// Recipe sent to `POST /overlay/media-board/items/{id}/edit`: only what
// differs from the source.
export function buildRecipe(edit, duration) {
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

// --- naming --------------------------------------------------------------
// A JPEG stays a JPEG (no transparency to keep); everything else is saved as
// a PNG.
export function exportMimeFor(sourceMime) {
  return /^image\/jpe?g$/i.test(String(sourceMime || '')) ? 'image/jpeg' : 'image/png';
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
