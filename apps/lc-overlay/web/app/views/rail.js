// Guild rail (left column).

import { openContextMenu } from '../core/contextmenu.js';
import { h, icon } from '../core/dom.js';
import { guildAcronym, guildAcronymFontSize } from '../core/format.js';
import { canLoadIcon, rememberBrokenIcon } from '../core/guild-icons.js';
import { invoke } from '../core/ipc.js';
import { openGuildBoard } from '../core/rail.js';
import { describeError, guildLabel, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { applyGuilds, prefetchGuildInfo } from '../data.js';
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
      ? { label: 'Réactiver ici', icon: 'sound', onSelect: () => toggleMute(guild) }
      : { label: 'Couper ici', icon: 'mute', onSelect: () => toggleMute(guild) },
    active ? { label: 'Mon profil…', onSelect: () => openGuildDialog(guild) } : null,
    'separator',
    { label: 'Quitter la guilde…', icon: 'remove', danger: true, onSelect: () => confirmLeaveGuild(guild) },
  ]);
}

// A double click on a guild tile opens its board. The clicks themselves have
// already selected the guild; this only moves the view (see core/rail.js).
function showGuildBoard(guild) {
  if (openGuildBoard(state, guild.membership.guildId)) {
    renderHeader();
    renderView();
  }
}

export function renderRail() {
  const rail = document.getElementById('rail');
  rail.replaceChildren(
    ...state.guilds.map((guild) => {
      const guildId = guild.membership.guildId;
      const label = guild.membership.guildName || 'Guilde';
      const pending = state.pendingGuildId === guildId;
      // The tile highlights at once, before the shell answers: the click is
      // acknowledged by the rail itself, not by a toast.
      const active = (guildId === state.activeGuildId && state.page !== 'settings') || pending;
      const hasIcon = canLoadIcon(guild.membership.guildIconUrl);
      // The wrapper keeps the tile's 44 px footprint so the rail rhythm does
      // not change; the menu button is a sibling, never a button in a button.
      return h(
        'div',
        { class: 'guild-slot' },
        h(
          'button',
          {
            class: `guild${active ? ' is-active' : ''}${pending ? ' is-pending' : ''}${guild.membership.muted ? ' is-muted' : ''}${hasIcon ? ' has-icon' : ''}`,
            title: pending ? `${label} — changement…` : `${label} — ${guild.membership.muted ? 'coupée ici' : guild.statusLabel}`,
            'aria-label': label,
            'aria-current': active ? 'true' : null,
            'aria-busy': pending ? 'true' : null,
            onClick: () => selectGuild(guild.membership.guildId),
            onDblclick: () => showGuildBoard(guild),
            onPointerEnter: () => prefetchGuildInfo(guildId),
            onFocus: () => prefetchGuildInfo(guildId),
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

// The last click wins: a slow answer for a guild the user has already left
// must not repaint the board with the wrong guild.
let selectionAttempt = 0;

export async function selectGuild(guildId) {
  if (state.page === 'settings') {
    state.page = 'guild';
    renderRail();
    renderHeader();
    renderView();
  }
  if (guildId === state.activeGuildId || guildId === state.pendingGuildId) {
    return;
  }
  // Acknowledge the click here and now: the tile switches immediately, the
  // shell answer only commits the choice.
  const attempt = ++selectionAttempt;
  state.pendingGuildId = guildId;
  renderRail();
  try {
    const result = await invoke('guild_select', { payload: { guildId } });
    if (attempt === selectionAttempt && result) {
      applyGuilds(result.guilds);
    }
  } catch (error) {
    if (attempt === selectionAttempt) {
      notify(describeError(error?.code ?? error, 'Changement de guilde impossible.'), true);
    }
  } finally {
    if (
      attempt === selectionAttempt &&
      state.pendingGuildId === guildId &&
      state.activeGuildId !== guildId
    ) {
      state.pendingGuildId = null;
      renderRail();
    }
  }
}
