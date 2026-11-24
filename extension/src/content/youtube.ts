import {
  renderButtonContent,
  setButtonLabel,
  type InjectedButtonState,
} from './brand-logo';
import { openComposeModal, type ComposeSubmitPayload, type ComposeSubmitResult } from './composeModal';
import { initInpageTheme } from './inpage-theme';
import { YOUTUBE_BUTTON_LOGO_SIZE_PX, youtubeInpageStyles } from './inpage-styles';
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

const STYLE_ID = 'lce-youtube-style';
const BUTTON_ATTRIBUTE = 'data-lce-youtube-button';
const LEGACY_FLOATING_BUTTON_ID = 'lce-youtube-floating-button';
const SHORTS_FLOATING_BUTTON_ID = 'lce-youtube-shorts-floating-button';
const WATCH_FLOATING_BUTTON_ID = 'lce-youtube-watch-floating-button';
const WATCH_SLOT_ATTRIBUTE = 'data-lce-youtube-watch-slot';
const DEFAULT_BUTTON_TITLE = 'Envoyer la vidéo YouTube vers LiveChat';
const GET_AUTH_STATE_TYPE = 'lce/get-auth-state';
const AUTH_STATUS_CACHE_MS = 1500;





const WATCH_TARGET_SELECTORS = [
  'ytd-watch-metadata #top-level-buttons-computed',
  'ytd-watch-metadata #actions-inner',
  'ytd-watch-metadata #actions',
  '#above-the-fold #top-level-buttons-computed',
  'ytd-menu-renderer #top-level-buttons-computed',
] as const;
const SHORTS_TARGET_SELECTORS = [
  // Rendu 2026 : la rail d'actions est un « view model » (web component).
  'REEL-ACTION-BAR-VIEW-MODEL',
  // Anciens rendus, conservés en secours.
  'ytd-reel-player-overlay-renderer #actions',
  'ytd-reel-video-renderer #actions',
  'ytd-reel-video-renderer #actions-inner',
  '#shorts-container #actions',
] as const;

type ButtonVariant = 'watch' | 'shorts';




const ensureStyles = (): void => {
  if (document.getElementById(STYLE_ID)) {
    return;
  }

  const styleNode = document.createElement('style');
  styleNode.id = STYLE_ID;
  styleNode.textContent = youtubeInpageStyles;
  document.head.appendChild(styleNode);
};





let toastListenerRegistered = false;
let authStateKnown = false;
let authStateHasSettings = false;
let authStateCheckedAt = 0;
let authStatePromise: Promise<boolean> | null = null;









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

