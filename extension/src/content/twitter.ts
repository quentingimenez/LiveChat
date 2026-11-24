import {
  renderButtonContent,
  setButtonLabel,
  type InjectedButtonState,
} from './brand-logo';
import { openComposeModal, type ComposeSubmitPayload, type ComposeSubmitResult } from './composeModal';
import { initInpageTheme } from './inpage-theme';
import { TWITTER_BUTTON_LOGO_SIZE_PX, twitterInpageStyles } from './inpage-styles';
import { normalizeTwitterStatusUrl } from '../lib/url';
import {
  onTrustedPageEvent,
  rememberActionTarget,
  resolveActionButtonTarget,
  resolveAllowedActionTarget,
} from './shared';
import { startObservedScanner } from './scan-scheduler';

// Blocs partagés importés du noyau commun.
import {
  attachLongPressCompose,
  buttonResetTimers,
  clearButtonClickTimer,
  clearButtonResetTimer,
  isAuthStateResponse,
  isShowToastPayload,
  scheduleSingleClickAction,
  showToast,
  type ToastLevel,
} from './shared-core';
// Divergence par site conservée volontairement : readRuntime / hasExtensionAuth /
// registerToastListener / sendQuick / sendCompose gèrent ici l'invalidation du
// contexte extension (TikTok) avec un comportement propre au site.
// setButtonState / resetButtonStateLater utilisent la taille de logo et le
// libellé propres au site.

const STYLE_ID = 'lce-twitter-style';
const BUTTON_ATTRIBUTE = 'data-lce-twitter-button';
const SLOT_ATTRIBUTE = 'data-lce-twitter-slot';
const FLOATING_BUTTON_ID = 'lce-twitter-floating-button';
const GET_AUTH_STATE_TYPE = 'lce/get-auth-state';
const AUTH_STATUS_CACHE_MS = 1500;





// Le logo remplit la pastille de 34 px sans la déborder, à la taille des icônes
// d'action de X (~19 px) qui l'entourent.




const ensureStyles = (): void => {
  if (document.getElementById(STYLE_ID)) {
    return;
  }

  const styleNode = document.createElement('style');
  styleNode.id = STYLE_ID;
  styleNode.textContent = twitterInpageStyles;
  document.head.appendChild(styleNode);
};





let toastListenerRegistered = false;
let authStateKnown = false;
let authStateHasSettings = false;
let authStateCheckedAt = 0;
let authStatePromise: Promise<boolean> | null = null;









const readRuntime = (): typeof browser.runtime | null => {
  try {
    if (typeof browser === 'undefined') {
      return null;
    }

    return browser.runtime || null;
  } catch {
    return null;
  }
};



const hasExtensionAuth = async (): Promise<boolean> => {
  const now = Date.now();

  if (authStateKnown && now - authStateCheckedAt < AUTH_STATUS_CACHE_MS) {
    return authStateHasSettings;
  }

  if (authStatePromise) {
    return authStatePromise;
  }

  const runtime = readRuntime();

  if (!runtime || typeof runtime.sendMessage !== 'function') {
    authStateKnown = true;
    authStateHasSettings = false;
    authStateCheckedAt = now;
    return false;
  }

  authStatePromise = (async () => {
    try {
      const response = (await runtime.sendMessage({
        type: GET_AUTH_STATE_TYPE,
      })) as unknown;

      const isReady = isAuthStateResponse(response) && response.ok && response.hasSettings;

      authStateKnown = true;
      authStateHasSettings = isReady;
      authStateCheckedAt = Date.now();
      return isReady;
    } catch {
      authStateKnown = true;
      authStateHasSettings = false;
      authStateCheckedAt = Date.now();
      return false;
    } finally {
      authStatePromise = null;
    }
  })();

  return authStatePromise;
};

const registerToastListener = (): void => {
  if (toastListenerRegistered) {
    return;
  }

  const runtime = readRuntime();

  if (!runtime || !runtime.onMessage || typeof runtime.onMessage.addListener !== 'function') {
    return;
  }

  try {
    runtime.onMessage.addListener((message: unknown) => {
      if (!isShowToastPayload(message)) {
        return;
      }

      if (typeof message.message !== 'string' || !message.message.trim()) {
        return;
      }

      const level: ToastLevel =
        message.level === 'success' ? 'success' : message.level === 'info' ? 'info' : 'error';
      showToast(level, message.message);
    });

    toastListenerRegistered = true;
  } catch {
    // Ignore runtime invalidation while extension reloads.
  }
};

