// Hover preview and media detail window for the board (DOM layer only).
//
// Markup contract for the two nodes the board provides:
//
// previewBody() is appended by this module inside the `.hp-body` wrapper, under
// `.hp-media` and the seek bar. The board may use the preview classes styled by
// media-detail.css: `.hp-title`, `.hp-facts` with `.hp-sep`, `.hp-legend-field`
// with `.hp-legend-label` and `.hp-legend-input`, and `.hp-send` with
// `.hp-send-glyph`. Controls are ordinary `.btn` buttons; clicks on buttons,
// inputs, links and selects never open the window.
//
// detailBody() returns the information column itself, appended next to
// `.hp-page-stage` in `.hp-page-grid`; this module gives its root the
// `.hp-page-info` class when it is missing. That column scrolls without a
// visible scrollbar and pins its footer (the facts and the actions) at the
// bottom, while the stage stays in place. The classes styled by
// media-detail.css are `.hp-page-head`, `.hp-page-kicker`, `.hp-page-title`,
// `.hp-page-facts` (dt/dd), `.hp-page-footer`, `.hp-page-source`,
// `.hp-page-legend-block`, `.hp-page-label`, `.hp-page-legend` (`.is-empty`),
// `.hp-page-artwork` and its artwork block, `.hp-page-actions`,
// `.hp-page-secondary` and `.hp-page-actionzone`.
//
// describe(id).tools() returns an optional HTMLElement for the right side of
// the window's top bar (`.hp-page-tools`), or null when there is none: the
// board puts its favorite toggle there. refreshDetail() replaces that node too,
// like the information column, so a favorite change repaints both.
//
// Motion contract: every duration and curve comes from the single table in
// styles/motion.css, read through readMotion() on the document element at the
// start of each transition. Reduced motion and the debug `html.force-motion`
// class are resolved by those tokens, so this module never queries the system
// preference itself.

import { h } from '../core/dom.js';
import { DOUBLE_CLICK_WINDOW } from '../core/detail-actions.js';
import { bindRangeFill, paintRange } from '../core/range-fill.js';
import {
  readMotion,
  containRect,
  contentRect,
  flightTransform,
  growFrame,
  isVisibleIn,
  pageFlightFrames,
  mediaTrackFrames,
  HOLD_DISTANCE,
  holdReleased,
  HOVER_TIMING,
  VOLUME_DRAG_MARGIN,
  volumeDragMargin,
  volumeDragMarginContains,
  showDelay,
  placePreview,
  returnRadius,
  playTimeline,
} from '../core/detail-motion.js';
import { formatDuration } from '../core/format.js';
import { ratioAt, seekBy, seekToRatio } from '../core/media-controls.js';
import { createMediaPrefetcher } from '../core/media-prefetch.js';
import { playbackPlan } from '../core/media-playback.js';
import { createPreviewProgress } from '../core/preview-progress.js';
import { mediaPlaybackGain, playbackSettings } from '../core/sound.js';

const BODY_ESTIMATE = 220;
const MAX_WIDTH = 460;
const TOP_INSET = 76;
const SVG_NS = 'http://www.w3.org/2000/svg';

// The clock handed to playTimeline. Arrow functions read `window` when called,
// never at import time.
const CLOCK = {
  raf: (fn) => window.requestAnimationFrame(fn),
  setTimeout: (fn, ms) => window.setTimeout(fn, ms),
  clearTimeout: (id) => window.clearTimeout(id),
};

// One set of global listeners for every controller created by this module.
const controllers = new Set();
let listenersInstalled = false;

function installGlobalListeners() {
  if (listenersInstalled) {
    return;
  }
  listenersInstalled = true;

  window.addEventListener('pointermove', (event) => {
    for (const controller of controllers) {
      controller.movePointer(event.clientX, event.clientY);
    }
  }, { passive: true });

  window.addEventListener('keydown', (event) => {
    if (event.key === 'Tab') {
      for (const controller of controllers) {
        controller.releaseHold();
      }
    }
    if (event.key !== 'Escape' || event.defaultPrevented) {
      return;
    }
    for (const controller of controllers) {
      // The board owns Escape for the window: it calls closeDetail() itself.
      if (!controller.hasDetail()) {
        controller.closePreview();
      }
    }
  });

  window.addEventListener('scroll', () => {
    for (const controller of controllers) {
      controller.closePreview();
    }
  }, { capture: true, passive: true });

  window.addEventListener('resize', () => {
    for (const controller of controllers) {
      controller.closePreview();
    }
  }, { passive: true });
}

// --- small geometry / media helpers ---------------------------------------

function px(value) {
  const number = Number.parseFloat(value);
  return Number.isFinite(number) ? number : 0;
}

function bounds(rect) {
  const left = Number(rect && rect.left) || 0;
  const top = Number(rect && rect.top) || 0;
  const width = Number(rect && rect.width) || 0;
  const height = Number(rect && rect.height) || 0;
  return {
    left,
    top,
    width,
    height,
    right: Number.isFinite(rect && rect.right) ? rect.right : left + width,
    bottom: Number.isFinite(rect && rect.bottom) ? rect.bottom : top + height,
  };
}

function contentBoxOf(el) {
  const rect = el.getBoundingClientRect();
  const style = getComputedStyle(el);
  return contentRect(rect, {
    left: px(style.borderLeftWidth) + px(style.paddingLeft),
    right: px(style.borderRightWidth) + px(style.paddingRight),
    top: px(style.borderTopWidth) + px(style.paddingTop),
    bottom: px(style.borderBottomWidth) + px(style.paddingBottom),
  });
}

function radiiOf(el) {
  const style = getComputedStyle(el);
  return [
    style.borderTopLeftRadius || '0px',
    style.borderTopRightRadius || '0px',
    style.borderBottomRightRadius || '0px',
    style.borderBottomLeftRadius || '0px',
  ].join(' ');
}

function mediaRatioOf(el, fallback) {
  if (el && el.videoWidth > 0 && el.videoHeight > 0) {
    return el.videoWidth / el.videoHeight;
  }
  if (el && el.naturalWidth > 0 && el.naturalHeight > 0) {
    return el.naturalWidth / el.naturalHeight;
  }
  const value = Number(fallback);
  return Number.isFinite(value) && value > 0.05 ? value : 16 / 10;
}

function clampVolume(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) {
    return 1;
  }
  return Math.min(1, Math.max(0, number));
}

