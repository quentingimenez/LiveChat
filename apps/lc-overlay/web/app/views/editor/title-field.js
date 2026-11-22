// The media title, edited in place in the editor bar: the same control as the
// board's detail drawer (a pencil on hover, a framed field while editing).
//
// The frame owns the keyboard before the field does, so it asks this control to
// commit or to cancel instead of letting the input handle Enter and Escape.

import { h, icon } from '../../core/dom.js';
import { notify } from '../../core/toast.js';

// A media may have no title of its own: the board then shows one derived from
// its source. The field edits the empty value under this label.
const EMPTY_LABEL = 'Ajouter un titre';

// The value to send for a draft, or null when the draft changes nothing (the
// same title, only padded with spaces). Clearing a title is a real change.
export function titleToSave(draft, initial) {
  const next = String(draft ?? '').trim();
  return next === String(initial ?? '') ? null : next;
}

// What the bar shows once the server answered: the saved title, or the title
// derived from the source when the media has none.
export function titleAfterRename(saved, fallback) {
  const value = String(saved ?? '').trim();
  return value || String(fallback ?? '');
}

export function createTitleField({ title = '', value = '', rename = null, maxlength = 240 } = {}) {
  const fallback = String(title ?? '');
  let saved = String(value ?? '');
  let shown = saved || fallback;
  let draft = saved;
  let initial = saved;
  let editing = false;
  let settled = false;
  let field = button();
  const node = h('h2', { class: 'editor-title' }, field);

  function paint() {
    node.replaceChildren(field);
  }

  // The resting control: plain text with the pencil that only shows on hover or
  // keyboard focus, like the drawer's title.
  function button() {
    return h(
      'button',
      {
        class: `inline-edit is-title${shown ? '' : ' is-empty'}`,
        type: 'button',
        title: rename ? 'Modifier le titre du média' : null,
        'aria-label': 'Modifier le titre du média',
        disabled: !rename,
        onClick: start,
      },
      h('span', { class: 'inline-text is-title ellipsis' }, shown || EMPTY_LABEL),
      rename ? icon('edit') : null,
    );
  }

  function start() {
    if (!rename || editing) {
      return;
    }
    editing = true;
    settled = false;
    initial = saved;
    draft = saved;
    const input = h('input', {
      class: 'field inline-input is-title',
      type: 'text',
      value: saved,
      maxlength: String(maxlength),
      placeholder: EMPTY_LABEL,
      'aria-label': 'Titre du média',
      onInput: () => {
        draft = input.value;
      },
      // A re-render removes the field without the user leaving it: only a real
      // blur (the field still in the page) saves.
      onBlur: () => {
        if (input.isConnected) {
          commit();
        }
      },
    });
    field = input;
    paint();
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  }

  function finish() {
    editing = false;
    settled = true;
    field = button();
    paint();
  }

  async function commit() {
    if (!editing || settled) {
      return;
    }
    settled = true;
    // The field is swapped out while the save travels: keep the keyboard in the
    // bar when it was there, so Escape still reaches the editor afterwards.
    const hadFocus = node.contains(document.activeElement);
    const next = titleToSave(draft, initial);
    if (next === null) {
      finish();
      if (hadFocus) {
        field.focus();
      }
      return;
    }
    if (field instanceof HTMLInputElement) {
      field.disabled = true;
    }
    try {
      const result = await rename(next);
      saved = String(result ?? '');
      shown = titleAfterRename(saved, fallback);
    } catch (error) {
      draft = initial;
      notify(error.message, true);
    }
    finish();
    if (hadFocus) {
      field.focus();
    }
  }

  function cancel() {
    if (!editing) {
      return false;
    }
    draft = initial;
    finish();
    return true;
  }

  return {
    node,
    owns: (target) => target === field,
    focus: () => field.focus(),
    commit,
    cancel,
  };
}
