// Guild rail (left column).

import { run } from '../core/api.js';
import { h, icon } from '../core/dom.js';
import { initials } from '../core/format.js';
import { invoke } from '../core/ipc.js';
import { state } from '../core/state.js';
import { applyGuilds } from '../data.js';
import { openJoinDialog } from './join.js';

function dotClass(guild) {
  if (guild.playing) {
    return 'is-air';
  }
  switch (guild.connection.status) {
    case 'connected':
      return 'is-on';
    case 'connecting':
    case 'reconnecting':
      return 'is-busy';
    case 'error':
    case 'revoked':
    case 'outdated':
      return 'is-err';
    default:
      return '';
  }
}

export function renderRail() {
  const rail = document.getElementById('rail');
  rail.replaceChildren(
    ...state.guilds.map((guild) => {
      const label = guild.membership.guildName || 'Guilde';
      const active = guild.membership.guildId === state.activeGuildId;
      return h(
        'button',
        {
          class: `guild${active ? ' is-active' : ''}${guild.membership.muted ? ' is-muted' : ''}`,
          title: `${label} — ${guild.membership.muted ? 'en sourdine' : guild.statusLabel}`,
          'aria-current': active ? 'true' : null,
          onClick: () => selectGuild(guild.membership.guildId),
        },
        initials(label),
        h('span', { class: `dot ${dotClass(guild)}` }),
      );
    }),
    h('button', { class: 'guild is-add', title: 'Rejoindre une guilde', 'aria-label': 'Rejoindre une guilde', onClick: openJoinDialog }, icon('plus')),
    h('span', { class: 'rail-spacer' }),
  );
}

export async function selectGuild(guildId) {
  if (guildId === state.activeGuildId) {
    return;
  }
  const result = await run(() => invoke('guild_select', { payload: { guildId } }));
  if (result) {
    applyGuilds(result.guilds);
  }
}
