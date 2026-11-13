// "Mon profil" dialog of a guild: display name, extra device, browser
// extension pairing and leaving the guild.

import { api, run } from '../core/api.js';
import { h } from '../core/dom.js';
import { formatCode } from '../core/format.js';
import { invoke } from '../core/ipc.js';
import { can, describeError, guildLabel, ROLE_LABELS, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { confirmDialog, copyText, openDialog } from '../core/ui.js';
import { applyGuilds, loadGuild } from '../data.js';

export function confirmLeaveGuild(guild) {
  confirmDialog(`Quitter ${guildLabel(guild)} ?`, 'Tu ne recevras plus ses médias. Il te faudra un nouveau code pour revenir.', 'Quitter la guilde', async () => {
    const result = await run(() => invoke('guild_leave', { payload: { guildId: guild.membership.guildId } }));
    if (!result) {
      return;
    }
    if (!result.ok) {
      notify(describeError(result.reason), true);
    }
    applyGuilds(result.guilds);
  });
}

export function openGuildDialog(guild) {
  openDialog((close) => {
    const member = (state.info && state.info.member) || {};
    const name = h('input', { class: 'field', maxlength: '40', value: member.displayName || guild.membership.displayName || '' });
    const deviceCode = h('p', { class: 'meta', hidden: true });
    const extensionCode = h('p', { class: 'meta', hidden: true });
    const save = async () => {
      const body = await run(() => api('PATCH', '/overlay/profile', { displayName: name.value }), 'Profil enregistré.');
      if (body) {
        close();
        loadGuild();
      }
    };
    const makeDeviceCode = async () => {
      const body = await run(() => api('POST', '/overlay/device-code'));
      if (body && body.invite) {
        deviceCode.replaceChildren('Code valable 10 minutes : ', h('span', { class: 'code' }, formatCode(body.invite.code)));
        deviceCode.hidden = false;
      }
    };
    const makeExtensionCode = async () => {
      const body = await run(() => api('POST', '/overlay/ingest-code'));
      if (!body || !body.code) {
        return;
      }
      extensionCode.replaceChildren(
        'Dans les options de l’extension, saisis l’URL ',
        h('span', { class: 'code' }, body.apiBaseUrl),
        ' et le code ',
        h('span', { class: 'code' }, body.code),
        '. Valable 10 minutes.',
      );
      extensionCode.hidden = false;
      copyText(body.code, 'Code de l’extension copié.');
    };
    const leave = () => {
      close();
      confirmLeaveGuild(guild);
    };
    return [
      h('h2', null, 'Mon profil'),
      h('p', null, `Dans ${guildLabel(guild)}, en tant que ${(ROLE_LABELS[member.role] || 'membre').toLowerCase()}.`),
      h(
        'div',
        { class: 'stack' },
        h('label', null, 'Pseudo', name),
        h(
          'div',
          { class: 'row is-wrap' },
          h('button', { class: 'btn', onClick: makeDeviceCode }, 'Ajouter un autre appareil'),
          can('play') ? h('button', { class: 'btn', onClick: makeExtensionCode }, 'Appairer l’extension navigateur') : null,
        ),
        deviceCode,
        can('play') ? extensionCode : null,
      ),
      h(
        'div',
        { class: 'actions' },
        h('button', { class: 'btn is-quiet is-danger', onClick: leave }, 'Quitter la guilde'),
        h('span', { class: 'spacer' }),
        h('button', { class: 'btn is-quiet', onClick: close }, 'Fermer'),
        h('button', { class: 'btn', onClick: save }, 'Enregistrer'),
      ),
    ];
  });
}
