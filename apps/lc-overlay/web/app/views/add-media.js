// "Ajouter un média" dialog.

import { api, run } from '../core/api.js';
import { h } from '../core/dom.js';
import { notify } from '../core/toast.js';
import { openDialog } from '../core/ui.js';
import { loadItems } from '../data.js';

export function openAddDialog() {
  openDialog((close) => {
    const url = h('input', { class: 'field', type: 'url', placeholder: 'https://…', required: true });
    const title = h('input', { class: 'field', placeholder: 'Chat qui tombe', maxlength: '240' });
    const message = h('textarea', { class: 'field', rows: '2', maxlength: '500', placeholder: 'Texte affiché avec le média' });
    const submit = h('button', { class: 'btn', type: 'submit' }, 'Ajouter au board');
    const form = h(
      'form',
      {
        class: 'stack',
        onSubmit: async (event) => {
          event.preventDefault();
          submit.disabled = true;
          submit.textContent = 'Téléchargement…';
          const body = await run(() => api('POST', '/overlay/media-board/items', { url: url.value.trim(), title: title.value, message: message.value }));
          if (body) {
            close();
            notify(body.created ? 'Média ajouté au board.' : 'Ce média est déjà sur le board.');
            loadItems();
          } else {
            submit.disabled = false;
            submit.textContent = 'Ajouter au board';
          }
        },
      },
      h('label', null, 'Lien du média', url),
      h('label', null, 'Titre (facultatif)', title),
      h('label', null, 'Message (facultatif)', message),
      h('div', { class: 'actions' }, h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'), submit),
    );
    return [h('h2', null, 'Ajouter un média'), h('p', null, 'Vidéo, image ou son : TikTok, X, YouTube ou lien direct.'), form];
  });
}