const normalizeYoutubeUrl = (rawUrl: string): string | null => {
  let parsed: URL;

  try {
    parsed = new URL(rawUrl);
  } catch {
    return null;
  }

  const host = parsed.hostname.toLowerCase();
  const timestampFromQuery = parsed.searchParams.get('t') || parsed.searchParams.get('start');
  let timestamp = timestampFromQuery && timestampFromQuery.trim() ? timestampFromQuery.trim() : null;

  if (!timestamp) {
    const hash = parsed.hash.replace(/^#/, '').trim();

    if (/^\d+$/.test(hash)) {
      timestamp = hash;
    } else if (hash) {
      const hashParams = new URLSearchParams(hash);
      const timestampFromHash = hashParams.get('t') || hashParams.get('start');
      timestamp = timestampFromHash && timestampFromHash.trim() ? timestampFromHash.trim() : null;
    }
  }

  if (host === 'youtu.be') {
    const shortId = parsed.pathname.replace(/^\//, '').trim();

    if (!shortId) {
      return null;
    }

    const output = new URL('https://www.youtube.com/watch');
    output.searchParams.set('v', shortId);

    if (timestamp) {
      output.searchParams.set('t', timestamp);
    }

    return output.toString();
  }

  if (!['www.youtube.com', 'youtube.com', 'm.youtube.com'].includes(host)) {
    return null;
  }

  if (parsed.pathname.startsWith('/shorts/')) {
    const shortId = parsed.pathname.split('/').filter(Boolean)[1];

    if (!shortId) {
      return null;
    }

    const output = new URL(`https://www.youtube.com/shorts/${shortId}`);

    if (timestamp) {
      output.searchParams.set('t', timestamp);
    }

    return output.toString();
  }

  const videoId = parsed.searchParams.get('v');

  if (!videoId) {
    return null;
  }

  const output = new URL('https://www.youtube.com/watch');
  output.searchParams.set('v', videoId);

  if (timestamp) {
    output.searchParams.set('t', timestamp);
  }

  return output.toString();
};

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

const sendQuick = async (url: string): Promise<{ ok: boolean; message: string }> => {
  const targetUrl = resolveAllowedActionTarget('youtube', url, window.location.href);

  if (!targetUrl) {
    return {
      ok: false,
      message: 'URL YouTube invalide.',
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
      source: 'youtube',
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
  const targetUrl = resolveAllowedActionTarget('youtube', payload.url, window.location.href);

  if (!targetUrl) {
    return {
      ok: false,
      message: 'URL YouTube invalide.',
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
      source: 'youtube',
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
  renderButtonContent(button, state, YOUTUBE_BUTTON_LOGO_SIZE_PX);

  if (state === 'loading') {
    button.classList.add('is-loading');
  } else if (state === 'success') {
    button.classList.add('is-success');
  } else if (state === 'error') {
    button.classList.add('is-error');
  }

  setButtonLabel(button, title || DEFAULT_BUTTON_TITLE);
};

const resetButtonStateLater = (button: HTMLButtonElement, delayMs = 2200): void => {
  clearButtonResetTimer(button);

  const timer = window.setTimeout(() => {
    buttonResetTimers.delete(button);
    setButtonState(button, 'idle', DEFAULT_BUTTON_TITLE);
  }, delayMs);

  buttonResetTimers.set(button, timer);
};

const getCurrentYoutubeUrl = (): string | null => normalizeYoutubeUrl(window.location.href);

const resolveButtonTargetUrl = (button: HTMLButtonElement): string | null =>
  resolveActionButtonTarget(button, 'youtube', getCurrentYoutubeUrl(), window.location.href);

const createActionButton = (variant: ButtonVariant): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = `lce-button-youtube ${variant === 'shorts' ? 'lce-button-youtube-shorts' : 'lce-button-youtube-watch'}`;
  setButtonState(button, 'idle');

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
        const targetUrl = resolveButtonTargetUrl(button);

        if (!targetUrl) {
          const message = 'Impossible de détecter l’URL YouTube courante.';
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
      void (async () => {
        if (button.disabled) {
          return;
        }

        button.disabled = true;
        setButtonState(button, 'loading', 'Envoi en cours...');

        try {
          const targetUrl = resolveButtonTargetUrl(button);

          if (!targetUrl) {
            const message = 'Impossible de détecter l’URL YouTube courante.';
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
      })();
    });
  });

  onTrustedPageEvent<MouseEvent>(button, 'dblclick', (event) => {
    event.preventDefault();
    event.stopPropagation();
    clearButtonClickTimer(button);

    if (button.disabled) {
      return;
    }

    const targetUrl = resolveButtonTargetUrl(button);

    if (!targetUrl) {
      const message = 'Impossible de détecter l’URL YouTube courante.';
      setButtonState(button, 'error', message);
      showToast('error', message);
      resetButtonStateLater(button);
      return;
    }

    openComposeModal({
      initialUrl: targetUrl,
      sourceLabel: `YouTube (${variant === 'shorts' ? 'Shorts' : 'Watch'})`,
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
  });

  return button;
};

const hasVisibleButtonDescendant = (element: HTMLElement): boolean => {
  const buttons = Array.from(element.querySelectorAll<HTMLButtonElement>('button'));

  for (const button of buttons) {
    const rect = button.getBoundingClientRect();

    if (rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.top < window.innerHeight) {
      return true;
    }
  }

  return false;
};

const isContainerVisiblyPresent = (element: HTMLElement): boolean => {
  const rect = element.getBoundingClientRect();

  if (rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.top < window.innerHeight) {
    return true;
  }

  return hasVisibleButtonDescendant(element);
};

const isHitTestableButton = (button: HTMLButtonElement): boolean => {
  const rect = button.getBoundingClientRect();

  if (rect.width < 8 || rect.height < 8) {
    return false;
  }

  const x = clamp(rect.left + rect.width / 2, 1, window.innerWidth - 1);
  const y = clamp(rect.top + rect.height / 2, 1, window.innerHeight - 1);
  const topElement = document.elementFromPoint(x, y);

  if (!topElement) {
    return false;
  }

  if (topElement === button || button.contains(topElement)) {
    return true;
  }

  const nearestButton = topElement.closest('button');
  return nearestButton === button;
};

const scoreWatchContainer = (container: HTMLElement): number => {
  const buttons = Array.from(container.querySelectorAll<HTMLButtonElement>('button'));

  if (buttons.length === 0) {
    return -1;
  }

  const hitTestableCount = buttons.filter((button) => isHitTestableButton(button)).length;
  if (hitTestableCount === 0) {
    return -1;
  }

  const rect = container.getBoundingClientRect();
  const areaPenalty = rect.width * rect.height;
  const isTopLevelButtons = container.id === 'top-level-buttons-computed' || container.classList.contains('top-level-buttons');
  const topLevelBonus = isTopLevelButtons ? 5_000_000 : 0;
  return topLevelBonus + hitTestableCount * 1_000_000 - areaPenalty;
};

const resolveTargetContainer = (variant: ButtonVariant): HTMLElement | null => {
  const selectors = variant === 'shorts' ? SHORTS_TARGET_SELECTORS : WATCH_TARGET_SELECTORS;
  const candidates = new Set<HTMLElement>();

  for (const selector of selectors) {
    const nodes = Array.from(document.querySelectorAll<HTMLElement>(selector));

    for (const node of nodes) {
      candidates.add(node);
    }
  }

  const list = [...candidates];

  if (list.length === 0) {
    return null;
  }

  const visible = list.filter((node) => isContainerVisiblyPresent(node));
  const pool = visible.length > 0 ? visible : list;

  if (variant === 'watch') {
    pool.sort((a, b) => scoreWatchContainer(b) - scoreWatchContainer(a));
    return pool[0] || null;
  }

  pool.sort((a, b) => {
    const aRect = a.getBoundingClientRect();
    const bRect = b.getBoundingClientRect();
    const aRightSide = aRect.left > window.innerWidth * 0.5 ? 1 : 0;
    const bRightSide = bRect.left > window.innerWidth * 0.5 ? 1 : 0;

    if (aRightSide !== bRightSide) {
      return bRightSide - aRightSide;
    }

    return aRect.top - bRect.top;
  });

  return pool[0] || null;
};

const isStateClass = (button: HTMLButtonElement): boolean => {
  return button.classList.contains('is-loading') || button.classList.contains('is-success') || button.classList.contains('is-error');
};

const applyVariantClass = (button: HTMLButtonElement, variant: ButtonVariant): void => {
  button.classList.add('lce-button-youtube');
  button.classList.toggle('lce-button-youtube-watch', variant === 'watch');
  button.classList.toggle('lce-button-youtube-shorts', variant === 'shorts');
};

const clamp = (value: number, min: number, max: number): number => {
  return Math.min(max, Math.max(min, value));
};

const getShortsActionAnchorRect = (): DOMRect | null => {
  const candidates = Array.from(
    document.querySelectorAll<HTMLElement>(
      'ytd-reel-player-overlay-renderer #actions button, ytd-reel-video-renderer #actions button, #shorts-container #actions button, .ytReelPlayerOverlayViewModelActionsContainer button, REEL-ACTION-BAR-VIEW-MODEL button',
    ),
  );

  const visible = candidates
    .map((node) => ({ node, rect: node.getBoundingClientRect() }))
    .filter(({ rect }) => {
      if (rect.width < 16 || rect.height < 16) {
        return false;
      }

      if (rect.bottom < 0 || rect.top > window.innerHeight) {
        return false;
      }

      return rect.left > window.innerWidth * 0.5;
    });

  if (visible.length === 0) {
    return null;
  }

  visible.sort((a, b) => {
    if (Math.abs(a.rect.left - b.rect.left) > 2) {
      return b.rect.left - a.rect.left;
    }

    return a.rect.top - b.rect.top;
  });

  return visible[0]?.rect || null;
};

const removeShortsFloatingButton = (): void => {
  const floating = document.getElementById(SHORTS_FLOATING_BUTTON_ID);

  if (floating) {
    floating.remove();
  }
};

const removeWatchFloatingButton = (): void => {
  const floating = document.getElementById(WATCH_FLOATING_BUTTON_ID);

  if (floating) {
    floating.remove();
  }
};

const removeInlineWatchButtons = (): void => {
  const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>(`button[${BUTTON_ATTRIBUTE}]`));

  for (const button of buttons) {
    if (button.id === WATCH_FLOATING_BUTTON_ID || button.id === SHORTS_FLOATING_BUTTON_ID) {
      continue;
    }

    if (!button.classList.contains('lce-button-youtube-watch')) {
      continue;
    }

    const slot = button.closest<HTMLElement>(`[${WATCH_SLOT_ATTRIBUTE}]`);
    if (slot) {
      slot.remove();
      continue;
    }

    button.remove();
  }
};

const resolveWatchInlineMountContainer = (candidate: HTMLElement): HTMLElement | null => {
  if (candidate.id === 'top-level-buttons-computed' || candidate.classList.contains('top-level-buttons')) {
    return candidate;
  }

  const nestedTopLevel = candidate.querySelector<HTMLElement>('#top-level-buttons-computed, .top-level-buttons');
  if (nestedTopLevel) {
    return nestedTopLevel;
  }

  const watchMetadataRoot = candidate.closest<HTMLElement>('ytd-watch-metadata');

  if (watchMetadataRoot) {
    const scopedTopLevel = watchMetadataRoot.querySelector<HTMLElement>(
      '#menu ytd-menu-renderer #top-level-buttons-computed, #menu #top-level-buttons-computed, ytd-menu-renderer #top-level-buttons-computed',
    );

    if (scopedTopLevel) {
      return scopedTopLevel;
    }
  }

  return candidate;
};

const upsertShortsFloatingButton = (targetUrl: string, container: HTMLElement | null): void => {
  let button = document.getElementById(SHORTS_FLOATING_BUTTON_ID) as HTMLButtonElement | null;

  if (!button) {
    button = createActionButton('shorts');
    button.id = SHORTS_FLOATING_BUTTON_ID;
    button.setAttribute(BUTTON_ATTRIBUTE, '1');
    document.body.appendChild(button);
  }

  rememberActionTarget(button, targetUrl);
  applyVariantClass(button, 'shorts');
  button.classList.add('lce-button-youtube-shorts-floating');

  if (!isStateClass(button)) {
    // Le balayage repasse à chaque frame : on rappelle le libellé sans
    // reconstruire le logo, déjà en place à l'état repos.
    setButtonLabel(button, DEFAULT_BUTTON_TITLE);
  }

  const anchorRect = getShortsActionAnchorRect();
  const fallbackRect = container?.getBoundingClientRect() || null;
  const originLeft = anchorRect
    ? anchorRect.left + anchorRect.width / 2 - 24
    : fallbackRect
      ? fallbackRect.right - 24
      : window.innerWidth - 84;
  const originTop = anchorRect ? anchorRect.top - 62 : fallbackRect ? fallbackRect.top + 6 : 108;

  button.style.left = `${clamp(originLeft, 8, window.innerWidth - 56)}px`;
  button.style.top = `${clamp(originTop, 8, window.innerHeight - 56)}px`;
};

const upsertInlineWatchButton = (targetUrl: string, container: HTMLElement): void => {
  const mountContainer = resolveWatchInlineMountContainer(container);

  if (!mountContainer) {
    return;
  }

  const existingSlots = Array.from(document.querySelectorAll<HTMLElement>(`[${WATCH_SLOT_ATTRIBUTE}]`));
  let slot =
    existingSlots.find((candidate) => candidate.parentElement === mountContainer) || existingSlots[0] || null;
  let button =
    slot?.querySelector<HTMLButtonElement>(`button[${BUTTON_ATTRIBUTE}].lce-button-youtube-watch`) || null;

  for (const staleSlot of existingSlots) {
    if (staleSlot !== slot) {
      staleSlot.remove();
    }
  }

  if (!slot) {
    slot = document.createElement('div');
    slot.setAttribute(WATCH_SLOT_ATTRIBUTE, '1');
    slot.className = 'lce-youtube-watch-slot';
  }

  if (!button) {
    button = createActionButton('watch');
    button.setAttribute(BUTTON_ATTRIBUTE, '1');
  }

  button.removeAttribute('id');
  rememberActionTarget(button, targetUrl);
  applyVariantClass(button, 'watch');
  button.classList.remove('lce-button-youtube-watch-floating');

  if (!isStateClass(button)) {
    // Le balayage repasse à chaque frame : on rappelle le libellé sans
    // reconstruire le logo, déjà en place à l'état repos.
    setButtonLabel(button, DEFAULT_BUTTON_TITLE);
  }

  if (!slot.contains(button)) {
    slot.appendChild(button);
  }

  if (slot.parentElement !== mountContainer) {
    mountContainer.appendChild(slot);
  }
};

const removeLegacyFloatingButton = (): void => {
  const legacyFloatingButton = document.getElementById(LEGACY_FLOATING_BUTTON_ID);

  if (legacyFloatingButton) {
    legacyFloatingButton.remove();
  }
};

const isYoutubeFullscreenActive = (): boolean => {
  if (document.fullscreenElement) {
    return true;
  }

  if (document.querySelector('.html5-video-player.ytp-fullscreen')) {
    return true;
  }

  return Boolean(document.querySelector('ytd-player[fullscreen]'));
};

const scanYoutubeTargets = async (): Promise<void> => {
  removeLegacyFloatingButton();

  const isReady = await hasExtensionAuth();

  if (!isReady) {
    removeShortsFloatingButton();
    removeWatchFloatingButton();
    removeInlineWatchButtons();
    return;
  }

  if (isYoutubeFullscreenActive()) {
    removeShortsFloatingButton();
    removeWatchFloatingButton();
    removeInlineWatchButtons();
    return;
  }

  const currentUrl = getCurrentYoutubeUrl();

  if (!currentUrl) {
    removeShortsFloatingButton();
    removeWatchFloatingButton();
    removeInlineWatchButtons();
    return;
  }

  const variant: ButtonVariant = currentUrl.includes('/shorts/') ? 'shorts' : 'watch';
  if (variant === 'shorts') {
    removeWatchFloatingButton();
    removeInlineWatchButtons();
    const shortsContainer = resolveTargetContainer('shorts');
    upsertShortsFloatingButton(currentUrl, shortsContainer);
    return;
  }

  removeShortsFloatingButton();
  removeWatchFloatingButton();

  const watchContainer = resolveTargetContainer('watch');

  if (!watchContainer) {
    removeInlineWatchButtons();
    return;
  }

  upsertInlineWatchButton(currentUrl, watchContainer);
};

void initInpageTheme();
registerToastListener();
// Plein écran et redimensionnement passent par le même budget que les
// mutations : le rescan arrive au plus tard ~150 ms après l'événement.
startObservedScanner(scanYoutubeTargets, [
  { target: document, event: 'fullscreenchange' },
  { target: window, event: 'resize' },
]);
