// Device-wide settings page (Paramètres).
//
// A page of its own, not a guild tab: it works with zero guild and keeps the
// rail visible. Every change is applied immediately through `settings_set`;
// sound sliders are debounced, and the page always renders from the settings
// view returned by the shell.
//
// The display zone is placed on the overlay screen itself
// (`settings_placement_start`): the overlay becomes a window the user moves
// and resizes, and saves the zone. The alignment is previewed live through
// `settings_preview_layout`, which shows it with a template card.
//
// The guild « Réglages » tab (guild-settings.js) reuses these helpers for what
// one guild sets on this device.

import { run } from '../core/api.js';
import { acceleratorFromEvent, ALTGR_HINT, BARE_KEY_MESSAGE, UNSUPPORTED_KEY, UNSUPPORTED_KEY_MESSAGE, cancelsCapture } from '../core/accelerators.js';
import { resizeAvatar } from '../core/avatar.js';
import { avatarNode, h, icon, markSettingsDirty, registerIcons, resetOnDoubleClick, settingsControlActive, settingsFormDirty, SLIDER_RESET_HINT } from '../core/dom.js';
import { invoke, listen } from '../core/ipc.js';
import { previewPlaybackEnabled } from '../core/media-playback.js';
import { forgetFollower, PROFILE_SYNCED_EVENT } from '../core/profile-sync.js';
import { settingsBackInMargin } from '../core/settings-back.js';
import { capturedKeyLabel, forgetShortcutLabel, shortcutLabel, writeShortcutLabel } from '../core/shortcut-labels.js';
import { DEFAULT_VOLUME_PERCENT } from '../core/sound.js';
import { guildLabel, state } from '../core/state.js';
import { currentThemePreference, resolvedTheme, setThemePreference, subscribeTheme } from '../core/theme.js';
import { notify } from '../core/toast.js';
import { currentUpdateStatus, setUpdateStatus } from '../core/update-status.js';
import { applyGuilds } from '../data.js';
import { renderHeader } from './header.js';
import { renderRail, selectGuild } from './rail.js';
import { renderView } from './view.js';

// Where a media sits in the zone when it does not have the zone's shape.
export const ALIGNMENTS = [
  ['center', 'Centre'],
  ['top', 'En haut'],
  ['bottom', 'En bas'],
  ['left', 'À gauche'],
  ['right', 'À droite'],
  ['top-left', 'En haut à gauche'],
  ['top-right', 'En haut à droite'],
  ['bottom-left', 'En bas à gauche'],
  ['bottom-right', 'En bas à droite'],
];

const SHORTCUTS = [
  ['stop', 'Arrêter le média pour tous'],
  ['stopLocal', 'Arrêter le média chez moi'],
  ['muteAll', 'Couper tous les médias ici'],
  ['openBoard', 'Ouvrir la board'],
];

// Resolved from this module, so it works from web/board.html and from the
// dev harness page alike. The logo corners follow the surface: Craie on the
// dark board, Ardoise on the light one (docs/brand/README.md).
const LOGO_URL = new URL('../assets/logo.svg', import.meta.url).href;
const LOGO_LIGHT_URL = new URL('../assets/logo-light.svg', import.meta.url).href;

// The shared icon set has no « back » chevron; the page head needs one.
registerIcons({ back: 'M15 6l-6 6 6 6' });

const SLIDER_DEBOUNCE_MS = 250;
const PREVIEW_FRAME_MS = 16;

let deviceSettings = null;
let loading = false;
let loadError = null;
let shortcutCapture = null;
let shortcutErrors = {};
let identifyingDisplays = false;
let testingMedia = false;
let startingPlacement = false;
let checkingUpdates = false;
let installingUpdate = false;
let updateError = null;
const debounceTimers = new Map();
let previewDraft = null;
let previewGuildId = null;
let previewFrame = null;

// The device settings page, or the active guild's « Réglages » tab.
function settingsShown() {
  return state.page === 'settings' || (state.page === 'guild' && state.tab === 'settings' && Boolean(state.activeGuildId));
}

// Last settings view from the shell (`null` until loaded).
export function currentDeviceSettings() {
  return deviceSettings;
}

export function deviceSettingsError() {
  return loadError;
}

function errorMessage(error) {
  if (!error) {
    return 'Action impossible.';
  }
  return typeof error === 'string' ? error : error.message || String(error);
}

// Logo for the current surface: the dark board needs the Craie corners, the
// light one the Ardoise corners.
function logoForTheme() {
  return resolvedTheme() === 'light' ? LOGO_LIGHT_URL : LOGO_URL;
}

// Swaps the About logo when the theme changes without rebuilding the page: a
// full render would rebuild the theme <select> under the pointer.
function syncLogo() {
  const logo = document.querySelector('.settings-logo');
  if (logo) {
    logo.src = logoForTheme();
  }
}

