// Remote guild-setting updates must never replace a form the user is editing.
// This banner keeps the form intact and offers an explicit reload. It also
// re-focuses itself after a refused save, so the reason the save failed is
// visible even if the user had stopped looking at the banner.

import { h } from '../core/dom.js';

const REMOTE_MESSAGE =
  'Les réglages de cette guilde ont été modifiés sur un autre appareil.';
const CONFLICT_MESSAGE =
  'Ta saisie n’a pas été enregistrée : les réglages ont été modifiés sur un autre appareil.';

function reduceMotion() {
  return window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

function focusBanner(banner) {
  banner.scrollIntoView({ block: 'nearest', behavior: reduceMotion() ? 'auto' : 'smooth' });
  banner.focus({ preventScroll: true });
  banner.classList.add('is-attention');
  window.setTimeout(() => banner.classList.remove('is-attention'), 1400);
}

export function showSettingsRemoteUpdate({ focus = false, conflict = false, onReload } = {}) {
  const root = document.querySelector('#view .settings');
  if (!root) {
    return null;
  }
  const previous = root.querySelector('.settings-banner');
  if (previous && !conflict) {
    // A second background change must not rebuild the banner (or steal focus
    // from the form); the existing reload button already points at the latest
    // loader through its closure.
    if (focus) {
      focusBanner(previous);
    }
    return previous;
  }
  // Recreate instead of reusing when the message changes to the refused-save
  // wording: the button closure must also call the latest `onReload`.
  if (previous) {
    previous.remove();
  }
  const reload = h(
    'button',
    {
      class: 'btn is-quiet',
      type: 'button',
      onClick: async () => {
        reload.disabled = true;
        banner.remove();
        const loaded = onReload ? await onReload() : false;
        if (!loaded) {
          // Keep the form and the warning if the refresh failed; the next
          // explicit click can retry.
          showSettingsRemoteUpdate({ onReload, conflict });
        }
      },
    },
    'Recharger',
  );
  const banner = h(
    'div',
    { class: 'settings-banner', role: 'status', tabindex: '-1' },
    h('span', null, conflict ? CONFLICT_MESSAGE : REMOTE_MESSAGE),
    reload,
  );
  root.prepend(banner);

  if (focus) {
    focusBanner(banner);
  }
  return banner;
}