function escapeCssUrl(value) {
  return String(value).replace(/"/g, '%22');
}

function swatch(tag) {
  return typeof document.createElementNS === 'function'
    ? document.createElementNS(SVG_NS, tag)
    : document.createElement(tag);
}

function glyph(paths, options) {
  const settings = options || {};
  const svg = swatch('svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('fill', settings.fill || 'none');
  svg.setAttribute('stroke', settings.stroke || 'currentColor');
  svg.setAttribute('stroke-width', settings.strokeWidth || '1.8');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  for (const d of paths) {
    const path = swatch('path');
    path.setAttribute('d', d);
    svg.appendChild(path);
  }
  return svg;
}

const musicIcon = () => glyph([
  'M9 18V6l10-2v12M9 18a3 3 0 11-6 0 3 3 0 016 0zM19 16a3 3 0 11-6 0 3 3 0 016 0z',
], { strokeWidth: '1.6' });

const speakerIcon = (muted) => glyph([
  'M4 9v6h4l5 4V5L8 9H4z',
  muted ? 'M16 9l5 5M21 9l-5 5' : 'M16.5 8.5a5 5 0 010 7M19 6a9 9 0 010 12',
], { strokeWidth: '1.7' });

const playIcon = () => glyph(['M7 4.5v15l12-7.5z'], { fill: 'currentColor', stroke: 'none' });

const pauseIcon = () => glyph(['M9 5v14M15 5v14'], { strokeWidth: '2.4' });

const fullscreenIcon = () => glyph([
  'M9 4H4v5M15 4h5v5M9 20H4v-5M15 20h5v-5',
], { strokeWidth: '1.7' });

const fullscreenExitIcon = () => glyph([
  'M4 9h5V4M20 9h-5V4M4 15h5v5M20 15h-5v5',
], { strokeWidth: '1.7' });

const backIcon = () => glyph(['M19 12H5', 'M11 6l-6 6 6 6']);

const pictureIcon = () => glyph(['M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4']);

function previewGlyph(kind) {
  const box = document.createElement('div');
  box.className = 'hp-audio';
  box.style.height = '100%';
  box.appendChild(kind === 'image' ? pictureIcon() : musicIcon());
  if (kind === 'audio') {
    const label = document.createElement('span');
    label.textContent = 'Audio';
    box.appendChild(label);
  }
  return box;
}

function stageGlyph(kind) {
  const box = document.createElement('div');
  box.className = 'hp-page-audio-glyph';
  box.appendChild(kind === 'image' ? pictureIcon() : musicIcon());
  return box;
}

function posterImage(source) {
  const image = document.createElement('img');
  image.src = source;
  image.alt = '';
  image.draggable = false;
  image.decoding = 'async';
  return image;
}

function normalizeKind(desc) {
  return desc && (desc.kind === 'video' || desc.kind === 'audio') ? desc.kind : 'image';
}

// --- module API ------------------------------------------------------------

export function createMediaDetail(options) {
  const opts = options || {};
  let previewEl = null;
  let previewCard = null;
  let previewIdValue = null;
  let showTimer = 0;
  let hideTimer = 0;
  // When a preview closed: the next one opens warm for HOVER_TIMING.warmWindow.
  let lastPreviewClosedAt = Number.NEGATIVE_INFINITY;
  let detailPage = null;
  let detailId = null;
  let detailTrigger = null;
  let detailPreviousFocus = null;
  // After a close, the preview does not reopen under a pointer that never
  // moved: it waits for a real move (or Tab).
  let hoverHold = false;
  let pointerX = -1;
  let pointerY = -1;
  let holdX = -1;
  let holdY = -1;
  // Level of the preview's own volume bar, kept across previews. `null` follows
  // the card's default, the quieter of the general and guild volumes; 100 % is
  // full volume, independent of the device settings.
  let previewLevel = null;
  let lastAudibleLevel = null;
  // A held volume slider keeps a transparent band around the preview: the
  // pointer may finish above or below it without taking the preview away.
  let volumeMargin = null;
  let volumeDragging = false;

  const grid = () => (typeof opts.grid === 'function' ? opts.grid() : null);
  const view = () => (typeof opts.view === 'function' ? opts.view() : null);
  // The preview belongs to one card, so the board is told which card is being
  // hovered: an open card menu only holds the preview off its own card.
  const hoverEnabled = (card) => (typeof opts.hoverEnabled === 'function'
    ? opts.hoverEnabled(card?.dataset?.itemId) !== false
    : true);
  const describe = (id) => (typeof opts.describe === 'function' ? opts.describe(id) : null);
  // The media bytes start loading while the pointer rests on a card, so the
  // preview and the window open on what the browser already has.
  const prefetcher = createMediaPrefetcher();
  // Durations and curves of styles/motion.css, read at the start of each
  // transition. The tokens already carry the reduced-motion values.
  const motion = () => readMotion(window.getComputedStyle(document.documentElement));

  function clearTimers() {
    window.clearTimeout(showTimer);
    window.clearTimeout(hideTimer);
    showTimer = 0;
    hideTimer = 0;
  }

  function cancelHide() {
    window.clearTimeout(hideTimer);
    hideTimer = 0;
  }

  function topInset() {
    const viewEl = view();
    const head = viewEl && typeof viewEl.querySelector === 'function'
      ? viewEl.querySelector('.board-head')
      : document.querySelector('.board-head');
    if (!head) {
      return TOP_INSET;
    }
    return Math.max(TOP_INSET, Math.round(bounds(head.getBoundingClientRect()).bottom) + 8);
  }

  function bottomInset() {
    const viewEl = view();
    const bottom = viewEl
      ? bounds(viewEl.getBoundingClientRect()).bottom
      : window.innerHeight;
    return Math.min(window.innerHeight, Math.round(bottom)) - 16;
  }

  function mediaSize(ratio) {
    const pad = 16;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const maxW = Math.min(MAX_WIDTH, vw - pad * 2);
    const maxMediaH = Math.min(vh * 0.62, 540);
    const shape = Number.isFinite(ratio) && ratio > 0.05 ? ratio : 16 / 10;
    let width;
    let height;

    if (shape >= maxW / maxMediaH) {
      width = maxW;
      height = width / shape;
    } else {
      height = maxMediaH;
      width = height * shape;
    }

    const maxTotal = Math.max(240, bottomInset() - topInset() - pad);
    if (height + BODY_ESTIMATE > maxTotal) {
      height = Math.max(150, maxTotal - BODY_ESTIMATE);
      width = height * shape;
    }

    return {
      width: Math.max(180, Math.round(width)),
      height: Math.max(120, Math.round(height)),
    };
  }

  function ratioFor(desc, thumb, kind) {
    const given = Number(desc && desc.ratio);
    if (Number.isFinite(given) && given > 0.05) {
      return given;
    }
    const visual = thumb.querySelector('img, video');
    const width = Number(visual && (visual.videoWidth || visual.naturalWidth)) || 0;
    const height = Number(visual && (visual.videoHeight || visual.naturalHeight)) || 0;
    if (width > 0 && height > 0) {
      return width / height;
    }
    return kind === 'audio' ? 2 : 16 / 10;
  }

  function close(immediate) {
    clearVolumeDragMargin();
    clearTimers();
    const el = previewEl;
    const card = previewCard;
    previewEl = null;
    previewCard = null;
    previewIdValue = null;

    if (!el) {
      return;
    }
    // A preview really existed and closed: the next one may open warm.
    lastPreviewClosedAt = performance.now();
    if (el._progress) {
      el._progress.destroy();
    }
    if (el._player) {
      el._player.pause();
    }
    el.classList.remove('is-in');
    el.classList.add('is-out');
    if (immediate) {
      if (card) {
        card.classList.remove('is-previewing');
      }
      el.remove();
    } else {
      const { previewClose } = motion();
      window.setTimeout(() => {
        // The card stays hidden while the preview shrinks back onto it: the
        // real card would otherwise show under the closing preview. It comes
        // back with the removal, unless a new preview took that card again.
        if (card && card !== previewCard) {
          card.classList.remove('is-previewing');
        }
        el.remove();
      }, previewClose + 40);
    }
  }

  function scheduleHide() {
    // While the slider is held, the pointer may leave the preview through the
    // top or bottom; the band keeps the preview alive until the release.
    if (volumeDragging) {
      return;
    }
    window.clearTimeout(showTimer);
    showTimer = 0;
    if (!previewEl) {
      return;
    }
    window.clearTimeout(hideTimer);
    hideTimer = window.setTimeout(() => close(false), HOVER_TIMING.hide);
  }

  function clearVolumeDragMargin() {
    volumeDragging = false;
    window.removeEventListener('pointerup', finishVolumeDrag, true);
    window.removeEventListener('pointercancel', finishVolumeDrag, true);
    if (volumeMargin) {
      volumeMargin.remove();
      volumeMargin = null;
    }
  }

  function finishVolumeDrag(event) {
    if (!volumeDragging) {
      return;
    }
    // A native slider keeps the pointer captured while it is dragged, so this
    // band never hovers under the hand. Decide from the release point itself:
    // a release inside the band leaves it in place until the pointer leaves,
    // so the card does not close under the hand that just set the level.
    const band = volumeMargin ? volumeMargin.getBoundingClientRect() : null;
    const released =
      event && Number.isFinite(event.clientX) && Number.isFinite(event.clientY)
        ? { x: event.clientX, y: event.clientY }
        : null;
    const inMargin = Boolean(
      band &&
        (released
          ? volumeDragMarginContains(band, released)
          : volumeMargin.matches(':hover')),
    );
    volumeDragging = false;
    if (inMargin) {
      return;
    }
    clearVolumeDragMargin();
    if (!(previewEl && previewEl.matches(':hover'))) {
      scheduleHide();
    }
  }

  function attachVolumeDragMargin() {
    if (!previewEl) {
      return;
    }
    clearVolumeDragMargin();
    const band = volumeDragMargin(previewEl.getBoundingClientRect(), VOLUME_DRAG_MARGIN);
    const margin = document.createElement('div');
    margin.className = 'hp-volume-drag-margin';
    margin.setAttribute('aria-hidden', 'true');
    margin.style.left = `${band.left}px`;
    margin.style.top = `${band.top}px`;
    margin.style.width = `${band.width}px`;
    margin.style.height = `${band.height}px`;
    margin.addEventListener('pointerenter', cancelHide);
    margin.addEventListener('pointerleave', () => {
      if (volumeDragging) {
        return;
      }
      clearVolumeDragMargin();
      scheduleHide();
    });
    document.body.appendChild(margin);
    volumeMargin = margin;
    volumeDragging = true;
    window.addEventListener('pointerup', finishVolumeDrag, true);
    window.addEventListener('pointercancel', finishVolumeDrag, true);
  }

  function scheduleShow(card) {
    if (hoverHold || !card || card === previewCard || detailPage) {
      return;
    }
    if (!hoverEnabled(card)) {
      return;
    }
    cancelHide();
    window.clearTimeout(showTimer);
    // Wait once for intent, then follow the pointer: an open preview, or one
    // that just closed, opens warm instead of waiting the cold delay.
    const delay = showDelay({
      previewOpen: Boolean(previewEl),
      now: performance.now(),
      lastClosedAt: lastPreviewClosedAt,
    });
    showTimer = window.setTimeout(() => {
      showTimer = 0;
      open(card);
    }, delay);
  }

  function holdHover() {
    hoverHold = true;
    holdX = pointerX;
    holdY = pointerY;
  }

  function attachProgress(player) {
    const track = h('div', { class: 'hp-progress', 'aria-hidden': 'true' });
    const fill = h('span');
    track.appendChild(fill);

    // The fill is a linear compositor animation over the time left; the player
    // events only say when to restart it. The old code painted it on every
    // `timeupdate` (about four a second) behind a 90 ms transition, which is
    // what stepped and froze the bar.
    const progress = createPreviewProgress({ player, fill });

    const seekTo = (event) => {
      if (!Number.isFinite(player.duration) || player.duration <= 0) {
        return;
      }
      const rect = track.getBoundingClientRect();
      if (!rect.width) {
        return;
      }
      const ratio = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
      player.currentTime = ratio * player.duration;
      progress.paint();
    };

    let seeking = false;
    track.addEventListener('pointerdown', (event) => {
      if (event.pointerType === 'mouse' && event.button !== 0) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      seeking = true;
      track.classList.add('is-seeking');
      try {
        track.setPointerCapture(event.pointerId);
      } catch {
        // Pointer capture is an optimisation; seeking still works without it.
      }
      seekTo(event);
    });
    track.addEventListener('pointermove', (event) => {
      if (!seeking) {
        return;
      }
      event.preventDefault();
      seekTo(event);
    });
    const endSeek = (event) => {
      if (!seeking) {
        return;
      }
      seeking = false;
      track.classList.remove('is-seeking');
      try {
        track.releasePointerCapture(event.pointerId);
      } catch {
        // The pointer may already be gone.
      }
    };
    track.addEventListener('pointerup', endSeek);
    track.addEventListener('pointercancel', endSeek);
    track.addEventListener('click', (event) => event.stopPropagation());
    return { track, destroy: progress.destroy };
  }

  function attachPlayer(el, media, player, card, desc) {
    el.classList.add('is-playable');
    el._player = player;
    // The bar sets an absolute level: 100 % is full volume. The element gets
    // that level through the overlay's perceptual curve, so it sounds like the
    // overlay at the same setting. Until the user moves it, the preview starts
    // at the card's default level.
    const defaultLevel = clampVolume(desc && desc.volume);
    const currentLevel = () => (previewLevel === null ? defaultLevel : previewLevel);
    // The tooltip names the action, not a sentence: the preview is paused
    // until its player says otherwise.
    media.title = 'Lecture';

    player.addEventListener('play', () => {
      el.classList.remove('is-paused');
      media.title = 'Pause';
    });
    player.addEventListener('pause', () => {
      if (!el.classList.contains('is-out')) {
        el.classList.add('is-paused');
        media.title = 'Lecture';
      }
    });

    const pauseVeil = document.createElement('div');
    pauseVeil.className = 'hp-paused';
    pauseVeil.setAttribute('aria-hidden', 'true');
    pauseVeil.appendChild(playIcon());
    media.appendChild(pauseVeil);

    const volume = document.createElement('div');
    volume.className = 'hp-volume';

    const mute = document.createElement('button');
    mute.type = 'button';
    mute.className = 'hp-mute';

    const pop = document.createElement('div');
    pop.className = 'hp-volume-pop';
    const slider = document.createElement('input');
    slider.type = 'range';
    slider.className = 'hp-volume-range';
    slider.min = '0';
    slider.max = '100';
    slider.step = '1';
    slider.setAttribute('aria-label', 'Volume de l’aperçu');
    slider.setAttribute('aria-orientation', 'vertical');
    pop.appendChild(slider);
    volume.append(mute, pop);

    const refreshVolume = () => {
      const percent = Math.round(currentLevel() * 100);
      slider.value = String(percent);
      slider.style.setProperty('--hp-volume-level', `${percent}%`);
      slider.setAttribute('aria-valuetext', `${percent} %`);
      const audible = !player.muted && player.volume > 0;
      mute.replaceChildren(speakerIcon(!audible));
      mute.setAttribute('aria-label', audible ? 'Couper le son' : 'Activer le son');
      mute.setAttribute('title', audible ? 'Couper le son' : 'Activer le son');
      mute.setAttribute('aria-pressed', String(audible));
    };
    player.volume = mediaPlaybackGain(currentLevel());
    player.muted = !playbackSettings().previewSound;
    refreshVolume();

    slider.addEventListener('input', () => {
      const level = clampVolume(Number(slider.value) / 100);
      previewLevel = level;
      player.volume = mediaPlaybackGain(level);
      if (level > 0) {
        lastAudibleLevel = level;
        player.muted = false;
      } else {
        player.muted = true;
      }
      refreshVolume();
    });
    mute.addEventListener('click', (event) => {
      event.stopPropagation();
      if (!player.muted && player.volume > 0) {
        player.muted = true;
      } else {
        // Coming back from zero restores the last level that produced sound,
        // rather than an arbitrary default.
        if (previewLevel === 0) {
          previewLevel = lastAudibleLevel;
        }
        player.volume = mediaPlaybackGain(currentLevel());
        player.muted = false;
      }
      refreshVolume();
    });
    player.addEventListener('volumechange', refreshVolume);
    slider.addEventListener('pointerdown', () => {
      attachVolumeDragMargin();
    });
    slider.addEventListener('pointerup', finishVolumeDrag);
    slider.addEventListener('pointercancel', finishVolumeDrag);
    media.appendChild(volume);

    media.tabIndex = 0;
    media.setAttribute('role', 'button');
    const refreshPlayLabel = () => {
      media.setAttribute('aria-label', el.classList.contains('is-paused') ? 'Lire l’aperçu' : 'Mettre l’aperçu en pause');
    };
    refreshPlayLabel();

    const togglePlayback = () => {
      if (player.paused) {
        const attempt = player.play();
        if (attempt && typeof attempt.catch === 'function') {
          attempt.catch(() => el.classList.add('is-paused'));
        }
      } else {
        player.pause();
      }
    };

    // Negative sentinels: the first press after page load must never be read
    // as the second press of a double click (performance.now() starts near 0).
    let lastMediaClickAt = Number.NEGATIVE_INFINITY;
    let lastMediaPointerAt = Number.NEGATIVE_INFINITY;
    let clickStartPaused = null;

    // The single click toggles at once, so a double click puts playback back
    // where it was before its first click. Nothing is pending when there was
    // no single click: this is a no-op then.
    const restoreBeforeOpen = () => {
      if (clickStartPaused !== null && player.paused !== clickStartPaused) {
        if (clickStartPaused) {
          player.pause();
        } else {
          const attempt = player.play();
          if (attempt && typeof attempt.catch === 'function') {
            attempt.catch(() => {});
          }
        }
      }
      clickStartPaused = null;
    };

    // pointerdown recognises a double press more reliably than click/detail.
    media.addEventListener('pointerdown', (event) => {
      if (event.target.closest('.hp-volume')) {
        return;
      }
      const now = performance.now();
      if (now - lastMediaPointerAt < DOUBLE_CLICK_WINDOW) {
        lastMediaPointerAt = 0;
        restoreBeforeOpen();
        openDetail(card);
        return;
      }
      lastMediaPointerAt = now;
    });

    media.addEventListener('click', (event) => {
      if (event.target.closest('.hp-volume')) {
        return;
      }
      const now = performance.now();
      const isDoubleClick = event.detail > 1 || now - lastMediaClickAt < DOUBLE_CLICK_WINDOW;
      if (isDoubleClick) {
        // The first click already toggled playback: restore the state the
        // preview had before the double click.
        restoreBeforeOpen();
        openDetail(card);
        return;
      }
      lastMediaClickAt = now;
      clickStartPaused = player.paused;
      togglePlayback();
    });

    media.addEventListener('dblclick', (event) => {
      if (event.target.closest('.hp-volume')) {
        return;
      }
      event.preventDefault();
      restoreBeforeOpen();
      openDetail(card);
    });

    media.addEventListener('keydown', (event) => {
      if (event.target.closest('.hp-volume')) {
        return;
      }
      if (event.key !== ' ' && event.key !== 'Enter') {
        return;
      }
      event.preventDefault();
      togglePlayback();
    });

    player.addEventListener('play', refreshPlayLabel);
    player.addEventListener('pause', refreshPlayLabel);
  }

  // A picture without a player opens the window on a double click. A single
  // click has nothing to toggle, so it does nothing.
  function attachStillOpen(media, card) {
    let lastPointerAt = Number.NEGATIVE_INFINITY;
    media.addEventListener('pointerdown', (event) => {
      const now = performance.now();
      if (now - lastPointerAt < DOUBLE_CLICK_WINDOW) {
        lastPointerAt = 0;
        openDetail(card);
        return;
      }
      lastPointerAt = now;
    });
    media.addEventListener('dblclick', (event) => {
      event.preventDefault();
      openDetail(card);
    });
  }

  function attachBodyOpen(body, card) {
    let lastBodyClickAt = Number.NEGATIVE_INFINITY;
    let lastBodyPointerAt = Number.NEGATIVE_INFINITY;
    body.addEventListener('pointerdown', (event) => {
      if (event.target.closest('.hp-media')) {
        return;
      }
      if (event.target.closest('button, textarea, input, a, select')) {
        return;
      }
      const now = performance.now();
      if (now - lastBodyPointerAt < DOUBLE_CLICK_WINDOW) {
        lastBodyPointerAt = 0;
        openDetail(card);
        return;
      }
      lastBodyPointerAt = now;
    });
    body.addEventListener('click', (event) => {
      if (event.target.closest('button, textarea, input, a, select')) {
        return;
      }
      const now = performance.now();
      if (event.detail > 1 || now - lastBodyClickAt < DOUBLE_CLICK_WINDOW) {
        lastBodyClickAt = 0;
        openDetail(card);
        return;
      }
      lastBodyClickAt = now;
    });
    body.addEventListener('dblclick', (event) => {
      if (event.target.closest('button, textarea, input, a, select')) {
        return;
      }
      event.preventDefault();
      openDetail(card);
    });
  }

  // The card under the pointer warms its media after a short debounce. This is
  // independent of the hover preview setting: a double click must open at once
  // even when the preview is off.
  function prefetchCard(card) {
    if (!card || detailPage) {
      return;
    }
    const id = card.dataset && card.dataset.itemId ? card.dataset.itemId : '';
    if (!id) {
      return;
    }
    const desc = describe(id);
    if (desc) {
      prefetcher.schedule(id, desc);
    }
  }

  // Leaving the card before the debounce drops the pending load: sweeping
  // across the grid starts no request. A load already started is kept warm.
  function cancelPrefetch(card) {
    const id = card && card.dataset && card.dataset.itemId ? card.dataset.itemId : '';
    if (id) {
      prefetcher.cancel(id);
    }
  }

  function build(card, desc) {
    const thumb = card.querySelector('.thumb');
    if (!thumb) {
      return null;
    }

    const kind = normalizeKind(desc);
    const ratio = ratioFor(desc, thumb, kind);
    const size = mediaSize(ratio);
    const url = typeof desc.url === 'string' ? desc.url : '';
    const poster = typeof desc.poster === 'string' ? desc.poster : '';
    // A prefetched element is already loading its bytes: reuse it instead of
    // starting a second request for the same media.
    const prefetched = prefetcher.take({ kind, url, expiresAt: desc.expiresAt });
    const el = h('div', { class: 'hp-overlay' });
    // A tall preview may be narrow; keep the title and buttons readable, and
    // never narrower than the card: its first frame is the card itself.
    el.style.width = `${Math.ceil(Math.max(260, size.width, card.getBoundingClientRect().width))}px`;

    const media = h('div', { class: 'hp-media' });
    media.style.height = `${size.height}px`;
    let player = null;
    let still = null;
    // The card player starts from the device switches: a video may autoplay
    // muted, an audio waits until its sound is on.
    const settings = playbackSettings();
    const playback = playbackPlan({ kind, autoplay: settings.previewAutoplay, sound: settings.previewSound });

    if (kind === 'video' && url) {
      const carried = prefetched && prefetched.kind === 'video' ? prefetched.element : null;
      player = carried || document.createElement('video');
      player.className = 'hp-video';
      if (!player.src) {
        player.src = url;
      }
      if (poster) {
        player.poster = poster;
      }
      player.muted = playback.muted;
      player.defaultMuted = true;
      player.loop = true;
      player.autoplay = false;
      player.playsInline = true;
      player.preload = carried ? 'auto' : 'metadata';
      if (playback.muted) {
        player.setAttribute('muted', '');
      } else {
        player.removeAttribute('muted');
      }
      player.setAttribute('playsinline', '');
      player.removeAttribute('aria-hidden');
      player.setAttribute('aria-label', 'Aperçu vidéo');
      media.appendChild(player);
    } else if (kind === 'audio') {
      if (poster) {
        media.appendChild(posterImage(poster));
      } else {
        media.appendChild(previewGlyph('audio'));
      }
      if (url) {
        const carried = prefetched && prefetched.kind === 'audio' ? prefetched.element : null;
        player = carried || document.createElement('audio');
        player.className = 'hp-audio-player';
        if (!player.src) {
          player.src = url;
        }
        player.muted = playback.muted;
        player.defaultMuted = true;
        player.loop = true;
        player.autoplay = false;
        player.preload = carried ? 'auto' : 'metadata';
        if (playback.muted) {
          player.setAttribute('muted', '');
        } else {
          player.removeAttribute('muted');
        }
        player.removeAttribute('aria-hidden');
        player.setAttribute('aria-label', 'Aperçu audio');
        media.insertBefore(player, media.firstChild);
      }
    } else {
      const source = url || poster;
      if (source) {
        if (prefetched && prefetched.kind === 'image' && url) {
          still = prefetched.element;
          still.alt = '';
          still.draggable = false;
          still.decoding = 'async';
          media.appendChild(still);
        } else {
          still = posterImage(source);
          media.appendChild(still);
        }
      } else {
        media.appendChild(previewGlyph(kind));
      }
    }

    if (player) {
      attachPlayer(el, media, player, card, desc);
    } else if (media.querySelector('img, .hp-audio')) {
      attachStillOpen(media, card);
    }

    const progress = player ? attachProgress(player) : null;

    const badge = card.querySelector('.card-badge');
    if (badge && badge.textContent && badge.textContent.trim()) {
      const pill = h('span', { class: 'hp-badge' });
      if (badge.classList.contains('is-live')
        || badge.classList.contains('is-on-air')
        || badge.classList.contains('is-air')) {
        pill.classList.add('is-live');
      }
      pill.textContent = badge.textContent.trim();
      media.appendChild(pill);
    }

    const body = h('div', { class: 'hp-body' }, desc.previewBody ? desc.previewBody() : null);
    attachBodyOpen(body, card);

    el.setAttribute('role', 'group');
    el.setAttribute('aria-label', `Aperçu de ${desc.label || 'média'}`);
    el.append(media, body);
    if (progress) {
      media.after(progress.track);
    }
    el._poster = still;
    el._playback = playback;
    el._progress = progress;
    return el;
  }

  function open(card) {
    if (detailPage || hoverHold) {
      return;
    }
    if (!card || card === previewCard) {
      return;
    }
    if (!hoverEnabled(card)) {
      return;
    }

    close(true);

    const thumb = card.querySelector('.thumb');
    if (!thumb) {
      return;
    }
    const id = card.dataset && card.dataset.itemId ? card.dataset.itemId : '';
    const desc = describe(id);
    if (!desc) {
      return;
    }
    // The preview loads the media itself; a pending prefetch would start a
    // second request for the same bytes. A warm entry is taken by build().
    prefetcher.cancel(id);
    const el = build(card, desc);
    if (!el) {
      return;
    }

    el.style.left = '0px';
    el.style.top = '0px';
    el.style.visibility = 'hidden';
    // The first frame is the card itself (growFrame), but the preview has to
    // be in the document to be measured. Transitions stay off while it is
    // placed: a first style without the custom properties would otherwise
    // animate from their fallbacks (scale 1, no clip) and the preview would
    // pop up larger than its card before settling.
    el.classList.add('is-placing');
    document.body.appendChild(el);

    if (el._player) {
      el._player.muted = el._playback.muted;
      if (el._playback.autoplay) {
        const attempt = el._player.play();
        if (attempt && typeof attempt.catch === 'function') {
          attempt.catch(() => el.classList.add('is-paused'));
        }
      } else {
        el.classList.add('is-paused');
      }
    }

    // The start frame has to read as the card itself, not as a second card
    // showing the preview's title, meta line and controls. A clone of the card,
    // sized to its rect and laid over the preview's own content, is that start
    // frame; media-detail.css fades it out while the preview grows over it.
    // The clone keeps the card styles but drops what it may not carry: the
    // live state classes, unique ids, and the players (a second video or audio
    // would autoplay and could not be duplicated). The poster img stays: it is
    // the exact pixels of the card's thumbnail. Absolutely positioned, the
    // ghost does not change the preview size measured below.
    const cardRect = card.getBoundingClientRect();
    const ghost = card.cloneNode(true);
    ghost.classList.remove('is-previewing', 'is-picked', 'is-selected');
    ghost.classList.add('hp-card-ghost');
    ghost.setAttribute('aria-hidden', 'true');
    ghost.setAttribute('inert', '');
    ghost.removeAttribute('id');
    for (const node of ghost.querySelectorAll('[id]')) {
      node.removeAttribute('id');
    }
    for (const node of ghost.querySelectorAll('video, audio')) {
      node.remove();
    }
    ghost.style.width = `${cardRect.width}px`;
    ghost.style.height = `${cardRect.height}px`;
    el.appendChild(ghost);

    const viewEl = view();
    const gridEl = grid();
    const content = viewEl
      ? viewEl.getBoundingClientRect()
      : gridEl
        ? gridEl.getBoundingClientRect()
        : null;
    const box = content
      ? bounds(content)
      : { left: 0, top: 0, right: window.innerWidth, bottom: window.innerHeight };
    const pad = 16;
    const safe = {
      left: Math.max(pad, box.left + 8),
      right: Math.min(window.innerWidth - pad, box.right - 8),
      top: Math.max(topInset(), box.top + 8),
      bottom: bottomInset(),
    };
    const size = { width: el.offsetWidth, height: el.offsetHeight };
    const media = el.querySelector('.hp-media');
    const mediaHeight = media ? media.offsetHeight : 0;
    const thumbHeight = thumb.getBoundingClientRect().height;
    const cardRadius = parseFloat(getComputedStyle(card).borderTopLeftRadius) || 0;
    const placed = placePreview(cardRect, size, safe);
    const frame = growFrame({
      card: cardRect,
      thumbHeight,
      cardRadius,
      box: { left: placed.left, top: placed.top, ...size },
      mediaHeight,
    });

    el.style.left = `${Math.round(placed.left)}px`;
    el.style.top = `${Math.round(placed.top)}px`;
    el.style.setProperty('--hp-x', `${frame.dx}px`);
    el.style.setProperty('--hp-y', `${frame.dy}px`);
    el.style.setProperty('--hp-s', String(frame.scale));
    el.style.setProperty('--hp-crop', `${frame.crop}px`);
    el.style.setProperty('--hp-shift', `${frame.shift}px`);
    el.style.setProperty('--hp-clip', `${frame.clipBottom}px`);
    el.style.setProperty('--hp-radius', `${frame.radius}px`);
    el.style.visibility = '';
    el.classList.remove('is-out');

    previewEl = el;
    previewCard = card;
    previewIdValue = id;
    card.classList.add('is-previewing');

    if (el._poster && !el._poster.complete) {
      el._poster.addEventListener('load', () => {
        // Only under the pointer: it may have left, or moved to another card.
        if (previewEl === el && !hideTimer && (card.matches(':hover') || el.matches(':hover'))) {
          close(true);
          open(card);
        }
      }, { once: true });
    }

    el.addEventListener('pointerenter', cancelHide);
    el.addEventListener('pointerleave', scheduleHide);

    // The preview covers its card whole, so the card itself no longer receives
    // a right click: the preview takes it, closes on the card it grew from
    // (the menu paints above it, so the move stays readable), and lets the
    // board open the same menu the card would have opened.
    el.addEventListener('contextmenu', (event) => {
      if (!opts.onCardMenu) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      close();
      opts.onCardMenu(event, card);
    });

    // Commit the card-sized start frame while transitions are off, then give
    // them back: the opening runs from that frame to the full preview.
    el.getBoundingClientRect();
    el.classList.remove('is-placing');
    window.requestAnimationFrame(() => el.classList.add('is-in'));
  }

  // The window covers the view, never the rail, the header or the on-air
  // footer: those stay visible and usable. Measured from the view's own box
  // (the floor is the whole window when there is no view yet).
  function placeShell(shell) {
    const viewEl = view();
    const rect = viewEl ? bounds(viewEl.getBoundingClientRect()) : null;
    const box = rect && rect.width > 0 && rect.height > 0
      ? rect
      : { left: 0, top: 0, width: window.innerWidth, height: window.innerHeight };
    shell.style.left = `${Math.round(box.left)}px`;
    shell.style.top = `${Math.round(box.top)}px`;
    shell.style.width = `${Math.round(box.width)}px`;
    shell.style.height = `${Math.round(box.height)}px`;
  }

  // Keeps the shell on the view while it is open: the window can be resized and
  // the view itself shrinks when the on-air bar appears. Returns the cleanup
  // that closeDetail calls before the close transition starts.
  function watchShell(page) {
    const shell = page._shell;
    const onResize = () => placeShell(shell);
    window.addEventListener('resize', onResize);
    const viewEl = view();
    const observer = viewEl && typeof ResizeObserver === 'function'
      ? new ResizeObserver(onResize)
      : null;
    if (observer && viewEl) {
      observer.observe(viewEl);
    }
    return () => {
      window.removeEventListener('resize', onResize);
      observer?.disconnect();
    };
  }

  function detailColumn(desc) {
    const node = desc.detailBody ? desc.detailBody() : null;
    if (!node) {
      return h('div', { class: 'hp-page-info' });
    }
    if (node.classList && !node.classList.contains('hp-page-info')) {
      node.classList.add('hp-page-info');
    }
    return node;
  }

  // The top bar's own controls, on the right: the board's favorite toggle today.
  function detailTools(desc) {
    return desc && typeof desc.tools === 'function' ? desc.tools() : null;
  }

  function detailVideo(url, poster, volume) {
    const settings = playbackSettings();
    const plan = playbackPlan({ kind: 'video', autoplay: settings.detailAutoplay, sound: settings.detailSound });
    const player = document.createElement('video');
    player.className = 'hp-page-video';
    player.src = url;
    if (poster) {
      player.poster = poster;
    }
    player.controls = false;
    player.autoplay = plan.autoplay;
    player.muted = plan.muted;
    player.defaultMuted = plan.muted;
    player.loop = true;
    player.playsInline = true;
    player.setAttribute('playsinline', '');
    if (plan.muted) {
      player.setAttribute('muted', '');
    }
    player.setAttribute('aria-label', 'Média en grand');
    player.volume = mediaPlaybackGain(volume);
    return player;
  }

  function detailAudio(url, volume) {
    const settings = playbackSettings();
    const plan = playbackPlan({ kind: 'audio', autoplay: settings.detailAutoplay, sound: settings.detailSound });
    const player = document.createElement('audio');
    player.src = url;
    player.controls = false;
    player.muted = plan.muted;
    player.defaultMuted = plan.muted;
    player.autoplay = plan.autoplay;
    player.preload = 'metadata';
    if (plan.muted) {
      player.setAttribute('muted', '');
    }
    player.setAttribute('aria-label', 'Média audio');
    player.volume = mediaPlaybackGain(volume);
    return player;
  }

  // Chromium's native bar lives in a closed shadow tree: its fill cannot be
  // animated linearly, and it is the last stepped bar of the overlay. This bar
  // keeps the media element for playback and replaces only its chrome, with the
  // same compositor-driven fill as the hover preview. It is attached to the
  // stage, never to a still: an image has no player and gets no bar.
  function attachPlayerControls(stage, player, kind, initialLevel) {
    player.controls = false;
    let level = clampVolume(initialLevel);

    const root = h('div', {
      class: 'hp-controls',
      role: 'group',
      'aria-label': 'Contrôles du média',
    });
    const playButton = h('button', {
      type: 'button',
      class: 'hp-ctl-button hp-ctl-play',
      'aria-label': 'Lire',
      title: 'Lire',
      onClick: () => togglePlay(),
    });
    const currentTime = h('span', { class: 'hp-ctl-clock' }, '0:00');
    const totalTime = h('span', { class: 'hp-ctl-clock' }, '0:00');
    const clock = h('span', { class: 'hp-ctl-time' }, currentTime, ' / ', totalTime);
    const fill = h('span', { class: 'hp-ctl-fill' });
    const track = h('div', {
      class: 'hp-ctl-track',
      role: 'slider',
      tabindex: '0',
      'aria-label': 'Progression',
      'aria-orientation': 'horizontal',
      'aria-valuemin': '0',
      'aria-valuemax': '0',
      'aria-valuenow': '0',
      'aria-valuetext': '0:00 sur 0:00',
    }, fill);
    const muteButton = h('button', {
      type: 'button',
      class: 'hp-ctl-button hp-ctl-mute',
      'aria-label': 'Couper le son',
      title: 'Couper le son',
      onClick: () => toggleMute(),
    });
    const volume = h('input', {
      type: 'range',
      class: 'hp-ctl-volume',
      min: '0',
      max: '100',
      step: '1',
      'aria-label': 'Volume',
      onInput: () => applyVolume(),
    });
    bindRangeFill(volume);
    const fullscreen = kind === 'video' && typeof stage.requestFullscreen === 'function'
      ? h('button', {
        type: 'button',
        class: 'hp-ctl-button hp-ctl-fullscreen',
        'aria-label': 'Plein écran',
        title: 'Plein écran',
        onClick: () => toggleFullscreen(),
      })
      : null;

    root.append(playButton, clock, track, muteButton, volume);
    if (fullscreen) {
      root.append(fullscreen);
    }

    // The same linear fill as the hover preview: the media clock only says
    // where to start, the compositor moves it between two frames.
    const progress = createPreviewProgress({ player, fill });
    let dragging = false;
    let destroyed = false;
    const bindings = [];

    const bind = (target, type, handler, options) => {
      target.addEventListener(type, handler, options);
      bindings.push([target, type, handler, options]);
    };

    const mediaDuration = () => (
      Number.isFinite(player.duration) && player.duration > 0 ? player.duration : 0
    );
    const clockOf = (seconds) => formatDuration(seconds) || '0:00';

    const syncPlay = () => {
      const paused = player.paused || player.ended;
      playButton.replaceChildren(paused ? playIcon() : pauseIcon());
      const label = paused ? 'Lire' : 'Mettre en pause';
      playButton.setAttribute('aria-label', label);
      playButton.title = label;
    };

    const syncTime = () => {
      const duration = mediaDuration();
      const position = Math.min(duration, Math.max(0, Number(player.currentTime) || 0));
      currentTime.textContent = clockOf(position);
      totalTime.textContent = clockOf(duration);
      track.setAttribute('aria-valuemax', String(Math.round(duration)));
      track.setAttribute('aria-valuenow', String(Math.floor(position)));
      track.setAttribute('aria-valuetext', `${clockOf(position)} sur ${clockOf(duration)}`);
    };

    const syncVolume = () => {
      const muted = player.muted || level === 0;
      muteButton.replaceChildren(speakerIcon(muted));
      const label = muted ? 'Activer le son' : 'Couper le son';
      muteButton.setAttribute('aria-label', label);
      muteButton.title = label;
      const percent = Math.round(level * 100);
      volume.value = String(percent);
      volume.setAttribute('aria-valuetext', `${percent} %`);
      paintRange(volume);
    };

    const syncFullscreen = () => {
      if (!fullscreen) {
        return;
      }
      const active = document.fullscreenElement === stage;
      fullscreen.replaceChildren(active ? fullscreenExitIcon() : fullscreenIcon());
      const label = active ? 'Quitter le plein écran' : 'Plein écran';
      fullscreen.setAttribute('aria-label', label);
      fullscreen.title = label;
    };

    const togglePlay = () => {
      if (player.paused || player.ended) {
        if (player.ended) {
          player.currentTime = 0;
          progress.paint();
          syncTime();
        }
        const attempt = player.play();
        if (attempt && typeof attempt.catch === 'function') {
          attempt.catch(() => {});
        }
      } else {
        player.pause();
      }
      syncPlay();
    };

    const applyVolume = () => {
      level = clampVolume(Number(volume.value) / 100);
      player.volume = mediaPlaybackGain(level);
      player.muted = level === 0;
      syncVolume();
    };

    const toggleMute = () => {
      if (player.muted || level === 0) {
        player.muted = false;
        if (level === 0) {
          level = 0.5;
          player.volume = mediaPlaybackGain(level);
        }
      } else {
        player.muted = true;
      }
      syncVolume();
    };

    const seekToPointer = (event) => {
      const duration = mediaDuration();
      if (!duration) {
        return;
      }
      player.currentTime = seekToRatio(ratioAt(event.clientX, track.getBoundingClientRect()), duration);
      progress.paint();
      syncTime();
    };

    const onPointerDown = (event) => {
      if (event.button !== undefined && event.button !== 0) {
        return;
      }
      dragging = true;
      if (typeof track.setPointerCapture === 'function' && event.pointerId !== undefined) {
        track.setPointerCapture(event.pointerId);
      }
      if (typeof track.focus === 'function') {
        track.focus({ preventScroll: true });
      }
      event.preventDefault();
      seekToPointer(event);
    };

    const onPointerMove = (event) => {
      if (!dragging) {
        return;
      }
      event.preventDefault();
      seekToPointer(event);
    };

    const onPointerUp = (event) => {
      if (!dragging) {
        return;
      }
      dragging = false;
      if (typeof track.releasePointerCapture === 'function' && event.pointerId !== undefined) {
        track.releasePointerCapture(event.pointerId);
      }
    };

    const seekByKey = (event) => {
      const duration = mediaDuration();
      if (!duration) {
        return;
      }
      let next = null;
      if (event.key === 'ArrowLeft') {
        next = seekBy(player.currentTime, duration, -5);
      } else if (event.key === 'ArrowRight') {
        next = seekBy(player.currentTime, duration, 5);
      } else if (event.key === 'PageDown') {
        next = seekBy(player.currentTime, duration, -10);
      } else if (event.key === 'PageUp') {
        next = seekBy(player.currentTime, duration, 10);
      } else if (event.key === 'Home') {
        next = 0;
      } else if (event.key === 'End') {
        next = duration;
      }
      if (next === null) {
        return;
      }
      event.preventDefault();
      player.currentTime = next;
      progress.paint();
      syncTime();
    };

    const toggleFullscreen = () => {
      if (!document.fullscreenElement) {
        const attempt = stage.requestFullscreen();
        if (attempt && typeof attempt.catch === 'function') {
          attempt.catch(() => {});
        }
      } else if (typeof document.exitFullscreen === 'function') {
        const attempt = document.exitFullscreen();
        if (attempt && typeof attempt.catch === 'function') {
          attempt.catch(() => {});
        }
      }
    };

    bind(player, 'play', syncPlay);
    bind(player, 'playing', syncPlay);
    bind(player, 'pause', syncPlay);
    bind(player, 'ended', syncPlay);
    bind(player, 'timeupdate', syncTime);
    bind(player, 'durationchange', syncTime);
    bind(player, 'loadedmetadata', syncTime);
    bind(player, 'seeking', syncTime);
    bind(player, 'seeked', syncTime);
    bind(player, 'emptied', () => {
      syncTime();
      syncPlay();
    });
    bind(player, 'volumechange', syncVolume);
    bind(track, 'pointerdown', onPointerDown);
    bind(track, 'pointermove', onPointerMove);
    bind(track, 'pointerup', onPointerUp);
    bind(track, 'pointercancel', onPointerUp);
    bind(track, 'keydown', seekByKey);
    if (fullscreen) {
      bind(document, 'fullscreenchange', syncFullscreen);
    }

    syncPlay();
    syncTime();
    syncVolume();
    syncFullscreen();
    stage.appendChild(root);

    return {
      root,
      destroy() {
        if (destroyed) {
          return;
        }
        destroyed = true;
        for (const [target, type, handler, options] of bindings) {
          target.removeEventListener(type, handler, options);
        }
        bindings.length = 0;
        progress.destroy();
        root.remove();
      },
    };
  }

  function buildStage(desc, carriedVisual, carriedPlayer, prefetched) {
    const kind = normalizeKind(desc);
    const url = typeof desc.url === 'string' ? desc.url : '';
    const poster = typeof desc.poster === 'string' ? desc.poster : '';
    const volume = clampVolume(desc.volume);
    const stage = h('div', { class: 'hp-page-stage' });
    const art = poster || (kind === 'image' ? url : '');
    if (art) {
      stage.style.setProperty('--hp-art', `url("${escapeCssUrl(art)}")`);
    }

    let player = null;
    if (kind === 'video' && url) {
      const carriedVideo = carriedVisual && carriedVisual.tagName === 'VIDEO' ? carriedVisual : null;
      const prefetchedVideo = prefetched && prefetched.kind === 'video' ? prefetched.element : null;
      player = carriedVideo || prefetchedVideo || detailVideo(url, poster, volume);
      if (!carriedVideo) {
        if (prefetchedVideo) {
          // The prefetched element was created detached: it joins the stage
          // before the flight, which then only carries the page itself.
          prefetchedVideo.className = 'hp-page-video';
          prefetchedVideo.controls = false;
          prefetchedVideo.loop = true;
          prefetchedVideo.autoplay = false;
          prefetchedVideo.muted = true;
          if (poster) {
            prefetchedVideo.poster = poster;
          }
          prefetchedVideo.removeAttribute('aria-hidden');
          prefetchedVideo.setAttribute('aria-label', 'Média en grand');
          prefetchedVideo.setAttribute('playsinline', '');
        }
        stage.appendChild(player);
      }
    } else if (kind === 'audio') {
      const audioBox = h('div', { class: 'hp-page-audio' });
      if (poster) {
        audioBox.appendChild(posterImage(poster));
      } else {
        audioBox.appendChild(stageGlyph('audio'));
      }
      if (carriedPlayer && carriedPlayer.tagName === 'AUDIO') {
        // The same element keeps playing across the move.
        player = carriedPlayer;
        player.className = '';
        player.controls = false;
        player.setAttribute('aria-label', 'Média audio');
        audioBox.appendChild(player);
      } else if (prefetched && prefetched.kind === 'audio') {
        player = prefetched.element;
        player.className = '';
        player.controls = false;
        player.muted = true;
        player.removeAttribute('aria-hidden');
        player.setAttribute('aria-label', 'Média audio');
        audioBox.appendChild(player);
      } else if (url) {
        player = detailAudio(url, volume);
        audioBox.appendChild(player);
      }
      stage.appendChild(audioBox);
    } else if (carriedVisual && carriedVisual.tagName === 'IMG') {
      // The preview picture lands in the stage at the end of the flight.
    } else if (prefetched && prefetched.kind === 'image') {
      const image = prefetched.element;
      image.alt = '';
      image.draggable = false;
      image.decoding = 'async';
      stage.appendChild(image);
    } else if (url || poster) {
      stage.appendChild(posterImage(url || poster));
    } else {
      stage.appendChild(stageGlyph(kind));
    }

    if (player) {
      player.controls = false;
    }
    const controls = player ? attachPlayerControls(stage, player, kind, volume) : null;
    return { stage, player, kind, poster, controls };
  }

  function transitionOut(page, card, thumb) {
    const { detailClose: duration, easeMove } = motion();
    const shell = page._shell;
    const backdrop = page._backdrop;
    const stage = page.querySelector('.hp-page-stage');
    // Measured before the page starts shrinking onto the card.
    const stageBox = stage ? contentBoxOf(stage) : null;
    const animations = [];
    let flight = null;
    let surface = null;

    shell.style.pointerEvents = 'none';
    shell.classList.add('is-transitioning');
    page.dataset.transition = 'closing';

    const shellRect = shell.getBoundingClientRect();
    const pageRect = page.getBoundingClientRect();
    const pageStyle = getComputedStyle(page);
    const pageRadiusCss = pageStyle.borderTopLeftRadius;
    const pageRadius = parseFloat(pageStyle.borderTopLeftRadius) || 0;
    const cardRect = card.getBoundingClientRect();
    const cardRadiusCss = getComputedStyle(card).borderTopLeftRadius;
    const cardRadius = parseFloat(cardRadiusCss) || 0;

    // Closing during the backdrop's fade-in starts from where it is, not from
    // full opacity: read it before stopping that fade.
    const backdropFrom = Number(getComputedStyle(backdrop).opacity);
    if (backdrop._fadeIn) {
      backdrop._fadeIn.cancel();
      backdrop._fadeIn = null;
    }
    animations.push(backdrop.animate([{ opacity: backdropFrom }, { opacity: 0 }], {
      duration,
      easing: 'ease-in-out',
      fill: 'forwards',
    }));

    const box = (rect, radius) => ({
      left: `${rect.left - shellRect.left}px`,
      top: `${rect.top - shellRect.top}px`,
      width: `${rect.width}px`,
      height: `${rect.height}px`,
      borderRadius: radius,
    });

    surface = document.createElement('div');
    surface.className = 'hp-surface';
    surface.setAttribute('aria-hidden', 'true');
    surface.style.background = pageStyle.backgroundColor;
    surface.style.border = `${pageStyle.borderTopWidth} solid ${pageStyle.borderTopColor}`;
    surface.style.boxShadow = pageStyle.boxShadow;
    Object.assign(surface.style, box(cardRect, cardRadiusCss), { opacity: '0' });
    shell.insertBefore(surface, page);
    animations.push(surface.animate([
      { ...box(pageRect, pageRadiusCss), opacity: 1, offset: 0 },
      { opacity: 1, offset: 0.7 },
      { ...box(cardRect, cardRadiusCss), opacity: 0, offset: 1 },
    ], { duration, easing: easeMove, fill: 'forwards' }));

    // Closing reverses the container transform: the content shrinks onto the
    // card with the surface instead of being masked in place. The opening
    // frames are reversed and their offsets recomputed so the keyframes stay
    // ordered from 0 (the page) to 1 (the card). The shell is removed when the
    // transition ends, so the inline transform origin has nothing to restore.
    const pageFrames = pageFlightFrames({
      origin: cardRect,
      page: pageRect,
      originRadius: cardRadius,
      pageRadius,
    });
    page.style.transformOrigin = '0 0';
    animations.push(page.animate(
      pageFrames
        .reverse()
        .map((frame) => ({ ...frame, offset: 1 - frame.offset })),
      { duration, easing: easeMove, fill: 'forwards' },
    ));
    // The content leaves first: it must not pile up inside the card.
    animations.push(page.animate([
      { opacity: 1, offset: 0 },
      { opacity: 0, offset: 0.35 },
      { opacity: 0, offset: 1 },
    ], { duration, easing: 'linear', fill: 'forwards' }));

    const visual = stage ? stage.querySelector(':scope > video, :scope > img') : null;
    const thumbRect = thumb.getBoundingClientRect();
    if (visual && thumbRect.width > 0 && thumbRect.height > 0) {
      const from = containRect(stageBox, mediaRatioOf(visual, page._ratio));
      if (from.width > 0 && from.height > 0) {
        const thumbRadius = returnRadius(radiiOf(thumb), thumbRect.top, cardRect.top, cardRadiusCss);
        const geometry = (rect) => ({
          left: `${rect.left}px`,
          top: `${rect.top}px`,
          width: `${rect.width}px`,
          height: `${rect.height}px`,
        });
        flight = document.createElement('div');
        flight.className = 'hp-flight is-return';
        Object.assign(flight.style, geometry(thumbRect), { opacity: '0' });
        document.body.appendChild(flight);
        if (visual.tagName === 'VIDEO') {
          visual.controls = false;
        }
        // Moving a media element within the same task does not pause it.
        flight.appendChild(visual);
        // The media stays in the stage of the shrinking page, then lets go
        // onto the thumbnail, on the page's easing.
        const track = mediaTrackFrames({
          origin: cardRect,
          page: pageRect,
          media: from,
          outside: thumbRect,
          leaving: true,
        });
        animations.push(flight.animate(
          track.map(({ offset, rect }) => ({ offset, ...geometry(rect) })),
          { duration, easing: easeMove, fill: 'forwards' },
        ));
        animations.push(flight.animate([
          { borderRadius: '0px', opacity: 1, offset: 0 },
          { opacity: 1, offset: 0.72 },
          { borderRadius: thumbRadius, opacity: 0, offset: 1 },
        ], { duration, easing: easeMove, fill: 'forwards' }));
      }
    }

    let finished = false;
    const finish = () => {
      if (finished) {
        return;
      }
      finished = true;
      if (page._controls) {
        page._controls.destroy();
        page._controls = null;
      }
      if (page._player) {
        page._player.pause();
      }
      flight?.remove();
      surface?.remove();
      shell.remove();
      notifyClosed(page);
    };
    playTimeline(animations, duration, { onDone: finish }, CLOCK);
  }

  function notifyClosed(page) {
    if (!page || page._closed) {
      return;
    }
    page._closed = true;
    if (typeof opts.onClose === 'function') {
      opts.onClose(page._id);
    }
  }

  function fadeOut(page) {
    const shell = page._shell;
    if (page._controls) {
      page._controls.destroy();
      page._controls = null;
    }
    if (page._player) {
      page._player.pause();
    }
    if (!shell) {
      page.remove();
      notifyClosed(page);
      return;
    }
    shell.style.pointerEvents = 'none';
    let removed = false;
    const remove = () => {
      if (removed) {
        return;
      }
      removed = true;
      shell.remove();
      notifyClosed(page);
    };
    if (typeof shell.animate !== 'function') {
      remove();
      return;
    }
    const fade = shell.animate([{ opacity: 1 }, { opacity: 0 }], {
      duration: 140,
      easing: 'ease-in',
      fill: 'forwards',
    });
    fade.finished.then(remove, remove);
    window.setTimeout(remove, 400);
  }

  function transitionIn(page, shell, backdrop, carry) {
    const { detailOpen: duration, easeEnter, backdropIn, backdropLead } = motion();
    const animations = [];
    const visual = carry && carry.visual ? carry.visual : null;
    let flight = null;
    let surface = null;
    // The page's own flight, when it travels from an origin rect: the media
    // follows it instead of flying straight to the page's final place.
    let pageTrack = null;
    let finished = false;

    page.dataset.transition = 'preparing';
    shell.style.pointerEvents = 'none';
    shell.classList.add('is-transitioning');

    const finish = () => {
      if (finished) {
        return;
      }
      finished = true;
      if (visual && carry.slot && visual.parentNode !== carry.slot) {
        carry.slot.appendChild(visual);
      }
      if (carry && carry.land) {
        carry.land();
      }
      flight?.remove();
      surface?.remove();
      if (carry && carry.source) {
        // The preview is removed without going through its close(): its fill
        // would otherwise keep listening to the player now owned by the page.
        if (carry.source._progress) {
          carry.source._progress.destroy();
          carry.source._progress = null;
        }
        carry.source.remove();
      }
      for (const animation of animations) {
        animation.cancel();
      }
      page.style.transformOrigin = '';
      shell.classList.remove('is-transitioning');
      shell.style.pointerEvents = '';
      page._transition = null;
      page.dataset.transition = 'done';
      if (detailPage === page) {
        page.focus({ preventScroll: true });
      }
    };

    // Never exactly 0: at zero opacity Chromium throws away the prepared layer
    // and repaints in the middle of the movement.
    const HIDDEN = 0.01;
    // The backdrop stays out of the way while the window flies in from its
    // card, then darkens progressively: it starts backdropLead before the
    // window lands and keeps going after. Kept out of `animations`, which
    // finish() cancels on landing. Hidden during the delay (fill backwards).
    backdrop._fadeIn = backdrop.animate([{ opacity: HIDDEN }, { opacity: 1 }], {
      delay: Math.max(0, duration - backdropLead),
      duration: backdropIn,
      easing: 'ease-in-out',
      fill: 'backwards',
    });

    const originRect = carry && carry.origin && carry.origin.isConnected
      ? carry.origin.getBoundingClientRect()
      : null;
    // The stage is measured before the page starts travelling: once its
    // animation exists, the page's box is the transformed, scaled-down one.
    const slotBox = visual && carry.slot ? contentBoxOf(carry.slot) : null;
    if (originRect && originRect.width > 0 && originRect.height > 0) {
      const shellRect = shell.getBoundingClientRect();
      const pageRect = page.getBoundingClientRect();
      pageTrack = { origin: originRect, page: pageRect };
      const pageStyle = getComputedStyle(page);
      const pageRadiusCss = pageStyle.borderTopLeftRadius;
      const pageRadius = parseFloat(pageStyle.borderTopLeftRadius) || 0;
      const originRadiusCss = carry.origin === carry.source
        ? getComputedStyle(carry.origin).borderTopLeftRadius
        : '10px';
      const originRadius = parseFloat(originRadiusCss) || 0;
      const box = (rect, radius) => ({
        left: `${rect.left - shellRect.left}px`,
        top: `${rect.top - shellRect.top}px`,
        width: `${rect.width}px`,
        height: `${rect.height}px`,
        borderRadius: radius,
      });
      surface = document.createElement('div');
      surface.className = 'hp-surface';
      surface.setAttribute('aria-hidden', 'true');
      surface.style.background = pageStyle.backgroundColor;
      surface.style.border = `${pageStyle.borderTopWidth} solid ${pageStyle.borderTopColor}`;
      surface.style.boxShadow = pageStyle.boxShadow;
      Object.assign(surface.style, box(pageRect, pageRadiusCss));
      shell.insertBefore(surface, page);
      animations.push(surface.animate([
        box(originRect, originRadiusCss),
        box(pageRect, pageRadiusCss),
      ], { duration, easing: easeEnter }));
      // The content travels with the frame: the page is laid out at its final
      // place and this one animation carries it from the origin rect, with
      // the surface's box and timing, instead of uncovering it in place with
      // a clip mask. transform-origin 0 0 matches pageFlightFrames; the CSS
      // origin (50% 40%) is restored in finish() for the fallback branch.
      page.style.transformOrigin = '0 0';
      animations.push(page.animate(pageFlightFrames({
        origin: originRect,
        page: pageRect,
        originRadius,
        pageRadius,
      }), { duration, easing: easeEnter }));
      animations.push(page.animate([
        { opacity: HIDDEN, offset: 0 },
        { opacity: HIDDEN, offset: 0.15 },
        { opacity: 1, offset: 0.7 },
        { opacity: 1, offset: 1 },
      ], { duration, easing: 'linear' }));
    } else {
      animations.push(page.animate([
        { opacity: HIDDEN, transform: 'translateY(8px) scale(0.985)' },
        { opacity: 1, transform: 'none' },
      ], { duration, easing: easeEnter }));
    }

    if (carry && carry.source) {
      // The preview sits above the backdrop, covers the start of the surface
      // and fades early so its text does not linger.
      carry.source.style.zIndex = '185';
      animations.push(carry.source.animate([{ opacity: 1 }, { opacity: 0 }], {
        duration: Math.round(duration * 0.22),
        easing: 'linear',
        fill: 'forwards',
      }));
    }

    if (visual && carry.slot) {
      const from = containRect(visual.getBoundingClientRect(), carry.ratio);
      const to = containRect(slotBox, carry.ratio);
      if (from.width > 0 && from.height > 0 && to.width > 0 && to.height > 0) {
        flight = document.createElement('div');
        flight.className = 'hp-flight';
        flight.style.left = `${to.left}px`;
        flight.style.top = `${to.top}px`;
        flight.style.width = `${to.width}px`;
        flight.style.height = `${to.height}px`;
        document.body.appendChild(flight);
        // Moving the media within the same task does not pause it.
        flight.appendChild(visual);
        // With the page travelling, the media rides with it: it leaves the
        // preview and settles into the stage of the moving page, on the
        // page's easing so both stay together.
        const keyframes = pageTrack
          ? mediaTrackFrames({ ...pageTrack, media: to, outside: from })
            .map(({ offset, rect }) => ({ offset, transform: flightTransform(rect, to) }))
          : [{ transform: flightTransform(from, to) }, { transform: 'none' }];
        animations.push(flight.animate(keyframes, { duration, easing: easeEnter }));
      } else {
        carry.slot.appendChild(visual);
      }
    }

    page._transition = {
      finish: playTimeline(animations, duration, {
        onStart: () => {
          page.dataset.transition = 'running';
        },
        onDone: finish,
      }, CLOCK),
    };
  }

  function openDetail(card) {
    if (detailPage || !card) {
      return;
    }
    const thumb = card.querySelector('.thumb');
    if (!thumb) {
      return;
    }
    const id = card.dataset && card.dataset.itemId ? card.dataset.itemId : '';
    const desc = describe(id);
    if (!desc) {
      return;
    }
    // Reset transient window content before the information column is built,
    // so the first render already shows the fresh state.
    if (typeof opts.onBeforeOpen === 'function') {
      opts.onBeforeOpen(id);
    }

    // A preview of this card is the origin of the transition: its player and
    // its picture are carried, never recreated.
    const fromPreview = previewEl && previewCard === card ? previewEl : null;
    detailPreviousFocus = document.activeElement;
    detailTrigger = thumb;
    const carriedPlayer = fromPreview && fromPreview._player ? fromPreview._player : null;
    const carriedVisual = fromPreview && normalizeKind(desc) !== 'audio'
      ? fromPreview.querySelector('.hp-media > video, .hp-media > img')
      : null;

    if (fromPreview) {
      clearTimers();
      previewEl = null;
      previewCard = null;
      previewIdValue = null;
      card.classList.remove('is-previewing');
    } else {
      close(true);
    }

    const kind = normalizeKind(desc);
    const url = typeof desc.url === 'string' ? desc.url : '';
    const poster = typeof desc.poster === 'string' ? desc.poster : '';
    // A prefetched element is already loading: the window must use it, not
    // create a second player that starts from zero. A preview already took it.
    const prefetched = fromPreview ? null : prefetcher.take({ kind, url, expiresAt: desc.expiresAt });
    const carriedVideo = carriedVisual && carriedVisual.tagName === 'VIDEO' ? carriedVisual : null;
    const carriedImage = carriedVisual && carriedVisual.tagName === 'IMG' ? carriedVisual : null;
    const page = h('section', {
      class: 'hp-page',
      'data-layout': 'atelier',
      tabindex: '-1',
      role: 'dialog',
      'aria-modal': 'true',
      'aria-label': `Détails de ${desc.label || 'média'}`,
    });

    const back = h(
      'button',
      {
        type: 'button',
        class: 'btn is-quiet hp-back',
        'aria-label': 'Retour à la board',
        onClick: () => closeDetail(),
      },
      h('span', { class: 'hp-back-glyph' }, backIcon()),
      h('span', null, 'Board'),
    );
    const crumb = h('span', { class: 'hp-page-crumb' }, 'Détails du média');
    const tools = h('div', { class: 'hp-page-tools' }, detailTools(desc));
    const top = h('div', { class: 'hp-page-top' }, back, crumb, tools);

    const built = buildStage(desc, carriedVisual, carriedPlayer, prefetched);
    const info = detailColumn(desc);
    const body = h('div', { class: 'hp-page-grid' }, built.stage, info);
    page.append(top, body);

    const shell = h('div', { class: 'hp-page-shell' });
    const backdrop = h('div', { class: 'hp-backdrop' });
    shell.append(backdrop, page);
    shell.addEventListener('click', (event) => {
      // A double click triggered on pointerdown may let the release click fall
      // on the backdrop: ignore the opening click for a moment.
      const outside = event.target === shell || event.target === backdrop;
      if (outside && performance.now() - page._openedAt > 360) {
        closeDetail();
      }
    });
    shell.addEventListener('wheel', (event) => {
      if (event.target === shell || event.target === backdrop) {
        event.preventDefault();
      }
    }, { passive: false });

    const player = built.player;
    page._card = card;
    page._thumb = thumb;
    page._backdrop = backdrop;
    page._shell = shell;
    page._openedAt = performance.now();
    page._controls = built.controls;
    page._player = player;
    page._id = id;
    page._desc = desc;
    page._info = info;
    page._tools = tools;
    page._ratio = ratioFor(desc, thumb, kind);

    // The shell is measured from the view before it is inserted: its
    // transitions then work in the shell's own coordinates.
    placeShell(shell);
    document.body.appendChild(shell);
    detailPage = page;
    detailId = id;
    page._unwatch = watchShell(page);

    // The details player follows the detail switches, whatever element it is:
    // a player carried from the preview joins the window under its rules, and
    // a carried one would otherwise keep the preview's pause or sound state.
    const settings = playbackSettings();
    const plan = playbackPlan({ kind, autoplay: settings.detailAutoplay, sound: settings.detailSound });
    if (player) {
      player.autoplay = plan.autoplay;
      player.muted = plan.muted;
      player.defaultMuted = plan.muted;
      if (plan.muted) {
        player.setAttribute('muted', '');
      } else {
        player.removeAttribute('muted');
      }
      player.volume = mediaPlaybackGain(desc.volume);
      if (plan.autoplay) {
        const attempt = player.play();
        if (attempt && typeof attempt.catch === 'function') {
          attempt.catch(() => {});
        }
      } else {
        player.pause();
      }
    }

    const flyVisual = fromPreview
      ? (kind === 'video' && url ? carriedVideo : carriedImage)
      : null;
    if (flyVisual && poster && flyVisual === carriedVideo) {
      // The preview fades while the video flies: leave the poster in place
      // instead of an empty frame.
      const stayBehind = document.createElement('img');
      stayBehind.src = poster;
      stayBehind.alt = '';
      stayBehind.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;object-fit:contain';
      flyVisual.parentNode?.insertBefore(stayBehind, flyVisual);
    }

    transitionIn(page, shell, backdrop, {
      origin: fromPreview || thumb,
      visual: flyVisual,
      slot: built.stage,
      ratio: mediaRatioOf(flyVisual, page._ratio),
      source: fromPreview,
      land: () => {
        if (!flyVisual) {
          return;
        }
        if (flyVisual === carriedVideo) {
          flyVisual.className = 'hp-page-video';
          flyVisual.controls = false;
          flyVisual.setAttribute('aria-label', 'Média en grand');
        } else {
          flyVisual.className = '';
          flyVisual.draggable = false;
        }
      },
    });

    if (typeof opts.onOpen === 'function') {
      opts.onOpen(id);
    }
  }

  function refreshDetail() {
    const page = detailPage;
    if (!page) {
      return;
    }
    const desc = describe(detailId);
    if (!desc) {
      return;
    }
    const current = page._info;
    const next = detailColumn(desc);
    if (current && current.parentNode) {
      current.replaceWith(next);
    } else {
      const body = page.querySelector('.hp-page-grid');
      if (body) {
        body.appendChild(next);
      }
    }
    page._info = next;
    const tools = page._tools;
    if (tools) {
      const nextTools = detailTools(desc);
      tools.replaceChildren(...(nextTools ? [nextTools] : []));
    }
    page._desc = desc;
    page.setAttribute('aria-label', `Détails de ${desc.label || 'média'}`);
  }

  function closeDetail() {
    const page = detailPage;
    if (!page) {
      return;
    }
    // Closing during the opening first finishes the opening: the media is back
    // in the stage before it flies home.
    if (page._transition) {
      page._transition.finish();
    }
    detailPage = null;
    detailId = null;
    // Stop following the view before the shell starts its close transition:
    // the flight measures the shell's current box, which must not move.
    if (page._unwatch) {
      page._unwatch();
      page._unwatch = null;
    }
    holdHover();
    const card = page._card;
    const thumb = page._thumb;
    const shell = page._shell;
    const viewEl = view();
    const viewRect = viewEl ? viewEl.getBoundingClientRect() : null;
    const box = viewRect
      ? { top: viewRect.top, bottom: bounds(viewRect).bottom }
      : { top: 0, bottom: window.innerHeight };
    const visible = Boolean(
      card
      && thumb
      && card.isConnected
      && thumb.isConnected
      && isVisibleIn(card.getBoundingClientRect(), box),
    );

    if (shell && card && thumb && visible) {
      transitionOut(page, card, thumb);
    } else if (shell) {
      // The card was deleted or is out of view: a simple fade.
      fadeOut(page);
    } else {
      if (page._controls) {
        page._controls.destroy();
        page._controls = null;
      }
      if (page._player) {
        page._player.pause();
      }
      page.remove();
      notifyClosed(page);
    }

    const target = detailTrigger && detailTrigger.isConnected ? detailTrigger : detailPreviousFocus;
    detailTrigger = null;
    detailPreviousFocus = null;
    if (target && typeof target.focus === 'function') {
      window.requestAnimationFrame(() => {
        // A newer window may have opened during the closing transition.
        if (!detailPage) {
          target.focus({ preventScroll: true });
        }
      });
    }
  }

  // --- public controller ---------------------------------------------------

  const controller = {
    prefetchCard(card) {
      prefetchCard(card);
    },
    cancelPrefetch(card) {
      cancelPrefetch(card);
    },
    hoverCard(card) {
      if (!hoverEnabled(card) || detailPage) {
        return;
      }
      if (card && card === previewCard) {
        cancelHide();
        return;
      }
      scheduleShow(card);
    },
    leaveCard() {
      // A preview that grows under a still pointer gets its pointerenter
      // (which cancels a pending hide) before the card's mouseleave: the
      // pointer did not leave, it is on the preview, so nothing to hide.
      if (previewEl && previewEl.matches(':hover')) {
        return;
      }
      // The band around a held volume slider is part of the preview's hover
      // surface: leaving the card towards it must not close the preview.
      if (volumeMargin && volumeMargin.matches(':hover')) {
        return;
      }
      scheduleHide();
    },
    closePreview() {
      close(true);
    },
    openDetail(card) {
      openDetail(card);
    },
    refreshDetail() {
      refreshDetail();
    },
    closeDetail() {
      closeDetail();
    },
    // Follows the device's default level, like the board's other players: the
    // media keeps its own mute state, only its volume follows the sliders.
    setVolume(volume) {
      const page = detailPage;
      if (page && page._player) {
        page._player.volume = mediaPlaybackGain(volume);
      }
    },
    openId() {
      return detailId;
    },
    previewId() {
      return previewIdValue;
    },
  };

  const record = {
    movePointer(x, y) {
      pointerX = x;
      pointerY = y;
      if (!hoverHold) {
        return;
      }
      const released = holdReleased({ x: holdX, y: holdY }, { x, y });
      if (!released || Math.hypot(x - holdX, y - holdY) < HOLD_DISTANCE) {
        return;
      }
      hoverHold = false;
      const target = document.elementFromPoint ? document.elementFromPoint(x, y) : null;
      const card = target && target.closest ? target.closest('.card') : null;
      const gridEl = grid();
      if (card && gridEl && gridEl.contains(card)) {
        scheduleShow(card);
      }
    },
    releaseHold() {
      hoverHold = false;
    },
    hasDetail() {
      return Boolean(detailPage);
    },
    closePreview() {
      close(true);
    },
  };

  installGlobalListeners();
  controllers.add(record);
  return controller;
}
