// Thème clair des widgets injectés (toasts, bulle de composition, modale).
//
// Les content scripts dessinent chacun leurs styles « en dur », sur des sites
// qui imposent leurs propres couleurs : on n'y touche pas. Cette feuille ajoute
// des surcharges claires, préfixées par `html[data-lce-theme='light']`, donc
// plus spécifiques que les règles d'origine et actives seulement en thème clair.
//
// La préférence est la même que celle du popup et des options (`lib/theme.ts`).

import {
  THEME_STORAGE_KEY,
  normalizeThemePreference,
  readThemePreference,
  resolveTheme,
  systemPrefersLight,
  type ResolvedTheme,
} from '../lib/theme';

export const INPAGE_THEME_STYLE_ID = 'lce-theme-style';
const THEME_ATTRIBUTE = 'data-lce-theme';
const PREFERS_LIGHT_QUERY = '(prefers-color-scheme: light)';

export const inpageThemeStyles = `
html[data-lce-theme='light'] .lce-toast {
  color: #171b25;
  background: #ffffff;
  border-color: #d5dce8;
  border-left-color: #2f5bb7;
  box-shadow: 0 10px 28px rgba(24, 32, 60, 0.16);
}
html[data-lce-theme='light'] .lce-toast-success {
  background: #e8f6ef;
  border-color: #b4e0d0;
  border-left-color: #1f7d5c;
}
html[data-lce-theme='light'] .lce-toast-error {
  background: #fdeceb;
  border-color: #f0c4c0;
  border-left-color: #b3363f;
}
html[data-lce-theme='light'] .lce-toast-info {
  background: #eaf1fd;
  border-color: #c4d6f8;
  border-left-color: #2f5bb7;
}
html[data-lce-theme='light'] .lce-compose-popover {
  border-color: #d5dce8;
  background: #ffffff;
  box-shadow: 0 16px 40px rgba(24, 32, 60, 0.18);
  color: #171b25;
}
html[data-lce-theme='light'] .lce-compose-popover-title {
  color: #171b25;
}
html[data-lce-theme='light'] .lce-compose-popover-textarea {
  border-color: #d5dce8;
  background: #f7f9fc;
  color: #171b25;
}
html[data-lce-theme='light'] .lce-compose-popover-textarea:focus {
  border-color: #2f5bb7;
  box-shadow: 0 0 0 2px rgba(47, 91, 183, 0.18);
}
html[data-lce-theme='light'] .lce-compose-popover-textarea::placeholder {
  color: #6f7a90;
}
html[data-lce-theme='light'] .lce-compose-popover-error {
  color: #b3363f;
}
html[data-lce-theme='light'] .lce-compose-popover-button-cancel {
  background: #eef1f7;
  color: #171b25;
}
html[data-lce-theme='light'] .lce-compose-modal-overlay {
  background: rgba(23, 27, 37, 0.4);
}
html[data-lce-theme='light'] .lce-compose-modal {
  border-color: #d5dce8;
  background: #ffffff;
  color: #171b25;
  box-shadow: 0 22px 46px rgba(24, 32, 60, 0.2);
}
html[data-lce-theme='light'] .lce-compose-modal-source {
  color: #4f5972;
}
html[data-lce-theme='light'] .lce-compose-modal-close {
  background: #eef1f7;
  color: #171b25;
}
html[data-lce-theme='light'] .lce-compose-modal-close:hover {
  background: #e2e7f0;
}
html[data-lce-theme='light'] .lce-compose-modal-label {
  color: #3a455c;
}
html[data-lce-theme='light'] .lce-compose-modal-input,
html[data-lce-theme='light'] .lce-compose-modal-textarea {
  border-color: #d5dce8;
  background: #f7f9fc;
  color: #171b25;
}
html[data-lce-theme='light'] .lce-compose-modal-input:focus,
html[data-lce-theme='light'] .lce-compose-modal-textarea:focus {
  outline-color: rgba(47, 91, 183, 0.35);
  border-color: rgba(47, 91, 183, 0.9);
}
html[data-lce-theme='light'] .lce-compose-modal-checkbox {
  color: #3a455c;
}
html[data-lce-theme='light'] .lce-compose-modal-status-error {
  background: #fbe9ea;
  border-color: #ec9aa0;
  color: #8f2b33;
}
html[data-lce-theme='light'] .lce-compose-modal-button-cancel {
  background: #eef1f7;
  color: #171b25;
}
html[data-lce-theme='light'] .lce-compose-modal-button-cancel:hover {
  background: #e2e7f0;
}
`;

let started = false;

const applyTheme = (theme: ResolvedTheme): void => {
  if (typeof document === 'undefined' || !document.documentElement) {
    return;
  }

  document.documentElement.setAttribute(THEME_ATTRIBUTE, theme);
};

const ensureStyles = (): void => {
  if (typeof document === 'undefined' || document.getElementById(INPAGE_THEME_STYLE_ID)) {
    return;
  }

  const styleNode = document.createElement('style');
  styleNode.id = INPAGE_THEME_STYLE_ID;
  styleNode.textContent = inpageThemeStyles;
  (document.head || document.documentElement).appendChild(styleNode);
};

const syncTheme = async (): Promise<void> => {
  applyTheme(resolveTheme(await readThemePreference(), systemPrefersLight()));
};

// À appeler une fois au démarrage d'un content script : injecte les surcharges
// claires et garde le thème en phase avec le réglage partagé et le navigateur.
export const initInpageTheme = (): void => {
  if (started || typeof document === 'undefined') {
    return;
  }

  started = true;
  ensureStyles();
  void syncTheme();

  if (chrome?.storage?.onChanged) {
    chrome.storage.onChanged.addListener((changes, areaName) => {
      if (areaName !== 'local' || !(THEME_STORAGE_KEY in changes)) {
        return;
      }

      applyTheme(resolveTheme(normalizeThemePreference(changes[THEME_STORAGE_KEY]?.newValue), systemPrefersLight()));
    });
  }

  if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
    window.matchMedia(PREFERS_LIGHT_QUERY).addEventListener?.('change', () => {
      void syncTheme();
    });
  }
};
