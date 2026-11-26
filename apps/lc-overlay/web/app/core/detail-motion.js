// Pure geometry and timing helpers for the board's media detail window.
// The overlay passes frozen rects and a clock in, so this module never reads
// window, document or performance and the Node tests can drive it directly.
//
// Timings live in one table, styles/motion.css. MOTION_DEFAULTS mirrors its
// base values for a page whose stylesheet did not load, and readMotion takes a
// CSSStyleDeclaration (or a test fake) and reads the tokens back: reduced
// motion and the debug `html.force-motion` class are already resolved by the
// cascade, so neither branch needs a system query of its own.

// Base values of styles/motion.css, one field per token the window uses.
export const MOTION_DEFAULTS = {
  detailOpen: 320,
  detailClose: 210,
  previewClose: 150,
  backdropIn: 320,
  backdropLead: 90,
  easeEnter: 'cubic-bezier(0.22, 0.61, 0.36, 1)',
  easeMove: 'cubic-bezier(0.4, 0, 0.2, 1)',
};

// Hover preview timings in milliseconds:
// - cold: delay before the first preview of a run, long enough to show intent;
// - warm: delay before the next preview while one is open or one just closed;
// - warmWindow: how long after a preview closed the next one stays warm;
// - hide: delay before a preview the pointer left closes.
export const HOVER_TIMING = {
  cold: 250,
  warm: 60,
  warmWindow: 400,
  hide: 60,
};

// A volume drag can leave the preview through its top or bottom edge. The
// preview keeps a transparent band of this height above and below it while the
// slider is held, so the pointer stays on the preview instead of closing it.
export const VOLUME_DRAG_MARGIN = 32;

export function volumeDragMargin(rect, margin = VOLUME_DRAG_MARGIN) {
  const amount = Number(margin);
  const extra = Number.isFinite(amount) && amount > 0 ? amount : 0;
  return {
    left: rect.left,
    top: rect.top - extra,
    width: rect.width,
    height: rect.height + extra * 2,
  };
}

// A native slider keeps the pointer captured while it is dragged, so the band
// is never hovered under the hand. The release itself must decide whether the
// pointer ended inside the band.
export function volumeDragMarginContains(rect, point) {
  if (!rect || !point) {
    return false;
  }
  return (
    point.x >= rect.left &&
    point.x <= rect.left + rect.width &&
    point.y >= rect.top &&
    point.y <= rect.top + rect.height
  );
}

// Delay before opening a hover preview: the first one waits for the pointer to
// show intent, then the next ones follow it. A preview that is still open, or
// one that closed less than `warmWindow` ago, counts as warm. A missing, NaN or
// infinite `lastClosedAt` means « never closed »: cold unless a preview is open.
export function showDelay({ previewOpen, now, lastClosedAt }, timing = HOVER_TIMING) {
  if (previewOpen) {
    return timing.warm;
  }
  if (Number.isFinite(lastClosedAt) && now - lastClosedAt < timing.warmWindow) {
    return timing.warm;
  }
  return timing.cold;
}

const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

// A CSS duration in milliseconds: '320ms' -> 320, '0.32s' -> 320, ' 150ms '
// -> 150. Anything else (empty, unitless, negative, NaN, 'abc', null) is not
// a usable duration and yields `fallback`.
export function parseDuration(text, fallback) {
  if (typeof text !== 'string') {
    return fallback;
  }
  const match = /^(-?(?:\d+\.?\d*|\.\d+))(ms|s)$/.exec(text.trim());
  if (!match) {
    return fallback;
  }
  const amount = Number(match[1]);
  const milliseconds = match[2] === 's' ? amount * 1000 : amount;
  if (!Number.isFinite(milliseconds) || milliseconds < 0) {
    return fallback;
  }
  // Multiplying seconds by 1000 does not always land on a whole millisecond:
  // keep the table's integer milliseconds instead of a floating-point tail.
  return Math.round(milliseconds * 1000) / 1000;
}

// The motion values the board renders with, read once at the start of a
// transition from the computed style of the document element. A null style,
// or a missing / invalid token, falls back to MOTION_DEFAULTS. Curves are
// trimmed and must be a non-empty string.
export function readMotion(style) {
  const raw = (name) => (
    style && typeof style.getPropertyValue === 'function' ? style.getPropertyValue(name) : ''
  );
  const curve = (name, fallback) => {
    const text = raw(name);
    const value = typeof text === 'string' ? text.trim() : '';
    return value || fallback;
  };
  return {
    detailOpen: parseDuration(raw('--motion-detail-open'), MOTION_DEFAULTS.detailOpen),
    detailClose: parseDuration(raw('--motion-detail-close'), MOTION_DEFAULTS.detailClose),
    previewClose: parseDuration(raw('--motion-preview-close'), MOTION_DEFAULTS.previewClose),
    backdropIn: parseDuration(raw('--motion-backdrop-in'), MOTION_DEFAULTS.backdropIn),
    backdropLead: parseDuration(raw('--motion-backdrop-lead'), MOTION_DEFAULTS.backdropLead),
    easeEnter: curve('--ease-enter', MOTION_DEFAULTS.easeEnter),
    easeMove: curve('--ease-move', MOTION_DEFAULTS.easeMove),
  };
}

