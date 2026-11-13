// Administration tab: members, invites and guild settings.

import { api, run } from '../core/api.js';
import { avatarNode, h, icon } from '../core/dom.js';
import { formatCode, relativeTime } from '../core/format.js';
import { can, ROLE_LABELS, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { confirmDialog, copyText } from '../core/ui.js';
import { loadAdmin, loadGuild } from '../data.js';
import { renderView } from './view.js';

export function renderAdmin() {
  const tabs = [];
  if (can('manageMembers')) {
    tabs.push(['members', 'Membres']);
  }
  if (can('manageInvites')) {
    tabs.push(['invites', 'Invitations']);
  }
  if (can('manageSettings')) {
    tabs.push(['settings', 'Réglages']);
  }
  if (!tabs.some(([id]) => id === state.adminTab)) {
    state.adminTab = tabs.length ? tabs[0][0] : 'members';
  }
  const body =
    state.adminTab === 'invites' ? renderInvites() : state.adminTab === 'settings' ? renderGuildSettings() : renderMembers();
  return h(
    'div',
    null,
    h(
      'div',
      { class: 'subtabs' },
      tabs.map(([id, label]) =>
        h('button', { class: `chip${state.adminTab === id ? ' is-active' : ''}`, onClick: () => { state.adminTab = id; renderView(); loadAdmin(); } }, label),
      ),
    ),
    body,
  );
}

function renderMembers() {
  if (state.members.length === 0) {
    return h('p', { class: 'meta' }, 'Chargement des membres…');
  }
  return h(
    'div',
    { class: 'list' },
    state.members.map((member) =>
      h(
        'div',
        { class: 'line is-member' },
        avatarNode(member.displayName, member.avatar),
        h('div', { class: 'ellipsis' }, member.displayName, member.isSelf ? h('span', { class: 'sub' }, '  (toi)') : null),
        h('span', { class: 'sub seen' }, member.connected ? 'En ligne' : relativeTime(member.lastSeenAt)),
        h(
          'select',
          { class: 'field', 'aria-label': `Rôle de ${member.displayName}`, onChange: (event) => changeRole(member, event.target.value) },
          Object.entries(ROLE_LABELS).map(([id, label]) => h('option', { value: id, selected: member.role === id }, label)),
        ),
        member.isSelf
          ? h('span')
          : h('button', { class: 'btn is-quiet is-danger', onClick: () => openKickDialog(member) }, 'Exclure'),
      ),
    ),
  );
}

async function changeRole(member, role) {
  await run(() => api('PATCH', `/overlay/admin/members/${encodeURIComponent(member.id)}`, { role }), `${member.displayName} est maintenant ${ROLE_LABELS[role].toLowerCase()}.`);
  await loadAdmin();
  if (member.isSelf) {
    loadGuild();
  }
}

function openKickDialog(member) {
  confirmDialog(`Exclure ${member.displayName} ?`, 'Ses overlays sont déconnectés tout de suite. Il lui faudra un nouveau code pour revenir.', 'Exclure', async () => {
    await run(() => api('DELETE', `/overlay/admin/members/${encodeURIComponent(member.id)}`), `${member.displayName} a été exclu.`);
    loadAdmin();
  });
}

function inviteStatus(invite) {
  if (invite.revokedAt) {
    return 'Révoquée';
  }
  const uses = invite.maxUses ? `${invite.useCount} / ${invite.maxUses} utilisations` : `${invite.useCount} utilisation${invite.useCount > 1 ? 's' : ''}`;
  if (!invite.isActive) {
    return invite.maxUses && invite.useCount >= invite.maxUses ? `Épuisée, ${uses}` : `Expirée, ${uses}`;
  }
  if (!invite.expiresAt) {
    return `Sans expiration, ${uses}`;
  }
  const hours = Math.max(1, Math.round((new Date(invite.expiresAt).getTime() - Date.now()) / 3600000));
  return `Expire dans ${hours >= 48 ? `${Math.round(hours / 24)} j` : `${hours} h`}, ${uses}`;
}

function renderInvites() {
  const role = h('select', { class: 'field', 'aria-label': 'Rôle accordé' }, [
    h('option', { value: 'member' }, 'Membre'),
    h('option', { value: 'guest' }, 'Invité'),
    h('option', { value: 'admin' }, 'Admin'),
  ]);
  const expiry = h('select', { class: 'field', 'aria-label': 'Expiration' }, [
    h('option', { value: '24' }, 'Expire dans 24 h'),
    h('option', { value: '168' }, 'Expire dans 7 jours'),
    h('option', { value: '' }, "N'expire jamais"),
  ]);
  const uses = h('select', { class: 'field', 'aria-label': "Nombre d'utilisations" }, [
    h('option', { value: '1' }, '1 utilisation'),
    h('option', { value: '10' }, '10 utilisations'),
    h('option', { value: '' }, 'Illimité'),
  ]);
  const create = async () => {
    const body = await run(() =>
      api('POST', '/overlay/admin/invites', {
        role: role.value,
        expiresInHours: expiry.value ? Number(expiry.value) : null,
        maxUses: uses.value ? Number(uses.value) : null,
      }),
    );
    if (body && body.invite) {
      copyText(formatCode(body.invite.code), 'Code créé et copié.');
      loadAdmin();
    }
  };
  return h(
    'div',
    null,
    h('div', { class: 'panel row is-wrap' }, role, expiry, uses, h('span', { class: 'spacer' }), h('button', { class: 'btn', onClick: create }, icon('plus'), 'Créer un code')),
    state.invites.length === 0
      ? h('p', { class: 'meta' }, 'Aucune invitation pour le moment.')
      : h(
          'div',
          { class: 'list' },
          state.invites.map((invite) =>
            h(
              'div',
              { class: 'line is-invite' },
              h('span', { class: 'code' }, formatCode(invite.code)),
              h('span', null, h('span', { class: `badge is-${invite.role}` }, ROLE_LABELS[invite.role] || invite.role)),
              h('span', { class: 'sub ellipsis' }, inviteStatus(invite)),
              invite.isActive
                ? h('button', { class: 'btn is-quiet', 'aria-label': `Copier le code ${invite.code}`, title: 'Copier le code', onClick: () => copyText(formatCode(invite.code), 'Code copié.') }, icon('copy'))
                : h('span'),
              invite.isActive ? h('button', { class: 'btn is-quiet is-danger', onClick: () => revokeInvite(invite) }, 'Révoquer') : h('span'),
            ),
          ),
        ),
  );
}

async function revokeInvite(invite) {
  await run(() => api('POST', `/overlay/admin/invites/${encodeURIComponent(invite.code)}/revoke`), 'Invitation révoquée.');
  loadAdmin();
}

// Guild-wide settings (server side), not the device settings page.
function renderGuildSettings() {
  const guild = (state.info && state.info.guild) || { settings: {} };
  const name = h('input', { class: 'field', id: 'set-name', value: guild.name || '', maxlength: '60' });
  const defaultTime = h('input', { class: 'field', id: 'set-default', type: 'number', min: '0', max: '86400', value: guild.settings.defaultMediaTime ?? '', placeholder: 'Valeur du serveur', style: 'max-width:160px' });
  const maxTime = h('input', { class: 'field', id: 'set-max', type: 'number', min: '0', max: '86400', value: guild.settings.maxMediaTime ?? '', placeholder: 'Sans limite', style: 'max-width:160px' });
  const full = h('input', { type: 'checkbox', id: 'set-full', checked: Boolean(guild.settings.displayMediaFull) });
  const guests = h('input', { type: 'checkbox', id: 'set-guests', checked: guild.guestsCanViewBoard !== false });
  const save = async () => {
    const body = await run(
      () =>
        api('PATCH', '/overlay/admin/settings', {
          name: name.value,
          defaultMediaTime: defaultTime.value === '' ? null : Number(defaultTime.value),
          maxMediaTime: maxTime.value === '' ? null : Number(maxTime.value),
          displayMediaFull: full.checked,
          guestsCanViewBoard: guests.checked,
        }),
      'Réglages enregistrés.',
    );
    if (body) {
      await loadGuild();
      renderView();
    }
  };
  const discord = [
    h('h3', { class: 'section-title' }, 'Discord'),
    h('p', { class: 'meta' }, guild.discordGuildId ? `Guilde liée au serveur Discord ${guild.discordGuildId}.` : guild.standalone ? 'Cette guilde n’est liée à aucun serveur Discord.' : 'Cette guilde est un serveur Discord.'),
  ];
  if (guild.standalone && can('manageSettings')) {
    if (guild.discordGuildId) {
      const unlink = () => {
        confirmDialog('Délier le serveur Discord ?', 'Les commandes de ce serveur Discord ne viseront plus cette guilde.', 'Délier', async () => {
          await run(() => api('DELETE', '/overlay/admin/discord-link'));
          await loadGuild();
          renderView();
        });
      };
      discord.push(h('div', { class: 'row is-wrap' }, h('button', { class: 'btn is-quiet is-danger', onClick: unlink }, 'Délier le serveur Discord')));
    } else {
      const generated = h('div', { class: 'stack', hidden: true });
      const verifyDiscordLink = async () => {
        await loadGuild();
        const linked = Boolean(state.info && state.info.guild && state.info.guild.discordGuildId);
        renderView();
        notify(linked ? 'Serveur Discord lié.' : 'Pas encore lié : saisis la commande dans Discord.', !linked);
      };
      const generate = async () => {
        const body = await run(() => api('POST', '/overlay/admin/discord-link-code'));
        if (!body || !body.code) {
          return;
        }
        const command = `/lier code:${body.code}`;
        generated.replaceChildren(
          h(
            'p',
            { class: 'meta' },
            'Dans le serveur Discord, un administrateur tape /lier code:',
            h('span', { class: 'code' }, body.code),
            ' (ou /link code:',
            h('span', { class: 'code' }, body.code),
            '). Code valable 10 minutes.',
          ),
          h(
            'div',
            { class: 'row is-wrap' },
            h('button', { class: 'btn', onClick: () => copyText(command, 'Commande copiée.') }, 'Copier la commande'),
            h('button', { class: 'btn is-quiet', onClick: verifyDiscordLink }, 'Vérifier la liaison'),
          ),
        );
        generated.hidden = false;
      };
      discord.push(
        h('div', { class: 'row is-wrap' }, h('button', { class: 'btn', onClick: generate }, 'Générer un code de liaison')),
        generated,
      );
    }
  }
  return h(
    'div',
    null,
    h(
      'div',
      { class: 'form' },
      h('label', { for: 'set-name' }, 'Nom de la guilde'),
      name,
      h('label', { for: 'set-default' }, 'Durée par défaut (s)'),
      defaultTime,
      h('p', { class: 'hint' }, 'Durée d’affichage d’une image ou d’un texte sans durée propre.'),
      h('label', { for: 'set-max' }, 'Durée maximale (s)'),
      maxTime,
      h('label', { for: 'set-full' }, 'Affichage'),
      h('label', { class: 'switch' }, full, 'Médias en plein écran'),
      h('label', { for: 'set-guests' }, 'Invités'),
      h('label', { class: 'switch' }, guests, 'Peuvent consulter le board'),
    ),
    h('div', { class: 'row', style: 'margin-top:20px' }, h('button', { class: 'btn', onClick: save }, 'Enregistrer les réglages')),
    discord,
  );
}
