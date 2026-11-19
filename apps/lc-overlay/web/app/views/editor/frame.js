// Full-window frame shared by the image editor and the audio/video editor:
// title bar, undo/redo, stage, side panel, optional timeline, save actions.

import { h, icon, registerIcons, resetOnDoubleClick } from '../../core/dom.js';

registerIcons({
  undo: 'M8 8L4 12l4 4M4 12h10a5 5 0 010 10h-2',
  redo: 'M16 8l4 4-4 4M20 12H10a5 5 0 000 10h2',
  rotateLeft: 'M4 5v5h5M4.6 10A8 8 0 1112 20',
  rotateRight: 'M20 5v5h-5M19.4 10A8 8 0 1012 20',
  flipH: 'M12 3v18M8 7l-5 5 5 5zM16 7l5 5-5 5z',
  flipV: 'M3 12h18M7 8l5-5 5 5zM7 16l5 5 5-5z',
  crop: 'M7 3v14h14M3 7h14v14',
  edit: 'M4 20l4-1L19 8l-3-3L5 16zM14 7l3 3',
});
registerIcons({ pause: 'M7 5h3.5v14H7zM13.5 5H17v14h-3.5z' }, { filled: true });

function isTyping(target) {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  if (target.tagName === 'TEXTAREA' || target.tagName === 'SELECT') {
    return true;
  }
  if (target.tagName !== 'INPUT') {
    return false;
  }
  return !['range', 'checkbox', 'color', 'button'].includes(target.type);
}

// Opens the frame. `title` names the media. Returns the frame's slots and
// controls; the editor fills them once its media is loaded.
export function openEditorFrame({ title }) {
  const root = document.getElementById('dialog-root');
  let editor = null; // { onKey(event, typing) -> handled, isDirty(), undo(), redo(), canUndo(), canRedo(), destroy() }
  let closed = false;

  const undoButton = h('button', { class: 'btn is-quiet', title: 'Annuler (Ctrl+Z)', 'aria-label': 'Annuler la dernière action', disabled: true, onClick: () => editor && editor.undo() }, icon('undo'));
  const redoButton = h('button', { class: 'btn is-quiet', title: 'Rétablir (Ctrl+Y)', 'aria-label': 'Rétablir', disabled: true, onClick: () => editor && editor.redo() }, icon('redo'));
  const status = h('span', { class: 'editor-status', 'aria-live': 'polite' });
  const actions = h('div', { class: 'editor-actions' });
  const stage = h('div', { class: 'editor-stage' }, h('p', { class: 'editor-loading' }, 'Chargement du média…'));
  const rail = h('div', { class: 'editor-rail', hidden: true });
  const panel = h('aside', { class: 'editor-panel', hidden: true });
  const timeline = h('footer', { class: 'editor-timeline', hidden: true });
  const veil = h('div', { class: 'editor-veil', hidden: true });

  const node = h(
    'div',
    { class: 'editor', role: 'dialog', 'aria-modal': 'true', 'aria-label': `Éditeur : ${title}`, tabindex: '-1' },
    h(
      'header',
      { class: 'editor-bar' },
      h('h2', { class: 'ellipsis', title }, icon('edit'), `Éditer « ${title} »`),
      undoButton,
      redoButton,
      status,
      h('span', { class: 'spacer' }),
      actions,
    ),
    h('div', { class: 'editor-body' }, rail, stage, panel),
    timeline,
    veil,
  );

  function close() {
    if (closed) {
      return;
    }
    closed = true;
    document.removeEventListener('keydown', onKeydown, true);
    if (editor && editor.destroy) {
      editor.destroy();
    }
    // Only the editor leaves: a dialog under it (« Ajouter un média ») stays.
    node.remove();
  }

  // Leaving with unsaved work asks first.
  function requestClose() {
    if (!editor || !editor.isDirty()) {
      close();
      return;
    }
    veil.replaceChildren(
      h(
        'div',
        { class: 'editor-confirm', role: 'alertdialog', 'aria-label': 'Abandonner les modifications' },
        h('h3', null, 'Abandonner les modifications ?'),
        h('p', null, 'Ce que tu as fait dans l’éditeur ne sera pas enregistré.'),
        h(
          'div',
          { class: 'actions' },
          h('button', { class: 'btn', onClick: () => { veil.hidden = true; node.focus(); } }, 'Continuer l’édition'),
          h('button', { class: 'btn is-danger', onClick: close }, 'Abandonner'),
        ),
      ),
    );
    veil.hidden = false;
    veil.querySelector('button').focus();
  }

  // Keys are handled here, before the board's own shortcuts (drawer, capture).
  function onKeydown(event) {
    if (closed) {
      return;
    }
    event.stopPropagation();
    if (!veil.hidden) {
      if (event.key === 'Escape') {
        event.preventDefault();
        veil.hidden = true;
        node.focus();
      }
      return;
    }
    const typing = isTyping(event.target);
    const mod = event.ctrlKey || event.metaKey;
    if (mod && !event.altKey && event.key.toLowerCase() === 'z' && !typing) {
      event.preventDefault();
      if (editor) {
        (event.shiftKey ? editor.redo : editor.undo)();
      }
      return;
    }
    if (mod && !event.altKey && event.key.toLowerCase() === 'y' && !typing) {
      event.preventDefault();
      if (editor) {
        editor.redo();
      }
      return;
    }
    if (editor && editor.onKey && editor.onKey(event, typing)) {
      event.preventDefault();
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      if (typing) {
        event.target.blur();
        node.focus();
      } else {
        requestClose();
      }
    }
  }

  document.addEventListener('keydown', onKeydown, true);
  root.append(node);
  node.focus();

  const frame = {
    node,
    stage,
    rail,
    panel,
    timeline,
    close,
    requestClose,
    isClosed: () => closed,
    setEditor(next) {
      editor = next;
      frame.refreshHistory();
    },
    // Enables undo/redo from the editor's history.
    refreshHistory() {
      undoButton.disabled = !editor || !editor.canUndo();
      redoButton.disabled = !editor || !editor.canRedo();
    },
    setStatus(text) {
      status.textContent = text || '';
    },
    // `buttons`: [{ label, title, primary, disabled, onClick }]. « Fermer » is
    // always there.
    setActions(buttons) {
      actions.replaceChildren(
        h('button', { class: 'btn is-quiet', onClick: requestClose }, 'Fermer'),
        ...(buttons || []).map((button) =>
          h(
            'button',
            { class: `btn${button.primary ? ' is-primary' : ''}`, title: button.title || null, disabled: Boolean(button.disabled), onClick: button.onClick },
            button.label,
          ),
        ),
      );
    },
    // A blocking message over the whole editor (export in progress, failure).
    setBusy(text) {
      if (!text) {
        veil.hidden = true;
        return;
      }
      veil.replaceChildren(h('div', { class: 'editor-confirm' }, h('p', null, text)));
      veil.hidden = false;
    },
    showError(text) {
      stage.replaceChildren(h('div', { class: 'empty' }, h('h2', null, 'Média illisible'), h('p', null, text)));
      frame.setActions([]);
    },
  };
  frame.setActions([]);
  return frame;
}

