// A description that fits on one line keeps the width it needs: the 62ch
// reading measure is only the fallback for a sentence that has to wrap. CSS
// cannot compare a text's natural width with its container (`max-width` is a
// fixed value), so this pass measures the rendered sentences after each view
// render and on window resize.

// The copy that carries the 62ch reading cap: the settings cards (both pages)
// and the admin form hints.
const SELECTOR = '.settings-hint, .settings-desc, .form .hint';

// A text can be a fraction of a pixel wider than its container and still be
// laid out on one line.
const SUBPIXEL = 0.5;

export function fitsOnOneLine(naturalWidth, availableWidth) {
  return naturalWidth <= availableWidth + SUBPIXEL;
}

function contentWidth(node) {
  const parent = node.parentElement;
  if (!parent) {
    return 0;
  }
  const style = getComputedStyle(parent);
  const padding = (parseFloat(style.paddingLeft) || 0) + (parseFloat(style.paddingRight) || 0);
  return parent.clientWidth - padding;
}

function naturalWidth(node) {
  const range = document.createRange();
  range.selectNodeContents(node);
  // The measure caps the paragraph; nowrap renders the whole sentence on one
  // line so the range reports its natural width.
  const previous = node.style.whiteSpace;
  node.style.whiteSpace = 'nowrap';
  const width = range.getBoundingClientRect().width;
  node.style.whiteSpace = previous;
  return width;
}

function fit(node) {
  const available = contentWidth(node);
  if (available <= 0 || !node.isConnected) {
    return;
  }
  node.classList.toggle('is-one-line', fitsOnOneLine(naturalWidth(node), available));
}

export function fitSingleLineDescriptions(root = document) {
  if (!root) {
    return;
  }
  for (const node of root.querySelectorAll(SELECTOR)) {
    fit(node);
  }
}

export function initSingleLineDescriptions() {
  let frame = 0;
  window.addEventListener('resize', () => {
    if (frame) {
      return;
    }
    frame = requestAnimationFrame(() => {
      frame = 0;
      fitSingleLineDescriptions(document.getElementById('view'));
    });
  });
}
