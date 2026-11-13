// Pure geometry of the zone placement: moving and resizing the display zone
// on the overlay screen, with magnetism to the screen edges and centre.
//
// Rectangles are in CSS pixels of the overlay window ({ left, top, width,
// height }); zones are fractions of the screen ({ x, y, width, height }), as
// saved by the shell.

/** Distance (CSS px) under which an edge or the centre snaps to a guide. */
export const SNAP_THRESHOLD_PX = 8;
/** Smallest zone side, as a fraction of the screen side (as in the shell). */
export const MIN_ZONE_FRACTION = 0.05;

const FRACTION_SCALE = 10_000;

/** The 8 handles: corners and edges, by compass direction. */
export const HANDLES = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];

function roundFraction(value) {
  return Math.round(value * FRACTION_SCALE) / FRACTION_SCALE;
}

function clamp(value, minimum, maximum) {
  return Math.min(maximum, Math.max(minimum, value));
}

/** Same rules as `LayoutZone::normalize` in the shell. */
export function normalizeZone(zone) {
  const values = [zone?.x, zone?.y, zone?.width, zone?.height];
  if (!values.every((value) => typeof value === 'number' && Number.isFinite(value))) {
    return { x: 0, y: 0, width: 1, height: 1 };
  }
  const width = roundFraction(clamp(zone.width, MIN_ZONE_FRACTION, 1));
  const height = roundFraction(clamp(zone.height, MIN_ZONE_FRACTION, 1));
  return {
    x: roundFraction(clamp(zone.x, 0, 1 - width)),
    y: roundFraction(clamp(zone.y, 0, 1 - height)),
    width,
    height,
  };
}

export function zoneToRect(zone, bounds) {
  const normalized = normalizeZone(zone);
  return {
    left: normalized.x * bounds.width,
    top: normalized.y * bounds.height,
    width: normalized.width * bounds.width,
    height: normalized.height * bounds.height,
  };
}

export function rectToZone(rect, bounds) {
  return normalizeZone({
    x: rect.left / bounds.width,
    y: rect.top / bounds.height,
    width: rect.width / bounds.width,
    height: rect.height / bounds.height,
  });
}

function minimumSize(bounds) {
  return {
    width: MIN_ZONE_FRACTION * bounds.width,
    height: MIN_ZONE_FRACTION * bounds.height,
  };
}

/** Guides of one axis: the screen edges and its centre. */
function guidesOf(size) {
  return [0, size / 2, size];
}

/**
 * Closest guide to any of `points` within the threshold.
 * Returns { offset, at } (offset to add to the points) or null.
 */
function nearestSnap(points, guides) {
  let best = null;
  for (const point of points) {
    for (const guide of guides) {
      const offset = guide - point;
      if (Math.abs(offset) <= SNAP_THRESHOLD_PX && (best === null || Math.abs(offset) < Math.abs(best.offset))) {
        best = { offset, at: guide };
      }
    }
  }
  return best;
}

/**
 * Moves `start` by (dx, dy), kept on screen. With `snap`, its edges and centre
 * stick to the screen edges and centre.
 * Returns { rect, guides } where guides lists the snapped lines.
 */
export function moveRect(start, dx, dy, bounds, options = {}) {
  let left = clamp(start.left + dx, 0, bounds.width - start.width);
  let top = clamp(start.top + dy, 0, bounds.height - start.height);
  const guides = [];
  if (options.snap) {
    const snapX = nearestSnap([left, left + start.width / 2, left + start.width], guidesOf(bounds.width));
    if (snapX) {
      left = clamp(left + snapX.offset, 0, bounds.width - start.width);
      guides.push({ axis: 'x', at: snapX.at });
    }
    const snapY = nearestSnap([top, top + start.height / 2, top + start.height], guidesOf(bounds.height));
    if (snapY) {
      top = clamp(top + snapY.offset, 0, bounds.height - start.height);
      guides.push({ axis: 'y', at: snapY.at });
    }
  }
  return { rect: { left, top, width: start.width, height: start.height }, guides };
}

/** Moves one edge of an axis, snapped and kept between its limits. */
function dragEdge(value, minimum, maximum, guides, snap) {
  let next = clamp(value, minimum, maximum);
  let guide = null;
  if (snap) {
    const snapped = nearestSnap([next], guides);
    if (snapped && snapped.at >= minimum && snapped.at <= maximum) {
      next = snapped.at;
      guide = snapped.at;
    }
  }
  return { value: next, guide };
}

/**
 * Resizes `start` by dragging `handle` (one of HANDLES) by (dx, dy). The
 * opposite edges stay put; the zone keeps its minimum size and stays on
 * screen. Returns { rect, guides }.
 */
export function resizeRect(start, handle, dx, dy, bounds, options = {}) {
  const min = minimumSize(bounds);
  const snap = Boolean(options.snap);
  let left = start.left;
  let top = start.top;
  let right = start.left + start.width;
  let bottom = start.top + start.height;
  const guides = [];
  const xGuides = guidesOf(bounds.width);
  const yGuides = guidesOf(bounds.height);

  if (handle.includes('w')) {
    const edge = dragEdge(left + dx, 0, right - min.width, xGuides, snap);
    left = edge.value;
    if (edge.guide !== null) guides.push({ axis: 'x', at: edge.guide });
  } else if (handle.includes('e')) {
    const edge = dragEdge(right + dx, left + min.width, bounds.width, xGuides, snap);
    right = edge.value;
    if (edge.guide !== null) guides.push({ axis: 'x', at: edge.guide });
  }
  if (handle.includes('n')) {
    const edge = dragEdge(top + dy, 0, bottom - min.height, yGuides, snap);
    top = edge.value;
    if (edge.guide !== null) guides.push({ axis: 'y', at: edge.guide });
  } else if (handle.includes('s')) {
    const edge = dragEdge(bottom + dy, top + min.height, bounds.height, yGuides, snap);
    bottom = edge.value;
    if (edge.guide !== null) guides.push({ axis: 'y', at: edge.guide });
  }
  return { rect: { left, top, width: right - left, height: bottom - top }, guides };
}

/**
 * Keyboard nudge: moves the zone by (dx, dy), or with `resize` grows or
 * shrinks it from its right and bottom edges. No magnetism.
 */
export function nudgeRect(rect, dx, dy, bounds, options = {}) {
  if (options.resize) {
    return resizeRect(rect, 'se', dx, dy, bounds).rect;
  }
  return moveRect(rect, dx, dy, bounds).rect;
}
