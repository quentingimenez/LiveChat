// LiveChat — custom hover tooltips.
//
// The board used the native `title` attribute: Windows draws the bubble itself,
// with its own font, delay and colours, and no stylesheet can reach it. The
// text is moved to `data-tip` once, at startup and after each mutation, then
// shown by a layer of our own (styles/tooltip.css) placed around the trigger.
//
// `title` is also an accessible description. adoptTitle keeps it: it becomes
// the accessible name of a trigger that shows no text, and `aria-description`
// for the others (the visible text already names them).

const SHOW_DELAY = 350;
const HIDE_DELAY = 60;
// Distance between the trigger and the tip, and the smallest gap kept between
// the tip and the window edge.
const GAP = 8;
const MARGIN = 8;

const SHORTCUT_SUFFIX = /\s*\(([^()]*)\)\s*$/;
const MODIFIERS = new Set(['Ctrl', 'Maj', 'Alt', 'Shift', 'Cmd', 'Meta']);
const SINGLE_KEYS = new Set(['Échap', 'Entrée', 'Suppr', 'Espace', 'Tab', 'Début', 'Fin']);
const CHORD_KEY = /^(?:[A-Za-z0-9]|F\d{1,2}|Échap|Entrée|Suppr|Espace|Tab|Début|Fin|↑|↓|←|→)$/;

// Split « Annuler (Ctrl+Z) » into its label and its key chord. Only a real
// chord or a known single key is extracted: a parenthetical part of a sentence
// (« Ctrl + molette pour zoomer ») stays in the label.
export function splitShortcut(text) {
  const value = String(text ?? '').trim();
  const match = value.match(SHORTCUT_SUFFIX);
  if (!match) {
    return { label: value, key: '' };
  }
  const inner = match[1].trim();
  const label = value.slice(0, match.index).trim();
  if (!inner || inner.length > 24 || inner.includes(':')) {
    return { label: value, key: '' };
  }
  const parts = inner.split('+').map((part) => part.trim());
  const chord =
    parts.length > 1 &&
    parts.every((part) => part.length > 0) &&
    parts.slice(0, -1).every((part) => MODIFIERS.has(part)) &&
    CHORD_KEY.test(parts[parts.length - 1]);
  if (!chord && !(parts.length === 1 && SINGLE_KEYS.has(parts[0]))) {
    return { label: value, key: '' };
  }
  // A tip made only of the shortcut keeps its full text: a bare key is not a
  // label.
  return label ? { label, key: inner } : { label: value, key: '' };
}

// Where the tip goes: centred above the trigger, flipped below when the top
// edge is too close, and always kept inside the window. Pure so the placement
// can be tested without a DOM; `anchor` and `size` are rectangles.
export function placeTip(anchor, size, viewport) {
  const left = Math.max(
    MARGIN,
    Math.min(anchor.left + (anchor.width - size.width) / 2, viewport.width - size.width - MARGIN),
  );
  const above = anchor.top - size.height - GAP;
  const below = anchor.top + anchor.height + GAP;
  let top;
  if (above >= MARGIN) {
    top = above;
  } else if (below + size.height <= viewport.height - MARGIN) {
    top = below;
  } else {
    top = Math.max(MARGIN, Math.min(above, viewport.height - size.height - MARGIN));
  }
  return { left: Math.round(left), top: Math.round(top) };
}

// Move one `title` to the board's tooltip. Returns true when a title was
// adopted; an element without one is left alone (the observer sees removals
// too).
export function adoptTitle(el) {
  if (!el || el.nodeType !== 1) {
    return false;
  }
  const raw = el.getAttribute('title');
  if (raw === null) {
    return false;
  }
  el.removeAttribute('title');
  const text = raw.trim();
  if (!text) {
    return false;
  }
  el.setAttribute('data-tip', text);
  const hasName = el.hasAttribute('aria-label') || el.hasAttribute('aria-labelledby');
  const hasText = Boolean(el.textContent && el.textContent.trim());
  // The full text, shortcut included: the visible tip splits it, but screen
  // readers should still hear « Annuler (Ctrl+Z) ».
  if (!hasName && !hasText) {
    el.setAttribute('aria-label', text);
  } else if (
    !el.hasAttribute('aria-describedby') &&
    !el.hasAttribute('aria-description') &&
    el.getAttribute('aria-label') !== text
  ) {
    // The visible text or the label names the control; the title described
    // what the action does. `aria-description` keeps that sentence.
    el.setAttribute('aria-description', text);
  }
  return true;
}

// Adopt every title in a subtree, including the root itself: a node inserted
// with its own title must not be skipped by querySelectorAll.
export function adoptTitles(root = document) {
  if (!root || typeof root.querySelectorAll !== 'function') {
    return 0;
  }
  let count = 0;
  if (root.nodeType === 1 && root.matches('[title]') && adoptTitle(root)) {
    count += 1;
  }
  for (const el of root.querySelectorAll('[title]')) {
    if (adoptTitle(el)) {
      count += 1;
    }
  }
  return count;
}

