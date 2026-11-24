// Live preview of a media on the overlay: the display zone at the stage's own
// scale, the author header the overlay reserves, and the media at the size the
// overlay will give it.
//
// The overlay sizes a media with its natural size × the scale chosen on the
// board, capped by the zone. Rebuilding that frame here shows a scale change at
// once instead of on the next play.

import { h } from '../../core/dom.js';
import {
  OVERLAY_MEDIA_HEADER_GAP,
  OVERLAY_MEDIA_HEADER_HEIGHT,
  OVERLAY_MEDIA_RESERVED_HEIGHT,
  anchorScroll,
  displayZoneLimits,
  displayedImageSize,
  zoomedByWheel,
} from '../../core/editor-helpers.js';
import { createMiddlePan } from './middle-pan.js';

// Zone assumed when the shell does not know the real one: a 1080p screen, so
// the cap still shows.
const FALLBACK_DISPLAY_ZONE = { width: 1920, height: 1080 };
// Longest side of the preview canvas buffer. The canvas is laid out at the size
// the overlay would use, then the browser resamples it, like the overlay does
// with the saved media.
const BUFFER_MAX_SIDE = 2048;

// `frame()` returns the natural size of the media (its size after the recipe
// the editor will save) and the chosen scale, in percent:
// `{ width, height, scalePercent }`.
// `paint(canvas)` draws the media into the canvas the preview sized.
export function createDisplayPreview({ stage, displayZone, frame, paint }) {
  const knownZone = displayZone && Number(displayZone.width) > 0 && Number(displayZone.height) > 0;
  const zone = knownZone
    ? { width: Math.round(Number(displayZone.width)), height: Math.round(Number(displayZone.height)), known: true }
    : { width: FALLBACK_DISPLAY_ZONE.width, height: FALLBACK_DISPLAY_ZONE.height, known: false };

  let visible = false;
  let zoom = 'fit';
  // Multiplier on the fit/100 % scale: Ctrl + molette lets the scene be
  // inspected closer than the fit, without leaving the preview.
  let zoomFactor = 1;
  let needsLayout = true;
  let queued = false;

  const canvas = h('canvas', { class: 'display-preview-image' });
  const header = h('div', { class: 'display-preview-header' });
  const screen = h('div', { class: 'display-preview-screen' }, header, canvas);
  const zoneLabel = h('span', { class: 'display-preview-zone' });
  const zoomChip = h('button', {
    class: 'btn is-quiet display-preview-zoom',
    type: 'button',
    hidden: true,
    onClick: () => resetZoom(),
  });
  const zoomButtons = new Map();
  const zoomButton = (id, label) => {
    const button = h('button', { class: 'btn', type: 'button', onClick: () => setZoom(id) }, label);
    zoomButtons.set(id, button);
    return button;
  };
  const bar = h(
    'div',
    { class: 'display-preview-bar' },
    h('span', { class: 'display-preview-title' }, 'Aperçu sur l’overlay'),
    zoneLabel,
    h('span', { class: 'spacer' }),
    zoomChip,
    zoomButton('fit', 'Ajuster'),
    zoomButton('100', '100 %'),
  );
  const scroll = h('div', { class: 'display-preview-scroll' }, screen);
  scroll.title = 'Ctrl + molette : zoomer · molette maintenue : déplacer';
  const node = h('div', { class: 'display-preview', hidden: true }, bar, scroll);

  // Molette maintenue : la zone suit le pointeur, le cadre défile à l'opposé.
  const pan = createMiddlePan(scroll, (dx, dy) => {
    scroll.scrollLeft -= dx;
    scroll.scrollTop -= dy;
  }, {
    canStart: () => scroll.scrollWidth > scroll.clientWidth + 1
      || scroll.scrollHeight > scroll.clientHeight + 1,
  });

  function factorLabel() {
    return zoomFactor < 10 ? zoomFactor.toFixed(1).replace('.', ',') : String(Math.round(zoomFactor));
  }

  function syncChrome() {
    zoneLabel.textContent = zone.known
      ? `Zone ${zone.width} × ${zone.height} px · en-tête ${OVERLAY_MEDIA_RESERVED_HEIGHT} px`
      : `Zone inconnue, aperçu sur ${zone.width} × ${zone.height} px`;
    for (const [id, button] of zoomButtons) {
      const active = zoom === id;
      button.classList.toggle('is-on', active);
      button.setAttribute('aria-pressed', String(active));
    }
    zoomChip.hidden = !visible || zoomFactor <= 1.001;
    zoomChip.textContent = `×${factorLabel()}`;
    zoomChip.title = 'Ajuster l’aperçu (Ctrl + molette pour zoomer)';
    zoomChip.setAttribute('aria-label', `Aperçu zoomé ×${factorLabel()}. Cliquer pour ajuster.`);
  }

  function resetZoom() {
    zoomFactor = 1;
    scroll.scrollLeft = 0;
    scroll.scrollTop = 0;
    needsLayout = true;
    layout();
    syncChrome();
    requestPaint();
  }

  function setZoom(next) {
    if (next !== 'fit' && next !== '100') {
      return;
    }
    zoom = next;
    resetZoom();
  }

  // Stage pixels per overlay pixel: the whole zone fits, or 1:1 for 100 %,
  // times the Ctrl + molette multiplier.
  function previewScale() {
    if (zoom === '100') {
      return zoomFactor;
    }
    const width = scroll.clientWidth - 32;
    const height = scroll.clientHeight - 32;
    if (!(width > 0) || !(height > 0)) {
      return 0;
    }
    return Math.min(1, width / zone.width, height / zone.height) * zoomFactor;
  }

  function ensureBuffer(width, height) {
    if (canvas.width === width && canvas.height === height) {
      return;
    }
    canvas.width = width;
    canvas.height = height;
  }

  // Lays the screen, the header band and the media out at the stage's scale.
  function layout() {
    const scale = previewScale();
    if (!(scale > 0)) {
      return false;
    }
    const shown = frame();
    if (!(shown && shown.width > 0 && shown.height > 0)) {
      return false;
    }
    // Same arithmetic as the overlay: natural size × scale, fitted into the zone
    // minus its header.
    const displayed = displayedImageSize({
      width: shown.width,
      height: shown.height,
      scalePercent: shown.scalePercent,
      ...displayZoneLimits({ zoneWidth: zone.width, zoneHeight: zone.height }),
    });
    const screenWidth = Math.max(1, Math.round(zone.width * scale));
    const screenHeight = Math.max(1, Math.round(zone.height * scale));
    const headerHeight = Math.max(1, Math.round(OVERLAY_MEDIA_HEADER_HEIGHT * scale));
    const headerGap = Math.max(0, Math.round(OVERLAY_MEDIA_HEADER_GAP * scale));
    const pictureWidth = Math.max(1, Math.round(displayed.width * scale));
    const pictureHeight = Math.max(1, Math.round(displayed.height * scale));
    // Header and media are one block, centred: that is where the overlay puts
    // the frame. The header is as wide as the frame, so it stops with the media
    // instead of spanning the whole zone.
    const top = Math.max(0, Math.round((screenHeight - headerHeight - headerGap - pictureHeight) / 2));
    const left = Math.max(0, Math.round((screenWidth - pictureWidth) / 2));
    screen.style.width = `${screenWidth}px`;
    screen.style.height = `${screenHeight}px`;
    header.style.top = `${top}px`;
    header.style.left = `${left}px`;
    header.style.width = `${Math.max(1, Math.min(pictureWidth, screenWidth))}px`;
    header.style.height = `${headerHeight}px`;
    header.textContent = headerHeight >= 20 && pictureWidth >= 150 ? 'En-tête de l’overlay' : '';
    canvas.style.top = `${top + headerHeight + headerGap}px`;
    canvas.style.left = `${left}px`;
    canvas.style.width = `${pictureWidth}px`;
    canvas.style.height = `${pictureHeight}px`;
    const ratio = Math.min(window.devicePixelRatio || 1, 2, BUFFER_MAX_SIDE / Math.max(pictureWidth, pictureHeight));
    ensureBuffer(Math.max(1, Math.round(pictureWidth * ratio)), Math.max(1, Math.round(pictureHeight * ratio)));
    return true;
  }

  // Ctrl + molette zoome l'aperçu autour du pointeur ; le cadre reste
  // défilable, comme la zone réelle le sera si le média la dépasse.
  scroll.addEventListener('wheel', (event) => {
    if (!(event.ctrlKey || event.metaKey)) {
      return;
    }
    // Le zoom de la page ne doit jamais prendre la main, même à la borne.
    event.preventDefault();
    const next = zoomedByWheel(zoomFactor, event, { min: 1, max: 4 });
    if (next === null) {
      return;
    }
    const before = screen.getBoundingClientRect();
    const scrollLeft = scroll.scrollLeft;
    const scrollTop = scroll.scrollTop;
    zoomFactor = next;
    needsLayout = true;
    layout();
    const after = screen.getBoundingClientRect();
    const moved = anchorScroll({
      scrollLeft,
      scrollTop,
      pointer: { x: event.clientX, y: event.clientY },
      before,
      after,
    });
    scroll.scrollLeft = moved.left;
    scroll.scrollTop = moved.top;
    syncChrome();
    requestPaint();
  }, { passive: false });

  function requestPaint() {
    if (!visible || queued) {
      return;
    }
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      if (!visible) {
        return;
      }
      if (needsLayout) {
        needsLayout = !layout();
      }
      if (!needsLayout) {
        paint(canvas);
      }
    });
  }

  // A change of the media, of its scale or of the stage: lay the frame out
  // again before drawing.
  function invalidate() {
    needsLayout = true;
    requestPaint();
  }

  function show() {
    if (visible) {
      return;
    }
    visible = true;
    node.hidden = false;
    needsLayout = true;
    syncChrome();
    requestPaint();
  }

  function hide() {
    visible = false;
    node.hidden = true;
  }

  const resizeObserver = new ResizeObserver(() => invalidate());
  resizeObserver.observe(stage);

  syncChrome();

  return {
    node,
    zone,
    isVisible: () => visible,
    show,
    hide,
    invalidate,
    requestPaint,
    setZoom,
    destroy: () => {
      pan.destroy();
      resizeObserver.disconnect();
    },
  };
}
