// Device-wide settings page (Paramètres).
//
// A page of its own, not a guild tab: it works with zero guild and keeps the
// rail visible. Every change is applied immediately through `settings_set`;
// sliders are debounced, and the page always renders from the settings view
// returned by the shell.

import { run } from '../core/api.js';
import { acceleratorFromEvent, acceleratorLabel } from '../core/accelerators.js';
import { resizeAvatar } from '../core/avatar.js';
import { avatarNode, h } from '../core/dom.js';
import { invoke, listen } from '../core/ipc.js';
import { guildLabel, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { applyGuilds } from '../data.js';
import { renderHeader } from './header.js';
import { renderRail } from './rail.js';
import { renderView } from './view.js';

const POSITIONS = [
  ['center', 'Centre'],
  ['top', 'En haut'],
  ['bottom', 'En bas'],
  ['top-left', 'En haut à gauche'],
  ['top-right', 'En haut à droite'],
  ['bottom-left', 'En bas à gauche'],
  ['bottom-right', 'En bas à droite'],
];

const SHORTCUTS = [
  ['stop', 'Arrêter le média'],
  ['muteAll', 'Sourdine générale'],
];

// Resolved from this module, so it works from web/board.html and from the
// dev harness page alike.
const LOGO_URL = new URL('../assets/logo.svg', import.meta.url).href;

const SLIDER_DEBOUNCE_MS = 250;
const BARE_KEY_MESSAGE = 'Utilise Ctrl, Alt ou Maj avec une lettre, un chiffre ou une touche F.';

let deviceSettings = null;
let loading = false;
let loadError = null;
let shortcutCapture = null;
let shortcutErrors = {};
let identifyingDisplays = false;
let testingMedia = false;
let checkingUpdates = false;
let updateMessage = null;
const debounceTimers = new Map();

function errorMessage(error) {
  if (!error) {
    return 'Action impossible.';
  }
  return typeof error === 'string' ? error : error.message || String(error);
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
    loadError = null;
    // Do not rebuild the controls under the pointer while a control is edited.
    if (state.page === 'settings' && !settingsControlActive()) {
      renderView();
    }
  });
  listen('overlay://navigate', (payload) => {
    if (payload && payload.view === 'settings') {
      openSettings();
    }
  });
}

export async function loadDeviceSettings() {
  if (loading) {
    return;
  }
  loading = true;
  loadError = null;
  if (state.page === 'settings') {
    renderView();
  }
  try {
    const view = await invoke('settings_get');
    if (!view || typeof view !== 'object') {
      throw new Error('Réponse inattendue du shell.');
    }
    deviceSettings = view;
  } catch (error) {
    loadError = errorMessage(error);
  } finally {
    loading = false;
  }
  if (state.page === 'settings') {
    renderView();
  }
}

