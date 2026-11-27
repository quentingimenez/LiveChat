// Administration tab: members, devices, bans and codes (invitations and
// pairing codes). The server settings of the guild (`renderServerSettings`)
// are shown in its « Réglages » tab.

import { api, run } from '../core/api.js';
import { isObsolete, newestVersion, platformLabel, presenceSinceLabel as devicePresenceLabel, sessionModeLabel } from '../core/device-helpers.js';
import { avatarNode, clearSettingsDirty, h, icon, resetOnDoubleClick } from '../core/dom.js';
import { formatCode, relativeTime } from '../core/format.js';
import { bindRangeFill } from '../core/range-fill.js';
import { activeGuild, can, ROLE_LABELS, state } from '../core/state.js';
import { DEFAULT_VOLUME_PERCENT } from '../core/sound.js';
import { notify } from '../core/toast.js';
import { confirmDialog, copyText, openDialog } from '../core/ui.js';
import { loadAdmin, loadGuild, loadGuildInfo } from '../data.js';
import { showSettingsRemoteUpdate } from './settings-banner.js';
import { renderView } from './view.js';

// ------------------------------------------------------------- list states

// A failed read keeps the list already shown (`null || previous` in data.js)
// and a successful one installs a new array. Watching those identities is how
// this view tells a failed load from an empty list, without a loading flag in
// the shared state.
const LIST_REFS = {
  members: () => [state.members],
  devices: () => [state.devices],
  codes: () => [state.invites, state.pairingCodes],
  bans: () => [state.bans],
  audit: () => [state.auditEntries],
};

// Tabs whose list has loaded at least once: their counter is unknown before
// that, because the server reads bans and journal entries lazily.
const loadedTabs = new Set();
let watchedGuildId = null;
let loadWatch = { tab: null, refs: [], renders: 0, pending: false, failed: false };

function refsChanged(next, previous) {
  return next.some((value, index) => value !== previous[index]);
}

// The first render after a tab change starts the wait; `loadAdmin` renders
// once more when its reads are done. If the arrays did not move, nothing
// loaded.
function observeLoad(tab) {
  const refsOf = LIST_REFS[tab];
  if (!refsOf) {
    return { loading: false, failed: false };
  }
  if (loadWatch.tab !== tab) {
    loadWatch = { tab, refs: refsOf(), renders: 0, pending: true, failed: false };
  }
  loadWatch.renders += 1;
  const next = refsOf();
  if (loadWatch.pending && loadWatch.renders >= 2) {
    loadWatch.failed = next.some((value, index) => value === null || value === loadWatch.refs[index]);
    loadWatch.refs = next;
    loadWatch.pending = false;
    if (!loadWatch.failed) {
      loadedTabs.add(tab);
    }
  } else if (!loadWatch.pending && refsChanged(next, loadWatch.refs)) {
    // A later successful read (retry, background refresh) replaces the arrays:
    // it also clears an error left by an earlier failed attempt.
    loadWatch.refs = next;
    loadWatch.failed = false;
    loadedTabs.add(tab);
  }
  return { loading: loadWatch.pending, failed: loadWatch.failed };
}

// The inline state already says the read failed: keep the retry quiet so the
// same error is not toasted on top of it.
function retryList(tab) {
  loadWatch = { tab, refs: LIST_REFS[tab](), renders: 0, pending: true, failed: false };
  renderView();
  loadAdmin({ quiet: true });
}

