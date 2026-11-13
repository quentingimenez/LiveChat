// Header of the active guild: name, role, mute, profile and tabs.

import { run } from '../core/api.js';
import { h, icon } from '../core/dom.js';
import { invoke } from '../core/ipc.js';
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
  header.replaceChildren(
    h(
      'div',
      { class: 'header-top' },
      h('h1', { class: 'guild-name' }, guildLabel(guild)),
      role ? h('span', { class: `badge is-${role}` }, ROLE_LABELS[role] || role) : null,
      guild.membership.muted ? h('span', { class: 'badge is-off' }, 'En sourdine') : null,
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
        guild.membership.muted ? 'Rétablir' : 'Sourdine',
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

export async function toggleMute(guild) {
  const result = await run(() =>
    invoke('guild_set_muted', { payload: { guildId: guild.membership.guildId, muted: !guild.membership.muted } }),
  );
  if (result) {
    applyGuilds(result.guilds);
  }
}