// ------------------------------------------------------- « Retour » placement

let backPlacementFrame = null;

// Reads the current layout and moves « Retour » into the left margin only when
// it fits entirely there. Re-running after a resize or a scroll keeps the
// decision in step with the view; the CSS transition animates the change.
function updateBackPlacement() {
  const view = document.getElementById('view');
  const button = view && view.querySelector('.settings-back');
  if (!view || !button) {
    return;
  }
  const inMargin = settingsBackInMargin({
    scrollerWidth: view.clientWidth,
    buttonWidth: button.offsetWidth,
  });
  button.classList.toggle('is-in-margin', inMargin);
}

function scheduleBackPlacement() {
  if (backPlacementFrame !== null) {
    return;
  }
  backPlacementFrame = requestAnimationFrame(() => {
    backPlacementFrame = null;
    updateBackPlacement();
  });
}

// ------------------------------------------------------------------ navigation

export function openSettings() {
  state.page = 'settings';
  renderRail();
  renderHeader();
  renderView();
  loadDeviceSettings();
}

export function initDeviceSettings() {
  listen('overlay://device-settings', (payload) => {
    if (!payload || typeof payload !== 'object') {
      return;
    }
    deviceSettings = payload;
    // The update status lives in core/update-status.js: the « À propos »
    // section reads it back from there.
    setUpdateStatus(payload.update);
    loadError = null;
    // Do not rebuild the controls under the pointer while a control is edited.
    // A guild switch paints the view once the new guild's info lands: painting
    // here would show the « Réglages » tab with the previous guild's settings.
    if (settingsShown() && !settingsControlActive() && !state.switchingGuild) {
      renderView();
    }
  });
  listen('overlay://navigate', (payload) => {
    if (payload && payload.view === 'settings') {
      openSettings();
    }
  });
  // A followed Discord profile was copied into the device profile: refresh the
  // page when it is open and idle. A focused field or an edited form keeps its
  // DOM, so the user's caret and typed value survive.
  document.addEventListener(PROFILE_SYNCED_EVENT, (event) => {
    const patch = event.detail;
    if (!patch || !deviceSettings) {
      return;
    }
    deviceSettings = {
      ...deviceSettings,
      profile: { ...(deviceSettings.profile || {}), ...patch },
    };
    if (settingsShown() && !settingsControlActive() && !settingsFormDirty() && !state.switchingGuild) {
      renderView();
    }
  });
  subscribeTheme(syncLogo);
  // « Retour » moves between the flow and the left margin as the window
  // resizes; the same check follows the scroll so a late layout shift cannot
  // leave it in the wrong place.
  const view = document.getElementById('view');
  if (view) {
    view.addEventListener('scroll', scheduleBackPlacement, { passive: true });
  }
  window.addEventListener('resize', scheduleBackPlacement);
  // Dirty-tracking is delegated: the settings view is rebuilt often, and a
  // captured reference would go stale. Any edit marks the current form, even
  // after focus moves to a heading or another control.
  document.addEventListener('input', (event) => markSettingsDirty(event.target));
  document.addEventListener('change', (event) => markSettingsDirty(event.target));
  // The shell's settings are local and cheap: read them once at startup so the
  // guild's « Réglages » tab never opens on its loading line. If the user gets
  // there first, `loading` makes this call a no-op and the tab renders when the
  // first read lands.
  queueMicrotask(loadDeviceSettings);
}

export async function loadDeviceSettings() {
  if (loading) {
    return;
  }
  loading = true;
  loadError = null;
  if (settingsShown()) {
    renderView();
  }
  try {
    const view = await invoke('settings_get');
    if (!view || typeof view !== 'object') {
      throw new Error('Réponse inattendue du shell.');
    }
    deviceSettings = view;
    setUpdateStatus(view.update);
  } catch (error) {
    loadError = errorMessage(error);
  } finally {
    loading = false;
  }
  if (settingsShown()) {
    renderView();
  }
}

// ---------------------------------------------------------------- settings set

async function sendSettings(patch, options = {}) {
  try {
    const response = await invoke('settings_set', { payload: patch });
    if (response && response.settings) {
      deviceSettings = response.settings;
    }
    if (response && response.ok === false && !options.inline) {
      notify(response.message || 'Valeur invalide.', true);
    }
    return response;
  } catch (error) {
    if (!options.inline) {
      notify(errorMessage(error), true);
    }
    return null;
  }
}

export async function applyPatch(patch) {
  await sendSettings(patch);
  if (settingsShown() && !settingsControlActive()) {
    renderView();
  }
}

export function debounce(key, action) {
  clearTimeout(debounceTimers.get(key));
  debounceTimers.set(
    key,
    setTimeout(() => {
      debounceTimers.delete(key);
      action();
    }, SLIDER_DEBOUNCE_MS),
  );
}

