// "Say a text" dialog: the board's version of the Discord commands /dire and
// /cdire. The server synthesizes the text (Piper) and queues it for the
// guild's overlays.

import { api } from '../core/api.js';
import { h } from '../core/dom.js';
import { SAY_CAPTION_MAX_CHARS, isSaySubmitKey, sayErrorMessage, sayPayload, speechCounter } from '../core/say-helpers.js';
import { state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { openDialog } from '../core/ui.js';

export function openSayDialog() {
  openDialog((close) => {
    let sending = false;

    const text = h('textarea', {
      class: 'field say-text',
      rows: '5',
      required: true,
      placeholder: 'Le texte à lire à voix haute (Maj+Entrée\u00a0: retour à la ligne)',
      'aria-describedby': 'say-counter',
    });
    const counter = h('span', { id: 'say-counter', class: 'say-counter', 'aria-live': 'polite' });
    const caption = h('input', {
      class: 'field',
      maxlength: String(SAY_CAPTION_MAX_CHARS),
      placeholder: 'Légende affichée pendant la lecture',
    });
    const anonymous = h('input', { type: 'checkbox' });
    const error = h('p', { class: 'error-text', role: 'alert', hidden: true });
    const submit = h('button', { class: 'btn is-primary', type: 'submit' }, 'Lire');

    function refresh() {
      const count = speechCounter(text.value);
      counter.textContent = count.label;
      counter.classList.toggle('is-over', count.tooLong);
      submit.disabled = sending || !count.valid;
    }

    function showError(message) {
      error.textContent = message;
      error.hidden = !message;
    }

    text.addEventListener('input', () => {
      showError('');
      refresh();
    });

    // Enter sends, Shift+Enter adds a line break.
    text.addEventListener('keydown', (event) => {
      if (!isSaySubmitKey(event)) {
        return;
      }
      event.preventDefault();
      if (!submit.disabled) {
        form.requestSubmit();
      }
    });

    const form = h(
      'form',
      {
        class: 'stack',
        onSubmit: async (event) => {
          event.preventDefault();
          if (sending) {
            return;
          }
          if (!state.activeGuildId) {
            close();
            return;
          }
          const payload = sayPayload({ text: text.value, caption: caption.value, anonymous: anonymous.checked });
          if (!payload) {
            showError(sayErrorMessage(400, speechCounter(text.value).tooLong ? 'text_too_long' : 'invalid_payload'));
            return;
          }
          sending = true;
          submit.textContent = 'Synthèse en cours…';
          showError('');
          refresh();
          try {
            await api('POST', '/overlay/say', payload);
            close();
            notify('C’est envoyé : le texte va être lu sur les overlays.');
          } catch (failure) {
            // The dialog stays open with the text, so it can be fixed and resent.
            showError(sayErrorMessage(failure && failure.status, failure && failure.code, failure && failure.message));
            sending = false;
            submit.textContent = 'Lire';
            refresh();
          }
        },
      },
      h('label', null, 'Texte à lire', text),
      h('div', { class: 'say-meta' }, counter),
      h('label', null, 'Légende affichée (facultatif)', caption),
      h('label', { class: 'switch say-anonymous' }, anonymous, h('span', null, 'Rester anonyme')),
      error,
      h('div', { class: 'actions' }, h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'), submit),
    );

    refresh();
    return [h('h2', null, 'Lire un texte'), form];
  });
}
