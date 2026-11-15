// Device sound settings seen by the board: the detail preview plays at the
// same level as the overlays (general volume × guild volume), and the header
// shows whether the guild's media sound is cut.

import { invoke, listen } from './ipc.js';

let sound = { volume: 1, mediaMuted: false, guilds: {} };

function level(value) {
  const number = Number(value);
  return Number.isFinite(number) ? Math.min(1, Math.max(0, number)) : 1;
}

function apply(view) {
  if (!view || typeof view !== 'object') {
    return;
  }
  sound = {
    volume: level(view.volume),
    mediaMuted: view.mediaMuted === true,
    guilds: view.guilds && typeof view.guilds === 'object' ? view.guilds : {},
  };
  document.dispatchEvent(new CustomEvent('livechat:sound'));
}

export function initSound() {
  invoke('settings_get').then(apply).catch(() => {});
  listen('overlay://device-settings', apply);
}

// The guild's own « couper le son des médias » switch.
export function guildMediaMuted(guildId) {
  const guild = guildId ? sound.guilds[guildId] : undefined;
  return Boolean(guild && guild.mediaMuted);
}

// Every media plays without sound (general switch).
export function allMediaMuted() {
  return sound.mediaMuted;
}

export async function setGuildMediaMuted(guildId, muted) {
  const response = await invoke('settings_set', { payload: { guilds: { [guildId]: { mediaMuted: muted } } } });
  if (response && response.settings) {
    apply(response.settings);
  }
  return response;
}

// Volume (0 to 1) of a preview for this guild.
export function previewVolume(guildId) {
  const guild = guildId ? sound.guilds[guildId] : undefined;
  return level(sound.volume * (guild && guild.volume !== undefined ? level(guild.volume) : 1));
}