// Layout of a guild on this device: its own, else the device one.
export function guildLayout(settings, guildId) {
  const guild = guildId && settings && settings.guilds ? settings.guilds[guildId] : null;
  return (guild && guild.layout) || (settings && settings.layout) || {};
}

// Sends the draft layout (saved layout plus `changes`) to the overlay, at most
// once per frame. Nothing is saved here. A timer rather than
// requestAnimationFrame, which stops while the page is hidden. With a
// `guildId` the draft is that guild's layout.
export function previewLayout(changes, guildId = null) {
  const saved = guildId ? guildLayout(deviceSettings, guildId) : (deviceSettings && deviceSettings.layout) || {};
  previewDraft = { ...saved, ...previewDraft, ...changes };
  previewGuildId = guildId;
  if (previewFrame !== null) {
    return;
  }
  previewFrame = setTimeout(() => {
    previewFrame = null;
    const layout = previewDraft;
    const target = previewGuildId;
    previewDraft = null;
    const payload = target ? { layout, guildId: target } : { layout };
    invoke('settings_preview_layout', { payload }).catch((error) => {
      console.warn('Aperçu de la mise en page impossible :', errorMessage(error));
    });
  }, PREVIEW_FRAME_MS);
}

// ------------------------------------------------------------------ rendering

export function section(title, ...children) {
  const id = sectionId(title);
  return h(
    'section',
    { class: 'settings-section', 'aria-labelledby': id },
    h('h3', { class: 'settings-title', id }, title),
    ...children.flat(Infinity),
  );
}

// Accent- and case-insensitive id, so a section can name the heading it holds.
function sectionId(title) {
  return `settings-${title
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')}`;
}

// The page saves as its controls are used: one line under the title says so,
// instead of a note repeated above every section.
export function autosaveNote() {
  return h('p', { class: 'settings-tag' }, 'Paramètres sauvegardés automatiquement');
}

// An explanation, inside the control column of the control it belongs to.
// The wrapper spans the flex line so the sentence always starts a new line
// under its control, while `--measure` keeps it readable.
export function hint(text) {
  return h('div', { class: 'settings-hint-wrap' }, h('p', { class: 'settings-hint' }, text));
}

// Boolean setting: the row label names it, `.switch` draws the state.
export function switchRow(label, input, id, text) {
  return controlRow(label, [h('label', { class: 'switch' }, input), text ? hint(text) : null], id);
}

export function controlRow(label, control, id) {
  const labelNode = id
    ? h('label', { class: 'settings-label', for: id }, label)
    : h('span', { class: 'settings-label' }, label);
  return h('div', { class: 'settings-row' }, labelNode, h('div', { class: 'settings-control' }, control));
}

export function rangeControl({ id, min, max, value, format, onInput, defaultValue }) {
  const output = h('span', { class: 'settings-value' }, format(value));
  const input = h('input', {
    id,
    class: 'settings-range',
    type: 'range',
    min: String(min),
    max: String(max),
    step: '1',
    value: String(value),
    onInput: (event) => {
      const next = Number(event.target.value);
      output.textContent = format(next);
      onInput(next);
    },
  });
  resetOnDoubleClick(input, defaultValue);
  return [input, output];
}

export function selectedDisplay(settings, displays) {
  return (
    displays.find((display) => display.key === settings.selectedDisplayKey) ||
    displays.find((display) => display.primary) ||
    displays[0] ||
    null
  );
}

export function isWholeScreen(zone) {
  return zone.x === 0 && zone.y === 0 && zone.width === 1 && zone.height === 1;
}

// « 1280 × 720 px à 640, 360 » on the overlay screen, or fractions when its
// size is unknown.
export function zoneSummary(zone, display) {
  if (isWholeScreen(zone)) {
    return 'Tout l’écran';
  }
  if (!display || !display.width || !display.height) {
    const percent = (value) => `${Math.round(value * 100)} %`;
    return `${percent(zone.width)} × ${percent(zone.height)} de l’écran`;
  }
  const px = (fraction, side) => Math.round(fraction * side);
  return `${px(zone.width, display.width)} × ${px(zone.height, display.height)} px à ${px(zone.x, display.width)}, ${px(zone.y, display.height)}`;
}