// Rect actually painted by a media with `object-fit: contain`, centred in
// `box`. An unknown ratio or an empty box leaves the box untouched.
export function containRect(box, ratio) {
  if (!(ratio > 0) || !box.width || !box.height) {
    return { left: box.left, top: box.top, width: box.width, height: box.height };
  }
  let width = box.width;
  let height = width / ratio;
  if (height > box.height) {
    height = box.height;
    width = height * ratio;
  }
  return {
    left: box.left + (box.width - width) / 2,
    top: box.top + (box.height - height) / 2,
    width,
    height,
  };
}

// Content box of a rect: borders and padding removed, never negative.
export function contentRect(rect, edges) {
  return {
    left: rect.left + edges.left,
    top: rect.top + edges.top,
    width: Math.max(0, rect.width - edges.left - edges.right),
    height: Math.max(0, rect.height - edges.top - edges.bottom),
  };
}

// Keyframes for the detail window's container transform. The page is laid out
// at its final `page` rect and, with transform-origin 0 0, transformed back
// onto `origin`: the surface interpolates the same box linearly, so the content
// travels with the growing frame instead of being uncovered in place. A short
// frame cuts what lies below its bottom edge; the radius is divided by the
// scale so the transformed corners keep the interpolated CSS radius. The last
// frame is exactly the identity. Numbers written in the strings are rounded to
// three decimals, and a negative zero is written as zero.
export function pageFlightFrames({ origin, page, originRadius, pageRadius, steps = 12 }) {
  const requested = Number(steps);
  const count = Number.isFinite(requested) ? Math.max(1, Math.floor(requested)) : 1;
  const frames = [];
  for (let index = 0; index <= count; index += 1) {
    const offset = index / count;
    const left = origin.left + (page.left - origin.left) * offset;
    const top = origin.top + (page.top - origin.top) * offset;
    const width = origin.width + (page.width - origin.width) * offset;
    const height = origin.height + (page.height - origin.height) * offset;
    const scale = width / page.width;
    const clipBottom = Math.max(0, page.height - height / scale);
    const radius = (originRadius + (pageRadius - originRadius) * offset) / scale;
    frames.push({
      offset,
      transform: `translate(${round(left - page.left)}px, ${round(top - page.top)}px) scale(${round(scale)})`,
      clipPath: `inset(0px 0px ${round(clipBottom)}px 0px round ${round(radius)}px)`,
    });
  }
  return frames;
}

// Keyframe rects for the media carried between the detail window and a rect
// outside it (the preview's media, or the card's thumbnail). `media` is the
// media's rect in the page laid out at `page`; the page itself travels as in
// pageFlightFrames, so at each step the media is aimed where the moving page
// shows it. Opening (offset p) leaves `outside` and latches onto the page
// early, by 1 - (1 - p)^2; closing (`leaving`) stays with the shrinking page
// and lets go late, by p^2, landing on `outside`. Rect numbers are rounded to
// three decimals.
export function mediaTrackFrames({ origin, page, media, outside, steps = 12, leaving = false }) {
  const requested = Number(steps);
  const count = Number.isFinite(requested) ? Math.max(1, Math.floor(requested)) : 1;
  const lerp = (from, to, t) => from + (to - from) * t;
  const frames = [];
  for (let index = 0; index <= count; index += 1) {
    const offset = index / count;
    // Where the page is along the opening: closing runs it backwards.
    const along = leaving ? 1 - offset : offset;
    const left = lerp(origin.left, page.left, along);
    const top = lerp(origin.top, page.top, along);
    const scale = lerp(origin.width, page.width, along) / page.width;
    const tracked = {
      left: left + (media.left - page.left) * scale,
      top: top + (media.top - page.top) * scale,
      width: media.width * scale,
      height: media.height * scale,
    };
    const [from, to, weight] = leaving
      ? [tracked, outside, offset * offset]
      : [outside, tracked, 1 - (1 - offset) * (1 - offset)];
    frames.push({
      offset,
      rect: {
        left: round(lerp(from.left, to.left, weight)),
        top: round(lerp(from.top, to.top, weight)),
        width: round(lerp(from.width, to.width, weight)),
        height: round(lerp(from.height, to.height, weight)),
      },
    });
  }
  return frames;
}

// Transform that makes a box laid out at `to` look like `from`, with
// transform-origin 0 0: the media flies from its thumbnail to the stage.
export function flightTransform(from, to) {
  return `translate(${from.left - to.left}px, ${from.top - to.top}px) scale(${from.width / to.width}, ${from.height / to.height})`;
}

