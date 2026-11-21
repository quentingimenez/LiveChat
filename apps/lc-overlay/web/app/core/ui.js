// Modal dialogs and clipboard helper.

import { h } from './dom.js';
import { notify } from './toast.js';

// Opens a modal dialog. `build(close)` returns the dialog's children.
//
// The frame is shared by every dialog: a sticky title row carrying the close
// button, a scrolling body and a fixed footer. To stay backward compatible,
// the last `.actions` child returned by `build` is moved into the footer;
// everything else scrolls.
let dialogCount = 0;

export function openDialog(build, options = {}) {
  const root = document.getElementById('dialog-root');
  const previousFocus = document.activeElement;
  let closed = false;

  const close = () => {
    if (closed) {
      return;
    }
    closed = true;
    document.removeEventListener('keydown', onKeydown, true);
    root.replaceChildren();
    if (previousFocus && typeof previousFocus.focus === 'function' && previousFocus.isConnected) {
      previousFocus.focus();
    }
  };

  const children = build(close).filter(Boolean);
  const dialog = h(
    'div',
    {
      class: options.width === 'wide' ? 'dialog is-wide' : 'dialog',
      role: 'dialog',
      'aria-modal': 'true',
    },
  );
  const body = h('div', { class: 'dialog-body' }, ...children);
  // Actions can be nested in the caller's layout (profile sections, for
  // instance): take the last one and pin it in the footer.
  const actions = [...body.querySelectorAll('.actions')].at(-1) || null;
  const actionForm = actions ? actions.closest('form') : null;
  if (actions) {
    actions.remove();
    // A footer lives outside the scrolling body, so a submit button extracted
    // from a form loses its owner. Re-associate it, or a click does nothing and
    // `form.requestSubmit(button)` throws.
    associateSubmitButtons(actions, actionForm);
  }
  const title = body.querySelector('h2');
  if (title) {
    const id = `dialog-title-${(dialogCount += 1)}`;
    title.id = id;
    dialog.setAttribute('aria-labelledby', id);
  } else if (options.label) {
    dialog.setAttribute('aria-label', options.label);
  }

  // Resolve the initial focus before the close button joins the body: a
  // dialog that offers its own control must not open on « Fermer » instead.
  const preferred = body.querySelector('[data-autofocus]') || body.querySelector('input:not([type=hidden]), textarea, select, button');

  const closeButton = h(
    'button',
    { class: 'btn is-quiet icon-btn dialog-close', type: 'button', 'aria-label': 'Fermer', title: 'Fermer', onClick: close },
    h('span', { 'aria-hidden': 'true' }, '×'),
  );
  if (title && body.firstElementChild === title) {
    // The close button lives in the sticky title row, inside the scroll
    // container: it is positioned from the content column, so it clears the
    // body's scrollbar instead of covering it.
    body.prepend(h('div', { class: 'dialog-head' }, title, closeButton));
    dialog.append(body);
  } else {
    // Nothing to pin the button to: keep it overlaid on the frame.
    dialog.append(closeButton, body);
  }
  if (actions) {
    dialog.append(h('div', { class: 'dialog-foot' }, actions));
  }

  const backdrop = h(
    'div',
    {
      class: 'backdrop',
      onMousedown: (event) => {
        if (event.target === backdrop) {
          close();
        }
      },
    },
    dialog,
  );

  // The dialog traps Tab and closes on Escape, wherever the focus is.
  function onKeydown(event) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }
    if (event.key !== 'Tab') {
      return;
    }
    const focusable = [...dialog.querySelectorAll(FOCUSABLE)].filter((element) => element.getClientRects().length > 0);
    if (focusable.length === 0) {
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  root.replaceChildren(backdrop);
  document.addEventListener('keydown', onKeydown, true);

  if (preferred) {
    preferred.focus();
  } else {
    dialog.querySelector('.dialog-close')?.focus();
  }
}

const FOCUSABLE = [
  'button:not([disabled])',
  '[href]',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ');

// A submit button that leaves its form's subtree keeps working through the
// `form` attribute. Exported for tests: the DOM wiring itself has no Node
// harness here.
export function associateSubmitButtons(actions, form) {
  if (!form) {
    return;
  }
  if (!form.id) {
    form.id = `dialog-form-${(dialogCount += 1)}`;
  }
  for (const submit of actions.querySelectorAll('button[type="submit"], input[type="submit"]')) {
    submit.setAttribute('form', form.id);
  }
}

export function confirmDialog(title, text, actionLabel, action, options = {}) {
  const tone = options.tone === 'neutral' ? '' : 'is-danger';
  openDialog((close) => [
    h('h2', null, title),
    h('p', null, text),
    h(
      'div',
      { class: 'actions' },
      h('button', { class: 'btn is-quiet', type: 'button', 'data-autofocus': '', onClick: close }, 'Annuler'),
      h('button', { class: `btn ${tone}`.trim(), type: 'button', onClick: async () => { close(); await action(); } }, actionLabel),
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
