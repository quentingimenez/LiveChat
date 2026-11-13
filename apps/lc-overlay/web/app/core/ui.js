// Modal dialogs and clipboard helper.

import { h } from './dom.js';
import { notify } from './toast.js';

// Opens a modal dialog. `build(close)` returns the dialog's children.
export function openDialog(build) {
  const root = document.getElementById('dialog-root');
  const close = () => root.replaceChildren();
  const dialog = h('div', { class: 'dialog', role: 'dialog', 'aria-modal': 'true' });
  const backdrop = h('div', { class: 'backdrop', onMousedown: (event) => { if (event.target === backdrop) { close(); } } }, dialog);
  backdrop.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      close();
    }
  });
  dialog.append(...build(close));
  root.replaceChildren(backdrop);
  const first = dialog.querySelector('input:not([type=hidden]), textarea, select, button');
  if (first) {
    first.focus();
  }
}

export function confirmDialog(title, text, actionLabel, action) {
  openDialog((close) => [
    h('h2', null, title),
    h('p', null, text),
    h(
      'div',
      { class: 'actions' },
      h('button', { class: 'btn is-quiet', onClick: close }, 'Annuler'),
      h('button', { class: 'btn is-danger', onClick: async () => { close(); await action(); } }, actionLabel),
    ),
  ]);
}

export async function copyText(text, message) {
  try {
    await navigator.clipboard.writeText(text);
    notify(message);
  } catch {
    notify(`Code : ${text}`);
  }
}
