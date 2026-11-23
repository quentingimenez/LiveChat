// Header of the active guild: name, role, mute, media sound, profile and tabs.

import { isActionPending } from '../core/action.js';
import { performAction } from '../core/action-ui.js';
import { h, icon } from '../core/dom.js';
import { invoke } from '../core/ipc.js';
import { allMediaMuted, guildMediaMuted, setGuildMediaMuted } from '../core/sound.js';
import { activeGuild, canAdmin, guildLabel, ROLE_LABELS, state } from '../core/state.js';
import { applyGuilds, loadAdmin, loadGuildInfo, loadQueue } from '../data.js';
import { openGuildDialog } from './profile.js';
import { renderRail } from './rail.js';
import { renderView } from './view.js';

export function renderHeader() {
  const header = document.getElementById('header');
  const guild = activeGuild();
  if (state.page === 'settings' || !guild) {
    header.replaceChildren();
    header.hidden = true;
    return;
  }
  header.hidden = false;
  const role = (state.info && state.info.member && state.info.member.role) || guild.membership.role;
  const connected = state.info && state.info.guild ? state.info.guild.connectedCount : null;
  // The queue badge means "queue loaded and holding this many items": before
  // the first read it stays empty instead of flashing the previous guild's
  // count.
  const queueCount = state.queueLoaded ? state.queue.pending.length : 0;
  // The Administration tab shows at once for a guild whose role is admin; the
  // real permissions replace that guess as soon as the guild info lands.
  const canManage = state.info ? canAdmin() : role === 'admin';
  const tabs = [
    ['board', 'Board', null],
    ['queue', 'File d’attente', queueCount || null],
  ];
  if (canManage) {
    tabs.push(['admin', 'Administration', null]);
  }
  // The visible label stays short; the accessible name says which settings.
  tabs.push(['settings', 'Réglages', null, 'Réglages de la guilde']);
  const guildId = guild.membership.guildId;
  const mediaMuted = guildMediaMuted(guildId);
  const muteKey = `guild-mute:${guildId}`;
  const soundKey = `guild-sound:${guildId}`;
  header.replaceChildren(
    h(
      'div',
      { class: 'header-top' },
      h(
        'div',
        { class: 'header-title' },
        h('h1', { class: 'guild-name' }, guildLabel(guild)),
        role ? h('span', { class: `badge is-${role}` }, ROLE_LABELS[role] || role) : null,
        guild.membership.muted ? h('span', { class: 'badge is-off' }, 'Coupée ici') : null,
        !guild.membership.muted && (mediaMuted || allMediaMuted())
          ? h(
              'span',
              { class: 'badge is-off', title: allMediaMuted() && !mediaMuted ? 'Coupé pour toutes les guildes dans les Paramètres' : null },
              'Son coupé',
            )
          : null,
        guild.connection.status !== 'connected' && !guild.membership.muted
          ? h('span', { class: 'badge is-off' }, guild.statusLabel)
          : null,
      ),
      h(
        'div',
        { class: 'header-actions' },
        connected !== null
          ? h('span', { class: 'meta header-connection' }, `${connected} overlay${connected > 1 ? 's' : ''} connecté${connected > 1 ? 's' : ''}`)
          : null,
        h(
          'button',
          {
            class: 'btn is-quiet',
            disabled: isActionPending(muteKey) || null,
            'aria-busy': isActionPending(muteKey) ? 'true' : null,
            title: guild.membership.muted ? 'Recevoir à nouveau les médias de cette guilde' : 'Ne plus recevoir les médias de cette guilde',
            onClick: (event) => toggleMute(guild, event.currentTarget),
          },
          icon(guild.membership.muted ? 'mute' : 'sound'),
          guild.membership.muted ? 'Réactiver ici' : 'Couper ici',
        ),
        guild.membership.muted
          ? null
          : h(
              'button',
              {
                class: 'btn is-quiet',
                disabled: isActionPending(soundKey) || null,
                'aria-busy': isActionPending(soundKey) ? 'true' : null,
                title: mediaMuted ? 'Rétablir le son des médias de cette guilde' : 'Afficher les médias de cette guilde sans leur son',
                onClick: (event) => toggleMediaSound(guildId, !mediaMuted, event.currentTarget),
              },
              icon(mediaMuted ? 'sound' : 'mute'),
              mediaMuted ? 'Rétablir' : 'Muet',
            ),
        h('button', { class: 'btn is-quiet', onClick: () => openGuildDialog(guild) }, 'Profil'),
      ),
    ),
    h(
      'div',
      { class: 'tabs', role: 'tablist' },
      tabs.map(([id, label, count, accessibleLabel]) =>
        h(
          'button',
          {
            class: `tab${state.tab === id ? ' is-active' : ''}`,
            role: 'tab',
            title: accessibleLabel || null,
            'aria-label': accessibleLabel || (count ? `${label} (${count})` : null),
            'aria-selected': state.tab === id ? 'true' : 'false',
            onClick: () => setTab(id),
          },
          label,
          count ? h('span', { class: 'count' }, count) : null,
        ),
      ),
    ),
  );
}

export function setTab(tab) {
  state.page = 'guild';
  state.tab = tab;
  renderHeader();
  renderView();
  if (tab === 'admin') {
    loadAdmin();
  }
  if (tab === 'queue') {
    loadQueue();
  }
  // Opening the tab is the moment the user expects current server values. A
  // field-focused refresh is dropped by `loadGuildInfo`, so typing is safe.
  if (tab === 'settings' && state.activeGuildId) {
    loadGuildInfo({ quiet: true, force: true });
  }
}

async function toggleMediaSound(guildId, muted, control) {
  return performAction(control, `guild-sound:${guildId}`, () => setGuildMediaMuted(guildId, muted), {
    onSuccess: (response) => {
      if (response && response.ok === false) {
        return;
      }
      renderHeader();
    },
    onFinally: () => renderHeader(),
  });
}

// The device settings changed (tray, Paramètres): refresh the media sound state.
document.addEventListener('livechat:sound', () => renderHeader());

export async function toggleMute(guild, control) {
  const guildId = guild.membership.guildId;
  const next = !guild.membership.muted;
  await performAction(
    control,
    `guild-mute:${guildId}`,
    () => invoke('guild_set_muted', { payload: { guildId, muted: next } }),
    {
      optimistic: () => {
        guild.membership.muted = next;
        renderRail();
        renderHeader();
      },
      rollback: () => {
        guild.membership.muted = !next;
        renderRail();
        renderHeader();
      },
      onSuccess: (result) => {
        if (result && result.guilds) {
          applyGuilds(result.guilds);
        }
      },
      onFinally: () => {
        renderRail();
        renderHeader();
      },
    },
  );
}
