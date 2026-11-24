// Zoomable preview (Ctrl + wheel) shared by the picture and the video editors.
//
// The content keeps its own layout: the editor sizes it at "fitted size x
// zoom" in `onResize` and hands that size to `fit()`. Growing it by its size
// rather than a `transform` keeps the crop and selection handles at their
// screen size.
//
// The frame never scrolls natively. It keeps a virtual scroll, draws its own
// scrollbars in gutters that are always reserved, and places the content
// itself:
// - by default the content keeps its zoomed size and slides under the frame;
// - `windowed`, the content is a fixed window as big as what can be seen, and
//   the editor shows the scrolled part in it (`onPan(view)`). The video needs
//   this: a video element that moves on screen loses Chromium's hardware
//   overlay and looks washed out on a wide-gamut screen for the whole gesture,
//   while a still one whose `object-view-box` changes keeps it.
//
// A scrollbar that takes room only while the content overflows feeds back
// into the zoomed size: the content shrinks, stops overflowing, the bar goes,
// it grows back... and the picture flips between two sizes on every frame.
// The gutters are kept at every zoom for that reason.

import { h } from '../../core/dom.js';
import {
  MAX_STAGE_ZOOM,
  MIN_STAGE_ZOOM,
  anchoredScroll,
  clamp,
  stageAxis,
  wheelScrollDelta,
  zoomedByWheel,
} from '../../core/editor-helpers.js';
import { createMiddlePan } from './middle-pan.js';

export const STAGE_PADDING = 24;
// Thickness of the scrollbar gutters, in CSS pixels (see `.stage-zoom-bar`).
export const STAGE_BAR = 10;
const MIN_THUMB = 24;
// Time constant of the wheel's glide, in milliseconds.
const GLIDE_MS = 45;

function createBar(axis, onScroll, onPage) {
  const horizontal = axis === 'x';
  const thumb = h('span', { class: 'stage-zoom-thumb' });
  const node = h('div', { class: `stage-zoom-bar is-${axis}`, 'aria-hidden': 'true' }, thumb);
  node.hidden = true;
  let state = stageAxis({ full: 0, room: 0 });

  const trackLength = () => (horizontal ? node.clientWidth : node.clientHeight);
  const thumbLength = () => Math.min(trackLength(), Math.max(MIN_THUMB, (trackLength() * state.length) / (state.length + state.max)));

  function sync(next) {
    state = next;
    node.hidden = !state.overflow;
    if (!state.overflow) {
      return;
    }
    const length = thumbLength();
    const at = state.max > 0 ? ((trackLength() - length) * state.scroll) / state.max : 0;
    thumb.style[horizontal ? 'width' : 'height'] = `${length}px`;
    thumb.style.transform = horizontal ? `translateX(${at}px)` : `translateY(${at}px)`;
  }

  // Dragging the thumb scrolls by the content its travel stands for; a press
  // on the track pages towards the pointer.
  node.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || !state.overflow) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    if (event.target !== thumb) {
      const box = thumb.getBoundingClientRect();
      const before = horizontal ? event.clientX < box.left : event.clientY < box.top;
      onPage(before ? -1 : 1);
      return;
    }
    const start = horizontal ? event.clientX : event.clientY;
    const from = state.scroll;
    const travel = Math.max(1, trackLength() - thumbLength());
    thumb.setPointerCapture(event.pointerId);
    thumb.classList.add('is-dragging');
    const move = (moveEvent) => {
      const delta = (horizontal ? moveEvent.clientX : moveEvent.clientY) - start;
      onScroll(from + (delta * state.max) / travel);
    };
    const up = () => {
      thumb.classList.remove('is-dragging');
      thumb.removeEventListener('pointermove', move);
      thumb.removeEventListener('pointerup', up);
      thumb.removeEventListener('pointercancel', up);
    };
    thumb.addEventListener('pointermove', move);
    thumb.addEventListener('pointerup', up);
    thumb.addEventListener('pointercancel', up);
  });

  return { node, sync };
}

