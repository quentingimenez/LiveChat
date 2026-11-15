// « Réglages » tab of the active guild: what this guild sets on this device
// (display zone, text, sound) and, for its admins, the server settings.
//
// Display: a guild follows the device layout and text setting until it sets
// its own. Its zone is placed on the overlay screen like the device one
// (`settings_placement_start` with `guildId`), its alignment previewed live.

import { h } from '../core/dom.js';
import { activeGuild, can, guildLabel } from '../core/state.js';
import { renderServerSettings } from './admin.js';
import {
  ALIGNMENTS,
  applyPatch,
  cancelPlacement,
  controlRow,
  currentDeviceSettings,
  debounce,
  deviceSettingsError,
  guildLayout,
  loadDeviceSettings,
  placementStarting,
  previewLayout,
  rangeControl,
  section,
  selectedDisplay,
  startPlacement,
  toggleGuildMute,
  zoneSummary,
} from './settings.js';

// Text display choices: `null` follows the device.
const TEXT_CHOICES = [
  ['device', 'Comme sur l’appareil'],
  ['show', 'Afficher'],
  ['hide', 'Masquer'],
];

function patchGuild(guildId, patch) {
  return applyPatch({ guilds: { [guildId]: patch } });
}

function renderDisplay(settings, guildId) {
  const own = (settings.guilds && settings.guilds[guildId]) || {};
  const hasOwnLayout = Boolean(own.layout);
  const layout = guildLayout(settings, guildId);
  const zone = { x: 0, y: 0, width: 1, height: 1, ...(layout.zone || {}) };
  const displays = Array.isArray(settings.displays) ? settings.displays : [];
  const placing = Boolean(settings.placing);
  const starting = placementStarting();

  const zoneControls = placing
    ? [
        h('span', { class: 'settings-zone is-live' }, 'Placement en cours sur l’écran de l’overlay'),
        h('button', { class: 'btn', type: 'button', onClick: cancelPlacement }, 'Annuler le placement'),
      ]
    : [
        h(
          'span',
          { class: 'settings-zone' },
          hasOwnLayout ? zoneSummary(zone, selectedDisplay(settings, displays)) : `Comme sur l’appareil (${zoneSummary(zone, selectedDisplay(settings, displays)).toLowerCase()})`,
        ),
        h(
          'button',
          { class: 'btn is-air', type: 'button', disabled: starting, onClick: () => startPlacement(guildId) },
          starting ? 'Ouverture…' : 'Placer sur l’écran',
        ),
      ];

  const alignment = h(
    'select',
    {
      id: 'guild-settings-align',
      class: 'field',
      onChange: (event) => {
        previewLayout({ align: event.target.value }, guildId);
        patchGuild(guildId, { layout: { align: event.target.value } });
      },
    },
    ALIGNMENTS.map(([value, label]) => h('option', { value }, label)),
  );
  alignment.value = layout.align || 'center';

  const textChoice = typeof own.showText === 'boolean' ? (own.showText ? 'show' : 'hide') : 'device';
  const text = h(
    'select',
    {
      id: 'guild-settings-text',
      class: 'field',
      onChange: (event) => {
        const value = event.target.value;
        patchGuild(guildId, { showText: value === 'device' ? null : value === 'show' });
      },
    },
    TEXT_CHOICES.map(([value, label]) => h('option', { value }, label)),
  );
  text.value = textChoice;

  return section(
    'Affichage sur cet appareil',
    h(
      'p',
      { class: 'settings-hint' },
      'Où et comment les médias de cette guilde s’affichent sur ton stream. Sans réglage propre, la guilde suit les Paramètres de l’appareil.',
    ),
    controlRow('Zone d’affichage', zoneControls),
    controlRow('Alignement dans la zone', alignment, 'guild-settings-align'),
    hasOwnLayout && !placing
      ? h(
          'div',
          { class: 'row is-wrap settings-row-actions' },
          h(
            'button',
            { class: 'btn is-quiet', type: 'button', onClick: () => patchGuild(guildId, { layout: null }) },
            'Suivre l’affichage de l’appareil',
          ),
        )
      : null,
    controlRow('Texte des médias', text, 'guild-settings-text'),
  );
}

function renderSound(settings, guild) {
  const guildId = guild.membership.guildId;
  const own = (settings.guilds && settings.guilds[guildId]) || {};
  const volume = Number.isFinite(own.volume) ? own.volume : 1;
  const mute = h('input', {
    id: 'guild-settings-mute',
    type: 'checkbox',
    checked: Boolean(guild.membership.muted),
    onChange: (event) => toggleGuildMute(guild, event.target.checked),
  });
  const mediaMute = h('input', {
    id: 'guild-settings-media-mute',
    type: 'checkbox',
    checked: Boolean(own.mediaMuted),
    onChange: (event) => patchGuild(guildId, { mediaMuted: event.target.checked }),
  });
  return section(
    'Son sur cet appareil',
    controlRow(
      'Volume de la guilde',
      rangeControl({
        id: 'guild-settings-volume',
        min: 0,
        max: 100,
        value: Math.round(volume * 100),
        format: (value) => `${value} %`,
        onInput: (value) => debounce(`guildVolume:${guildId}`, () => patchGuild(guildId, { volume: value / 100 })),
      }),
      'guild-settings-volume',
    ),
    h('p', { class: 'settings-hint is-under' }, 'Multiplié par le volume général des Paramètres.'),
    controlRow('Couper le son des médias', mediaMute, 'guild-settings-media-mute'),
    h('p', { class: 'settings-hint is-under' }, 'Les médias de cette guilde restent affichés, sans leur son.'),
    controlRow('Mettre en pause', mute, 'guild-settings-mute'),
    h('p', { class: 'settings-hint is-under' }, 'Plus aucun média de cette guilde ne s’affiche sur cet appareil.'),
  );
}

export function renderGuildSettings() {
  const guild = activeGuild();
  const settings = currentDeviceSettings();
  const deviceSections = [];
  if (settings) {
    deviceSections.push(renderDisplay(settings, guild.membership.guildId), renderSound(settings, guild));
  } else {
    const error = deviceSettingsError();
    deviceSections.push(
      error
        ? h('div', { class: 'empty' }, h('p', null, error), h('button', { class: 'btn', onClick: loadDeviceSettings }, 'Réessayer'))
        : h('p', { class: 'meta' }, 'Chargement des réglages…'),
    );
    if (!error) {
      // After this render: loading re-renders the view.
      queueMicrotask(loadDeviceSettings);
    }
  }
  return h(
    'div',
    { class: 'settings' },
    h('div', { class: 'settings-head' }, h('p', { class: 'meta' }, `Réglages de ${guildLabel(guild)}.`)),
    deviceSections,
    can('manageSettings') ? section('Serveur', h('p', { class: 'settings-hint is-lead' }, 'Pour toute la guilde, sur tous les appareils.'), renderServerSettings()) : null,
  );
}