function renderDisplay(settings) {
  const displays = Array.isArray(settings.displays) ? settings.displays : [];
  const layout = settings.layout || {};
  const zone = { x: 0, y: 0, width: 1, height: 1, ...(layout.zone || {}) };
  const placing = Boolean(settings.placing);

  const screen = h(
    'select',
    {
      id: 'settings-screen',
      class: 'field is-long',
      disabled: placing,
      // The overlay moves once the screen is saved: preview it there.
      onChange: (event) =>
        applyPatch({ selectedDisplayKey: event.target.value || null }).then(() => previewLayout({})),
    },
    h('option', { value: '' }, 'Écran principal'),
    displays.map((display) => h('option', { value: display.key }, display.label)),
  );
  screen.value = settings.selectedDisplayKey || '';

  const identify = h(
    'button',
    { class: 'btn', type: 'button', disabled: identifyingDisplays, onClick: identifyDisplays },
    identifyingDisplays ? 'Identification…' : 'Identifier les écrans',
  );

  const zoneActions = h(
    'div',
    { class: 'settings-zone-actions' },
    placing
      ? h('button', { class: 'btn', type: 'button', onClick: cancelPlacement }, 'Annuler le placement')
      : [
          h(
            'button',
            { class: 'btn is-primary', type: 'button', disabled: startingPlacement, onClick: startPlacement },
            startingPlacement ? 'Ouverture…' : 'Placer sur l’écran',
          ),
          isWholeScreen(zone)
            ? null
            : h(
                'button',
                {
                  class: 'btn is-quiet',
                  type: 'button',
                  onClick: () => applyPatch({ layout: { zone: null } }).then(() => previewLayout({})),
                },
                'Revenir à tout l’écran',
              ),
        ],
  );

  const zoneControls = [
    placing
      ? h('span', { class: 'settings-zone is-live' }, 'Placement en cours sur l’écran de l’overlay')
      : h('span', { class: 'settings-zone' }, zoneSummary(zone, selectedDisplay(settings, displays))),
    zoneActions,
  ];

  const alignment = h(
    'select',
    {
      id: 'settings-align',
      class: 'field',
      onChange: (event) => {
        previewLayout({ align: event.target.value });
        applyPatch({ layout: { align: event.target.value } });
      },
    },
    ALIGNMENTS.map(([value, label]) => h('option', { value }, label)),
  );
  alignment.value = layout.align || 'center';

  const showText = h('input', {
    id: 'settings-show-text',
    type: 'checkbox',
    checked: Boolean(settings.showText),
    onChange: (event) => applyPatch({ showText: event.target.checked }),
  });

  // On by default: only an explicit `false` turns the hover preview off.
  const hoverPreview = h('input', {
    id: 'settings-hover-preview',
    type: 'checkbox',
    checked: settings.hoverPreview !== false,
    onChange: (event) => {
      setPreviewPlaybackEnabled(event.target.checked);
      applyPatch({ hoverPreview: event.target.checked });
    },
  });

  // On by default: only an explicit `false` hides the floating button.
  const backToTop = h('input', {
    id: 'settings-back-to-top',
    type: 'checkbox',
    checked: settings.backToTop !== false,
    onChange: (event) => applyPatch({ backToTop: event.target.checked }),
  });

  const testButton = h(
    'button',
    { class: 'btn', type: 'button', disabled: testingMedia || placing, onClick: testMedia },
    testingMedia ? 'Lancement…' : 'Afficher',
  );

  const screenRow = controlRow(
    'Écran',
    [
      screen,
      identify,
      hint('Affiche un numéro sur chaque écran pendant quelques secondes.'),
    ],
    'settings-screen',
  );
  screenRow.classList.add('settings-display-row');

  const zoneRow = controlRow('Zone d’affichage', [
    ...zoneControls,
    hint(
      placing
        ? 'Déplace la zone et tire ses coins sur l’écran de l’overlay, puis valide (Entrée) ou annule (Échap).'
        : 'Les médias, leur auteur et leur légende restent dans cette zone. Place-la directement sur l’écran, comme une fenêtre.',
    ),
  ]);
  zoneRow.classList.add('settings-display-row');

  const alignmentRow = controlRow('Alignement dans la zone', alignment, 'settings-align');
  alignmentRow.classList.add('settings-display-row');

  return section(
    'Affichage',
    screenRow,
    h('div', { class: 'settings-zone-group' }, zoneRow, alignmentRow),
    switchRow('Texte des médias', showText, 'settings-show-text'),
    switchRow(
      'Ouvrir l’aperçu au survol',
      hoverPreview,
      'settings-hover-preview',
      'Désactivé : un clic ouvre les détails, un double-clic joue le média sur les overlays.',
    ),
    switchRow(
      'Bouton « Revenir en haut »',
      backToTop,
      'settings-back-to-top',
      'Désactivé : la board ne montre plus le bouton flottant qui ramène la grille en haut.',
    ),
    controlRow('Média de test', [testButton]),
  );
}

// Keeps the two preview switches in step with the hover-preview switch without
// re-rendering the page: the clicked control keeps the focus, which makes
// applyPatch skip the re-render.
function setPreviewPlaybackEnabled(enabled) {
  for (const id of ['settings-preview-autoplay', 'settings-preview-sound']) {
    const input = document.getElementById(id);
    if (input) {
      input.disabled = !enabled;
    }
  }
  const note = document.getElementById('settings-preview-disabled');
  if (note) {
    note.hidden = enabled;
  }
}

