// Guild rail (left column).

import { run } from '../core/api.js';
import { openContextMenu } from '../core/contextmenu.js';
import { h, icon } from '../core/dom.js';
import { guildAcronym, guildAcronymFontSize } from '../core/format.js';
import { canLoadIcon, rememberBrokenIcon } from '../core/guild-icons.js';
import { invoke } from '../core/ipc.js';
import { guildLabel, state } from '../core/state.js';
import { applyGuilds } from '../data.js';
import { renderHeader, toggleMute } from './header.js';
import { openJoinDialog } from './join.js';
import { confirmLeaveGuild, openGuildDialog } from './profile.js';
import { openSettings } from './settings.js';
import { renderView } from './view.js';

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

// Icon fallback of a guild without a picture: Discord's acronym, clipped to the
// tile when the name has many words.
function acronymFace(label) {
  const acronym = guildAcronym(label) || '?';
  return h(
    'span',
    { class: 'guild-acronym', style: `font-size: ${guildAcronymFontSize(acronym)}px` },
    acronym,
  );
}

function guildFace(label, iconUrl) {
  // A broken icon stays broken for the session: skip the doomed <img> so the
  // acronym does not vanish and come back at every rail rebuild.
  if (!canLoadIcon(iconUrl)) {
    return acronymFace(label);
  }
  const image = h('img', {
    class: 'guild-icon',
    src: iconUrl,
    alt: '',
    draggable: 'false',
    referrerpolicy: 'no-referrer',
    // A dead icon URL falls back to the acronym instead of an empty button.
    onError: () => {
      rememberBrokenIcon(iconUrl);
      const button = image.parentElement;
      if (button) {
        // Restore the plain active background: the fallback is text, not an icon.
        button.classList.remove('has-icon');
      }
      image.replaceWith(acronymFace(label));
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
      ? { label: 'Reprendre', icon: 'sound', onSelect: () => toggleMute(guild) }
      : { label: 'Mettre en pause', icon: 'mute', onSelect: () => toggleMute(guild) },
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
      const active = guild.membership.guildId === state.activeGuildId && state.page !== 'settings';
      const hasIcon = canLoadIcon(guild.membership.guildIconUrl);
      // The wrapper keeps the tile's 44 px footprint so the rail rhythm does
      // not change; the menu button is a sibling, never a button in a button.
      return h(
        'div',
        { class: 'guild-slot' },
        h(
          'button',
          {
            class: `guild${active ? ' is-active' : ''}${guild.membership.muted ? ' is-muted' : ''}${hasIcon ? ' has-icon' : ''}`,
            title: `${label} — ${guild.membership.muted ? 'en pause' : guild.statusLabel}`,
            'aria-label': label,
            'aria-current': active ? 'true' : null,
            onClick: () => selectGuild(guild.membership.guildId),
            onContextMenu: (event) => openGuildMenu(event, guild),
          },
          guildFace(label, guild.membership.guildIconUrl),
          h('span', { class: `dot ${dotClass(guild)}` }),
        ),
        h(
          'button',
          {
            class: 'guild-menu',
            type: 'button',
            title: `Actions de ${label}`,
            'aria-label': `Actions de ${label}`,
            'aria-haspopup': 'menu',
            onClick: (event) => openGuildMenu(event, guild),
            onContextMenu: (event) => openGuildMenu(event, guild),
          },
          '⋯',
        ),
      );
    }),
    h('button', { class: 'guild is-add', title: 'Rejoindre une guilde', 'aria-label': 'Rejoindre une guilde', onClick: openJoinDialog }, icon('plus')),
    h('span', { class: 'rail-spacer' }),
    h(
      'button',
      {
        class: `guild is-settings${state.page === 'settings' ? ' is-active' : ''}`,
        title: 'Paramètres',
        'aria-label': 'Paramètres',
        'aria-current': state.page === 'settings' ? 'page' : null,
        onClick: openSettings,
      },
      icon('settings'),
    ),
  );
}

export async function selectGuild(guildId) {
  if (state.page === 'settings') {
    state.page = 'guild';
    renderRail();
    renderHeader();
    renderView();
  }
  if (guildId === state.activeGuildId) {
    return;
  }
  const result = await run(() => invoke('guild_select', { payload: { guildId } }));
  if (result) {
    applyGuilds(result.guilds);
  }
}
