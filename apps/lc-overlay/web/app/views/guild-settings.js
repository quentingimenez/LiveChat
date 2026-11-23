// « Réglages » tab of the active guild: what this guild sets on this device
// (display zone, text, sound) and, for its admins, the server settings.
//
// Display: a guild follows the device layout and text setting until it sets
// its own. Its zone is placed on the overlay screen like the device one
// (`settings_placement_start` with `guildId`), its alignment previewed live.

import { api } from '../core/api.js';
import { h } from '../core/dom.js';
import { formatBoardUsage, formatBytes } from '../core/format.js';
import { activeGuild, can, state } from '../core/state.js';
import { DEFAULT_VOLUME_PERCENT } from '../core/sound.js';
import { notify } from '../core/toast.js';
import { loadGuild } from '../data.js';
import {
  confirmDiscordLeave,
  hasDiscordServer,
  openPurgeGuildDialog,
  openTypedGuildDialog,
} from '../guild-danger.js';
import { renderServerSettings, saveGuildSettings } from './admin.js';
import { renderView } from './view.js';
import {
  ALIGNMENTS,
  applyPatch,
  cancelPlacement,
  controlRow,
  currentDeviceSettings,
  debounce,
  deviceSettingsError,
  guildLayout,
  hint,
  autosaveNote,
  loadDeviceSettings,
  openSettings,
  placementStarting,
  previewLayout,
  rangeControl,
  section,
  selectedDisplay,
  startPlacement,
  switchRow,
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
          { class: 'btn is-primary', type: 'button', disabled: starting, onClick: () => startPlacement(guildId) },
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
    controlRow('Zone d’affichage', zoneControls),
    // Back to the device layout: a named action.
    hasOwnLayout && !placing
      ? h(
          'div',
          { class: 'settings-action' },
          h(
            'button',
            { class: 'btn is-quiet', type: 'button', onClick: () => patchGuild(guildId, { layout: null }) },
            'Suivre l’affichage de cet appareil',
          ),
        )
      : null,
    controlRow('Alignement dans la zone', alignment, 'guild-settings-align'),
    controlRow('Texte des médias', [text], 'guild-settings-text'),
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
      [
        ...rangeControl({
          id: 'guild-settings-volume',
          min: 0,
          max: 100,
          value: Math.round(volume * 100),
          format: (value) => `${value} %`,
          defaultValue: DEFAULT_VOLUME_PERCENT,
          onInput: (value) => debounce(`guildVolume:${guildId}`, () => patchGuild(guildId, { volume: value / 100 })),
        }),
        hint('Multiplié par le volume général des Paramètres.'),
      ],
      'guild-settings-volume',
    ),
    switchRow('Son coupé', mediaMute, 'guild-settings-media-mute'),
    switchRow('Couper cette guilde sur cet appareil', mute, 'guild-settings-mute'),
  );
}

// ------------------------------------------------------------- board (admin)

// Guild quota in Go, one decimal at most; empty when the server limit applies.
function quotaInGo(mediaBoard) {
  if (!mediaBoard || mediaBoard.quotaMb == null) {
    return '';
  }
  return String(Math.round((mediaBoard.quotaMb / 1024) * 10) / 10);
}

// « Vider le board » asks the guild name, like the server console's strong
// confirmation. The server compares it exactly (`confirmation_mismatch`).
function openClearBoardDialog(guildName) {
  openTypedGuildDialog({
    guildName,
    title: 'Vider le board ?',
    text: 'Tous les médias sont supprimés, la lecture est arrêtée et les fichiers orphelins sont effacés. Action immédiate et définitive.',
    confirmLabel: 'Vider le board',
    inputId: 'clear-board-name',
    action: (confirmation) => api('POST', '/overlay/admin/media-board/clear', { confirm: confirmation }),
    onSuccess: async (body) => {
      const deleted = Number((body.deleted && body.deleted.mediaBoardItems) || 0);
      notify(`Board vidé : ${deleted} média${deleted === 1 ? '' : 's'} supprimé${deleted === 1 ? '' : 's'}.`);
      await loadGuild();
      renderView();
    },
  });
}