function renderPlayback(settings) {
  // Without the hover preview there is no preview player, so its two switches
  // stay visible (with their saved values) but cannot be used.
  const previewEnabled = previewPlaybackEnabled(settings);
  const previewAutoplay = h('input', {
    id: 'settings-preview-autoplay',
    type: 'checkbox',
    checked: settings.previewAutoplay !== false,
    disabled: !previewEnabled,
    onChange: (event) => applyPatch({ previewAutoplay: event.target.checked }),
  });
  const previewSound = h('input', {
    id: 'settings-preview-sound',
    type: 'checkbox',
    checked: settings.previewSound === true,
    disabled: !previewEnabled,
    onChange: (event) => applyPatch({ previewSound: event.target.checked }),
  });
  const detailAutoplay = h('input', {
    id: 'settings-detail-autoplay',
    type: 'checkbox',
    checked: settings.detailAutoplay !== false,
    onChange: (event) => applyPatch({ detailAutoplay: event.target.checked }),
  });
  const detailSound = h('input', {
    id: 'settings-detail-sound',
    type: 'checkbox',
    checked: settings.detailSound === true,
    onChange: (event) => applyPatch({ detailSound: event.target.checked }),
  });
  const previewNote = h(
    'div',
    { class: 'settings-hint-wrap', id: 'settings-preview-disabled' },
    h(
      'p',
      { class: 'settings-hint' },
      'L’aperçu au survol est désactivé : ces deux réglages ne s’appliquent pas. Active « Ouvrir l’aperçu au survol » pour les utiliser.',
    ),
  );
  previewNote.hidden = previewEnabled;
  return section(
    'Lecture',
    previewNote,
    switchRow(
      'Lancer les aperçus automatiquement',
      previewAutoplay,
      'settings-preview-autoplay',
      'Désactivé : les aperçus restent sur leur première image ; un clic lance la lecture.',
    ),
    switchRow(
      'Son des aperçus',
      previewSound,
      'settings-preview-sound',
      'Désactivé : les aperçus démarrent en silence ; un audio attend qu’on active le son.',
    ),
    switchRow(
      'Lancer les détails automatiquement',
      detailAutoplay,
      'settings-detail-autoplay',
      'Désactivé : les détails s’ouvrent en pause ; la lecture démarre avec le bouton du lecteur.',
    ),
    switchRow(
      'Son des détails',
      detailSound,
      'settings-detail-sound',
      'Désactivé : les détails s’ouvrent en silence. Un audio ne démarre pas seul tant que ce son est coupé.',
    ),
    hint(
      'Les lecteurs démarrent au plus bas des volumes général et de la guilde, sur une échelle de 0 à 100 %.',
    ),
  );
}

function renderSound(settings) {
  const volume = Number.isFinite(settings.volume) ? settings.volume : 1;
  const generalMute = h('input', {
    id: 'settings-general-mute',
    type: 'checkbox',
    checked: Boolean(settings.generalMute),
    onChange: (event) => applyPatch({ generalMute: event.target.checked }),
  });
  const mediaMute = h('input', {
    id: 'settings-media-mute',
    type: 'checkbox',
    checked: Boolean(settings.mediaMuted),
    onChange: (event) => applyPatch({ mediaMuted: event.target.checked }),
  });
  const guilds = Array.isArray(state.guilds) ? state.guilds : [];
  return section(
    'Son',
    controlRow(
      'Volume général',
      [
        ...rangeControl({
          id: 'settings-volume',
          min: 0,
          max: 100,
          value: Math.round(volume * 100),
          format: (value) => `${value} %`,
          defaultValue: DEFAULT_VOLUME_PERCENT,
          onInput: (value) => debounce('volume', () => applyPatch({ volume: value / 100 })),
        }),
        hint(SLIDER_RESET_HINT),
      ],
      'settings-volume',
    ),
    switchRow('Son coupé', mediaMute, 'settings-media-mute'),
    switchRow(
      'Couper tous les médias ici',
      generalMute,
      'settings-general-mute',
      'Ferme les connexions : plus rien n’est reçu ni affiché.',
    ),
    guilds.length === 0
      ? hint('Aucune guilde rejointe pour le moment.')
      : [
          h('p', { class: 'settings-subtitle' }, 'Volume par guilde'),
          guilds.map((guild) => renderGuildVolume(guild, settings)),
        ],
  );
}

