// « Update available » banner at the top of the board window.
//
// A slim line above the toolbar, on every page and tab of the main window. It
// is part of the layout (it never covers the header) and disappears entirely
// when there is nothing to say. The status comes from core/update-status.js —
// the same source as the Settings page. « Plus tard » postpones the install
// prompt for that version only (localStorage): a newer version shows the
// banner again.

import { h, icon } from '../core/dom.js';
import { invoke } from '../core/ipc.js';
import { currentUpdateStatus, setUpdateStatus, subscribeUpdateStatus, updateNotice } from '../core/update-status.js';

const DISMISSED_KEY = 'livechat.updateNotice.dismissedVersion';

let installing = false;
let installError = null;
// Notice of the install in flight or of the last failed one: the shell drops
// the `ready` phase once the installer is launched (or cannot be), so the
// banner keeps it to stay visible while it runs and to show the error.
let failedNotice = null;

function errorMessage(error) {
  if (!error) {
    return 'Action impossible.';
  }
  return typeof error === 'string' ? error : error.message || String(error);
}

function dismissedVersion() {
  try {
    return window.localStorage.getItem(DISMISSED_KEY) || null;
  } catch {
    return null;
  }
}

function postpone(version) {
  try {
    window.localStorage.setItem(DISMISSED_KEY, version);
  } catch {
    // Without storage the banner only comes back on the next status change.
  }
  installError = null;
  failedNotice = null;
  renderUpdateNotice();
}

// The notice to show. A newer update replaces the failed one; the same
// version keeps its error message, even once the shell moved the status away
// from `ready`. While an install runs, the banner stays on its notice.
function displayedNotice() {
  const notice = updateNotice(currentUpdateStatus(), dismissedVersion());
  if (installError && notice && notice.version !== failedNotice?.version) {
    installError = null;
    failedNotice = null;
  }
  return notice || (installError || installing ? failedNotice : null);
}

// On success LiveChat quits and the installer relaunches it; a failure is
// shown in the banner itself.
async function installUpdate() {
  if (installing) {
    return;
  }
  const notice = displayedNotice();
  if (!notice || notice.kind !== 'ready') {
    return;
  }
  installing = true;
  installError = null;
  failedNotice = notice; // the banner keeps showing it while it runs
  renderUpdateNotice();
  try {
    await invoke('overlay_install_update');
  } catch (error) {
    installError = errorMessage(error);
  } finally {
    installing = false;
    renderUpdateNotice();
  }
}

function renderUpdateNotice() {
  const root = document.getElementById('update-notice');
  if (!root) {
    return;
  }
  const shown = displayedNotice();
  if (!shown) {
    root.hidden = true;
    root.replaceChildren();
    return;
  }
  const actions =
    shown.kind === 'ready'
      ? [
          h(
            'button',
            { class: 'btn', type: 'button', disabled: installing, onClick: installUpdate },
            installing ? 'Redémarrer et installer…' : 'Redémarrer et installer',
          ),
          h('button', { class: 'btn is-quiet', type: 'button', onClick: () => postpone(shown.version) }, 'Plus tard'),
        ]
      : [];
  // replaceChildren() would print a `null` child as text: build the list.
  const children = [icon('refresh'), h('span', { class: 'update-notice-text', title: shown.message }, shown.message)];
  if (installError) {
    children.push(h('span', { class: 'update-notice-error' }, installError));
  }
  root.replaceChildren(...children, h('span', { class: 'spacer' }), ...actions);
  root.className = `update-notice is-${shown.kind}`;
  root.hidden = false;
}

export function initUpdateNotice() {
  subscribeUpdateStatus(renderUpdateNotice);
  renderUpdateNotice();
  // An update that became `ready` before the board listened is never
  // re-emitted (the updater skips a new check): read the current status once.
  invoke('settings_get')
    .then((view) => setUpdateStatus(view && view.update))
    .catch(() => {});
}