// Whether the card is visible enough in the board to animate back into it.
export function isVisibleIn(rect, area) {
  if (!rect.width || !rect.height) return false;
  const middle = rect.top + rect.height / 2;
  return middle > area.top && middle < area.bottom;
}

// The preview does not reopen under a pointer that never really moved.
export const HOLD_DISTANCE = 4;

export function holdReleased(hold, point) {
  return Math.hypot(point.x - hold.x, point.y - hold.y) >= HOLD_DISTANCE;
}

// Round a computed field to three decimals, without a negative zero: a tiny
// negative value rounds to -0, which must be written as 0.
const round = (value) => {
  const rounded = Math.round(value * 1000) / 1000;
  return rounded === 0 ? 0 : rounded;
};

// Geometry that makes the preview's first frame the card itself. The preview
// is laid out at its final `box`, then transformed with its origin at the top
// left: `dx`/`dy`/`scale` put it exactly on the card. The media is cropped to
// the height of the card's thumbnail (`band`, in preview pixels) and pulled up
// by half the remaining media; the progress bar and body follow by `shift`, so
// the title starts right under the band. Everything below the card's bottom
// edge is clipped by `clipBottom`, with the corners rounded back to the card's
// own radius. All values are computed from the unrounded scale.
export function growFrame({ card, thumbHeight, cardRadius, box, mediaHeight }) {
  const scale = clamp(card.width / box.width, 0.35, 1);
  const dx = card.left - box.left;
  const dy = card.top - box.top;
  const band = thumbHeight / scale;
  const visible = Math.min(mediaHeight, band);
  const crop = (mediaHeight - visible) / 2;
  const shift = -(mediaHeight - visible);
  const clipBottom = Math.max(0, box.height - card.height / scale);
  const radius = cardRadius / scale;
  return {
    dx: round(dx),
    dy: round(dy),
    scale: round(scale),
    crop: round(crop),
    shift: round(shift),
    clipBottom: round(clipBottom),
    radius: round(radius),
  };
}

// Place the hover preview around the card centre, clamped into the safe zone.
// It returns the final box only: growFrame owns the translation and the scale
// that grow the preview out of the card.
export function placePreview(cardRect, size, area) {
  const centreX = cardRect.left + cardRect.width / 2;
  const centreY = cardRect.top + cardRect.height / 2;
  const left = clamp(
    centreX - size.width / 2,
    area.left,
    Math.max(area.left, area.right - size.width),
  );
  const top = clamp(
    centreY - size.height / 2,
    area.top,
    Math.max(area.top, area.bottom - size.height),
  );
  return {
    left: Math.round(left),
    top: Math.round(top),
  };
}

// A thumbnail flush with the card top borrows the card's top corners. Any
// other radius stays as measured on the thumbnail.
export function returnRadius(thumbRadii, thumbTop, cardTop, cardRadius) {
  if (/^0px 0px 0px 0px$/.test(thumbRadii) && Math.abs(thumbTop - cardTop) < 2) {
    return `${cardRadius} ${cardRadius} 0px 0px`;
  }
  return thumbRadii;
}

// Play animations that were created paused. Motion starts on the third frame:
// the first sends the initial state to the compositor, the second lets it
// rasterise the new layers, and the third runs once that raster is done. Each
// frame then advances the timeline by at most 20 ms, so a slow frame delays
// the motion instead of making it jump. The clock timers cover a tab that
// produces no frames, and the returned function ends the timeline at once (an
// Escape during the opening). `onDone` runs exactly once.
export function playTimeline(animations, duration, { onStart, onDone }, clock) {
  const MAX_STEP = 20;
  let started = false;
  let stopped = false;
  let safety = 0;
  let prepaint = 0;
  for (const animation of animations) animation.pause();

  const done = () => {
    if (stopped) return;
    stopped = true;
    clock.clearTimeout(safety);
    clock.clearTimeout(prepaint);
    onDone();
  };

  const start = () => {
    if (started || stopped) return;
    started = true;
    clock.clearTimeout(prepaint);
    onStart?.();
    let elapsed = 0;
    let last = null;
    const step = (now) => {
      if (stopped) return;
      if (last !== null) elapsed += Math.min(Math.max(0, now - last), MAX_STEP);
      last = now;
      const time = Math.min(elapsed, duration);
      for (const animation of animations) animation.currentTime = time;
      if (elapsed >= duration) done();
      else clock.raf(step);
    };
    clock.raf(step);
    safety = clock.setTimeout(done, duration * 3 + 500);
  };

  const afterFrames = (count) => {
    if (count === 0) start();
    else clock.raf(() => afterFrames(count - 1));
  };
  afterFrames(3);
  prepaint = clock.setTimeout(start, 400);
  return done;
}
