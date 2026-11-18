// Header of the active guild: name, role, mute, media sound, profile and tabs.

import { run } from '../core/api.js';
import { h, icon } from '../core/dom.js';
import { invoke } from '../core/ipc.js';
import { allMediaMuted, guildMediaMuted, setGuildMediaMuted } from '../core/sound.js';
import { activeGuild, can, guildLabel, ROLE_LABELS, state } from '../core/state.js';
import { applyGuilds, loadAdmin, loadQueue } from '../data.js';
import { openGuildDialog } from './profile.js';
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
  const tabs = [
    ['board', 'Board', null],
    ['queue', "File d'attente", state.queue.pending.length || null],
  ];
  if (can('manageMembers') || can('manageInvites') || can('manageSettings')) {
    tabs.push(['admin', 'Administration', null]);
  }
  tabs.push(['settings', 'Réglages', null]);
  const guildId = guild.membership.guildId;
  const mediaMuted = guildMediaMuted(guildId);
  header.replaceChildren(
    h(
      'div',
      { class: 'header-top' },
      h('h1', { class: 'guild-name' }, guildLabel(guild)),
      role ? h('span', { class: `badge is-${role}` }, ROLE_LABELS[role] || role) : null,
      guild.membership.muted ? h('span', { class: 'badge is-off' }, 'En pause') : null,
      !guild.membership.muted && (mediaMuted || allMediaMuted())
        ? h('span', { class: 'badge is-off', title: allMediaMuted() && !mediaMuted ? 'Coupé pour toutes les guildes dans les Paramètres' : null }, 'Son des médias coupé')
        : null,
      guild.connection.status !== 'connected' && !guild.membership.muted
        ? h('span', { class: 'badge is-off' }, guild.statusLabel)
        : null,
      h('span', { class: 'spacer' }),
      connected !== null ? h('span', { class: 'meta' }, `${connected} overlay${connected > 1 ? 's' : ''} connecté${connected > 1 ? 's' : ''}`) : null,
      h(
        'button',
        {
          class: 'btn is-quiet',
          title: guild.membership.muted ? 'Recevoir à nouveau les médias de cette guilde' : 'Ne plus recevoir les médias de cette guilde',
          onClick: () => toggleMute(guild),
        },
        icon(guild.membership.muted ? 'mute' : 'sound'),
        guild.membership.muted ? 'Reprendre' : 'Mettre en pause',
      ),
      guild.membership.muted
        ? null
        : h(
            'button',
            {
              class: 'btn is-quiet',
              title: mediaMuted ? 'Rétablir le son des médias de cette guilde' : 'Afficher les médias de cette guilde sans leur son',
              onClick: () => toggleMediaSound(guildId, !mediaMuted),
            },
            icon(mediaMuted ? 'sound' : 'mute'),
            mediaMuted ? 'Remettre le son' : 'Couper le son',
          ),
      h('button', { class: 'btn is-quiet', onClick: () => openGuildDialog(guild) }, 'Mon profil'),
    ),
    h(
      'div',
      { class: 'tabs', role: 'tablist' },
      tabs.map(([id, label, count]) =>
        h(
          'button',
          { class: `tab${state.tab === id ? ' is-active' : ''}`, role: 'tab', 'aria-selected': state.tab === id, onClick: () => setTab(id) },
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
}

async function toggleMediaSound(guildId, muted) {
  const response = await run(() => setGuildMediaMuted(guildId, muted));
  if (response && response.ok === false) {
    return;
  }
  renderHeader();
}

// The device settings changed (tray, Paramètres): refresh the media sound state.
document.addEventListener('livechat:sound', () => renderHeader());

export async function toggleMute(guild) {
  const result = await run(() =>
    invoke('guild_set_muted', { payload: { guildId: guild.membership.guildId, muted: !guild.membership.muted } }),
  );
  if (result) {
    applyGuilds(result.guilds);
  }
}