const sendQuick = async (url: string): Promise<{ ok: boolean; message: string }> => {
  const targetUrl = resolveAllowedActionTarget('twitter', url, window.location.origin);

  if (!targetUrl) {
    return {
      ok: false,
      message: 'URL X / Twitter invalide.',
    };
  }

  const runtime = readRuntime();

  if (!runtime || typeof runtime.sendMessage !== 'function') {
    return {
      ok: false,
      message: 'Contexte extension invalide. Recharge la page puis réessaie.',
    };
  }

  try {
    const response = (await runtime.sendMessage({
      type: 'lce/send-quick',
      url: targetUrl,
      source: 'twitter',
    })) as { ok?: unknown; message?: unknown };

    if (!response || typeof response.ok !== 'boolean' || typeof response.message !== 'string') {
      return {
        ok: false,
        message: 'Réponse invalide du service worker.',
      };
    }

    return {
      ok: response.ok,
      message: response.message,
    };
  } catch (error) {
    return {
      ok: false,
      message: error instanceof Error ? error.message : 'Erreur de communication avec le service worker.',
    };
  }
};

const sendCompose = async (payload: ComposeSubmitPayload): Promise<ComposeSubmitResult> => {
  const targetUrl = resolveAllowedActionTarget('twitter', payload.url, window.location.origin);

  if (!targetUrl) {
    return {
      ok: false,
      message: 'URL X / Twitter invalide.',
    };
  }

  const runtime = readRuntime();

  if (!runtime || typeof runtime.sendMessage !== 'function') {
    return {
      ok: false,
      message: 'Contexte extension invalide. Recharge la page puis réessaie.',
    };
  }

  try {
    const response = (await runtime.sendMessage({
      type: 'lce/send-compose',
      url: targetUrl,
      source: 'twitter',
      text: payload.text,
      forceRefresh: payload.forceRefresh,
      saveToBoard: payload.saveToBoard,
      durationSec: payload.durationSec ?? undefined,
    })) as { ok?: unknown; message?: unknown };

    if (!response || typeof response.ok !== 'boolean' || typeof response.message !== 'string') {
      return {
        ok: false,
        message: 'Réponse invalide du service worker.',
      };
    }

    return {
      ok: response.ok,
      message: response.message,
    };
  } catch (error) {
    return {
      ok: false,
      message: error instanceof Error ? error.message : 'Erreur de communication avec le service worker.',
    };
  }
};






const setButtonState = (
  button: HTMLButtonElement,
  state: InjectedButtonState,
  title?: string,
): void => {
  clearButtonResetTimer(button);
  button.classList.remove('is-loading', 'is-success', 'is-error');
  renderButtonContent(button, state, TWITTER_BUTTON_LOGO_SIZE_PX);

  if (state === 'loading') {
    button.classList.add('is-loading');
  } else if (state === 'success') {
    button.classList.add('is-success');
  } else if (state === 'error') {
    button.classList.add('is-error');
  }

  setButtonLabel(button, title || 'Envoyer ce tweet vers LiveChat');
};

const resetButtonStateLater = (button: HTMLButtonElement, delayMs = 2200): void => {
  clearButtonResetTimer(button);

  const timer = window.setTimeout(() => {
    buttonResetTimers.delete(button);
    setButtonState(button, 'idle');
  }, delayMs);

  buttonResetTimers.set(button, timer);
};

const resolveTweetStatusUrl = (article: HTMLElement): string | null => {
  const links = Array.from(article.querySelectorAll<HTMLAnchorElement>('a[href*="/status/"]'));

  for (const link of links) {
    const href = link.getAttribute('href') || link.href;
    const normalized = normalizeTwitterStatusUrl(href, window.location.origin);

    if (normalized) {
      return normalized;
    }
  }

  return normalizeTwitterStatusUrl(window.location.href, window.location.origin);
};

const runQuickAction = async (button: HTMLButtonElement, resolveTargetUrl: () => string | null): Promise<void> => {
  if (button.disabled) {
    return;
  }

  button.disabled = true;
  setButtonState(button, 'loading', 'Envoi en cours...');

  try {
    const targetUrl = resolveTargetUrl();

    if (!targetUrl) {
      const message = 'Impossible de détecter l’URL du tweet.';
      setButtonState(button, 'error', message);
      showToast('error', message);
      resetButtonStateLater(button);
      return;
    }

    const response = await sendQuick(targetUrl);
    setButtonState(button, response.ok ? 'success' : 'error', response.message);
    if (!response.ok) {
      showToast('error', response.message);
    }
    resetButtonStateLater(button);
  } finally {
    window.setTimeout(() => {
      button.disabled = false;
    }, 300);
  }
};

