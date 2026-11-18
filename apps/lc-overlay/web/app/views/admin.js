// Administration tab: members, devices, bans and codes (invitations and
// pairing codes). The server settings of the guild (`renderServerSettings`)
// are shown in its « Réglages » tab.

import { api, run } from '../core/api.js';
import { isObsolete, newestVersion, platformLabel, presenceLabel as devicePresenceLabel, sessionModeLabel } from '../core/device-helpers.js';
import { avatarNode, h, icon } from '../core/dom.js';
import { formatCode, relativeTime } from '../core/format.js';
import { activeGuild, can, ROLE_LABELS, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { confirmDialog, copyText, openDialog } from '../core/ui.js';
import { loadAdmin, loadGuild } from '../data.js';
import { renderView } from './view.js';

export function renderAdmin() {
  const tabs = [];
  if (can('manageMembers')) {
    tabs.push(['members', 'Membres', state.members.length]);
    tabs.push(['devices', 'Appareils', state.devices.length]);
  }
  if (can('manageInvites')) {
    tabs.push(['codes', 'Codes', activeCodeCount()]);
  }
  if (can('manageMembers')) {
    tabs.push(['bans', 'Bannis', null]);
  }
  if (can('manageSettings')) {
    tabs.push(['audit', 'Journal', null]);
  }
  if (!tabs.some(([id]) => id === state.adminTab)) {
    state.adminTab = tabs.length ? tabs[0][0] : 'members';
  }
  const views = { codes: renderCodes, bans: renderBans, devices: renderDevices, audit: renderAudit };
  const body = (views[state.adminTab] || renderMembers)();
  return h(
    'div',
    null,
    h(
      'div',
      { class: 'subtabs' },
      tabs.map(([id, label, count]) =>
        h(
          'button',
          { class: `chip${state.adminTab === id ? ' is-active' : ''}`, onClick: () => { state.adminTab = id; renderView(); loadAdmin(); } },
          label,
          count !== null ? h('span', { class: 'count' }, count) : null,
        ),
      ),
    ),
    body,
  );
}

// Active codes of both families (invitations and pairing codes), as the
// « Codes » tab lists them.
function activeCodeCount() {
  return (
    state.invites.filter((invite) => invite.isActive).length +
    state.pairingCodes.filter((code) => code.isActive).length
  );
}

// A muted overlay closes its socket, so the server sees it offline. For this
// device the reason is known: say so instead of a last-seen time.
function presenceLabel(member, selfMuted) {
  if (member.connected) {
    return 'En ligne';
  }
  if (member.isSelf && selfMuted) {
    return 'En pause';
  }
  return relativeTime(member.lastSeenAt);
}

function memberDevices(member) {
  const count = Number(member.deviceCount) || 0;
  return `${count} appareil${count > 1 ? 's' : ''}`;
}

function joinDate(iso) {
  if (!iso) {
    return 'date inconnue';
  }
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? 'date inconnue' : date.toLocaleDateString('fr-FR');
}

function memberMeta(member) {
  return h(
    'div',
    { class: 'sub member-meta' },
    `${memberDevices(member)} · arrivé le ${joinDate(member.createdAt)}`,
    member.discordLinked
      ? h('span', { class: 'badge', title: member.discordUserId || 'Compte Discord lié' }, 'Discord')
      : null,
  );
}

function renderMembers() {
  if (state.members.length === 0) {
    return h('p', { class: 'meta' }, 'Chargement des membres…');
  }
  const guild = activeGuild();
  const selfMuted = Boolean(guild && guild.membership.muted);
  return h(
    'div',
    { class: 'list' },
    state.members.map((member) =>
      h(
        'div',
        { class: 'line is-member' },
        avatarNode(member.displayName, member.avatar),
        h(
          'div',
          { class: 'member-cell' },
          h('div', { class: 'ellipsis' }, member.displayName, member.isSelf ? h('span', { class: 'sub' }, '  (toi)') : null),
          memberMeta(member),
        ),
        h('span', { class: 'sub seen' }, presenceLabel(member, selfMuted)),
        h(
          'select',
          { class: 'field', 'aria-label': `Rôle de ${member.displayName}`, onChange: (event) => changeRole(member, event.target.value) },
          Object.entries(ROLE_LABELS).map(([id, label]) => h('option', { value: id, selected: member.role === id }, label)),
        ),
        h(
          'span',
          { class: 'row member-actions' },
          h(
            'button',
            { class: 'btn is-quiet', title: 'Créer un code pour rattacher un appareil à ce membre.', onClick: () => openDeviceCodeDialog(member) },
            'Code d’appareil',
          ),
          member.isSelf
            ? null
            : h('button', { class: 'btn is-quiet is-danger', onClick: () => openKickDialog(member) }, 'Exclure'),
          member.isSelf
            ? null
            : h('button', { class: 'btn is-quiet is-danger', onClick: () => openBanDialog(member) }, 'Bannir'),
        ),
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

function openBanDialog(member) {
  const text = member.discordLinked
    ? 'Ses overlays et ses extensions sont révoqués tout de suite. Son compte Discord ne pourra plus obtenir de code ni utiliser le bot sur cette guilde.'
    : 'Ses overlays et ses extensions sont révoqués tout de suite. Sans compte Discord lié, rien ne l’empêche de revenir avec une nouvelle invitation : ne lui en donne pas.';
  confirmDialog(`Bannir ${member.displayName} ?`, text, 'Bannir', async () => {
    await run(() => api('POST', `/overlay/admin/members/${encodeURIComponent(member.id)}/ban`), `${member.displayName} a été banni.`);
    loadAdmin();
  });
}

// Code d'appareil for another member: a single-use code that attaches a new
// overlay to that member's account. The admin reads it out to them.
async function openDeviceCodeDialog(member) {
  const body = await run(() => api('POST', '/overlay/admin/device-codes', { memberId: member.id }));
  if (!body || !body.invite) {
    return;
  }
  const code = formatCode(body.invite.code);
  openDialog((close) => [
    h('h2', null, 'Code d’appareil'),
    h('p', null, `Pour ${member.displayName} : à saisir dans l’overlay pour rattacher un nouvel appareil à son compte.`),
    h(
      'div',
      { class: 'row is-wrap' },
      h('span', { class: 'code' }, code),
      h('button', { class: 'btn', onClick: () => copyText(code, 'Code copié.') }, icon('copy'), 'Copier'),
    ),
    h('p', { class: 'meta' }, 'Expire dans 10 min. Une seule utilisation.'),
    h('div', { class: 'actions' }, h('button', { class: 'btn', onClick: close }, 'Fermer')),
  ]);
}

function renderBans() {
  if (state.bans.length === 0) {
    return h('p', { class: 'meta' }, 'Personne n’est banni de cette guilde.');
  }
  return h(
    'div',
    { class: 'list' },
    state.bans.map((ban) =>
      h(
        'div',
        { class: 'line is-ban' },
        avatarNode(ban.displayName, ban.avatar),
        h('div', { class: 'ellipsis' }, ban.displayName, ban.discordUserId ? null : h('span', { class: 'sub' }, '  (sans compte Discord)')),
        h('span', { class: 'sub seen' }, `${relativeTime(ban.createdAt)}${ban.bannedBy ? `, par ${ban.bannedBy}` : ''}`),
        h('button', { class: 'btn is-quiet', onClick: () => openUnbanDialog(ban) }, 'Lever'),
      ),
    ),
  );
}

function openUnbanDialog(ban) {
  confirmDialog(`Lever le bannissement de ${ban.displayName} ?`, 'Il pourra revenir avec un nouveau code. Ses anciens appareils restent révoqués.', 'Lever', async () => {
    await run(() => api('DELETE', `/overlay/admin/bans/${encodeURIComponent(ban.id)}`), `Le bannissement de ${ban.displayName} est levé.`);
    loadAdmin();
  });
}

// ---------------------------------------------------------------- journal

// French labels of the actions written by the server audit middleware
// (`apps/lc-server/src/http/admin/audit.rs`). Console actions keep their
// `guild.*` / `auth.*` ids, overlay ones use `overlay.*`. Unknown actions are
// shown raw.
const AUDIT_ACTION_LABELS = {
  'auth.login_secret': 'Connexion par secret',
  'auth.login_discord': 'Connexion Discord',
  'auth.logout': 'Déconnexion',
  'guild.create': 'Guilde créée',
  'guild.admin_code': 'Code admin émis',
  'guild.settings': 'Réglages modifiés',
  'guild.stop': 'Lecture arrêtée',
  'guild.purge': 'Guilde supprimée',
  'guild.revoke_clients': 'Accès révoqués',
  'guild.board_clear': 'Board vidé',
  'guild.discord_unlink': 'Discord délié',
  'guild.discord_leave': 'Bot retiré du serveur',
  'guild.discord_link_code': 'Code de liaison Discord',
  'guild.device_code': 'Code d’appareil émis',
  'guild.invite_create': 'Invitation créée',
  'guild.invite_revoke': 'Invitation révoquée',
  'guild.member_update': 'Rôle modifié',
  'guild.member_remove': 'Membre exclu',
  'guild.member_ban': 'Membre banni',
  'guild.ban_remove': 'Bannissement levé',
  'guild.queue_clear': 'File vidée',
  'guild.queue_remove': 'Média retiré de la file',
  'guild.media_update': 'Média modifié',
  'guild.media_remove': 'Média supprimé',
  'guild.media_edit': 'Média réencodé',
  'overlay.revoke': 'Appareil révoqué',
  'overlay.update': 'Appareil modifié',
  'overlay.guild_settings': 'Réglages d’appareil',
  'overlay.member_update': 'Rôle modifié',
  'overlay.member_remove': 'Membre exclu',
  'overlay.member_ban': 'Membre banni',
  'overlay.ban_remove': 'Bannissement levé',
  'overlay.device_update': 'Appareil modifié',
  'overlay.device_revoke': 'Appareil révoqué',
  'overlay.device_guild_settings': 'Réglages d’appareil',
  'overlay.device_code': 'Code d’appareil émis',
  'overlay.invite_create': 'Invitation créée',
  'overlay.invite_revoke': 'Invitation révoquée',
  'overlay.pairing_expired_clean': 'Codes expirés purgés',
  'overlay.pairing_revoke': 'Code d’appairage révoqué',
  'overlay.settings': 'Réglages de la guilde modifiés',
  'overlay.board_clear': 'Board vidé',
  'overlay.discord_link_code': 'Code de liaison Discord',
  'overlay.discord_unlink': 'Discord délié',
  'overlay.media_update': 'Média modifié',
  'overlay.media_remove': 'Média supprimé',
  'overlay.media_edit': 'Média réencodé',
  'overlay.media_artwork': 'Image du média modifiée',
  'overlay.queue_remove': 'Média retiré de la file',
  'overlay.queue_clear': 'File vidée',
  'overlay.playback_stop': 'Lecture arrêtée pour tous',
  'ingest_client.create': 'Client d’ingestion créé',
  'ingest_client.revoke': 'Client d’ingestion révoqué',
  'pairing.expired_clean': 'Codes expirés purgés',
  'pairing.revoke': 'Code d’appairage révoqué',
  'media.cache_purge': 'Cache purgé',
  'media.orphans_clean': 'Médias orphelins purgés',
  'settings.update': 'Réglages du serveur modifiés',
  'rights.update': 'Matrice des droits modifiée',
};

function auditActionLabel(action) {
  return AUDIT_ACTION_LABELS[action] || action;
}

// Where the entry comes from. Console sessions use `secret` or `discord`;
// guild admins acting from the board app use the `overlay` actor kind.
function auditOrigin(entry) {
  return entry.actor && entry.actor.method === 'overlay' ? 'Overlay' : 'Panneau';
}

// Path of the mutation relative to the guild, so the column stays short:
// `/admin/api/guilds/42/settings` → `/settings`.
function auditTarget(path) {
  if (!path) {
    return '—';
  }
  const stripped = path.replace(/^\/admin\/api\/guilds\/[^/]+/, '');
  return stripped || '/';
}

function auditDate(entry) {
  const date = new Date(entry.at);
  return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString('fr-FR');
}

function renderAudit() {
  if (state.auditEntries.length === 0) {
    return h('p', { class: 'meta' }, 'Aucune action enregistrée pour cette guilde.');
  }
  return h(
    'div',
    null,
    h('p', { class: 'meta' }, 'Dernières actions d’administration sur cette guilde, depuis le panneau serveur ou l’overlay.'),
    h(
      'div',
      { class: 'list' },
      state.auditEntries.map((entry) =>
        h(
          'div',
          { class: 'line is-audit' },
          h('span', { class: 'sub', title: auditDate(entry) }, relativeTime(entry.at)),
          h('span', { class: 'badge' }, auditOrigin(entry)),
          h('span', { class: 'ellipsis', title: (entry.actor && (entry.actor.name || entry.actor.method)) || '' }, (entry.actor && (entry.actor.name || entry.actor.method)) || '—'),
          h('span', { class: 'ellipsis' }, auditActionLabel(entry.action)),
          h('span', { class: 'sub ellipsis mono', title: entry.path || '' }, auditTarget(entry.path)),
        ),
      ),
    ),
  );
}

// ---------------------------------------------------------------- devices

function deviceRoleBadge(device) {
  const role = device.role || 'member';
  return h('span', { class: `badge is-${role}` }, ROLE_LABELS[role] || role);
}

function renderDevices() {
  if (state.devices.length === 0) {
    return h('p', { class: 'meta' }, 'Chargement des appareils…');
  }
  const newest = newestVersion(state.devices);
  return h(
    'div',
    null,
    h('p', { class: 'meta' }, 'Chaque overlay appairé à cette guilde. « Régler » modifie le son et la pause de cet appareil.'),
    h('div', { class: 'list' }, state.devices.map((device) => renderDevice(device, newest))),
  );
}

function renderDevice(device, newest) {
  const platform = platformLabel(device.platform);
  return h(
    'div',
    { class: `line is-device${device.connected ? '' : ' is-off'}` },
    avatarNode(device.memberName || device.label, null),
    h(
      'div',
      { class: 'device-cell' },
      h('div', { class: 'ellipsis' }, device.label, device.isSelf ? h('span', { class: 'sub' }, '  · cet appareil') : null),
      h('div', { class: 'sub member-meta' }, device.memberName || 'Sans membre', deviceRoleBadge(device)),
    ),
    h(
      'div',
      { class: 'sub device-version' },
      h('span', null, device.overlayVersion || 'version inconnue'),
      isObsolete(device.overlayVersion, newest) ? h('span', { class: 'badge is-obsolete' }, 'Obsolète') : null,
      platform ? h('span', { class: 'device-platform' }, platform) : null,
    ),
    h('span', { class: 'sub seen' }, devicePresenceLabel(device)),
    h('span', { class: 'badge' }, sessionModeLabel(device.sessionMode)),
    h(
      'span',
      { class: 'row device-actions' },
      h('button', { class: 'btn is-quiet', onClick: () => openDeviceSettingsDialog(device) }, 'Régler'),
      h('button', { class: 'btn is-quiet', onClick: () => openRenameDeviceDialog(device) }, 'Renommer'),
      device.isSelf
        ? h('span', { class: 'badge is-off' }, 'Cet appareil')
        : h('button', { class: 'btn is-quiet is-danger', onClick: () => openRevokeDeviceDialog(device) }, 'Révoquer'),
    ),
  );
}

// Same semantics as the server console's device dialog: guild volume (0–100 %
// on top of a 0..1 value), media sound and pause, pushed to the device.
function openDeviceSettingsDialog(device) {
  openDialog((close) => {
    const current = device.guildSettings || { volume: 1, mediaMuted: false, paused: false };
    const initial = Math.round((Number(current.volume) || 0) * 100);
    const volume = h('input', {
      type: 'range',
      min: '0',
      max: '100',
      step: '5',
      value: String(initial),
      'aria-label': 'Volume de la guilde',
    });
    const output = h('output', { class: 'sub' }, `${initial} %`);
    volume.addEventListener('input', () => {
      output.textContent = `${volume.value} %`;
    });
    const muted = h('input', { type: 'checkbox', checked: Boolean(current.mediaMuted) });
    const paused = h('input', { type: 'checkbox', checked: Boolean(current.paused) });
    const save = async () => {
      const body = await run(() =>
        api('PATCH', `/overlay/admin/devices/${encodeURIComponent(device.id)}/guild-settings`, {
          volume: Number(volume.value) / 100,
          mediaMuted: muted.checked,
          paused: paused.checked,
        }),
      );
      if (!body) {
        return;
      }
      notify(body.delivered ? 'Réglages appliqués.' : 'Réglages enregistrés : ils s’appliqueront à sa prochaine connexion.');
      close();
      loadAdmin();
    };
    return [
      h('h2', null, `Régler ${device.label}`),
      h(
        'p',
        null,
        device.connected
          ? 'L’appareil est en ligne : les réglages s’appliquent tout de suite.'
          : 'L’appareil est hors ligne : les réglages s’appliqueront à sa prochaine connexion.',
      ),
      h(
        'div',
        { class: 'stack' },
        h('label', null, 'Volume de la guilde', h('div', { class: 'row device-range' }, volume, output)),
        h('p', { class: 'device-hint' }, 'Multiplié par le volume général de l’appareil.'),
        h('label', { class: 'switch' }, muted, 'Couper le son des médias'),
        h('p', { class: 'device-hint' }, 'Les médias restent affichés, sans leur son.'),
        h('label', { class: 'switch' }, paused, 'Mettre la guilde en pause'),
        h('p', { class: 'device-hint' }, 'Plus aucun média de cette guilde ne s’affiche sur cet appareil.'),
      ),
      h(
        'div',
        { class: 'actions' },
        h('button', { class: 'btn is-quiet', onClick: close }, 'Annuler'),
        h('button', { class: 'btn', onClick: save }, 'Enregistrer'),
      ),
    ];
  });
}

function openRenameDeviceDialog(device) {
  openDialog((close) => {
    const field = h('input', { class: 'field', maxlength: '40', value: device.label, 'aria-label': 'Nom de l’appareil' });
    const save = async () => {
      const label = field.value.trim();
      if (!label) {
        field.focus();
        return;
      }
      const body = await run(() => api('PATCH', `/overlay/admin/devices/${encodeURIComponent(device.id)}`, { label }), 'Appareil renommé.');
      if (!body) {
        return;
      }
      close();
      loadAdmin();
    };
    field.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') {
        event.preventDefault();
        save();
      }
    });
    return [
      h('h2', null, 'Renommer l’appareil'),
      h('p', null, 'Le nom affiché dans la liste des appareils de la guilde (40 caractères au plus).'),
      h('div', { class: 'stack' }, h('label', null, 'Nom de l’appareil', field)),
      h(
        'div',
        { class: 'actions' },
        h('button', { class: 'btn is-quiet', onClick: close }, 'Annuler'),
        h('button', { class: 'btn', onClick: save }, 'Enregistrer'),
      ),
    ];
  });
}

function openRevokeDeviceDialog(device) {
  confirmDialog(
    `Révoquer ${device.label} ?`,
    'L’appareil est déconnecté et son jeton invalidé. Le membre reste dans la guilde et peut appairer un nouvel appareil.',
    'Révoquer l’appareil',
    async () => {
      await run(() => api('DELETE', `/overlay/admin/devices/${encodeURIComponent(device.id)}`), `${device.label} a été révoqué.`);
      loadAdmin();
    },
  );
}

// Expiry of a code, with the same wording for invitations and pairing codes.
function remainingTime(iso) {
  const delay = new Date(iso).getTime() - Date.now();
  if (!Number.isFinite(delay) || delay <= 0) {
    return null;
  }
  const minutes = Math.max(1, Math.round(delay / 60000));
  if (minutes < 60) {
    return `Expire dans ${minutes} min`;
  }
  const hours = Math.round(minutes / 60);
  if (hours < 48) {
    return `Expire dans ${hours} h`;
  }
  return `Expire dans ${Math.round(hours / 24)} j`;
}

function inviteExpiry(invite) {
  if (invite.revokedAt) {
    return 'Révoqué';
  }
  if (!invite.isActive) {
    return invite.maxUses && invite.useCount >= invite.maxUses ? 'Utilisé' : 'Expiré';
  }
  return invite.expiresAt ? remainingTime(invite.expiresAt) || 'Expiré' : 'Sans expiration';
}

function pairingCodeExpiry(code) {
  if (code.usedAt) {
    return 'Utilisé';
  }
  if (!code.isActive) {
    return 'Expiré';
  }
  return code.expiresAt ? remainingTime(code.expiresAt) || 'Expiré' : 'Sans expiration';
}

function inviteUses(invite) {
  return invite.maxUses ? `${invite.useCount} / ${invite.maxUses}` : String(invite.useCount);
}

// Invitations and pairing codes share one list; each row wraps one of them.
function codeValue(row) {
  return row.invite ? row.invite.code : row.code.code;
}

function codeIsActive(row) {
  return Boolean(row.invite ? row.invite.isActive : row.code.isActive);
}

function codeCreatedAt(row) {
  const time = new Date(row.invite ? row.invite.createdAt : row.code.createdAt).getTime();
  return Number.isFinite(time) ? time : 0;
}

function codeRows() {
  const rows = [...state.invites.map((invite) => ({ invite })), ...state.pairingCodes.map((code) => ({ code }))];
  return rows.sort((left, right) => {
    const byDate = codeCreatedAt(right) - codeCreatedAt(left);
    return byDate !== 0 ? byDate : codeValue(left).localeCompare(codeValue(right));
  });
}

function codeBadge(row) {
  if (row.invite) {
    const role = row.invite.role;
    return h('span', { class: `badge is-${role}` }, `Invitation · ${ROLE_LABELS[role] || role}`);
  }
  return h('span', { class: 'badge' }, row.code.kind === 'extension' ? 'Extension' : '/pair-code');
}

function codeAuthor(row) {
  if (!row.invite) {
    return row.code.authorName || '—';
  }
  if (!row.invite.createdByMemberId) {
    return '—';
  }
  const member = state.members.find((entry) => entry.id === row.invite.createdByMemberId);
  return member ? member.displayName : '—';
}

function codeExpiry(row) {
  return row.invite ? inviteExpiry(row.invite) : pairingCodeExpiry(row.code);
}

function renderCodeRow(row) {
  const code = codeValue(row);
  const active = codeIsActive(row);
  return h(
    'div',
    { class: 'line is-code' },
    h('span', { class: 'code' }, formatCode(code)),
    codeBadge(row),
    h('span', { class: 'sub ellipsis code-author' }, codeAuthor(row)),
    h('span', { class: 'sub' }, codeExpiry(row)),
    h('span', { class: 'sub' }, row.invite ? inviteUses(row.invite) : ''),
    h(
      'span',
      { class: 'row' },
      active
        ? h('button', { class: 'btn is-quiet', 'aria-label': `Copier le code ${code}`, title: 'Copier le code', onClick: () => copyText(formatCode(code), 'Code copié.') }, icon('copy'))
        : null,
      active ? h('button', { class: 'btn is-quiet is-danger', onClick: () => openRevokeDialog(row) }, 'Révoquer') : null,
    ),
  );
}

function renderCodes() {
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
  const purge = async () => {
    const body = await run(() => api('DELETE', '/overlay/admin/pairing-codes/expired'));
    if (!body) {
      return;
    }
    const deleted = Number(body.deleted) || 0;
    notify(deleted > 0 ? `${deleted} code${deleted > 1 ? 's' : ''} supprimé${deleted > 1 ? 's' : ''}.` : 'Aucun code expiré à supprimer.');
    loadAdmin();
  };
  const rows = codeRows();
  return h(
    'div',
    null,
    h(
      'div',
      { class: 'panel row is-wrap' },
      role,
      expiry,
      uses,
      h('span', { class: 'spacer' }),
      h('button', { class: 'btn is-quiet', title: 'Supprime les codes d’appairage expirés ou utilisés.', onClick: purge }, 'Purger les codes expirés'),
      h('button', { class: 'btn', onClick: create }, icon('plus'), 'Créer un code'),
    ),
    rows.length === 0
      ? h('p', { class: 'meta' }, 'Aucun code pour le moment.')
      : h('div', { class: 'list' }, rows.map(renderCodeRow)),
  );
}

function openRevokeDialog(row) {
  const code = codeValue(row);
  confirmDialog(`Révoquer le code ${formatCode(code)} ?`, 'Il ne pourra plus être utilisé. Cette action est immédiate.', 'Révoquer', () => revokeCode(row));
}

async function revokeCode(row) {
  const code = codeValue(row);
  const path = row.invite
    ? `/overlay/admin/invites/${encodeURIComponent(code)}/revoke`
    : `/overlay/admin/pairing-codes/${encodeURIComponent(code)}/revoke`;
  const body = await run(() => api('POST', path));
  if (!body) {
    return;
  }
  if (body.revoked === false) {
    notify('Ce code n’était plus actif.', true);
  } else {
    notify(row.invite ? 'Invitation révoquée.' : 'Code révoqué.');
  }
  loadAdmin();
}

// Guild-wide settings (server side), not the device settings page.
// Server settings of the guild (admins), shown in its « Réglages » tab.
// Slash commands registered by the bot, copied from the server console
// (`apps/lc-admin-web/src/constants.ts`).
const DISCORD_COMMANDS = [
  'dispo',
  'msg',
  'dire',
  'pair-code',
  'help',
  'info',
  'overlays',
  'stop',
  'media-add',
  'lier',
  'cmsg',
  'cdire',
];

export function renderServerSettings() {
  const guild = (state.info && state.info.guild) || { settings: {} };
  const effective = (guild.settings && guild.settings.effective) || {};
  const name = h('input', { class: 'field', id: 'set-name', value: guild.name || '', maxlength: '60' });
  const defaultTime = h('input', { class: 'field', id: 'set-default', type: 'number', min: '0', max: '86400', value: guild.settings.defaultMediaTime ?? '', placeholder: `Valeur du serveur (${effective.defaultMediaTime ?? '—'} s)`, style: 'max-width:160px' });
  const maxTime = h('input', { class: 'field', id: 'set-max', type: 'number', min: '0', max: '86400', value: guild.settings.maxMediaTime ?? '', placeholder: effective.maxMediaTime != null ? `Valeur du serveur (${effective.maxMediaTime} s)` : 'Sans limite', style: 'max-width:160px' });
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
    h(
      'p',
      { class: 'meta' },
      `Commandes du bot : ${DISCORD_COMMANDS.map((command) => `/${command}`).join(', ')} (${DISCORD_COMMANDS.length} synchronisées).`,
    ),
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