// Convert every current title and keep watching the document: the board renders
// its views again on each load, guild switch and media change.
export function installTooltips(doc = document) {
  if (!doc || !doc.body) {
    return null;
  }
  const view = doc.defaultView;
  const tip = doc.createElement('div');
  tip.className = 'lc-tip';
  tip.setAttribute('role', 'tooltip');
  const label = doc.createElement('span');
  label.className = 'lc-tip-label';
  const key = doc.createElement('kbd');
  key.className = 'lc-tip-key';
  key.hidden = true;
  tip.append(label, key);
  doc.body.append(tip);

  let current = null;
  let pending = null;
  let showTimer = 0;
  let hideTimer = 0;

  function place(trigger) {
    tip.classList.remove('is-visible');
    tip.style.left = `${MARGIN}px`;
    tip.style.top = `${MARGIN}px`;
    const box = tip.getBoundingClientRect();
    const anchor = trigger.getBoundingClientRect();
    const at = placeTip(
      anchor,
      { width: box.width, height: box.height },
      { width: view.innerWidth, height: view.innerHeight },
    );
    tip.style.left = `${at.left}px`;
    tip.style.top = `${at.top}px`;
  }

  function render(trigger) {
    const raw = (trigger.getAttribute('data-tip') || '').trim();
    if (!raw) {
      return false;
    }
    const parsed = splitShortcut(raw);
    const explicit = trigger.getAttribute('data-tip-key');
    const shortcut = explicit === null ? parsed.key : explicit.trim();
    label.textContent = parsed.label;
    key.textContent = shortcut;
    key.hidden = !shortcut;
    place(trigger);
    tip.classList.add('is-visible');
    return true;
  }

  function hide() {
    clearTimeout(showTimer);
    clearTimeout(hideTimer);
    showTimer = 0;
    hideTimer = 0;
    pending = null;
    current = null;
    tip.classList.remove('is-visible');
  }

  function show(trigger) {
    clearTimeout(showTimer);
    clearTimeout(hideTimer);
    pending = null;
    current = trigger;
    if (!render(trigger)) {
      current = null;
    }
  }

  function scheduleShow(trigger) {
    clearTimeout(showTimer);
    pending = trigger;
    showTimer = setTimeout(() => {
      showTimer = 0;
      if (pending === trigger && trigger.isConnected) {
        show(trigger);
      }
    }, SHOW_DELAY);
  }

  function triggerOf(target) {
    return target && typeof target.closest === 'function' ? target.closest('[data-tip]') : null;
  }

  function onPointerOver(event) {
    if (event.pointerType === 'touch') {
      return;
    }
    const trigger = triggerOf(event.target);
    if (!trigger) {
      return;
    }
    clearTimeout(hideTimer);
    if (trigger === current) {
      return;
    }
    // Another trigger: the previous label must not stay on screen while the
    // new one waits for its delay.
    if (current) {
      hide();
    }
    scheduleShow(trigger);
  }

  function onPointerOut(event) {
    const trigger = triggerOf(event.target);
    if (!trigger) {
      return;
    }
    // Moving between the trigger and one of its children is not leaving it.
    if (event.relatedTarget && trigger.contains(event.relatedTarget)) {
      return;
    }
    if (pending === trigger) {
      pending = null;
      clearTimeout(showTimer);
    }
    if (current === trigger) {
      clearTimeout(hideTimer);
      hideTimer = setTimeout(hide, HIDE_DELAY);
    }
  }

  function onFocusIn(event) {
    const trigger = triggerOf(event.target);
    if (!trigger || trigger === current) {
      return;
    }
    if (current) {
      hide();
    }
    scheduleShow(trigger);
  }

  function onFocusOut(event) {
    if (current && triggerOf(event.target) === current) {
      hide();
    }
  }

  function onKeyDown(event) {
    if (event.key === 'Escape') {
      hide();
    }
  }

  function onPointerDown() {
    hide();
  }

  function onScroll() {
    if (current) {
      hide();
    }
  }

  function onVisibilityChange() {
    if (doc.hidden) {
      hide();
    }
  }

  doc.addEventListener('pointerover', onPointerOver, { passive: true });
  doc.addEventListener('pointerout', onPointerOut, { passive: true });
  doc.addEventListener('focusin', onFocusIn);
  doc.addEventListener('focusout', onFocusOut);
  // Capture: the editor also handles Escape and stops the event on its way up.
  doc.addEventListener('keydown', onKeyDown, true);
  doc.addEventListener('pointerdown', onPointerDown, { passive: true });
  doc.addEventListener('scroll', onScroll, { capture: true, passive: true });
  view.addEventListener('resize', onScroll);
  view.addEventListener('blur', onScroll);
  doc.addEventListener('visibilitychange', onVisibilityChange);

  adoptTitles(doc);
  const observer = new view.MutationObserver((records) => {
    for (const record of records) {
      if (record.type === 'attributes') {
        if (record.target.getAttribute('title') !== null) {
          adoptTitle(record.target);
        }
        // A play/pause button rewrites its own title while it is hovered: the
        // visible label follows.
        if (record.target === current && tip.classList.contains('is-visible')) {
          render(current);
        }
      } else {
        for (const node of record.addedNodes) {
          if (node.nodeType === 1) {
            adoptTitles(node);
          }
        }
      }
    }
    // The board rebuilds its views: a removed trigger must not leave its tip.
    if (current && !current.isConnected) {
      hide();
    }
  });
  observer.observe(doc.documentElement, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['title'],
  });

  return {
    show,
    hide,
    destroy() {
      hide();
      observer.disconnect();
      doc.removeEventListener('pointerover', onPointerOver);
      doc.removeEventListener('pointerout', onPointerOut);
      doc.removeEventListener('focusin', onFocusIn);
      doc.removeEventListener('focusout', onFocusOut);
      doc.removeEventListener('keydown', onKeyDown, true);
      doc.removeEventListener('pointerdown', onPointerDown);
      doc.removeEventListener('scroll', onScroll, true);
      view.removeEventListener('resize', onScroll);
      view.removeEventListener('blur', onScroll);
      doc.removeEventListener('visibilitychange', onVisibilityChange);
      tip.remove();
    },
  };
}

// The application calls this once, before the first render; later renders are
// covered by the observer.
let installed = null;

export function initTooltips() {
  if (!installed) {
    installed = installTooltips(document);
  }
  return installed;
}
