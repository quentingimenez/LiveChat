// Device sound settings seen by the board: the previews play at the same
// level as the overlay (general volume × guild volume, through the overlay's
// perceptual curve), and the header shows whether the guild's media sound is
// cut.

import { createOverlayLegacyUtils } from '../../dist/renderer/overlay/legacy-utils.js';
import { invoke, listen } from './ipc.js';

// The overlay renderer's own curve: one implementation for both.
const { toPerceptualGain } = createOverlayLegacyUtils();

let sound = { volume: 1, mediaMuted: false, guilds: {} };

// Both volumes start at 100 %: the shell stores them as 0..1 with a default of
// 1 (`DEFAULT_VOLUME` in the overlay's config.rs).
export const DEFAULT_VOLUME_PERCENT = 100;

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

// Media element volume (0 to 1) for a general and a guild volume setting:
// exactly what the overlay applies to a media of that guild. The shell sends
// the product of both settings in the `play` and the renderer turns it into
// a gain with its perceptual curve; a linear product here made the previews
// louder than the overlay (50 % gave 0.5 instead of 0.22).
export function mediaGain(volume, guildVolume) {
  return toPerceptualGain(level(volume) * (guildVolume === undefined ? 1 : level(guildVolume)));
}

// Volume (0 to 1) of a preview for this guild.
export function previewVolume(guildId) {
  const guild = guildId ? sound.guilds[guildId] : undefined;
  return mediaGain(sound.volume, guild ? guild.volume : undefined);
}
