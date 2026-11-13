// Guild rail (left column).

import { run } from '../core/api.js';
import { openContextMenu } from '../core/contextmenu.js';
import { h, icon } from '../core/dom.js';
import { initials } from '../core/format.js';
import { invoke } from '../core/ipc.js';
import { guildLabel, state } from '../core/state.js';
import { applyGuilds } from '../data.js';
import { toggleMute } from './header.js';
import { openJoinDialog } from './join.js';
import { confirmLeaveGuild, openGuildDialog } from './profile.js';

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

function guildFace(label, iconUrl) {
  if (!iconUrl) {
    return initials(label);
  }
  const image = h('img', {
    class: 'guild-icon',
    src: iconUrl,
    alt: '',
    draggable: 'false',
    referrerpolicy: 'no-referrer',
    // A dead icon URL falls back to the initials instead of an empty button.
    onError: () => {
      const button = image.parentElement;
      if (button) {
        // Restore the plain active background: the fallback is text, not an icon.
        button.classList.remove('has-icon');
      }
      image.replaceWith(initials(label));
    },
  });
  return image;
}

function openGuildMenu(event, guild) {
  const active = guild.membership.guildId === state.activeGuildId;
  openContextMenu(event, [
    { heading: guildLabel(guild) },
    active
      ? null
      : {
          label: 'Ouvrir la guilde',
          icon: 'play',
          onSelect: () => selectGuild(guild.membership.guildId),
        },
    guild.membership.muted
      ? { label: 'Rétablir le son', icon: 'sound', onSelect: () => toggleMute(guild) }
      : { label: 'Mettre en sourdine', icon: 'mute', onSelect: () => toggleMute(guild) },
    active ? { label: 'Mon profil…', onSelect: () => openGuildDialog(guild) } : null,
    'separator',
    { label: 'Quitter la guilde…', icon: 'remove', danger: true, onSelect: () => confirmLeaveGuild(guild) },
  ]);
}

export function renderRail() {
  const rail = document.getElementById('rail');
  rail.replaceChildren(
    ...state.guilds.map((guild) => {
      const label = guild.membership.guildName || 'Guilde';
      const active = guild.membership.guildId === state.activeGuildId;
      const hasIcon = Boolean(guild.membership.guildIconUrl);
      return h(
        'button',
        {
          class: `guild${active ? ' is-active' : ''}${guild.membership.muted ? ' is-muted' : ''}${hasIcon ? ' has-icon' : ''}`,
          title: `${label} — ${guild.membership.muted ? 'en sourdine' : guild.statusLabel}`,
          'aria-label': label,
          'aria-current': active ? 'true' : null,
          onClick: () => selectGuild(guild.membership.guildId),
          onContextMenu: (event) => openGuildMenu(event, guild),
        },
        guildFace(label, guild.membership.guildIconUrl),
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