const openComposeForButton = (
  button: HTMLButtonElement,
  resolveTargetUrl: () => string | null,
  sourceLabel: string,
): void => {
  if (button.disabled) {
    return;
  }

  const targetUrl = resolveTargetUrl();

  if (!targetUrl) {
    const message = 'Impossible de détecter l’URL du tweet.';
    setButtonState(button, 'error', message);
    showToast('error', message);
    resetButtonStateLater(button);
    return;
  }

  openComposeModal({
    initialUrl: targetUrl,
    sourceLabel,
    onSubmit: sendCompose,
    onSuccess: (message) => {
      setButtonState(button, 'success', message);
      resetButtonStateLater(button);
      showToast('success', message);
    },
    onError: (message) => {
      setButtonState(button, 'error', message);
      resetButtonStateLater(button);
      showToast('error', message);
    },
  });
};

const createActionButton = (article: HTMLElement): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'lce-button';
  setButtonState(button, 'idle');

  const resolveTargetUrl = (): string | null =>
    resolveActionButtonTarget(button, 'twitter', resolveTweetStatusUrl(article), window.location.origin);

  const longPressCompose = attachLongPressCompose({
    button,
    title: 'Envoyer vers LiveChat avec texte',
    placeholder: 'Ajouter un texte (optionnel)',
    onSubmit: async (text) => {
      if (button.disabled) {
        throw new Error('Envoi déjà en cours...');
      }

      button.disabled = true;
      setButtonState(button, 'loading', 'Envoi en cours...');

      try {
        const targetUrl = resolveTargetUrl();

        if (!targetUrl) {
          const message = 'Impossible de détecter l’URL du tweet.';
          setButtonState(button, 'error', message);
          showToast('error', message);
          resetButtonStateLater(button);
          throw new Error(message);
        }

        const response = await sendCompose({
          url: targetUrl,
          text,
          forceRefresh: false,
          saveToBoard: false,
          durationSec: null,
        });
        setButtonState(button, response.ok ? 'success' : 'error', response.message);
        if (!response.ok) {
          showToast('error', response.message);
        }
        resetButtonStateLater(button);

        if (!response.ok) {
          throw new Error(response.message);
        }
      } finally {
        window.setTimeout(() => {
          button.disabled = false;
        }, 300);
      }
    },
  });

  onTrustedPageEvent<MouseEvent>(button, 'click', (event) => {
    event.preventDefault();
    event.stopPropagation();

    if (event.detail > 1) {
      return;
    }

    if (longPressCompose.consumeSuppressedClick()) {
      return;
    }

    longPressCompose.closePopover();

    scheduleSingleClickAction(button, () => {
      void runQuickAction(button, resolveTargetUrl);
    });
  });

  onTrustedPageEvent<MouseEvent>(button, 'dblclick', (event) => {
    event.preventDefault();
    event.stopPropagation();
    clearButtonClickTimer(button);
    openComposeForButton(button, resolveTargetUrl, 'X / Twitter');
  });

  return button;
};

