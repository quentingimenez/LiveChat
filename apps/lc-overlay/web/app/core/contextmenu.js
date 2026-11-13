// Context menus of the board.
//
// `openContextMenu(event, items)` replaces the WebView's native menu with ours.
// `items` is a list of:
// - `{ label, icon?, danger?, disabled?, onSelect }`: an action;
// - `{ heading }`: a non-interactive title (e.g. the guild's name);
// - `'separator'`.
//
// `installContextMenuGuard()` blocks the native menu everywhere else, except
// in text fields, where it keeps cut, copy and paste.

import { h, icon } from './dom.js';

let current = null;

function isEditable(target) {
  if (!(target instanceof Element)) {
    return false;
  }
  const field = target.closest('input, textarea, [contenteditable=""], [contenteditable="true"]');
  if (!field) {
    return false;
  }
  if (field.tagName === 'INPUT') {
    const type = (field.getAttribute('type') || 'text').toLowerCase();
    return !['button', 'checkbox', 'radio', 'range', 'color', 'file', 'submit', 'reset', 'image'].includes(type);
  }
  return true;
}

export function installContextMenuGuard() {
  document.addEventListener('contextmenu', (event) => {
    if (!event.defaultPrevented && !isEditable(event.target)) {
      event.preventDefault();
    }
  });
}

export function closeContextMenu() {
  if (!current) {
    return;
  }
  const { menu, restoreFocus, cleanup } = current;
  current = null;
  cleanup();
  menu.remove();
  if (restoreFocus && restoreFocus.isConnected) {
    restoreFocus.focus({ preventScroll: true });
  }
}

// Where to open: at the pointer, or under the target for the keyboard (menu
// key, Shift+F10), which reports no pointer position.
function anchorPoint(event) {
  const fromKeyboard = event.button === -1 || (event.clientX === 0 && event.clientY === 0);
  if (!fromKeyboard) {
    return { x: event.clientX, y: event.clientY };
  }
  const target = event.currentTarget instanceof Element ? event.currentTarget : event.target;
  const rect = target instanceof Element ? target.getBoundingClientRect() : { left: 0, bottom: 0 };
  return { x: rect.left, y: rect.bottom };
}

function place(menu, point) {
  const margin = 6;
  const { width, height } = menu.getBoundingClientRect();
  const x = Math.min(point.x, window.innerWidth - width - margin);
  const y = point.y + height + margin > window.innerHeight ? Math.max(margin, point.y - height) : point.y;
  menu.style.left = `${Math.max(margin, x)}px`;
  menu.style.top = `${Math.max(margin, y)}px`;
}

function buildItem(item) {
  if (item === 'separator') {
    return h('div', { class: 'ctx-sep', role: 'separator' });
  }
  if (item.heading) {
    return h('div', { class: 'ctx-heading' }, item.heading);
  }
  return h(
    'button',
    {
      type: 'button',
      class: `ctx-item${item.danger ? ' is-danger' : ''}`,
      role: 'menuitem',
      disabled: Boolean(item.disabled),
      onClick: () => {
        closeContextMenu();
        item.onSelect();
      },
    },
    item.icon ? icon(item.icon) : h('span', { class: 'icon', 'aria-hidden': 'true' }),
    item.label,
  );
}

function moveFocus(menu, step) {
  const buttons = [...menu.querySelectorAll('.ctx-item:not(:disabled)')];
  if (buttons.length === 0) {
    return;
  }
  const index = buttons.indexOf(document.activeElement);
  let next;
  if (step === 'first') {
    next = 0;
  } else if (step === 'last') {
    next = buttons.length - 1;
  } else {
    next = index === -1 ? (step > 0 ? 0 : buttons.length - 1) : (index + step + buttons.length) % buttons.length;
  }
  buttons[next].focus();
}

export function openContextMenu(event, items) {
  event.preventDefault();
  event.stopPropagation();
  closeContextMenu();
  const entries = items.filter(Boolean);
  if (entries.length === 0) {
    return;
  }

  const menu = h('div', { class: 'ctx-menu', role: 'menu' }, ...entries.map(buildItem));
  menu.addEventListener('keydown', (keyEvent) => {
    const keys = { ArrowDown: 1, ArrowUp: -1, Home: 'first', End: 'last' };
    if (keyEvent.key in keys) {
      keyEvent.preventDefault();
      moveFocus(menu, keys[keyEvent.key]);
    } else if (keyEvent.key === 'Escape' || keyEvent.key === 'Tab') {
      keyEvent.preventDefault();
      closeContextMenu();
    }
  });
  // A right click inside the menu must not open the native one.
  menu.addEventListener('contextmenu', (menuEvent) => menuEvent.preventDefault());

  const onPointerDown = (pointerEvent) => {
    if (!menu.contains(pointerEvent.target)) {
      closeContextMenu();
    }
  };
  const onDismiss = () => closeContextMenu();
  document.addEventListener('pointerdown', onPointerDown, true);
  window.addEventListener('blur', onDismiss);
  window.addEventListener('resize', onDismiss);
  document.addEventListener('scroll', onDismiss, true);

  current = {
    menu,
    restoreFocus: document.activeElement instanceof HTMLElement ? document.activeElement : null,
    cleanup: () => {
      document.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('blur', onDismiss);
      window.removeEventListener('resize', onDismiss);
      document.removeEventListener('scroll', onDismiss, true);
    },
  };

  document.body.append(menu);
  place(menu, anchorPoint(event));
  // Keyboard users land on the first action; :focus-visible keeps the
  // highlight off for a mouse opening.
  moveFocus(menu, 'first');
}
