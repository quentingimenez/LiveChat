// Floating notifications in the bottom-right corner. They are positioned over
// the page and never take space in the layout, so showing one does not move
// the board.

import { h, icon } from './dom.js';

const MAX_TOASTS = 4;
const DEFAULT_TIMEOUT_MS = { info: 3500, success: 3500, error: 7000 };

function container() {
  let root = document.getElementById('toasts');
  if (!root) {
    root = h('div', { id: 'toasts', class: 'toasts', 'aria-live': 'polite' });
    document.body.append(root);
  }
  return root;
}

// Shows a toast and returns `{ update, close }`.
//
// - `kind`: `'info'` (default), `'success'` or `'error'`;
// - `timeout`: milliseconds before it closes; omitted, the default for its
//   kind; `null` or `0` keeps it open until `close()` (or its close button).
//
// `update({ message, kind, timeout })` changes a toast in place, with the
// same `timeout` rules.
export function toast({ message, kind = 'info', timeout } = {}) {
  const root = container();
  const text = h('span', { class: 'toast-text' });
  let timer = null;
  let currentKind = kind;
  const node = h('div', { class: 'toast' }, text);
  const close = () => {
    clearTimeout(timer);
    node.remove();
  };
  node.append(h('button', { class: 'toast-close', type: 'button', 'aria-label': 'Fermer la notification', onClick: close }, icon('close')));

  const update = (next = {}) => {
    if (next.message !== undefined) {
      text.textContent = next.message;
      node.dataset.message = next.message;
    }
    if (next.kind) {
      currentKind = next.kind;
    }
    node.className = `toast is-${currentKind}`;
    node.setAttribute('role', currentKind === 'error' ? 'alert' : 'status');
    clearTimeout(timer);
    const delay = 'timeout' in next && next.timeout !== undefined ? next.timeout : DEFAULT_TIMEOUT_MS[currentKind] || DEFAULT_TIMEOUT_MS.info;
    if (delay) {
      timer = setTimeout(close, delay);
    }
  };

  // The same message twice in a row restarts the last toast instead of
  // stacking a copy.
  const last = root.lastElementChild;
  if (last && last.dataset.message === message && last.classList.contains(`is-${kind}`) && last.__toast) {
    last.__toast.update({ message, kind, timeout });
    return last.__toast;
  }

  update({ message, kind, timeout });
  node.__toast = { update, close };
  root.append(node);
  while (root.children.length > MAX_TOASTS) {
    const oldest = root.firstElementChild;
    if (oldest.__toast) {
      oldest.__toast.close();
    } else {
      oldest.remove();
    }
  }
  return node.__toast;
}

// Short form kept from the former inline notice: `notify(text, isError)`.
export function notify(message, isError) {
  return toast({ message, kind: isError ? 'error' : 'info' });
}
