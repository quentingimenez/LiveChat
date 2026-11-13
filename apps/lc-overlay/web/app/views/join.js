// "Rejoindre une guilde" dialog.

import { resizeAvatar } from '../core/avatar.js';
import { avatarNode, h } from '../core/dom.js';
import { invoke } from '../core/ipc.js';
import { ERROR_LABELS, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { openDialog } from '../core/ui.js';
import { applyGuilds } from '../data.js';

export function openJoinDialog() {
  openDialog((close) => {
    const known = state.guilds[0] ? state.guilds[0].membership.serverUrl : '';
    const server = h('input', { class: 'field', type: 'url', placeholder: 'https://livechat.exemple.fr', value: known, required: true });
    const code = h('input', { class: 'field code', placeholder: 'K7QX-M2PD', maxlength: '40', autocomplete: 'off', spellcheck: 'false', required: true });
    const name = h('input', { class: 'field', placeholder: 'Ton pseudo (facultatif avec un code Discord)', maxlength: '40', value: state.profile.displayName || '' });
    let avatar = state.profile.avatar || null;
    const avatarPreview = h('span', null, avatarNode(name.value || '?', avatar));
    const file = h('input', {
      type: 'file',
      accept: 'image/png,image/jpeg,image/webp',
      hidden: true,
      onChange: async (event) => {
        const picked = event.target.files && event.target.files[0];
        if (picked) {
          avatar = await resizeAvatar(picked);
          avatarPreview.replaceChildren(avatarNode(name.value || '?', avatar));
        }
      },
    });
    const error = h('p', { class: 'error-text', hidden: true });
    const submit = h('button', { class: 'btn is-air', type: 'submit' }, 'Rejoindre');
    const form = h(
      'form',
      {
        class: 'stack',
        onSubmit: async (event) => {
          event.preventDefault();
          error.hidden = true;
          if (!code.value.trim() || !server.value.trim()) {
            error.textContent = 'Indique le serveur et le code.';
            error.hidden = false;
            return;
          }
          submit.disabled = true;
          submit.textContent = 'Connexion…';
          let response = null;
          try {
            response = await invoke('pairing_consume', {
              payload: { serverUrl: server.value.trim(), code: code.value.trim().replace(/[\s-]/g, ''), displayName: name.value.trim(), avatar },
            });
          } catch (failure) {
            response = { ok: false, message: String(failure) };
          }
          if (response.ok) {
            close();
            notify('Guilde rejointe.');
            applyGuilds(await invoke('guilds_list'));
            return;
          }
          submit.disabled = false;
          submit.textContent = 'Rejoindre';
          const message = response.message || '';
          if (/display_name_required/.test(message)) {
            error.textContent = ERROR_LABELS.display_name_required;
            error.hidden = false;
            name.focus();
            return;
          }
          error.textContent = /pairing_code_invalid_or_expired/.test(message)
            ? ERROR_LABELS.pairing_code_invalid_or_expired
            : message || 'Impossible de rejoindre la guilde.';
          error.hidden = false;
        },
      },
      h('label', null, 'Serveur', server),
      h('label', null, "Code d'invitation", code),
      h('label', null, 'Pseudo', name),
      h('div', { class: 'row' }, avatarPreview, h('button', { class: 'btn', type: 'button', onClick: () => file.click() }, 'Choisir un avatar'), file),
      h(
        'p',
        { class: 'file-hint' },
        'Code /pair-code de Discord : laisse le pseudo et l’avatar vides pour reprendre ceux de Discord.',
      ),
      error,
      h('div', { class: 'actions' }, h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'), submit),
    );
    return [h('h2', null, 'Rejoindre une guilde'), h('p', null, 'Le code reçu d’un admin fixe ton rôle de départ.'), form];
  });
}