function renderGuildVolume(guild, settings) {
  const guildId = guild.membership.guildId;
  const label = guildLabel(guild);
  const own = (settings.guilds && settings.guilds[guildId]) || {};
  const volume = Number.isFinite(own.volume) ? own.volume : 1;
  const mediaMute = h('input', {
    type: 'checkbox',
    'aria-label': `Couper le son des médias de ${label}`,
    checked: Boolean(own.mediaMuted),
    onChange: (event) => applyPatch({ guilds: { [guildId]: { mediaMuted: event.target.checked } } }),
  });
  const mute = h('input', {
    type: 'checkbox',
    'aria-label': `Couper ${label} sur cet appareil`,
    checked: Boolean(guild.membership.muted),
    onChange: (event) => toggleGuildMute(guild, event.target.checked),
  });
  const sliderId = `settings-guild-volume-${guildId}`;
  // Same grid as the rows above, so the guild slider and its value line up
  // with « Volume général »; the guild switches follow the value.
  return h(
    'div',
    { class: 'settings-row' },
    h('label', { class: 'settings-label settings-guild-name ellipsis', for: sliderId, title: label }, label),
    h(
      'div',
      { class: 'settings-control' },
      ...rangeControl({
        id: sliderId,
        min: 0,
        max: 100,
        value: Math.round(volume * 100),
        format: (value) => `${value} %`,
        defaultValue: DEFAULT_VOLUME_PERCENT,
        onInput: (value) =>
          debounce(`guildVolume:${guildId}`, () => applyPatch({ guilds: { [guildId]: { volume: value / 100 } } })),
      }),
      h('label', { class: 'switch settings-guild-mute' }, mute, 'Coupée ici'),
      h('label', { class: 'switch settings-guild-mute' }, mediaMute, 'Son coupé'),
    ),
  );
}

function renderShortcuts(settings) {
  const shortcuts = settings.shortcuts || {};
  return section(
    'Raccourcis',
    h(
      'div',
      { class: 'settings-shortcuts' },
      SHORTCUTS.map(([action, label]) => renderShortcutRow(action, label, shortcuts[action] || null)),
    ),
    hint(ALTGR_HINT),
  );
}

function renderShortcutRow(action, label, accelerator) {
  const capturing = shortcutCapture === action;
  const key = `shortcuts.${action}`;
  const message = shortcutErrors[key];
  const value = capturing
    ? null
    : h(
        'span',
        { class: 'settings-shortcut-value' },
        accelerator
          ? h('kbd', null, shortcutLabel(accelerator))
          : h('span', { class: 'meta' }, 'Aucun'),
      );
  const captureHint = capturing
    ? h(
        'p',
        { class: 'settings-hint is-inline settings-shortcut-capture' },
        'Appuie sur la combinaison… (Échap seul pour annuler)',
      )
    : null;
  return h(
    'div',
    { class: 'settings-row settings-shortcut-row' },
    h('span', { class: 'settings-label' }, label),
    h(
      'div',
      { class: 'settings-control is-wrap' },
      value,
      h(
        'button',
        {
          class: 'btn settings-shortcut-edit',
          type: 'button',
          disabled: capturing,
          onClick: () => startShortcutCapture(action),
        },
        'Modifier',
      ),
      h(
        'button',
        {
          class: 'btn is-quiet settings-shortcut-clear',
          type: 'button',
          disabled: capturing,
          onClick: () => clearShortcut(action),
        },
        'Effacer',
      ),
      captureHint,
      message ? h('p', { class: 'error-text' }, message) : null,
    ),
  );
}

function renderProfile(settings) {
  const profile = settings.profile || {};
  let avatar = profile.avatar || null;
  const preview = h('span', { class: 'settings-avatar' }, avatarNode(profile.displayName || '?', avatar));
  const name = h('input', {
    id: 'settings-profile-name',
    class: 'field',
    maxlength: '40',
    placeholder: 'Ton pseudo',
    value: profile.displayName || '',
    onInput: () => {
      forgetFollower();
      preview.replaceChildren(avatarNode(name.value || '?', avatar));
      debounce('profile.displayName', () => applyPatch({ profile: { displayName: name.value.trim() || null } }));
    },
  });
  const file = h('input', {
    type: 'file',
    accept: 'image/png,image/jpeg,image/webp',
    hidden: true,
    onChange: async (event) => {
      const picked = event.target.files && event.target.files[0];
      event.target.value = '';
      if (!picked) {
        return;
      }
      const resized = await resizeAvatar(picked);
      if (!resized) {
        notify('Cette image ne peut pas être lue.', true);
        return;
      }
      forgetFollower();
      const response = await sendSettings({ profile: { avatar: resized } });
      if (response && response.ok && state.page === 'settings') {
        renderView();
      }
    },
  });
  const pick = h('button', { class: 'btn', type: 'button', onClick: () => file.click() }, 'Changer l’avatar');
  const remove = h(
    'button',
    {
      class: 'btn is-quiet is-danger',
      type: 'button',
      disabled: !avatar,
      onClick: async () => {
        forgetFollower();
        const response = await sendSettings({ profile: { avatar: null } });
        if (response && response.ok && state.page === 'settings') {
          renderView();
        }
      },
    },
    'Retirer l’avatar',
  );
  // The avatar sits in the label column, the field and its buttons in the
  // control column: the name lines up with every other setting.
  return section(
    'Profil',
    h(
      'div',
      { class: 'settings-row' },
      h(
        'div',
        { class: 'settings-profile-id' },
        preview,
        h('label', { class: 'settings-label', for: 'settings-profile-name' }, 'Pseudo'),
      ),
      h('div', { class: 'settings-control' }, name, pick, remove, file),
    ),
    hint('Chaque guilde garde son propre profil, modifiable dans « Mon profil ».'),
  );
}