export function createStageZoom({ content, onResize, onPan = null, windowed = false }) {
  let zoom = 1;
  let visible = true;
  let destroyed = false;
  let extent = { width: 0, height: 0 };
  let scroll = { left: 0, top: 0 };
  let axes = { x: stageAxis({ full: 0, room: 0 }), y: stageAxis({ full: 0, room: 0 }) };
  // The pointer a zoom step is anchored on, until the editor hands its new
  // size to `fit`.
  let anchor = null;
  let glide = null;
  let glideFrame = 0;

  const barX = createBar('x', (left) => scrollTo(left, scroll.top), (direction) => glideBy(direction * room().width * 0.9, 0));
  const barY = createBar('y', (top) => scrollTo(scroll.left, top), (direction) => glideBy(0, direction * room().height * 0.9));
  content.classList.add('stage-zoom-content');
  const viewport = h('div', { class: 'stage-zoom' }, content, barX.node, barY.node);
  viewport.title = 'Ctrl + molette : zoomer · molette maintenue : déplacer';
  const chip = h('button', {
    class: 'btn stage-zoom-reset',
    type: 'button',
    hidden: true,
    onClick: () => reset(),
  });

  function syncChip() {
    const label = zoom < 10 ? zoom.toFixed(1).replace('.', ',') : String(Math.round(zoom));
    chip.hidden = !visible || zoom <= MIN_STAGE_ZOOM + 0.001;
    chip.textContent = `×${label}`;
    chip.setAttribute('aria-label', `Zoom de l’aperçu : ×${label}. Cliquer pour ajuster.`);
    chip.title = 'Ajuster l’aperçu (Ctrl + molette pour zoomer)';
  }

  // What the content can be shown in: the frame minus its scrollbar gutters.
  function room() {
    return {
      width: Math.max(0, viewport.clientWidth - STAGE_BAR),
      height: Math.max(0, viewport.clientHeight - STAGE_BAR),
    };
  }

  // The room for the fitted content, margins included.
  function available() {
    const { width, height } = room();
    return {
      width: Math.max(0, width - STAGE_PADDING * 2),
      height: Math.max(0, height - STAGE_PADDING * 2),
    };
  }

  // The visible part of the zoomed content, in its own pixels.
  function view() {
    return { x: axes.x.start, y: axes.y.start, width: axes.x.length, height: axes.y.length };
  }

  function measure() {
    const { width, height } = room();
    axes = {
      x: stageAxis({ full: extent.width, room: width, scroll: scroll.left }),
      y: stageAxis({ full: extent.height, room: height, scroll: scroll.top }),
    };
    scroll = { left: axes.x.scroll, top: axes.y.scroll };
  }

  // A window stays where the visible part starts; the whole content slides
  // by the scroll. Whole pixels keep a picture sharp.
  function place() {
    const x = windowed ? Math.max(0, axes.x.offset) : axes.x.offset;
    const y = windowed ? Math.max(0, axes.y.offset) : axes.y.offset;
    content.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
    barX.sync(axes.x);
    barY.sync(axes.y);
  }

  // Called by the editor with the content's zoomed size; returns the view it
  // has to show.
  function fit(size) {
    extent = { width: Math.max(0, size.width), height: Math.max(0, size.height) };
    if (anchor) {
      const { width, height } = room();
      scroll = {
        left: anchoredScroll({ pointer: anchor.x, room: width, before: { full: anchor.width, scroll: anchor.left }, after: extent.width }),
        top: anchoredScroll({ pointer: anchor.y, room: height, before: { full: anchor.height, scroll: anchor.top }, after: extent.height }),
      };
      anchor = null;
    }
    measure();
    place();
    return view();
  }

  function scrollTo(left, top) {
    const before = scroll;
    scroll = { left, top };
    measure();
    if (scroll.left === before.left && scroll.top === before.top) {
      return;
    }
    place();
    if (onPan) {
      onPan(view());
    }
  }

  function stopGlide() {
    glide = null;
    if (glideFrame) {
      cancelAnimationFrame(glideFrame);
      glideFrame = 0;
    }
  }

  // The wheel glides to its target instead of jumping a notch at a time.
  function glideBy(dx, dy) {
    const from = glide ? glide.target : scroll;
    glide = {
      target: {
        left: clamp(from.left + dx, 0, axes.x.max),
        top: clamp(from.top + dy, 0, axes.y.max),
      },
      last: null,
    };
    if (!glideFrame) {
      glideFrame = requestAnimationFrame(step);
    }
  }

  function step(now) {
    glideFrame = 0;
    if (!glide || destroyed) {
      glide = null;
      return;
    }
    const elapsed = glide.last === null ? 16 : now - glide.last;
    glide.last = now;
    const share = 1 - Math.exp(-Math.max(0, elapsed) / GLIDE_MS);
    const { target } = glide;
    const left = scroll.left + (target.left - scroll.left) * share;
    const top = scroll.top + (target.top - scroll.top) * share;
    const done = Math.abs(target.left - left) < 0.5 && Math.abs(target.top - top) < 0.5;
    scrollTo(done ? target.left : left, done ? target.top : top);
    if (done) {
      glide = null;
      return;
    }
    glideFrame = requestAnimationFrame(step);
  }

  function layout() {
    if (!destroyed) {
      onResize(zoom);
    }
  }

  function setZoom(next, event) {
    const value = clamp(next, MIN_STAGE_ZOOM, MAX_STAGE_ZOOM);
    if (Math.abs(value - zoom) < 0.001) {
      return;
    }
    stopGlide();
    if (event) {
      const box = viewport.getBoundingClientRect();
      anchor = {
        x: event.clientX - box.left,
        y: event.clientY - box.top,
        width: extent.width,
        height: extent.height,
        left: scroll.left,
        top: scroll.top,
      };
    }
    zoom = value;
    syncChip();
    layout();
    anchor = null;
  }

  function reset() {
    setZoom(MIN_STAGE_ZOOM);
  }

  viewport.addEventListener('wheel', (event) => {
    if (event.ctrlKey || event.metaKey) {
      // The page zoom must never take over, even at the limit.
      event.preventDefault();
      const next = zoomedByWheel(zoom, event);
      if (next !== null) {
        setZoom(next, event);
      }
      return;
    }
    const delta = wheelScrollDelta(event, room());
    const moves = (axes.x.overflow && delta.x !== 0) || (axes.y.overflow && delta.y !== 0);
    if (!moves) {
      return;
    }
    event.preventDefault();
    glideBy(axes.x.overflow ? delta.x : 0, axes.y.overflow ? delta.y : 0);
  }, { passive: false });

  // Held middle button: the content follows the hand, so the view moves the
  // other way. Nothing to move when the picture fits the frame.
  const pan = createMiddlePan(viewport, (dx, dy) => {
    stopGlide();
    scrollTo(scroll.left - dx, scroll.top - dy);
  }, {
    canStart: () => axes.x.overflow || axes.y.overflow,
  });

  // A new frame size (side panel, toolbar) lays the content out again.
  const observer = new ResizeObserver(() => layout());
  observer.observe(viewport);

  function setVisible(next) {
    visible = Boolean(next);
    viewport.hidden = !visible;
    syncChip();
    if (visible) {
      layout();
    }
  }

  syncChip();

  return {
    node: viewport,
    chip,
    available,
    fit,
    view,
    extent: () => ({ ...extent }),
    zoom: () => zoom,
    setZoom,
    reset,
    setVisible,
    destroy() {
      destroyed = true;
      stopGlide();
      pan.destroy();
      observer.disconnect();
    },
  };
}
