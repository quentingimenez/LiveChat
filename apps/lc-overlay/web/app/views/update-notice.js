// Bandeau de mise à jour, en haut de la board.
//
// L'installation automatique (réglage `autoUpdate`, activée par défaut) rend
// ce bandeau inutile : quand elle est désactivée, il annonce le téléchargement
// puis propose d'installer la version prête. « Plus tard » ne masque que la
// version concernée, pour qu'une suivante réapparaisse.
//
// The shell owns the download: the banner never fetches anything, it only runs
// `overlay_install_update` and reads the status store fed by the device
// settings events.

import { h, icon } from '../core/dom.js';
import { invoke, listen } from '../core/ipc.js';
import {
  currentUpdateStatus,
  setUpdateStatus,
  subscribeUpdateStatus,
  updateNotice,
} from '../core/update-status.js';

// Postponed version, scoped to this device: the same version must not come
// back on every board open, but the next one must.
const DISMISSED_KEY = 'livechat.updateNotice.dismissedVersion';

let autoUpdate = true;
let installing = false;
let checking = false;
let installError = null;

function message(error) {
  if (error && typeof error === 'object' && error.message) {
    return String(error.message);
  }
  return String(error);
}

function dismissedVersion() {
  try {
    return localStorage.getItem(DISMISSED_KEY);
  } catch {
    return null;
  }
}

function postpone(version) {
  try {
    localStorage.setItem(DISMISSED_KEY, version);
  } catch {
    // A full or disabled localStorage only means the banner comes back.
  }
  installError = null;
  renderUpdateNotice();
}

async function installUpdate() {
  if (installing) {
    return;
  }
  installing = true;
  installError = null;
  renderUpdateNotice();
  try {
    await invoke('overlay_install_update');
  } catch (error) {
    installError = message(error);
  } finally {
    installing = false;
    renderUpdateNotice();
  }
}

async function checkAgain() {
  if (checking) {
    return;
  }
  checking = true;
  installError = null;
  renderUpdateNotice();
  try {
    const status = await invoke('overlay_check_updates');
    if (status && typeof status === 'object') {
      setUpdateStatus(status);
    }
  } catch (error) {
    installError = message(error);
  } finally {
    checking = false;
    renderUpdateNotice();
  }
}

function installButton() {
  return h(
    'button',
    {
      class: 'btn is-primary',
      type: 'button',
      disabled: installing,
      onClick: installUpdate,
    },
    installing ? 'Redémarrage…' : 'Redémarrer pour installer',
  );
}

function renderError(container, error) {
  container.className = 'update-notice is-error';
  container.replaceChildren(
    icon('refresh'),
    h('span', { class: 'update-notice-text' }, `Échec de la mise à jour : ${error}`),
    h('span', { class: 'spacer' }),
    h(
      'button',
      { class: 'btn is-quiet', type: 'button', disabled: checking, onClick: checkAgain },
      checking ? 'Recherche…' : 'Rechercher une mise à jour',
    ),
  );
}

function renderNotice(container, notice) {
  container.className = `update-notice is-${notice.kind}`;
  const children = [icon('refresh'), h('span', { class: 'update-notice-text' }, notice.message)];
  if (installError) {
    children.push(h('span', { class: 'update-notice-error' }, `Échec : ${installError}`));
  }
  if (notice.kind === 'ready') {
    children.push(
      h('span', { class: 'spacer' }),
      installButton(),
      h(
        'button',
        { class: 'btn is-quiet', type: 'button', onClick: () => postpone(notice.version) },
        'Plus tard',
      ),
    );
  }
  container.replaceChildren(...children);
}

// Redraws the banner from the store, the setting and the postponed version.
export function renderUpdateNotice() {
  const container = document.getElementById('update-notice');
  if (!container) {
    return;
  }
  const notice = updateNotice(currentUpdateStatus(), {
    autoUpdate,
    dismissedVersion: dismissedVersion(),
  });
  if (installError && !notice) {
    container.hidden = false;
    renderError(container, installError);
    return;
  }
  if (!notice) {
    container.hidden = true;
    container.className = 'update-notice';
    container.replaceChildren();
    return;
  }
  container.hidden = false;
  renderNotice(container, notice);
}

// Boot: read the setting once, then follow the shell's device settings. The
// store subscription redraws on every updater state change.
export function initUpdateNotice() {
  listen('overlay://device-settings', (payload) => {
    if (!payload || typeof payload !== 'object') {
      return;
    }
    autoUpdate = payload.autoUpdate !== false;
    renderUpdateNotice();
  });
  subscribeUpdateStatus(renderUpdateNotice);
  invoke('settings_get')
    .then((view) => {
      if (view && typeof view === 'object') {
        autoUpdate = view.autoUpdate !== false;
        setUpdateStatus(view.update);
      }
      renderUpdateNotice();
    })
    .catch(() => renderUpdateNotice());
}