function settingsControlActive() {
  const active = document.activeElement;
  return Boolean(
    active &&
      active.closest &&
      active.closest('.settings') &&
      typeof active.matches === 'function' &&
      active.matches('input, select, textarea'),
  );
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

async function applyPatch(patch) {
  await sendSettings(patch);
  if (state.page === 'settings' && !settingsControlActive()) {
    renderView();
  }
}

function debounce(key, action) {
  clearTimeout(debounceTimers.get(key));
  debounceTimers.set(
    key,
    setTimeout(() => {
      debounceTimers.delete(key);
      action();
    }, SLIDER_DEBOUNCE_MS),
  );
}

// ------------------------------------------------------------------ rendering

function section(title, ...children) {
  return h('section', { class: 'settings-section' }, h('h3', null, title), ...children.flat(Infinity));
}

function controlRow(label, control, id) {
  const labelNode = id
    ? h('label', { class: 'settings-label', for: id }, label)
    : h('span', { class: 'settings-label' }, label);
  return h('div', { class: 'settings-row' }, labelNode, h('div', { class: 'settings-control' }, control));
}

function rangeControl({ id, min, max, value, format, onInput }) {
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
  return [input, output];
}

function renderDisplay(settings) {
  const displays = Array.isArray(settings.displays) ? settings.displays : [];
  const layout = settings.layout || {};
  const maxWidth = Number.isInteger(layout.maxWidth) ? layout.maxWidth : 100;
  const maxHeight = Number.isInteger(layout.maxHeight) ? layout.maxHeight : 100;
  const margin = Number.isInteger(layout.margin) ? layout.margin : 24;

  const screen = h(
    'select',
    {
      id: 'settings-screen',
      class: 'field',
      onChange: (event) => applyPatch({ selectedDisplayKey: event.target.value || null }),
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

  const position = h(
    'select',
    {
      id: 'settings-position',
      class: 'field',
      onChange: (event) => applyPatch({ layout: { anchor: event.target.value } }),
    },
    POSITIONS.map(([value, label]) => h('option', { value }, label)),
  );
  position.value = layout.anchor || 'center';

  const showText = h('input', {
    id: 'settings-show-text',
    type: 'checkbox',
    checked: Boolean(settings.showText),
    onChange: (event) => applyPatch({ showText: event.target.checked }),
  });

  const testButton = h(
    'button',
    { class: 'btn', type: 'button', disabled: testingMedia, onClick: testMedia },
    testingMedia ? 'Lancement…' : 'Lancer un média de test',
  );

  return section(
    'Affichage',
    controlRow('Écran', [screen, identify], 'settings-screen'),
    controlRow('Position', position, 'settings-position'),
    controlRow(
      'Largeur maximale',
      rangeControl({
        id: 'settings-max-width',
        min: 20,
        max: 100,
        value: maxWidth,
        format: (value) => `${value} %`,
        onInput: (value) => debounce('layout.maxWidth', () => applyPatch({ layout: { maxWidth: value } })),
      }),
      'settings-max-width',
    ),
    controlRow(
      'Hauteur maximale',
      rangeControl({
        id: 'settings-max-height',
        min: 20,
        max: 100,
        value: maxHeight,
        format: (value) => `${value} %`,
        onInput: (value) => debounce('layout.maxHeight', () => applyPatch({ layout: { maxHeight: value } })),
      }),
      'settings-max-height',
    ),
    controlRow(
      'Marge',
      [
        h('input', {
          id: 'settings-margin',
          class: 'field settings-number',
          type: 'number',
          min: '0',
          max: '200',
          step: '1',
          value: String(margin),
          onChange: (event) => {
            const value = Number(event.target.value);
            if (!Number.isInteger(value) || value < 0 || value > 200) {
              notify('La marge doit être un nombre entier entre 0 et 200 px.', true);
              renderView();
              return;
            }
            applyPatch({ layout: { margin: value } });
          },
        }),
        h('span', { class: 'meta' }, 'px'),
      ],
      'settings-margin',
    ),
    controlRow('Afficher le texte des médias', showText, 'settings-show-text'),
    controlRow('Média de test', testButton),
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
  const guilds = Array.isArray(state.guilds) ? state.guilds : [];
  return section(
    'Son',
    controlRow('Sourdine générale', generalMute, 'settings-general-mute'),
    controlRow(
      'Volume général',
      rangeControl({
        id: 'settings-volume',
        min: 0,
        max: 100,
        value: Math.round(volume * 100),
        format: (value) => `${value} %`,
        onInput: (value) => debounce('volume', () => applyPatch({ volume: value / 100 })),
      }),
      'settings-volume',
    ),
    guilds.length === 0
      ? h('p', { class: 'settings-hint' }, 'Aucune guilde rejointe pour le moment.')
      : [
          h('p', { class: 'settings-subtitle' }, 'Volume par guilde'),
          guilds.map((guild) => renderGuildVolume(guild, settings)),
        ],
  );
}

function renderGuildVolume(guild, settings) {
  const guildId = guild.membership.guildId;
  const label = guildLabel(guild);
  const stored = settings.guildVolumes && settings.guildVolumes[guildId];
  const volume = Number.isFinite(stored) ? stored : 1;
  const mute = h('input', {
    type: 'checkbox',
    'aria-label': `Sourdine de ${label}`,
    checked: Boolean(guild.membership.muted),
    onChange: (event) => toggleGuildMute(guild, event.target.checked),
  });
  const sliderId = `settings-guild-volume-${guildId}`;
  // Same grid as the rows above, so the guild slider and its value line up
  // with « Volume général »; the guild mute follows the value.
  return h(
    'div',
    { class: 'settings-row' },
    h('label', { class: 'settings-label settings-guild-name ellipsis', for: sliderId, title: label }, label),
    h(
      'div',
      { class: 'settings-control' },
      rangeControl({
        id: sliderId,
        min: 0,
        max: 100,
        value: Math.round(volume * 100),
        format: (value) => `${value} %`,
        onInput: (value) =>
          debounce(`guildVolume:${guildId}`, () => applyPatch({ guildVolumes: { [guildId]: value / 100 } })),
      }),
      h('label', { class: 'switch settings-guild-mute' }, mute, 'Sourdine'),
    ),
  );
}

function renderShortcuts(settings) {
  const shortcuts = settings.shortcuts || {};
  return section(
    'Raccourcis',
    SHORTCUTS.map(([action, label]) => renderShortcutRow(action, label, shortcuts[action] || null)),
  );
}

function renderShortcutRow(action, label, accelerator) {
  const capturing = shortcutCapture === action;
  const key = `shortcuts.${action}`;
  const message = shortcutErrors[key];
  const value = capturing
    ? h('span', { class: 'settings-hint is-inline' }, 'Appuie sur la combinaison… (Échap pour annuler)')
    : accelerator
      ? h('kbd', null, acceleratorLabel(accelerator))
      : h('span', { class: 'meta' }, 'Aucun');
  return h(
    'div',
    { class: 'settings-row' },
    h('span', { class: 'settings-label' }, label),
    h(
      'div',
      { class: 'settings-control is-wrap' },
      value,
      h(
        'button',
        { class: 'btn', type: 'button', disabled: capturing, onClick: () => startShortcutCapture(action) },
        'Modifier',
      ),
      h(
        'button',
        { class: 'btn is-quiet', type: 'button', disabled: capturing, onClick: () => clearShortcut(action) },
        'Effacer',
      ),
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
      const response = await sendSettings({ profile: { avatar: resized } });
      if (response && response.ok && state.page === 'settings') {
        renderView();
      }
    },
  });
  const remove = h(
    'button',
    {
      class: 'btn is-quiet',
      type: 'button',
      disabled: !avatar,
      onClick: async () => {
        const response = await sendSettings({ profile: { avatar: null } });
        if (response && response.ok && state.page === 'settings') {
          renderView();
        }
      },
    },
    'Retirer l’avatar',
  );
  return section(
    'Profil',
    h(
      'div',
      { class: 'settings-profile' },
      preview,
      h(
        'div',
        { class: 'settings-profile-fields' },
        h('label', { class: 'settings-field-label', for: 'settings-profile-name' }, 'Pseudo par défaut'),
        name,
        h(
          'div',
          { class: 'settings-control is-wrap' },
          h('button', { class: 'btn', type: 'button', onClick: () => file.click() }, 'Choisir un avatar'),
          remove,
          file,
        ),
      ),
    ),
    h(
      'p',
      { class: 'settings-hint' },
      'Ce pseudo et cet avatar servent par défaut aux prochaines guildes que tu rejoins. Chaque guilde a son propre profil, modifiable dans « Mon profil ».',
    ),
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
    controlRow('Lancer LiveChat au démarrage de Windows', autoStart, 'settings-auto-start'),
  );
}

function renderAbout(settings) {
  const updatesDisabled = !settings.updaterEnabled;
  const button = h(
    'button',
    {
      class: 'btn',
      type: 'button',
      disabled: updatesDisabled || checkingUpdates,
      onClick: checkUpdates,
    },
    checkingUpdates ? 'Recherche…' : 'Rechercher une mise à jour',
  );
  return section(
    'À propos',
    h(
      'div',
      { class: 'settings-about' },
      h('img', { class: 'settings-logo', src: LOGO_URL, alt: '', width: '64', height: '64' }),
      h(
        'div',
        null,
        h('p', { class: 'settings-about-name' }, 'LiveChat'),
        h('p', { class: 'meta' }, `Version ${settings.version || '—'}`),
      ),
    ),
    h('div', { class: 'settings-control is-wrap' }, button),
    updatesDisabled
      ? h('p', { class: 'settings-hint' }, 'Les mises à jour automatiques arrivent bientôt.')
      : null,
    updateMessage ? h('p', { class: 'settings-hint' }, updateMessage) : null,
  );
}

export function renderSettings() {
  if (!deviceSettings) {
    if (loadError) {
      return h(
        'div',
        { class: 'settings' },
        h('div', { class: 'settings-head' }, h('h2', null, 'Paramètres')),
        h(
          'div',
          { class: 'empty' },
          h('h2', null, 'Paramètres indisponibles'),
          h('p', null, loadError),
          h('button', { class: 'btn', onClick: loadDeviceSettings }, 'Réessayer'),
        ),
      );
    }
    return h(
      'div',
      { class: 'settings' },
      h('div', { class: 'settings-head' }, h('h2', null, 'Paramètres')),
      h('p', { class: 'meta' }, 'Chargement des paramètres…'),
    );
  }
  const settings = deviceSettings;
  return h(
    'div',
    { class: 'settings' },
    h(
      'div',
      { class: 'settings-head' },
      h('h2', null, 'Paramètres'),
      h('p', { class: 'meta' }, 'Réglages de cet appareil, appliqués à tous tes overlays.'),
    ),
    renderDisplay(settings),
    renderSound(settings),
    renderShortcuts(settings),
    renderProfile(settings),
    renderStartup(settings),
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
    if (state.page === 'settings') {
      renderView();
    }
  }
}

async function toggleGuildMute(guild, muted) {
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

async function saveShortcut(action, accelerator) {
  const key = `shortcuts.${action}`;
  const response = await sendSettings({ shortcuts: { [action]: accelerator } }, { inline: true });
  if (response && response.ok === false) {
    shortcutErrors[response.field || key] = response.message || 'Raccourci refusé.';
  } else if (response && response.ok) {
    delete shortcutErrors[key];
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
  updateMessage = null;
  renderView();
  try {
    const status = await invoke('overlay_check_updates');
    updateMessage = status && status.message ? status.message : 'Aucune mise à jour disponible.';
  } catch (error) {
    updateMessage = errorMessage(error);
  } finally {
    checkingUpdates = false;
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
  if (event.key === 'Escape') {
    shortcutCapture = null;
    renderView();
    return;
  }
  const accelerator = acceleratorFromEvent(event);
  if (accelerator === null) {
    return;
  }
  if (accelerator === undefined) {
    notify(BARE_KEY_MESSAGE, true);
    return;
  }
  const action = shortcutCapture;
  shortcutCapture = null;
  saveShortcut(action, accelerator);
});