function renderStartup(settings) {
  const autoStart = h('input', {
    id: 'settings-auto-start',
    type: 'checkbox',
    checked: Boolean(settings.autoStart),
    onChange: (event) => applyPatch({ autoStart: event.target.checked }),
  });
  return section(
    'Démarrage',
    switchRow('Lancer avec Windows', autoStart, 'settings-auto-start'),
  );
}

function renderAppearance() {
  const theme = h(
    'select',
    {
      id: 'settings-theme',
      class: 'field',
      onChange: (event) => {
        setThemePreference(event.target.value);
      },
    },
    h('option', { value: 'dark' }, 'Sombre'),
    h('option', { value: 'light' }, 'Clair'),
    h('option', { value: 'system' }, 'Système'),
  );
  theme.value = currentThemePreference();
  return section(
    'Apparence',
    controlRow('Thème', [theme, hint('« Système » suit le thème clair/sombre de Windows.')], 'settings-theme'),
  );
}

function renderAbout(settings) {
  const update = currentUpdateStatus() || {};
  const busy = checkingUpdates || update.state === 'checking' || update.state === 'downloading';
  const button =
    update.state === 'ready'
      ? h(
          'button',
          { class: 'btn', type: 'button', disabled: installingUpdate, onClick: installUpdate },
          `Redémarrer et installer la version ${update.availableVersion}`,
        )
      : h(
          'button',
          { class: 'btn', type: 'button', disabled: busy, onClick: checkUpdates },
          busy ? 'Recherche…' : 'Rechercher une mise à jour',
        );
  const message = updateError || update.message;
  return section(
    'À propos',
    h(
      'div',
      { class: 'settings-about' },
      h('img', { class: 'settings-logo', src: logoForTheme(), alt: '', width: '64', height: '64' }),
      h(
        'div',
        null,
        h('p', { class: 'settings-about-name' }, 'LiveChat'),
        h('p', { class: 'meta' }, `Version ${settings.version || '—'}`),
      ),
    ),
    h('div', { class: 'settings-control' }, button),
    message ? hint(message) : null,
  );
}

// « Retour » is a sibling of the heading, not a child of it: on a wide board it
// can leave the 880 px column and stick in the left margin while the page
// scrolls.
function settingsBack() {
  return h(
    'button',
    { class: 'btn is-quiet settings-back', type: 'button', onClick: () => selectGuild(state.activeGuildId) },
    icon('back'),
    'Retour',
  );
}

// Page head, shared by the three states of the page (loaded, loading,
// unavailable): the page title and its save status.
function renderHead() {
  return h(
    'div',
    { class: 'settings-head' },
    h('h2', null, 'Paramètres'),
    autosaveNote(),
  );
}

function settingsPage(...children) {
  return h('div', { class: 'settings' }, settingsBack(), renderHead(), ...children);
}

export function renderSettings() {
  // The button is measured once the view owns the new page: decide its place
  // after the current render task, when layout is available.
  queueMicrotask(scheduleBackPlacement);
  if (!deviceSettings) {
    if (loadError) {
      return settingsPage(
        h(
          'div',
          { class: 'empty' },
          h('h2', null, 'Paramètres indisponibles'),
          h('p', null, loadError),
          h('button', { class: 'btn', onClick: loadDeviceSettings }, 'Réessayer'),
        ),
      );
    }
    return settingsPage(h('p', { class: 'meta' }, 'Chargement des paramètres…'));
  }
  const settings = deviceSettings;
  // The order of the sections is fixed (theme.test.mjs pins Démarrage,
  // Apparence and À propos): Apparence stays between the two.
  return settingsPage(
    renderDisplay(settings),
    renderPlayback(settings),
    renderSound(settings),
    renderShortcuts(settings),
    renderProfile(settings),
    renderStartup(settings),
    renderAppearance(),
    renderAbout(settings),
  );
}

// -------------------------------------------------------------------- actions