// Small building blocks of the side panels.
export function section(title, ...children) {
  return h('section', { class: 'editor-section' }, title ? h('h3', null, title) : null, ...children);
}

// A row of exclusive choices. `options`: [[value, label, iconName?]].
export function choices(options, current, onPick, label) {
  return h(
    'div',
    { class: 'editor-choices', role: 'group', 'aria-label': label || null },
    options.map(([value, text, glyph]) =>
      h(
        'button',
        { class: `chip${value === current ? ' is-active' : ''}`, type: 'button', 'aria-pressed': String(value === current), onClick: () => onPick(value) },
        glyph ? icon(glyph) : null,
        text,
      ),
    ),
  );
}

// A labelled slider. `onInput` follows the thumb, `onCommit` fires when it
// is released; `format` renders the value shown next to the label. A double
// click puts the slider back to `defaultValue`.
export function slider({ label, min, max, step, value, format, onInput, onCommit, defaultValue }) {
  const output = h('span', { class: 'editor-value' }, format ? format(value) : String(value));
  const input = h('input', {
    type: 'range',
    min: String(min),
    max: String(max),
    step: String(step || 1),
    value: String(value),
    onInput: () => {
      const next = Number(input.value);
      output.textContent = format ? format(next) : String(next);
      onInput(next);
    },
    onChange: () => {
      if (onCommit) {
        onCommit(Number(input.value));
      }
    },
  });
  resetOnDoubleClick(input, defaultValue);
  return h('label', { class: 'editor-slider' }, h('span', { class: 'editor-slider-head' }, label, output), input);
}

export function toggle(label, checked, onChange) {
  const input = h('input', { type: 'checkbox', checked, onChange: () => onChange(input.checked) });
  return h('label', { class: 'switch' }, input, label);
}
