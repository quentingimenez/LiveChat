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

  box.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || !rect) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const handle = event.target.dataset.handle || 'move';
    const start = { ...rect };
    const startX = event.clientX;
    const startY = event.clientY;
    const shown = layer.getBoundingClientRect();
    const size = bounds();
    box.setPointerCapture(event.pointerId);
    const move = (moveEvent) => {
      const dx = ((moveEvent.clientX - startX) / shown.width) * size.width;
      const dy = ((moveEvent.clientY - startY) / shown.height) * size.height;
      const minSize = Math.max(8, Math.min(size.width, size.height) * 0.02);
      rect = dragCropRect(start, handle, dx, dy, size, aspect(), minSize);
      place();
      onChange(rect);
    };
    const up = () => {
      box.removeEventListener('pointermove', move);
      box.removeEventListener('pointerup', up);
      box.removeEventListener('pointercancel', up);
      if (onCommit) {
        onCommit(rect);
      }
    };
    box.addEventListener('pointermove', move);
    box.addEventListener('pointerup', up);
    box.addEventListener('pointercancel', up);
  });

  return {
    set(next) {
      rect = next ? { ...next } : null;
      layer.hidden = !rect;
      place();
    },
    get: () => (rect ? { ...rect } : null),
    remove: () => layer.remove(),
  };
}