async function identifyDisplays() {
  if (identifyingDisplays) {
    return;
  }
  identifyingDisplays = true;
  renderView();
  try {
    const result = await invoke('settings_identify_displays');
    const count = result && Number.isFinite(result.count) ? result.count : 0;
    notify(count === 0 ? 'Aucun écran détecté.' : `${count} écran${count > 1 ? 's' : ''} identifié${count > 1 ? 's' : ''}.`);
  } catch (error) {
    notify(errorMessage(error), true);
  } finally {
    identifyingDisplays = false;
    if (state.page === 'settings') {
      renderView();
    }
  }
}

async function testMedia() {
  if (testingMedia) {
    return;
  }
  testingMedia = true;
  renderView();
  try {
    const response = await invoke('settings_test_media');
    if (response && response.ok) {
      notify('Média de test lancé sur l’écran.');
    } else if (response && response.reason === 'overlay_busy') {
      notify('Un média est déjà à l’écran.', true);
    } else {
      notify('Impossible de lancer le média de test.', true);
    }
  } catch (error) {
    notify(errorMessage(error), true);
  } finally {
    testingMedia = false;
    if (settingsShown()) {
      renderView();
    }
  }
}

export function placementStarting() {
  return startingPlacement;
}

// Places the device zone, or `guildId`'s own zone.
export async function startPlacement(guildId = null) {
  if (startingPlacement) {
    return;
  }
  startingPlacement = true;
  renderView();
  try {
    const response = await invoke('settings_placement_start', guildId ? { payload: { guildId } } : {});
    if (response && response.ok === false) {
      notify(
        response.reason === 'overlay_not_ready'
          ? 'L’overlay n’est pas encore prêt. Réessaie dans un instant.'
          : 'Impossible de lancer le placement.',
        true,
      );
    }
  } catch (error) {
    notify(errorMessage(error), true);
  } finally {
    startingPlacement = false;
    if (settingsShown()) {
      renderView();
    }
  }
}

export async function cancelPlacement() {
  try {
    await invoke('settings_placement_cancel');
  } catch (error) {
    notify(errorMessage(error), true);
  }
}

export async function toggleGuildMute(guild, muted) {
  const result = await run(() =>
    invoke('guild_set_muted', { payload: { guildId: guild.membership.guildId, muted } }),
  );
  if (result) {
    applyGuilds(result.guilds);
  }
}

function startShortcutCapture(action) {
  shortcutCapture = action;
  renderView();
}

async function saveShortcut(action, accelerator, keyLabel = null) {
  const key = `shortcuts.${action}`;
  const response = await sendSettings({ shortcuts: { [action]: accelerator } }, { inline: true });
  if (response && response.ok === false) {
    shortcutErrors[response.field || key] = response.message || 'Raccourci refusé.';
  } else if (response && response.ok) {
    delete shortcutErrors[key];
    if (accelerator) {
      if (keyLabel) {
        writeShortcutLabel(accelerator, keyLabel);
      } else {
        forgetShortcutLabel(accelerator);
      }
    }
  }
  if (state.page === 'settings' && !settingsControlActive()) {
    renderView();
  }
}

async function clearShortcut(action) {
  shortcutCapture = null;
  await saveShortcut(action, null);
}

async function checkUpdates() {
  if (checkingUpdates) {
    return;
  }
  checkingUpdates = true;
  updateError = null;
  renderView();
  try {
    const status = await invoke('overlay_check_updates');
    if (status && typeof status === 'object') {
      setUpdateStatus(status);
    }
  } catch (error) {
    updateError = errorMessage(error);
  } finally {
    checkingUpdates = false;
  }
  if (state.page === 'settings') {
    renderView();
  }
}

// On success LiveChat quits and the installer relaunches it.
async function installUpdate() {
  if (installingUpdate) {
    return;
  }
  installingUpdate = true;
  updateError = null;
  renderView();
  try {
    await invoke('overlay_install_update');
  } catch (error) {
    updateError = errorMessage(error);
  } finally {
    installingUpdate = false;
  }
  if (state.page === 'settings') {
    renderView();
  }
}

document.addEventListener('keydown', (event) => {
  if (!shortcutCapture || state.page !== 'settings') {
    return;
  }
  event.preventDefault();
  if (cancelsCapture(event)) {
    shortcutCapture = null;
    renderView();
    return;
  }
  const accelerator = acceleratorFromEvent(event);
  if (accelerator === null) {
    return;
  }
  if (accelerator === UNSUPPORTED_KEY) {
    notify(UNSUPPORTED_KEY_MESSAGE, true);
    return;
  }
  if (accelerator === undefined) {
    notify(BARE_KEY_MESSAGE, true);
    return;
  }
  const action = shortcutCapture;
  const keyLabel = capturedKeyLabel(event);
  shortcutCapture = null;
  saveShortcut(action, accelerator, keyLabel);
});