function renderBoardSettings() {
  const guild = (state.info && state.info.guild) || {};
  const mediaBoard = guild.mediaBoard || null;
  const serverMaxBytes = mediaBoard && Number.isFinite(Number(mediaBoard.serverMaxBytes)) ? Number(mediaBoard.serverMaxBytes) : null;
  const serverMaxGo = serverMaxBytes ? Math.floor(serverMaxBytes / (1024 * 1024 * 1024)) : null;
  const quota = h('input', {
    class: 'field',
    id: 'set-board-quota',
    type: 'number',
    min: '1',
    max: serverMaxGo && serverMaxGo >= 1 ? String(serverMaxGo) : null,
    step: '1',
    value: quotaInGo(mediaBoard),
    placeholder: serverMaxGo && serverMaxGo >= 1 ? String(serverMaxGo) : '',
  });
  const save = async () => {
    const raw = quota.value.trim().replace(',', '.');
    const gigabytes = raw === '' ? null : Number(raw);
    if (gigabytes !== null && (!Number.isFinite(gigabytes) || gigabytes <= 0)) {
      notify('Quota invalide : indique un nombre de Go supérieur à 0.', true);
      quota.focus();
      return;
    }
    const boardQuotaMb = gigabytes === null ? null : Math.max(1, Math.round(gigabytes * 1024));
    await saveGuildSettings({ boardQuotaMb }, 'Quota du board enregistré.');
  };
  // The guild settings show what the board holds even before a quota is set:
  // the quota field right below is where that limit is chosen.
  const usage = (mediaBoard && formatBoardUsage(mediaBoard, { requireMax: false })) || 'inconnue';
  return section(
    'Board de la guilde',
    // What the board holds right now, then what the quota below changes.
    h('p', { class: 'settings-hint is-summary' }, `Occupation : ${usage}.`),
    h(
      'p',
      { class: 'settings-hint is-lead' },
      serverMaxBytes
        ? `Sans quota propre, la limite du serveur s’applique (${formatBytes(serverMaxBytes)}). Le quota d’une guilde ne peut pas la dépasser.`
        : 'Sans quota propre, la limite du serveur s’applique.',
    ),
    // The input, its named save button and the hint share the control column,
    // like the « Placer sur l’écran » button next to the display zone.
    controlRow(
      'Quota propre (Go)',
      [quota, h('button', { class: 'btn', type: 'button', onClick: save }, 'Enregistrer le board'), hint('Vide = quota du serveur.')],
      'set-board-quota',
    ),
    // Destructive action, set apart from the form above it.
    h(
      'div',
      { class: 'settings-danger' },
      h(
        'div',
        { class: 'settings-danger-body' },
        h('p', { class: 'settings-danger-title' }, 'Vider le board ?'),
        guild.name
          ? [
              h(
                'p',
                { class: 'settings-hint' },
                'Supprime tous les médias du board et les fichiers orphelins. Action immédiate et définitive.',
              ),
              h(
                'button',
                { class: 'btn is-danger', type: 'button', onClick: () => openClearBoardDialog(guild.name) },
                'Vider le board',
              ),
            ]
          : h(
              'p',
              { class: 'settings-hint' },
              'Le nom de la guilde est inconnu : le vidage demande son nom, il est donc indisponible.',
            ),
      ),
    ),
  );
}

function renderDangerZone() {
  const guild = (state.info && state.info.guild) || {};
  const guildName = guild.name || 'cette guilde';
  const rows = [];
  // A native guild always has a Discord server; a standalone one only once
  // linked. Without a server there is nobody to make leave. The leave action
  // needs no guild name: only the purge's typed confirmation does.
  if (hasDiscordServer(guild)) {
    rows.push(
      h(
        'div',
        { class: 'settings-danger' },
        h(
          'div',
          { class: 'settings-danger-body' },
          h('p', { class: 'settings-danger-title' }, 'Faire quitter le bot ?'),
          h(
            'p',
            { class: 'settings-hint' },
            `Le bot quitte le serveur Discord de « ${guildName} ». Les commandes n’y fonctionneront plus ; les données restent sur le serveur LiveChat.`,
          ),
          h(
            'button',
            { class: 'btn is-danger', type: 'button', onClick: () => confirmDiscordLeave(guild) },
            'Faire quitter le bot',
          ),
        ),
      ),
    );
  }
  rows.push(
    h(
      'div',
      { class: 'settings-danger' },
      h(
        'div',
        { class: 'settings-danger-body' },
        h('p', { class: 'settings-danger-title' }, 'Supprimer la guilde ?'),
        guild.name
          ? [
              h(
                'p',
                { class: 'settings-hint' },
                'Membres, overlays, codes, file d’attente, board et réglages sont supprimés définitivement.',
              ),
              h(
                'button',
                { class: 'btn is-danger', type: 'button', onClick: () => openPurgeGuildDialog(guild) },
                'Supprimer la guilde',
              ),
            ]
          : h(
              'p',
              { class: 'settings-hint' },
              'Le nom de la guilde est inconnu : la suppression demande son nom, elle est donc indisponible.',
            ),
      ),
    ),
  );
  return section('Zone de danger', ...rows);
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
    // Order: what this device sets for the guild, then what the guild sets on
    // the server.
    h(
      'div',
      { class: 'settings-head' },
      h('h2', null, 'Réglages de la guilde'),
      autosaveNote(),
      h(
        'button',
        { class: 'btn is-quiet settings-device-link', type: 'button', onClick: openSettings },
        'Paramètres de cet appareil',
      ),
    ),
    deviceSections,
    can('manageSettings') ? section('Serveur de la guilde', h('p', { class: 'settings-hint is-lead' }, 'Pour toute la guilde, sur tous les appareils.'), renderServerSettings()) : null,
    can('manageSettings') ? renderBoardSettings() : null,
    can('purgeGuild') ? renderDangerZone() : null,
  );
}