export function renderAdmin() {
  const guild = activeGuild();
  const guildId = guild ? guild.membership.guildId : null;
  if (guildId !== watchedGuildId) {
    watchedGuildId = guildId;
    loadedTabs.clear();
    loadWatch = { tab: null, refs: [], renders: 0, pending: false, failed: false };
  }

  // Members, devices and active codes are read together whenever the tab
  // opens; bans and journal entries only once their tab is selected.
  const definitions = [
    ['members', 'Membres', () => state.members.length, 'manageMembers'],
    ['devices', 'Appareils', () => state.devices.length, 'manageMembers'],
    ['codes', 'Codes', activeCodeCount, 'manageInvites'],
    ['bans', 'Bannis', () => (loadedTabs.has('bans') ? state.bans.length : null), 'manageMembers'],
    ['audit', 'Journal', () => (loadedTabs.has('audit') ? state.auditEntries.length : null), 'manageSettings'],
  ];
  const available = definitions.filter(([, , , permission]) => can(permission));
  if (!available.some(([id]) => id === state.adminTab)) {
    state.adminTab = available.length ? available[0][0] : 'members';
  }
  const status = observeLoad(state.adminTab);
  const views = { codes: renderCodes, bans: renderBans, devices: renderDevices, audit: renderAudit };
  const body = (views[state.adminTab] || renderMembers)(status);
  return h(
    'div',
    null,
    h(
      'div',
      { class: 'subtabs', role: 'group', 'aria-label': 'Sections d’administration' },
      available.map(([id, label, count]) => {
        const value = count();
        return h(
          'button',
          {
            class: 'chip',
            type: 'button',
            'aria-pressed': state.adminTab === id ? 'true' : 'false',
            onClick: () => { state.adminTab = id; renderView(); loadAdmin(); },
          },
          label,
          value !== null ? h('span', { class: 'count' }, value) : null,
        );
      }),
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

// One list: an optional lead panel, then the column head and the rows, or one
// explicit state (loading, error, empty), then an optional trailing node (used
// by the journal filters to report an empty result under the list). The tab
// chips and the columns say what the list holds, so no sentence introduces it.
function renderList({ kind, tab, status, count, labels, rows, empty, lead = null, after = null }) {
  const phase = count > 0 ? 'ready' : status.failed ? 'error' : status.loading ? 'loading' : 'empty';
  return h(
    'div',
    null,
    lead,
    phase === 'ready' ? listHead(kind, labels) : null,
    phase === 'ready' ? h('div', { class: 'list' }, rows) : listState(phase, tab, empty),
    after,
  );
}

// Column labels use the same grid tracks as the rows. They are a visual aid:
// each cell keeps its own text for assistive technologies.
function listHead(kind, labels) {
  return h(
    'div',
    { class: `list-head is-${kind}`, 'aria-hidden': 'true' },
    labels.map((label) => h('span', null, label)),
  );
}

function listState(phase, tab, empty) {
  if (phase === 'loading') {
    return h(
      'div',
      { class: 'list-state is-loading', role: 'status', 'aria-busy': 'true' },
      h('span', { class: 'sr-only' }, 'Chargement…'),
      Array.from({ length: 4 }, () =>
        h('div', { class: 'line is-skeleton' }, h('span', { class: 'skeleton-block' })),
      ),
    );
  }
  if (phase === 'error') {
    return h(
      'div',
      { class: 'list-state is-error', role: 'alert' },
      h('p', null, 'Impossible de charger cette liste.'),
      h('button', { class: 'btn is-quiet', type: 'button', onClick: () => retryList(tab) }, 'Réessayer'),
    );
  }
  return h('p', { class: 'list-state is-empty' }, empty);
}

// A cell keeps the label of its column: the desktop head names the track, and
// below 1120px the same label is shown above the value when the row folds into
// a card. The wrapper is transparent on desktop, so the value stays in the
// shared grid.
function listCell(label, ...children) {
  return h('div', { class: 'list-cell', 'data-label': label }, ...children);
}

// A muted overlay closes its socket, so the server sees it offline. For this
// device the reason is known: say so instead of a last-seen time.
function presenceLabel(member, selfMuted) {
  if (member.connected) {
    return 'En ligne';
  }
  if (member.isSelf && selfMuted) {
    return 'Guilde coupée ici';
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

function absoluteDate(iso) {
  if (!iso) {
    return '—';
  }
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString('fr-FR');
}

function memberMeta(member) {
  return h(
    'div',
    { class: 'sub member-meta' },
    `${memberDevices(member)} · arrivé le ${joinDate(member.createdAt)}`,
    member.discordLinked
      ? h(
          'span',
          { class: 'badge', title: member.discordUserId ? `Compte Discord lié (${member.discordUserId})` : 'Compte Discord lié' },
          'Discord lié',
        )
      : null,
  );
}

// ---------------------------------------------------------------- members

function renderMembers(status) {
  const guild = activeGuild();
  const selfMuted = Boolean(guild && guild.membership.muted);
  const rows = state.members.map((member) =>
    h(
      'div',
      { class: 'line is-member' },
      h(
        'div',
        { class: 'line-main' },
        avatarNode(member.displayName, member.avatar),
        listCell(
          'Membre',
          h(
            'div',
            { class: 'member-cell' },
            h('div', { class: 'ellipsis' }, member.displayName, member.isSelf ? h('span', { class: 'sub' }, '\u00a0(toi)') : null),
            memberMeta(member),
          ),
        ),
        // The role control sits on the identity line in the card layout: on
        // the meta line it read as a second, oversized button under the name.
        // On desktop its select is placed back in the Rôle track by CSS.
        h(
          'div',
          { class: 'list-cell member-role', 'data-label': 'Rôle' },
          h(
            'select',
            {
              class: 'field member-role-select',
              'aria-label': `Rôle de ${member.displayName}`,
              onChange: (event) => onRoleChange(member, event),
            },
            Object.entries(ROLE_LABELS).map(([id, label]) => h('option', { value: id, selected: member.role === id }, label)),
          ),
        ),
      ),
      h(
        'div',
        { class: 'line-meta' },
        listCell('Vu', h('span', { class: 'sub seen' }, presenceLabel(member, selfMuted))),
      ),
      h(
        'div',
        { class: 'line-actions', 'data-label': 'Action' },
        h(
          'button',
          { class: 'btn is-quiet', type: 'button', title: 'Créer un code pour rattacher un appareil à ce membre.', onClick: () => openDeviceCodeDialog(member) },
          'Code d’appairage…',
        ),
        member.isSelf
          ? null
          : h(
              'div',
              { class: 'line-danger' },
              h('button', { class: 'btn is-quiet is-danger', type: 'button', onClick: () => openKickDialog(member) }, 'Exclure…'),
              h('button', { class: 'btn is-quiet is-danger', type: 'button', onClick: () => openBanDialog(member) }, 'Bannir…'),
            ),
      ),
    ),
  );
  return renderList({
    kind: 'member',
    tab: 'members',
    status,
    count: state.members.length,
    labels: ['', 'Membre', 'Vu', 'Rôle', 'Action'],
    rows,
    empty: 'Aucun membre dans cette guilde pour le moment.',
  });
}

// A role change on one's own account is not blocked (the server protects the
// last admin), but it asks first. The row keeps its current role while the
// dialog is open, so cancelling leaves the screen truthful.
function onRoleChange(member, event) {
  const select = event.target;
  const role = select.value;
  if (role === member.role) {
    return;
  }
  if (!member.isSelf) {
    changeRole(member, role);
    return;
  }
  select.value = member.role;
  const current = ROLE_LABELS[member.role] || member.role;
  const next = ROLE_LABELS[role] || role;
  confirmDialog(
    'Changer ton propre rôle\u00a0?',
    `Tu vas passer de «\u00a0${current}\u00a0» à «\u00a0${next}\u00a0». Si tu n’es plus admin, tu perdras l’accès à cette administration.`,
    'Changer mon rôle',
    () => changeRole(member, role),
    { tone: 'neutral' },
  );
}

async function changeRole(member, role) {
  const label = ROLE_LABELS[role] ? ROLE_LABELS[role].toLowerCase() : role;
  await run(() => api('PATCH', `/overlay/admin/members/${encodeURIComponent(member.id)}`, { role }), `${member.displayName} est maintenant ${label}.`);
  await loadAdmin();
  if (member.isSelf) {
    loadGuild();
  }
}

function openKickDialog(member) {
  confirmDialog(`Exclure ${member.displayName}\u00a0?`, 'Ses overlays sont déconnectés tout de suite. La personne devra saisir un nouveau code pour revenir.', 'Exclure', async () => {
    await run(() => api('DELETE', `/overlay/admin/members/${encodeURIComponent(member.id)}`), `${member.displayName} a été exclu.`);
    loadAdmin();
  });
}

function openBanDialog(member) {
  const text = member.discordLinked
    ? 'Ses overlays et ses extensions sont révoqués tout de suite. Son compte Discord ne pourra plus obtenir de code ni utiliser le bot sur cette guilde.'
    : 'Ses overlays et ses extensions sont révoqués tout de suite. Sans compte Discord lié, rien ne l’empêche de revenir avec une nouvelle invitation\u00a0: ne lui en donne pas.';
  confirmDialog(`Bannir ${member.displayName}\u00a0?`, text, 'Bannir', async () => {
    await run(() => api('POST', `/overlay/admin/members/${encodeURIComponent(member.id)}/ban`), `${member.displayName} a été banni.`);
    loadAdmin();
  });
}

// Pairing code for another member: a single-use code that attaches a new
// overlay to that member's account. The admin reads it out to them.
async function openDeviceCodeDialog(member) {
  const body = await run(() => api('POST', '/overlay/admin/device-codes', { memberId: member.id }));
  if (!body || !body.invite) {
    return;
  }
  const code = formatCode(body.invite.code);
  openDialog((close) => [
    h('h2', null, 'Code d’appairage'),
    h('p', null, `Pour ${member.displayName}\u00a0: à saisir dans l’overlay pour rattacher un nouvel appareil à son compte.`),
    h(
      'div',
      { class: 'row is-wrap' },
      h('span', { class: 'code' }, code),
      h('button', { class: 'btn', type: 'button', onClick: () => copyText(code, 'Code copié.') }, icon('copy'), 'Copier'),
    ),
    h('p', { class: 'meta' }, 'Expire dans 10\u00a0min. Une seule utilisation.'),
    h('div', { class: 'actions' }, h('button', { class: 'btn', type: 'button', onClick: close }, 'Fermer')),
  ]);
}

// ------------------------------------------------------------------- bans

function renderBans(status) {
  const rows = state.bans.map((ban) =>
    h(
      'div',
      { class: 'line is-ban' },
      h(
        'div',
        { class: 'line-main' },
        avatarNode(ban.displayName, ban.avatar),
        listCell(
          'Banni',
          h('div', { class: 'ellipsis' }, ban.displayName, ban.discordUserId ? null : h('span', { class: 'sub' }, '\u00a0(sans compte Discord)')),
        ),
      ),
      h(
        'div',
        { class: 'line-meta' },
        listCell('Date', h('span', { class: 'sub', title: absoluteDate(ban.createdAt) }, relativeTime(ban.createdAt))),
        listCell('Banni par', h('span', { class: 'sub ellipsis', title: ban.bannedBy || '' }, ban.bannedBy || '—')),
      ),
      h(
        'div',
        { class: 'line-actions', 'data-label': 'Action' },
        h('button', { class: 'btn is-quiet', type: 'button', onClick: () => openUnbanDialog(ban) }, 'Lever le bannissement'),
      ),
    ),
  );
  return renderList({
    kind: 'ban',
    tab: 'bans',
    status,
    count: state.bans.length,
    labels: ['', 'Banni', 'Date', 'Banni par', 'Action'],
    rows,
    empty: 'Personne n’est banni de cette guilde.',
  });
}

function openUnbanDialog(ban) {
  confirmDialog(
    `Lever le bannissement de ${ban.displayName}\u00a0?`,
    'La personne pourra revenir avec un nouveau code. Ses anciens appareils restent révoqués.',
    'Lever le bannissement',
    async () => {
      await run(() => api('DELETE', `/overlay/admin/bans/${encodeURIComponent(ban.id)}`), `Le bannissement de ${ban.displayName} est levé.`);
      loadAdmin();
    },
    { tone: 'neutral' },
  );
}

// ---------------------------------------------------------------- devices

function deviceRoleBadge(device) {
  const role = device.role || 'member';
  return h('span', { class: `badge is-${role}` }, ROLE_LABELS[role] || role);
}

function renderDevices(status) {
  const newest = newestVersion(state.devices);
  const rows = state.devices.map((device) => renderDevice(device, newest));
  return renderList({
    kind: 'device',
    tab: 'devices',
    status,
    count: state.devices.length,
    labels: ['', 'Appareil', 'Version', 'Vu', 'Session', 'Action'],
    rows,
    empty: 'Aucun appareil appairé pour le moment.',
  });
}

function renderDevice(device, newest) {
  const platform = platformLabel(device.platform);
  return h(
    'div',
    { class: `line is-device${device.connected ? '' : ' is-off'}` },
    h(
      'div',
      { class: 'line-main' },
      avatarNode(device.memberName || device.label, null),
      listCell(
        'Appareil',
        h(
          'div',
          { class: 'device-cell' },
          h('div', { class: 'ellipsis' }, device.label),
          h('div', { class: 'sub member-meta' }, device.memberName || 'Sans membre', deviceRoleBadge(device)),
        ),
      ),
    ),
    h(
      'div',
      { class: 'line-meta' },
      listCell(
        'Version',
        h(
          'div',
          { class: 'sub device-version' },
          h('span', null, device.overlayVersion || 'version inconnue'),
          isObsolete(device.overlayVersion, newest) ? h('span', { class: 'badge is-obsolete' }, 'Obsolète') : null,
          platform ? h('span', { class: 'device-platform' }, platform) : null,
        ),
      ),
      listCell('Vu', h('span', { class: 'sub seen' }, devicePresenceLabel(device))),
      listCell('Session', h('span', { class: 'badge' }, sessionModeLabel(device.sessionMode))),
    ),
    h(
      'div',
      { class: 'line-actions', 'data-label': 'Action' },
      h('button', { class: 'btn is-quiet', type: 'button', onClick: () => openDeviceSettingsDialog(device) }, 'Régler'),
      h('button', { class: 'btn is-quiet', type: 'button', onClick: () => openRenameDeviceDialog(device) }, 'Renommer…'),
      device.isSelf
        ? h('span', { class: 'badge is-off' }, 'Cet appareil')
        : h('button', { class: 'btn is-quiet is-danger', type: 'button', onClick: () => openRevokeDeviceDialog(device) }, 'Révoquer…'),
    ),
  );
}

// Same semantics as the server console's device dialog: guild volume (0–100 %
// on top of a 0..1 value), media sound and local cut, pushed to the device.
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
    resetOnDoubleClick(volume, DEFAULT_VOLUME_PERCENT);
    bindRangeFill(volume);
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
      notify(body.delivered ? 'Réglages appliqués.' : 'Réglages enregistrés\u00a0: ils s’appliqueront à sa prochaine connexion.');
      close();
      loadAdmin();
    };
    return [
      h('h2', null, `Régler ${device.label}`),
      h(
        'p',
        null,
        device.connected
          ? 'L’appareil est en ligne\u00a0: les réglages s’appliquent tout de suite.'
          : 'L’appareil est hors ligne\u00a0: les réglages s’appliqueront à sa prochaine connexion.',
      ),
      h(
        'div',
        { class: 'stack' },
        h('label', null, 'Volume de la guilde', h('div', { class: 'row device-range' }, volume, output)),
        h('p', { class: 'device-hint' }, 'Multiplié par le volume général de l’appareil.'),
        h('label', { class: 'switch' }, muted, 'Couper le son des médias'),
        h('label', { class: 'switch' }, paused, 'Couper cette guilde sur cet appareil'),
      ),
      h(
        'div',
        { class: 'actions' },
        h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'),
        h('button', { class: 'btn', type: 'button', onClick: save }, 'Enregistrer'),
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
      h('div', { class: 'stack' }, h('label', null, 'Nom de l’appareil', field)),
      h(
        'div',
        { class: 'actions' },
        h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'),
        h('button', { class: 'btn', type: 'button', onClick: save }, 'Enregistrer'),
      ),
    ];
  });
}

function openRevokeDeviceDialog(device) {
  confirmDialog(
    `Révoquer ${device.label}\u00a0?`,
    'L’appareil est déconnecté et son jeton invalidé. Le membre reste dans la guilde et peut appairer un nouvel appareil.',
    'Révoquer l’appareil',
    async () => {
      await run(() => api('DELETE', `/overlay/admin/devices/${encodeURIComponent(device.id)}`), `${device.label} a été révoqué.`);
      loadAdmin();
    },
  );
}

// ------------------------------------------------------------------- codes

// Expiry of a code, with the same wording for invitations and pairing codes.
function remainingTime(iso) {
  const delay = new Date(iso).getTime() - Date.now();
  if (!Number.isFinite(delay) || delay <= 0) {
    return null;
  }
  const minutes = Math.max(1, Math.round(delay / 60000));
  if (minutes < 60) {
    return `Expire dans ${minutes}\u00a0min`;
  }
  const hours = Math.round(minutes / 60);
  if (hours < 48) {
    return `Expire dans ${hours}\u00a0h`;
  }
  const days = Math.round(hours / 24);
  return `Expire dans ${days}\u00a0jour${days > 1 ? 's' : ''}`;
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
  const origin = row.code.kind === 'extension' ? 'Créé par une extension.' : 'Créé par la commande /pair-code.';
  return h('span', { class: 'badge', title: origin }, 'Appairage');
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
    h(
      'div',
      { class: 'line-main' },
      listCell('Code', h('span', { class: 'code' }, formatCode(code))),
      listCell('Type', codeBadge(row)),
    ),
    h(
      'div',
      { class: 'line-meta' },
      listCell('Auteur', h('span', { class: 'sub ellipsis code-author' }, codeAuthor(row))),
      listCell('Expiration', h('span', { class: 'sub' }, codeExpiry(row))),
      listCell('Utilisations', h('span', { class: 'sub' }, row.invite ? inviteUses(row.invite) : '—')),
    ),
    h(
      'div',
      { class: 'line-actions', 'data-label': 'Action' },
      active
        ? h(
            'button',
            {
              class: 'btn is-quiet',
              type: 'button',
              'aria-label': `Copier le code ${code}`,
              title: 'Copier le code',
              onClick: () => copyText(formatCode(code), 'Code copié.'),
            },
            icon('copy'),
          )
        : null,
      active ? h('button', { class: 'btn is-quiet is-danger', type: 'button', onClick: () => openRevokeDialog(row) }, 'Révoquer…') : null,
    ),
  );
}

// A labelled select in the creation panel.
function codeField(label, control) {
  return h('label', { class: 'code-field' }, h('span', { class: 'code-field-label' }, label), control);
}

function renderCodes(status) {
  const role = h('select', { class: 'field' }, [
    h('option', { value: 'member' }, 'Membre'),
    h('option', { value: 'guest' }, 'Invité'),
    h('option', { value: 'admin' }, 'Admin'),
  ]);
  const expiry = h('select', { class: 'field' }, [
    h('option', { value: '24' }, 'Expire dans 24\u00a0h'),
    h('option', { value: '168' }, 'Expire dans 7\u00a0jours'),
    h('option', { value: '' }, 'N’expire jamais'),
  ]);
  const uses = h('select', { class: 'field' }, [
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
  const purge = () => {
    confirmDialog(
      'Supprimer les codes expirés\u00a0?',
      'Les codes d’appairage expirés ou déjà utilisés sont supprimés définitivement. Les invitations ne sont pas concernées.',
      'Supprimer',
      async () => {
        const body = await run(() => api('DELETE', '/overlay/admin/pairing-codes/expired'));
        if (!body) {
          return;
        }
        const deleted = Number(body.deleted) || 0;
        notify(deleted > 0 ? `${deleted} code${deleted > 1 ? 's' : ''} supprimé${deleted > 1 ? 's' : ''}.` : 'Aucun code expiré à supprimer.');
        loadAdmin();
      },
    );
  };
  const rows = codeRows();
  const list = renderList({
    kind: 'code',
    tab: 'codes',
    status,
    count: rows.length,
    labels: ['Code', 'Type', 'Auteur', 'Expiration', 'Utilisations', 'Action'],
    rows: rows.map(renderCodeRow),
    empty: 'Aucun code pour le moment.',
    lead: h(
      'div',
      { class: 'panel code-create' },
      h(
        'div',
        { class: 'code-fields' },
        codeField('Rôle accordé', role),
        codeField('Expiration', expiry),
        codeField('Utilisations', uses),
      ),
      h(
        'div',
        { class: 'code-actions' },
        h('button', { class: 'btn is-quiet', type: 'button', onClick: purge }, 'Supprimer les codes expirés…'),
        h('button', { class: 'btn is-primary', type: 'button', onClick: create }, icon('plus'), 'Créer un code'),
      ),
    ),
  });
  scheduleCodeColumnMeasure(list);
  return list;
}

// The code fields must start at the same x on every row (Auteur, Expiration,
// Utilisations), while the two gutters between them stay equal and shrink
// together as the card narrows. Their widest text is not known in CSS before
// the rows exist, so measure it once after the render and expose the three
// widths to the grid declared in admin.css.
function scheduleCodeColumnMeasure(root) {
  if (typeof requestAnimationFrame !== 'function') {
    return;
  }
  requestAnimationFrame(() => {
    if (root.isConnected) {
      measureCodeColumns(root);
    }
  });
}

function measureCodeColumns(root) {
  const list = root.querySelector('.list');
  if (!list) {
    return;
  }
  const columns = [[], [], []];
  for (const meta of list.querySelectorAll('.line.is-code .line-meta')) {
    const cells = Array.from(meta.children).filter((child) => child.classList.contains('list-cell'));
    if (cells.length !== 3) {
      continue;
    }
    cells.forEach((cell, index) => columns[index].push(cell));
  }
  if (!columns[0].length) {
    return;
  }
  const widths = columns.map((cells) => Math.max(...cells.map(measureCodeCell)));
  if (!widths.every((width) => width > 0)) {
    return;
  }
  list.classList.add('has-code-columns');
  list.style.setProperty('--code-author-width', `${Math.ceil(widths[0]) + 1}px`);
  list.style.setProperty('--code-expiry-width', `${Math.ceil(widths[1]) + 1}px`);
  list.style.setProperty('--code-uses-width', `${Math.ceil(widths[2]) + 1}px`);
}

// The probe is a clone of the cell with the card layout forced by CSS, appended
// out of flow inside the same row: it measures the natural width of the label
// and value without depending on the current viewport or on the row's flex
// shrinking. It is removed immediately after the read.
function measureCodeCell(cell) {
  const probe = cell.cloneNode(true);
  probe.classList.add('code-cell-probe');
  cell.parentElement.append(probe);
  const width = probe.getBoundingClientRect().width;
  probe.remove();
  return width;
}

function openRevokeDialog(row) {
  const code = codeValue(row);
  confirmDialog(`Révoquer le code ${formatCode(code)}\u00a0?`, 'Il ne pourra plus être utilisé. Cette action est immédiate.', 'Révoquer', () => revokeCode(row));
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

// ----------------------------------------------------------------- journal

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
  'guild.device_code': 'Code d’appairage émis',
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
  'overlay.device_code': 'Code d’appairage émis',
  'overlay.invite_create': 'Invitation créée',
  'overlay.invite_revoke': 'Invitation révoquée',
  'overlay.pairing_expired_clean': 'Codes expirés supprimés',
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
  'pairing.expired_clean': 'Codes expirés supprimés',
  'pairing.revoke': 'Code d’appairage révoqué',
  'media.cache_purge': 'Cache supprimé',
  'media.orphans_clean': 'Médias orphelins supprimés',
  'settings.update': 'Réglages du serveur modifiés',
  'rights.update': 'Matrice des droits modifiée',
};

function auditActionLabel(action) {
  return AUDIT_ACTION_LABELS[action] || action;
}

// Where the entry comes from. Console sessions use `secret` or `discord`;
// guild admins acting from the board app use the `overlay` actor kind.
function auditOrigin(entry) {
  return entry.actor && entry.actor.method === 'overlay' ? 'Overlay' : 'Panneau serveur';
}

function auditActor(entry) {
  if (!entry.actor) {
    return '—';
  }
  // The owner console authenticates with the shared secret and carries no
  // member name: name it in French instead of showing the technical value.
  if (entry.actor.method === 'secret') {
    return 'Propriétaire';
  }
  return entry.actor.name || entry.actor.method || '—';
}

// The text query reads the displayed values, accent- and case-insensitively:
// « proprietaire » finds « Propriétaire ». Combining marks are dropped after
// the NFD decomposition, so the comparison works on plain letters.
function auditFilterText(value) {
  return String(value ?? '')
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .trim();
}

// One searchable string per row. The raw ISO timestamp is kept next to the
// displayed date, so a query can use either form.
function auditSearchText(...parts) {
  return auditFilterText(
    parts.filter((part) => part !== null && part !== undefined && part !== '').join(' '),
  );
}

function matchesAuditFilters(entry, query, origin, action) {
  const needle = auditFilterText(query);
  return (
    (!needle || entry.search.includes(needle)) &&
    (!origin || entry.origin === origin) &&
    (!action || entry.action === action)
  );
}

// Hide rows in place from the dataset written when they were built: no DOM is
// rebuilt, so the search field keeps its focus and the caret while typing.
function filterAuditRows(list, empty, counter, controls) {
  const total = list.children.length;
  let visible = 0;
  for (const row of list.children) {
    const match = matchesAuditFilters(
      {
        search: row.dataset.auditSearch || '',
        origin: row.dataset.auditOrigin || '',
        action: row.dataset.auditAction || '',
      },
      controls.query.value,
      controls.origin.value,
      controls.action.value,
    );
    row.hidden = !match;
    if (match) {
      visible += 1;
    }
  }
  empty.hidden = visible > 0;
  counter.textContent =
    visible === total
      ? `${visible} action${visible > 1 ? 's' : ''}`
      : `${visible} sur ${total} action${total > 1 ? 's' : ''}`;
}

function resetAuditFilters(controls, refresh) {
  controls.query.value = '';
  controls.origin.value = '';
  controls.action.value = '';
  refresh();
  controls.query.focus();
}

function auditFilterControls(actions, total, onReset) {
  const query = h('input', {
    class: 'field audit-search',
    type: 'search',
    placeholder: 'Rechercher (date, origine, auteur, action)',
    'aria-label': 'Rechercher dans le journal',
  });
  const origin = h(
    'select',
    { class: 'field', 'aria-label': 'Origine' },
    h('option', { value: '' }, 'Toutes les origines'),
    h('option', { value: 'Overlay' }, 'Overlay'),
    h('option', { value: 'Panneau serveur' }, 'Panneau serveur'),
  );
  const action = h(
    'select',
    { class: 'field', 'aria-label': 'Action' },
    h('option', { value: '' }, 'Toutes les actions'),
    actions.map((id) => h('option', { value: id }, auditActionLabel(id))),
  );
  const counter = h(
    'span',
    { class: 'sub audit-count', role: 'status', 'aria-live': 'polite' },
    `${total} action${total > 1 ? 's' : ''}`,
  );
  // Rendered once with the list and only toggled by the filters: it is not the
  // standard empty state, which would hide the list title and the filters.
  const empty = h(
    'p',
    { class: 'list-state is-empty audit-empty', role: 'status', hidden: true },
    'Aucun résultat pour ces filtres.',
  );
  const node = h(
    'div',
    { class: 'panel audit-filters' },
    query,
    origin,
    action,
    h('button', { class: 'btn is-quiet', type: 'button', onClick: onReset }, 'Réinitialiser'),
    counter,
  );
  return { node, query, origin, action, counter, empty };
}

function renderAudit(status) {
  const rows = state.auditEntries.map((entry) => {
    const actor = auditActor(entry);
    const origin = auditOrigin(entry);
    const actionLabel = auditActionLabel(entry.action);
    const date = absoluteDate(entry.at);
    const row = h(
      'div',
      { class: 'line is-audit' },
      h(
        'div',
        { class: 'line-main' },
        listCell('Date', h('time', { class: 'sub journal-date', datetime: entry.at || null }, date)),
        listCell('Origine', h('span', { class: 'badge' }, origin)),
      ),
      h(
        'div',
        { class: 'line-meta' },
        listCell('Auteur', h('span', { class: 'ellipsis', title: actor === '—' ? '' : actor }, actor)),
        listCell('Action', h('span', { class: 'ellipsis' }, actionLabel)),
      ),
    );
    // Index consumed by the filters. The action id is kept next to its label so
    // the action select can match the same rows the label displays.
    row.dataset.auditSearch = auditSearchText(date, entry.at, origin, actor, actionLabel, entry.action);
    row.dataset.auditOrigin = origin;
    row.dataset.auditAction = entry.action;
    return row;
  });
  const actions = [...new Set(state.auditEntries.map((entry) => entry.action).filter(Boolean))].sort(
    (left, right) => auditActionLabel(left).localeCompare(auditActionLabel(right), 'fr'),
  );
  let apply = () => {};
  const filters = auditFilterControls(actions, rows.length, () => resetAuditFilters(filters, apply));
  const card = renderList({
    kind: 'audit',
    tab: 'audit',
    status,
    count: state.auditEntries.length,
    labels: ['Date', 'Origine', 'Auteur', 'Action'],
    rows,
    empty: 'Aucune action d’administration enregistrée pour cette guilde.',
    lead: rows.length > 0 ? filters.node : null,
    after: rows.length > 0 ? filters.empty : null,
  });
  if (rows.length > 0) {
    const list = card.querySelector('.list');
    apply = () => filterAuditRows(list, filters.empty, filters.counter, filters);
    filters.query.addEventListener('input', apply);
    filters.origin.addEventListener('change', apply);
    filters.action.addEventListener('change', apply);
    apply();
  }
  return card;
}

// ---------------------------------------------------------- server settings

// Server settings of the guild (admins), rendered in its « Réglages » tab.
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

// The board sends the version it read with every settings write. The server
// refuses a write based on a version that another device already replaced
// (409 `settings_conflict`), instead of silently overwriting it.
function withSettingsVersion(payload) {
  const version = state.info && state.info.guild && state.info.guild.settingsVersion;
  return version ? { ...payload, expectedSettingsVersion: version } : payload;
}

export async function saveGuildSettings(payload, success) {
  try {
    await api('PATCH', '/overlay/admin/settings', withSettingsVersion(payload));
    clearSettingsDirty();
    notify(success);
    await loadGuild();
    renderView();
    return true;
  } catch (error) {
    if (error.code === 'settings_conflict') {
      // Do not reload yet: the typed values are still in the form, and the
      // user must see why the save failed. Focusing the banner makes the
      // reason visible even if the button was clicked from below the fold.
      showSettingsRemoteUpdate({
        focus: true,
        conflict: true,
        onReload: () => loadGuildInfo({ quiet: true, force: true, allowDirty: true }),
      });
      return false;
    }
    notify(error.message, true);
    return false;
  }
}

export function renderServerSettings() {
  const guild = (state.info && state.info.guild) || { settings: {} };
  const effective = (guild.settings && guild.settings.effective) || {};
  const name = h('input', { class: 'field', id: 'set-name', value: guild.name || '', maxlength: '60' });
  const defaultTime = h('input', {
    class: 'field',
    id: 'set-default',
    type: 'number',
    min: '0',
    max: '86400',
    value: guild.settings.defaultMediaTime ?? '',
    placeholder: `Valeur du serveur (${effective.defaultMediaTime ?? '—'}\u00a0s)`,
    'aria-describedby': 'set-default-hint',
  });
  const maxTime = h('input', {
    class: 'field',
    id: 'set-max',
    type: 'number',
    min: '0',
    max: '86400',
    value: guild.settings.maxMediaTime ?? '',
    placeholder: effective.maxMediaTime != null ? `Valeur du serveur (${effective.maxMediaTime}\u00a0s)` : 'Sans limite',
  });
  const full = h('input', { type: 'checkbox', id: 'set-full', checked: Boolean(guild.settings.displayMediaFull) });
  const guests = h('input', { type: 'checkbox', id: 'set-guests', checked: guild.guestsCanViewBoard !== false });
  const save = () =>
    saveGuildSettings(
      {
        name: name.value,
        defaultMediaTime: defaultTime.value === '' ? null : Number(defaultTime.value),
        maxMediaTime: maxTime.value === '' ? null : Number(maxTime.value),
        displayMediaFull: full.checked,
        guestsCanViewBoard: guests.checked,
      },
      'Réglages enregistrés.',
    );
  const discord = [
    h('h3', { class: 'section-title' }, 'Discord'),
    h(
      'p',
      { class: 'settings-desc' },
      guild.discordGuildId
        ? `Guilde liée au serveur Discord ${guild.discordGuildId}.`
        : guild.standalone
          ? 'Cette guilde n’est liée à aucun serveur Discord.'
          : 'Cette guilde est un serveur Discord.',
    ),
    h(
      'p',
      { class: 'settings-desc' },
      `Commandes du bot\u00a0: ${DISCORD_COMMANDS.map((command) => `/${command}`).join(', ')} (${DISCORD_COMMANDS.length}\u00a0commandes synchronisées).`,
    ),
  ];
  if (guild.standalone && can('manageSettings')) {
    if (guild.discordGuildId) {
      const unlink = () => {
        confirmDialog('Délier le serveur Discord\u00a0?', 'Les commandes de ce serveur Discord ne viseront plus cette guilde.', 'Délier', async () => {
          await run(() => api('DELETE', '/overlay/admin/discord-link'));
          await loadGuild();
          renderView();
        });
      };
      discord.push(h('div', { class: 'row is-wrap' }, h('button', { class: 'btn is-quiet is-danger', type: 'button', onClick: unlink }, 'Délier le serveur Discord…')));
    } else {
      const generated = h('div', { class: 'stack', hidden: true });
      const verifyDiscordLink = async () => {
        await loadGuild();
        const linked = Boolean(state.info && state.info.guild && state.info.guild.discordGuildId);
        renderView();
        notify(linked ? 'Serveur Discord lié.' : 'Pas encore lié\u00a0: saisis la commande dans Discord.', !linked);
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
            'Dans le serveur Discord, un administrateur tape /lier code\u00a0:',
            h('span', { class: 'code' }, body.code),
            ' (ou /link code\u00a0:',
            h('span', { class: 'code' }, body.code),
            '). Code valable 10\u00a0minutes.',
          ),
          h(
            'div',
            { class: 'row is-wrap' },
            h('button', { class: 'btn', type: 'button', onClick: () => copyText(command, 'Commande copiée.') }, 'Copier la commande'),
            h('button', { class: 'btn is-quiet', type: 'button', onClick: verifyDiscordLink }, 'Vérifier la liaison'),
          ),
        );
        generated.hidden = false;
      };
      discord.push(
        h('div', { class: 'row is-wrap' }, h('button', { class: 'btn', type: 'button', onClick: generate }, 'Générer un code de liaison')),
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
      h('label', { for: 'set-default' }, 'Durée par défaut (secondes)'),
      defaultTime,
      h('p', { class: 'hint', id: 'set-default-hint' }, 'Durée d’affichage d’une image ou d’un texte sans durée propre.'),
      h('label', { for: 'set-max' }, 'Durée maximale (secondes)'),
      maxTime,
      h('label', { for: 'set-full' }, 'Affichage'),
      h('label', { class: 'switch' }, full, 'Médias en plein écran'),
      h('label', { for: 'set-guests' }, 'Invités'),
      h('label', { class: 'switch' }, guests, 'Peuvent consulter le board'),
    ),
    h('div', { class: 'row form-actions' }, h('button', { class: 'btn', type: 'button', onClick: save }, 'Enregistrer les réglages')),
    discord,
  );
}
