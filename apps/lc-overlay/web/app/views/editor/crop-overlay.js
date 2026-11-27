// Crop rectangle laid over a picture or a video: a box with eight handles,
// dragged in the media's own pixel space.

import { h } from '../../core/dom.js';
import { dragCropRect } from '../../core/editor-helpers.js';

const HANDLES = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];

// `host` is positioned over the media and has its displayed size.
// `bounds()` gives the media size in pixels, `aspect()` the locked ratio (0
// when free). `onChange(rect)` fires while dragging, `onCommit(rect)` on
// release.
export function createCropOverlay({ host, bounds, aspect, onChange, onCommit }) {
  let rect = null;
  let drag = null;
  const box = h(
    'div',
    { class: 'crop-box' },
    HANDLES.map((name) => h('span', { class: `crop-handle is-${name}`, 'data-handle': name })),
  );
  const layer = h('div', { class: 'crop-layer' }, box);
  host.append(layer);

  function place() {
    if (!rect) {
      return;
    }
    const size = bounds();
    box.style.left = `${(rect.x / size.width) * 100}%`;
    box.style.top = `${(rect.y / size.height) * 100}%`;
    box.style.width = `${(rect.width / size.width) * 100}%`;
    box.style.height = `${(rect.height / size.height) * 100}%`;
  }

  // The pointer in the media's own pixels. Measured at every call: the stage
  // may scroll or zoom under the pointer while the gesture is held, and the
  // layer moves with it.
  function pointerBase(clientX, clientY) {
    const shown = layer.getBoundingClientRect();
    const size = bounds();
    if (!(shown.width > 0) || !(shown.height > 0)) {
      return null;
    }
    return {
      x: ((clientX - shown.left) / shown.width) * size.width,
      y: ((clientY - shown.top) / shown.height) * size.height,
    };
  }

  function applyDrag() {
    if (!drag) {
      return;
    }
    const point = pointerBase(drag.clientX, drag.clientY);
    if (!point) {
      return;
    }
    // The delta is measured in the media's pixels from the pointerdown point:
    // when the stage scrolls or zooms, the same pointer lands on another media
    // pixel and the edge follows it instead of staying nailed to the picture.
    const dx = point.x - drag.startX;
    const dy = point.y - drag.startY;
    rect = dragCropRect(drag.origin, drag.handle, dx, dy, drag.size, aspect(), drag.minSize);
    place();
    onChange(rect);
  }

  function stopDrag() {
    box.removeEventListener('pointermove', moveDrag);
    box.removeEventListener('pointerup', endDrag);
    box.removeEventListener('pointercancel', endDrag);
    const dragged = drag;
    drag = null;
    if (dragged && onCommit) {
      onCommit(rect);
    }
  }

  function moveDrag(event) {
    if (!drag) {
      return;
    }
    drag.clientX = event.clientX;
    drag.clientY = event.clientY;
    applyDrag();
  }

  function endDrag() {
    if (!drag) {
      return;
    }
    stopDrag();
  }

  box.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || !rect) {
      return;
    }
    const point = pointerBase(event.clientX, event.clientY);
    if (!point) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const size = bounds();
    drag = {
      handle: event.target.dataset.handle || 'move',
      origin: { ...rect },
      size,
      minSize: Math.max(8, Math.min(size.width, size.height) * 0.02),
      startX: point.x,
      startY: point.y,
      clientX: event.clientX,
      clientY: event.clientY,
    };
    box.setPointerCapture(event.pointerId);
    box.addEventListener('pointermove', moveDrag);
    box.addEventListener('pointerup', endDrag);
    box.addEventListener('pointercancel', endDrag);
  });

  // Replays the held gesture against the current position of the layer: the
  // editors call it for every frame the stage scrolls or zooms.
  function replay() {
    applyDrag();
  }

  return {
    set(next) {
      rect = next ? { ...next } : null;
      layer.hidden = !rect;
      place();
    },
    get: () => (rect ? { ...rect } : null),
    replay,
    remove() {
      drag = null;
      layer.remove();
    },
  };
}
