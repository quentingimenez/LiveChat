// On-screen editor of the display zone, shown on the overlay window while the
// shell runs a placement (`overlay://placement`).
//
// The zone is drawn over the screen with a handle at each corner and edge:
// dragging it moves it, dragging a handle resizes it, like a window. Edges and
// centre snap to the screen edges and centre (Alt held: no magnetism). Arrow
// keys move it by one physical pixel (Shift: 10), Ctrl + arrows resize it.
// Every change is applied to the overlay layout at once, so the template card
// behind follows; nothing is saved before « Valider » (or Enter).

import {
  HANDLES,
  moveRect,
  nudgeRect,
  rectToZone,
  resizeRect,
  zoneToRect,
} from './placement-geometry.js';

const TOOLBAR_GAP_PX = 12;
const NUDGE_FAST_STEPS = 10;

const ARROWS = {
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
  ArrowUp: [0, -1],
  ArrowDown: [0, 1],
};

const HANDLE_LABELS = {
  nw: 'Coin haut gauche',
  n: 'Bord haut',
  ne: 'Coin haut droit',
  e: 'Bord droit',
  se: 'Coin bas droit',
  s: 'Bord bas',
  sw: 'Coin bas gauche',
  w: 'Bord gauche',
};

function element(doc, tag, className, text) {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/**
 * @param {object} deps
 * @param {Document} deps.document
 * @param {Window} deps.window
 * @param {(layout: object) => void} deps.applyLayout applies a layout to the overlay
 * @param {(payload: object) => Promise<unknown>} deps.finishPlacement shell command
 */
export function createPlacementEditor(deps) {
  const doc = deps.document;
  const win = deps.window;
  const state = {
    active: false,
    busy: false,
    bounds: { width: 1, height: 1 },
    rect: { left: 0, top: 0, width: 1, height: 1 },
    align: 'center',
    drag: null,
  };
  let nodes = null;

  function physicalPixel() {
    const ratio = Number(win.devicePixelRatio);
    return Number.isFinite(ratio) && ratio > 0 ? 1 / ratio : 1;
  }

  function physical(value) {
    return Math.round(value / physicalPixel());
  }

  function readBounds() {
    return {
      width: Math.max(1, win.innerWidth),
      height: Math.max(1, win.innerHeight),
    };
  }

  function build() {
    const layer = element(doc, 'div', 'placement-layer');
    layer.id = 'placement-layer';
    layer.tabIndex = -1;
    layer.hidden = true;
    layer.setAttribute('role', 'dialog');
    layer.setAttribute('aria-label', 'Placement de la zone d’affichage');

    const guides = element(doc, 'div', 'placement-guides');
    const zone = element(doc, 'div', 'placement-zone');
    const readout = element(doc, 'div', 'placement-readout');
    const size = element(doc, 'span', 'placement-readout-size');
    const origin = element(doc, 'span', 'placement-readout-origin');
    readout.append(size, origin);
    zone.append(readout);
    for (const handle of HANDLES) {
      const knob = element(doc, 'span', `placement-handle is-${handle}`);
      knob.dataset.handle = handle;
      knob.title = HANDLE_LABELS[handle];
      zone.append(knob);
    }

    const toolbar = element(doc, 'div', 'placement-toolbar');
    const text = element(doc, 'div', 'placement-toolbar-text');
    text.append(
      element(
        doc,
        'p',
        'placement-hint',
        'Glissez la zone pour la déplacer, ses coins et ses bords pour la redimensionner.',
      ),
    );
    const keys = element(doc, 'p', 'placement-keys');
    keys.append(
      element(doc, 'kbd', '', 'Flèches'),
      ' 1 px, ',
      element(doc, 'kbd', '', 'Maj'),
      ' 10 px, ',
      element(doc, 'kbd', '', 'Ctrl'),
      ' redimensionne, ',
      element(doc, 'kbd', '', 'Alt'),
      ' sans magnétisme',
    );
    text.append(keys);
    const actions = element(doc, 'div', 'placement-actions');
    const fullscreen = element(doc, 'button', 'placement-button is-quiet', 'Plein écran');
    fullscreen.type = 'button';
    fullscreen.dataset.action = 'fullscreen';
    const cancel = element(doc, 'button', 'placement-button', 'Annuler');
    cancel.type = 'button';
    cancel.dataset.action = 'cancel';
    cancel.title = 'Échap';
    const save = element(doc, 'button', 'placement-button is-primary', 'Valider');
    save.type = 'button';
    save.dataset.action = 'save';
    save.title = 'Entrée';
    actions.append(fullscreen, cancel, save);
    toolbar.append(text, actions);

    layer.append(guides, zone, toolbar);
    doc.body.append(layer);

    zone.addEventListener('pointerdown', onPointerDown);
    zone.addEventListener('pointermove', onPointerMove);
    zone.addEventListener('pointerup', onPointerEnd);
    zone.addEventListener('pointercancel', onPointerEnd);
    zone.addEventListener('lostpointercapture', onPointerEnd);
    actions.addEventListener('click', onAction);
    return { layer, guides, zone, size, origin, toolbar, buttons: [fullscreen, cancel, save] };
  }

  function render(guides = []) {
    if (!nodes) return;
    const { rect } = state;
    Object.assign(nodes.zone.style, {
      left: `${rect.left}px`,
      top: `${rect.top}px`,
      width: `${rect.width}px`,
      height: `${rect.height}px`,
    });
    nodes.size.textContent = `${physical(rect.width)} × ${physical(rect.height)} px`;
    nodes.origin.textContent = `à ${physical(rect.left)}, ${physical(rect.top)}`;
    renderGuides(guides);
    placeToolbar();
  }

  function renderGuides(guides) {
    nodes.guides.replaceChildren(
      ...guides.map((guide) => {
        const line = element(doc, 'span', `placement-guide is-${guide.axis}`);
        line.style[guide.axis === 'x' ? 'left' : 'top'] = `${guide.at}px`;
        return line;
      }),
    );
  }

  // Below the zone when there is room, else above, else inside its bottom.
  function placeToolbar() {
    const { toolbar } = nodes;
    const { rect, bounds } = state;
    const width = toolbar.offsetWidth;
    const height = toolbar.offsetHeight;
    let top = rect.top + rect.height + TOOLBAR_GAP_PX;
    if (top + height > bounds.height - TOOLBAR_GAP_PX) {
      top = rect.top - TOOLBAR_GAP_PX - height;
      if (top < TOOLBAR_GAP_PX) {
        top = rect.top + rect.height - TOOLBAR_GAP_PX - height;
      }
    }
    top = Math.min(Math.max(TOOLBAR_GAP_PX, top), Math.max(TOOLBAR_GAP_PX, bounds.height - height - TOOLBAR_GAP_PX));
    const centre = rect.left + rect.width / 2 - width / 2;
    const left = Math.min(Math.max(TOOLBAR_GAP_PX, centre), Math.max(TOOLBAR_GAP_PX, bounds.width - width - TOOLBAR_GAP_PX));
    toolbar.style.left = `${left}px`;
    toolbar.style.top = `${top}px`;
  }

  function applyDraft() {
    deps.applyLayout({ zone: rectToZone(state.rect, state.bounds), align: state.align });
  }

  function setRect(rect, guides) {
    state.rect = rect;
    render(guides);
    applyDraft();
  }

  function onPointerDown(event) {
    if (!state.active || state.busy || event.button !== 0) return;
    const handle = event.target instanceof HTMLElement ? event.target.dataset.handle : undefined;
    state.drag = {
      handle: handle || null,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      startRect: { ...state.rect },
    };
    nodes.zone.setPointerCapture(event.pointerId);
    nodes.layer.classList.add(handle ? 'is-resizing' : 'is-moving');
    event.preventDefault();
  }

  function onPointerMove(event) {
    const drag = state.drag;
    if (!drag || event.pointerId !== drag.pointerId) return;
    const dx = event.clientX - drag.startX;
    const dy = event.clientY - drag.startY;
    const options = { snap: !event.altKey };
    const result = drag.handle
      ? resizeRect(drag.startRect, drag.handle, dx, dy, state.bounds, options)
      : moveRect(drag.startRect, dx, dy, state.bounds, options);
    setRect(result.rect, result.guides);
  }

  function onPointerEnd(event) {
    const drag = state.drag;
    if (!drag || event.pointerId !== drag.pointerId) return;
    state.drag = null;
    if (nodes.zone.hasPointerCapture(event.pointerId)) {
      nodes.zone.releasePointerCapture(event.pointerId);
    }
    nodes.layer.classList.remove('is-resizing', 'is-moving');
    render();
  }

  function onKeyDown(event) {
    if (!state.active || state.busy) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      finish(false);
      return;
    }
    if (event.key === 'Enter') {
      // A focused button handles Enter itself (« Annuler » must not save).
      if (event.target instanceof HTMLButtonElement) return;
      event.preventDefault();
      finish(true);
      return;
    }
    const arrow = ARROWS[event.key];
    if (!arrow || state.drag) return;
    event.preventDefault();
    const step = physicalPixel() * (event.shiftKey ? NUDGE_FAST_STEPS : 1);
    const rect = nudgeRect(state.rect, arrow[0] * step, arrow[1] * step, state.bounds, {
      resize: event.ctrlKey,
    });
    setRect(rect);
  }

  function onAction(event) {
    const button = event.target instanceof Element ? event.target.closest('button') : null;
    if (!button || !state.active || state.busy) return;
    if (button.dataset.action === 'fullscreen') {
      setRect({ left: 0, top: 0, width: state.bounds.width, height: state.bounds.height });
    } else if (button.dataset.action === 'cancel') {
      finish(false);
    } else if (button.dataset.action === 'save') {
      finish(true);
    }
  }

  // The screen changed under the placement (resolution, scale): keep the
  // same zone fractions.
  function onResize() {
    if (!state.active) return;
    const zone = rectToZone(state.rect, state.bounds);
    state.bounds = readBounds();
    state.rect = zoneToRect(zone, state.bounds);
    render();
    applyDraft();
  }

  function setBusy(busy) {
    state.busy = busy;
    if (!nodes) return;
    nodes.layer.classList.toggle('is-busy', busy);
    for (const button of nodes.buttons) button.disabled = busy;
  }

  function finish(save) {
    if (!state.active || state.busy) return;
    setBusy(true);
    const zone = save ? rectToZone(state.rect, state.bounds) : null;
    Promise.resolve(deps.finishPlacement({ zone }))
      .catch((error) => {
        console.warn('[placement] fin du placement refusée :', error?.message || error);
      })
      // The shell ends the placement in every case (overlay://placement
      // { active: false }); stop here too in case that event is lost.
      .finally(() => stop());
  }

  function start(layout) {
    if (!nodes) nodes = build();
    state.bounds = readBounds();
    state.align = typeof layout?.align === 'string' ? layout.align : 'center';
    state.rect = zoneToRect(layout?.zone, state.bounds);
    state.drag = null;
    if (!state.active) {
      state.active = true;
      doc.addEventListener('keydown', onKeyDown);
      win.addEventListener('resize', onResize);
    }
    setBusy(false);
    nodes.layer.hidden = false;
    render();
    applyDraft();
    win.focus();
    nodes.layer.focus({ preventScroll: true });
  }

  function stop() {
    if (!state.active) return;
    state.active = false;
    state.drag = null;
    doc.removeEventListener('keydown', onKeyDown);
    win.removeEventListener('resize', onResize);
    setBusy(false);
    if (nodes) {
      nodes.layer.hidden = true;
      nodes.layer.classList.remove('is-resizing', 'is-moving');
      renderGuides([]);
    }
  }

  /**
   * New saved settings arrived (alignment changed from the board): keep the
   * zone being placed, take the alignment.
   */
  function syncLayout(layout) {
    if (!state.active) return;
    if (typeof layout?.align === 'string') state.align = layout.align;
    applyDraft();
  }

  return {
    start,
    stop,
    syncLayout,
    isActive: () => state.active,
  };
}