const createFloatingButton = (): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'lce-button lce-button-floating';
  setButtonState(button, 'idle');

  const resolveTargetUrl = (): string | null =>
    resolveActionButtonTarget(
      button,
      'twitter',
      normalizeTwitterStatusUrl(window.location.href, window.location.origin),
      window.location.origin,
    );

  const longPressCompose = attachLongPressCompose({
    button,
    title: 'Envoyer vers LiveChat avec texte',
    placeholder: 'Ajouter un texte (optionnel)',
    onSubmit: async (text) => {
      if (button.disabled) {
        throw new Error('Envoi déjà en cours...');
      }

      button.disabled = true;
      setButtonState(button, 'loading', 'Envoi en cours...');

      try {
        const targetUrl = resolveTargetUrl();

        if (!targetUrl) {
          const message = 'Impossible de détecter l’URL du tweet.';
          setButtonState(button, 'error', message);
          showToast('error', message);
          resetButtonStateLater(button);
          throw new Error(message);
        }

        const response = await sendCompose({
          url: targetUrl,
          text,
          forceRefresh: false,
          saveToBoard: false,
          durationSec: null,
        });
        setButtonState(button, response.ok ? 'success' : 'error', response.message);
        if (!response.ok) {
          showToast('error', response.message);
        }
        resetButtonStateLater(button);

        if (!response.ok) {
          throw new Error(response.message);
        }
      } finally {
        window.setTimeout(() => {
          button.disabled = false;
        }, 300);
      }
    },
  });

  onTrustedPageEvent<MouseEvent>(button, 'click', (event) => {
    event.preventDefault();
    event.stopPropagation();

    if (event.detail > 1) {
      return;
    }

    if (longPressCompose.consumeSuppressedClick()) {
      return;
    }

    longPressCompose.closePopover();

    scheduleSingleClickAction(button, () => {
      void runQuickAction(button, resolveTargetUrl);
    });
  });

  onTrustedPageEvent<MouseEvent>(button, 'dblclick', (event) => {
    event.preventDefault();
    event.stopPropagation();
    clearButtonClickTimer(button);
    openComposeForButton(button, resolveTargetUrl, 'X / Twitter');
  });

  return button;
};

const upsertTweetButton = (article: HTMLElement): boolean => {
  const actionGroup = article.querySelector<HTMLElement>('div[role="group"]');

  if (!actionGroup) {
    return false;
  }

  const statusUrl = resolveTweetStatusUrl(article);

  if (!statusUrl) {
    return false;
  }

  let slot = actionGroup.querySelector<HTMLElement>(`[${SLOT_ATTRIBUTE}]`);
  let button = actionGroup.querySelector<HTMLButtonElement>(`button[${BUTTON_ATTRIBUTE}]`);

  if (!slot) {
    slot = document.createElement('div');
    slot.className = 'lce-twitter-action-slot';
    slot.setAttribute(SLOT_ATTRIBUTE, '1');
  }

  if (!button) {
    button = createActionButton(article);
    button.setAttribute(BUTTON_ATTRIBUTE, '1');
  }

  if (!slot.contains(button)) {
    slot.appendChild(button);
  }

  if (slot.parentElement !== actionGroup) {
    actionGroup.appendChild(slot);
  }

  rememberActionTarget(button, statusUrl);
  return true;
};

const upsertFloatingButton = (targetUrl: string): void => {
  let floatingButton = document.getElementById(FLOATING_BUTTON_ID) as HTMLButtonElement | null;

  if (!floatingButton) {
    floatingButton = createFloatingButton();
    floatingButton.id = FLOATING_BUTTON_ID;
    document.body.appendChild(floatingButton);
  }

  rememberActionTarget(floatingButton, targetUrl);
};

const removeAllTwitterButtons = (): void => {
  const inlineButtons = Array.from(document.querySelectorAll<HTMLElement>(`[${BUTTON_ATTRIBUTE}]`));
  inlineButtons.forEach((node) => node.remove());

  const inlineSlots = Array.from(document.querySelectorAll<HTMLElement>(`[${SLOT_ATTRIBUTE}]`));
  inlineSlots.forEach((node) => node.remove());

  const floatingButton = document.getElementById(FLOATING_BUTTON_ID);

  if (floatingButton) {
    floatingButton.remove();
  }
};

const scanTweets = async (): Promise<void> => {
  const isReady = await hasExtensionAuth();

  if (!isReady) {
    removeAllTwitterButtons();
    return;
  }

  const tweetArticles = Array.from(document.querySelectorAll<HTMLElement>('article[data-testid="tweet"]'));
  let firstStatusUrl: string | null = normalizeTwitterStatusUrl(window.location.href, window.location.origin);
  let hasInlineButton = false;

  tweetArticles.forEach((article) => {
    const statusUrl = resolveTweetStatusUrl(article);

    if (!firstStatusUrl && statusUrl) {
      firstStatusUrl = statusUrl;
    }

    hasInlineButton = upsertTweetButton(article) || hasInlineButton;
  });

  if (!hasInlineButton && firstStatusUrl) {
    upsertFloatingButton(firstStatusUrl);
    return;
  }

  const floatingButton = document.getElementById(FLOATING_BUTTON_ID);
  if (floatingButton) {
    floatingButton.remove();
  }
};

void initInpageTheme();
registerToastListener();
startObservedScanner(scanTweets);
