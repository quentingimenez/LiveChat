import {
  renderButtonContent,
  setButtonLabel,
  type InjectedButtonState,
} from './brand-logo';
import { openComposeModal, type ComposeSubmitPayload, type ComposeSubmitResult } from './composeModal';
import { initInpageTheme } from './inpage-theme';
import { TIKTOK_BUTTON_LOGO_SIZE_PX, tiktokInpageStyles } from './inpage-styles';
import { onTrustedPageEvent, resolveAllowedActionTarget } from './shared';
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

const STYLE_ID = 'lce-tiktok-style';
const INLINE_BUTTON_ID = 'lce-tiktok-inline-button';
const INLINE_SLOT_ID = 'lce-tiktok-inline-slot';
const DEFAULT_BUTTON_TITLE = 'Envoyer ce TikTok vers LiveChat';
const GET_ACTIVE_MEDIA_URL_TYPE = 'lce/get-active-media-url';
const GET_AUTH_STATE_TYPE = 'lce/get-auth-state';
const TIKTOK_GET_CAPTURED_URL_TYPE = 'lce/tiktok-get-captured-url';
const TIKTOK_SYNC_ACTIVE_ITEM_TYPE = 'lce/tiktok-sync-active-item';
const AUTH_STATUS_CACHE_MS = 1500;





const ACTION_ANCHOR_SELECTORS = [
  '[data-e2e="browse-share-icon"]',
  '[data-e2e="share-icon"]',
  '[data-e2e*="share-icon"]',
  '[data-e2e="browse-comment-icon"]',
  '[data-e2e="comment-icon"]',
  '[data-e2e*="comment-icon"]',
  '[data-e2e="browse-like-icon"]',
  '[data-e2e="like-icon"]',
  '[data-e2e*="like-icon"]',
] as const;

interface TikTokActiveTarget {
  itemId: string | null;
  url: string | null;
}

let latestActionContainer: HTMLElement | null = null;
let lastSyncedActiveTargetKey: string | null = null;




const ensureStyles = (): void => {
  if (document.getElementById(STYLE_ID)) {
    return;
  }

  const styleNode = document.createElement('style');
  styleNode.id = STYLE_ID;
  styleNode.textContent = tiktokInpageStyles;
  document.head.appendChild(styleNode);
};





let toastListenerRegistered = false;
let runtimeContextInvalidated = false;
let authStateKnown = false;
let authStateHasSettings = false;
let authStateCheckedAt = 0;
let authStatePromise: Promise<boolean> | null = null;

const isExtensionContextInvalidatedError = (error: unknown): boolean => {
  if (error instanceof Error) {
    return /extension context invalidated/i.test(error.message);
  }

  if (typeof error === 'string') {
    return /extension context invalidated/i.test(error);
  }

  if (error && typeof error === 'object' && 'message' in error) {
    const message = (error as { message?: unknown }).message;

    if (typeof message === 'string') {
      return /extension context invalidated/i.test(message);
    }
  }

  return false;
};

const markRuntimeContextInvalidatedIfNeeded = (error: unknown): void => {
  if (isExtensionContextInvalidatedError(error)) {
    runtimeContextInvalidated = true;
  }
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
  } catch (error) {
    markRuntimeContextInvalidatedIfNeeded(error);
    // Ignore runtime invalidation while extension reloads.
  }
};

const isTikTokHostname = (hostname: string): boolean => {
  const normalizedHost = hostname.toLowerCase();
  return normalizedHost === 'tiktok.com' || normalizedHost.endsWith('.tiktok.com');
};

const extractTikTokItemIdFromText = (value: string | null | undefined): string | null => {
  if (!value) {
    return null;
  }

  const match = value.match(/\b(\d{15,22})\b/);
  return match?.[1] || null;
};

const extractTikTokItemIdFromUrl = (rawUrl: string, base?: string): string | null => {
  try {
    const parsed = new URL(rawUrl, base);
    const mediaMatch = parsed.pathname.match(/^\/(?:@[^/]+\/)?(?:video|photo)\/(\d{15,22})(?:\/|$)/i);

    if (mediaMatch) {
      return mediaMatch[1];
    }

    return extractTikTokItemIdFromText(parsed.searchParams.get('item_id'));
  } catch {
    return extractTikTokItemIdFromText(rawUrl);
  }
};

const normalizeTikTokMediaUrl = (rawUrl: string, base?: string): string | null => {
  let parsed: URL;

  try {
    parsed = new URL(rawUrl, base);
  } catch {
    return null;
  }

  if (!isTikTokHostname(parsed.hostname)) {
    return null;
  }

  const namedMediaMatch = parsed.pathname.match(/^\/@([^/]+)\/(video|photo)\/(\d{15,22})(?:\/|$)/i);

  if (namedMediaMatch) {
    const handle = namedMediaMatch[1];
    const mediaType = namedMediaMatch[2].toLowerCase();
    const mediaId = namedMediaMatch[3];
    return `https://www.tiktok.com/@${handle}/${mediaType}/${mediaId}`;
  }

  const genericMediaMatch = parsed.pathname.match(/^\/(video|photo)\/(\d{15,22})(?:\/|$)/i);

  if (genericMediaMatch) {
    const mediaType = genericMediaMatch[1].toLowerCase();
    const mediaId = genericMediaMatch[2];
    return `https://www.tiktok.com/${mediaType}/${mediaId}`;
  }

  return null;
};

const toTikTokActiveTargetFromUrl = (rawUrl: string | null | undefined): TikTokActiveTarget => {
  if (!rawUrl) {
    return {
      itemId: null,
      url: null,
    };
  }

  const normalizedUrl = normalizeTikTokMediaUrl(rawUrl, window.location.href);
  const itemId = extractTikTokItemIdFromUrl(normalizedUrl || rawUrl, window.location.href);

  return {
    itemId,
    url: normalizedUrl,
  };
};

const buildGenericTikTokVideoUrl = (itemId: string): string => `https://www.tiktok.com/video/${itemId}`;

const findNormalizedTikTokUrlInAnchors = (root: ParentNode, onlyVisible: boolean): string | null => {
  const anchors = Array.from(root.querySelectorAll<HTMLAnchorElement>('a[href]')).slice(0, 200);

  for (const anchor of anchors) {
    if (onlyVisible && !isElementVisible(anchor)) {
      continue;
    }

    const href = anchor.getAttribute('href') || anchor.href;
    const normalized = normalizeTikTokMediaUrl(href, window.location.href);

    if (normalized) {
      return normalized;
    }
  }

  return null;
};

const extractTikTokItemIdFromElement = (element: Element): string | null => {
  const prioritizedAttributes = [
    'data-item-id',
    'data-video-id',
    'data-aweme-id',
    'aweme-id',
    'data-e2e',
    'href',
    'src',
    'poster',
    'id',
  ] as const;

  for (const attributeName of prioritizedAttributes) {
    const attributeValue = element.getAttribute(attributeName);

    if (!attributeValue) {
      continue;
    }

    const fromUrl = extractTikTokItemIdFromUrl(attributeValue, window.location.href);

    if (fromUrl) {
      return fromUrl;
    }

    const fromText = extractTikTokItemIdFromText(attributeValue);

    if (fromText) {
      return fromText;
    }
  }

  if (element instanceof HTMLAnchorElement) {
    const fromHref = extractTikTokItemIdFromUrl(element.getAttribute('href') || element.href, window.location.href);

    if (fromHref) {
      return fromHref;
    }
  }

  if (element instanceof HTMLVideoElement) {
    const fromVideoSources =
      extractTikTokItemIdFromUrl(element.currentSrc || '', window.location.href) ||
      extractTikTokItemIdFromUrl(element.src || '', window.location.href) ||
      extractTikTokItemIdFromUrl(element.poster || '', window.location.href);

    if (fromVideoSources) {
      return fromVideoSources;
    }
  }

  return null;
};

const findTikTokItemIdInRoot = (root: ParentNode, deepSearch = true): string | null => {
  if (root instanceof Element) {
    const fromRoot = extractTikTokItemIdFromElement(root);

    if (fromRoot) {
      return fromRoot;
    }
  }

  if (!deepSearch) {
    return null;
  }

  const candidates = Array.from(
    root.querySelectorAll<HTMLElement>(
      '[data-item-id], [data-video-id], [data-aweme-id], [aweme-id], a[href*="/video/"], a[href*="/photo/"], video[src], video[poster]',
    ),
  ).slice(0, 220);

  for (const candidate of candidates) {
    const fromCandidate = extractTikTokItemIdFromElement(candidate);

    if (fromCandidate) {
      return fromCandidate;
    }
  }

  return null;
};

const resolveTikTokTargetAroundElement = (startElement: HTMLElement | null, maxDepth = 8): TikTokActiveTarget => {
  if (!startElement) {
    return {
      itemId: null,
      url: null,
    };
  }

  let node: HTMLElement | null = startElement;
  let depth = 0;

  while (node && node !== document.body && depth < maxDepth) {
    const nodeRect = node.getBoundingClientRect();
    const viewportArea = Math.max(1, window.innerWidth * window.innerHeight);
    const nodeArea = Math.max(0, nodeRect.width) * Math.max(0, nodeRect.height);
    const allowDeepSearch = nodeArea <= viewportArea * 0.72;

    const itemId = findTikTokItemIdInRoot(node, allowDeepSearch);

    if (itemId) {
      return {
        itemId,
        url: buildGenericTikTokVideoUrl(itemId),
      };
    }

    if (allowDeepSearch) {
      const fromVisibleAnchors = findNormalizedTikTokUrlInAnchors(node, true);

      if (fromVisibleAnchors) {
        return toTikTokActiveTargetFromUrl(fromVisibleAnchors);
      }

      const fromAnchors = findNormalizedTikTokUrlInAnchors(node, false);

      if (fromAnchors) {
        return toTikTokActiveTargetFromUrl(fromAnchors);
      }
    }

    node = node.parentElement;
    depth += 1;
  }

  return {
    itemId: null,
    url: null,
  };
};

const resolveMostVisibleVideoElement = (): HTMLVideoElement | null => {
  const visibleVideos = Array.from(document.querySelectorAll<HTMLVideoElement>('video')).filter((video) =>
    isElementVisible(video),
  );

  if (visibleVideos.length === 0) {
    return null;
  }

  let bestVideo: HTMLVideoElement | null = null;
  let bestScore = Number.NEGATIVE_INFINITY;

  for (const video of visibleVideos) {
    const rect = video.getBoundingClientRect();
    const visibleWidth = Math.max(0, Math.min(rect.right, window.innerWidth) - Math.max(rect.left, 0));
    const visibleHeight = Math.max(0, Math.min(rect.bottom, window.innerHeight) - Math.max(rect.top, 0));
    const visibleArea = visibleWidth * visibleHeight;
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height / 2;
    const distanceFromViewportCenter = Math.abs(centerX - window.innerWidth / 2) + Math.abs(centerY - window.innerHeight / 2);
    const playbackBonus = video.paused ? 0 : 5_000_000;
    const readyBonus = video.readyState >= 2 ? 100_000 : 0;
    const score = visibleArea - distanceFromViewportCenter * 16 + playbackBonus + readyBonus;

    if (score > bestScore) {
      bestScore = score;
      bestVideo = video;
    }
  }

  return bestVideo;
};

const resolveActiveVideoTarget = (): TikTokActiveTarget => {
  const visibleVideo = resolveMostVisibleVideoElement();

  if (!visibleVideo) {
    return {
      itemId: null,
      url: null,
    };
  }

  const directItemId =
    extractTikTokItemIdFromUrl(visibleVideo.currentSrc || '', window.location.href) ||
    extractTikTokItemIdFromUrl(visibleVideo.src || '', window.location.href) ||
    extractTikTokItemIdFromUrl(visibleVideo.poster || '', window.location.href) ||
    findTikTokItemIdInRoot(visibleVideo);

  if (directItemId) {
    return {
      itemId: directItemId,
      url: buildGenericTikTokVideoUrl(directItemId),
    };
  }

  return resolveTikTokTargetAroundElement(visibleVideo, 4);
};

const resolveDomTikTokTarget = (preferredContainer: HTMLElement | null, deepSearch: boolean): TikTokActiveTarget => {
  const fromLocation = toTikTokActiveTargetFromUrl(window.location.href);

  if (fromLocation.url) {
    return fromLocation;
  }

  const fromActiveVideo = resolveActiveVideoTarget();

  if (fromActiveVideo.url) {
    return fromActiveVideo;
  }

  if (preferredContainer && document.contains(preferredContainer)) {
    const fromContainer = resolveTikTokTargetAroundElement(preferredContainer, 8);

    if (fromContainer.url) {
      return fromContainer;
    }
  }

  if (deepSearch) {
    const fromVisibleAnchors = findNormalizedTikTokUrlInAnchors(document, true);

    if (fromVisibleAnchors) {
      return toTikTokActiveTargetFromUrl(fromVisibleAnchors);
    }

    const fromAnchors = findNormalizedTikTokUrlInAnchors(document, false);

    if (fromAnchors) {
      return toTikTokActiveTargetFromUrl(fromAnchors);
    }
  }

  return {
    itemId: null,
    url: null,
  };
};

const syncActiveTikTokTarget = (target: TikTokActiveTarget): void => {
  const runtime = readRuntime();

  if (!runtime || typeof runtime.sendMessage !== 'function') {
    return;
  }

  const normalizedUrl = normalizeTikTokMediaUrl(target.url || '', window.location.href);
  const key = `${target.itemId || ''}|${normalizedUrl || ''}`;

  if (key === lastSyncedActiveTargetKey) {
    return;
  }

  lastSyncedActiveTargetKey = key;

  try {
    const maybePromise = runtime.sendMessage({
      type: TIKTOK_SYNC_ACTIVE_ITEM_TYPE,
      itemId: target.itemId || null,
      url: normalizedUrl,
    });

    if (maybePromise && typeof (maybePromise as Promise<unknown>).catch === 'function') {
      void (maybePromise as Promise<unknown>).catch((error) => {
        markRuntimeContextInvalidatedIfNeeded(error);
      });
    }
  } catch (error) {
    markRuntimeContextInvalidatedIfNeeded(error);
  }
};

const getPreferredActionContainer = (): HTMLElement | null => {
  if (latestActionContainer && document.contains(latestActionContainer)) {
    return latestActionContainer;
  }

  return resolveActionContainer();
};

const requestCapturedTikTokUrl = async (domUrlCandidate: string | null): Promise<string | null> => {
  const runtime = readRuntime();

  if (!runtime || typeof runtime.sendMessage !== 'function') {
    return null;
  }

  try {
    const response = (await runtime.sendMessage({
      type: TIKTOK_GET_CAPTURED_URL_TYPE,
      domUrl: domUrlCandidate,
    })) as { ok?: unknown; url?: unknown };

    if (!response || response.ok !== true || typeof response.url !== 'string') {
      return null;
    }

    return normalizeTikTokMediaUrl(response.url, window.location.href);
  } catch (error) {
    markRuntimeContextInvalidatedIfNeeded(error);
    return null;
  }
};

const resolveCurrentTikTokMediaUrl = async (): Promise<string | null> => {
  const preferredContainer = getPreferredActionContainer();
  const domTarget = resolveDomTikTokTarget(preferredContainer, false);

  if (domTarget.url) {
    syncActiveTikTokTarget(domTarget);
    return domTarget.url;
  }

  const capturedUrl = await requestCapturedTikTokUrl(domTarget.url || null);

  if (capturedUrl) {
    const capturedTarget = toTikTokActiveTargetFromUrl(capturedUrl);
    syncActiveTikTokTarget(capturedTarget);
    return capturedUrl;
  }

  const deepDomTarget = resolveDomTikTokTarget(preferredContainer, true);

  if (deepDomTarget.url) {
    syncActiveTikTokTarget(deepDomTarget);
    return deepDomTarget.url;
  }

  return null;
};

const readRuntime = (): typeof browser.runtime | null => {
  if (runtimeContextInvalidated) {
    return null;
  }

  try {
    if (typeof browser === 'undefined') {
      return null;
    }

    return browser.runtime || null;
  } catch (error) {
    markRuntimeContextInvalidatedIfNeeded(error);
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
    } catch (error) {
      markRuntimeContextInvalidatedIfNeeded(error);
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
  const targetUrl = resolveAllowedActionTarget('tiktok', url, window.location.href);

  if (!targetUrl) {
    return {
      ok: false,
      message: 'URL TikTok invalide.',
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
      source: 'tiktok',
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
    markRuntimeContextInvalidatedIfNeeded(error);
    return {
      ok: false,
      message: error instanceof Error ? error.message : 'Erreur de communication avec le service worker.',
    };
  }
};

const sendCompose = async (payload: ComposeSubmitPayload): Promise<ComposeSubmitResult> => {
  const targetUrl = resolveAllowedActionTarget('tiktok', payload.url, window.location.href);

  if (!targetUrl) {
    return {
      ok: false,
      message: 'URL TikTok invalide.',
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
      source: 'tiktok',
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
    markRuntimeContextInvalidatedIfNeeded(error);
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
  renderButtonContent(button, state, TIKTOK_BUTTON_LOGO_SIZE_PX);

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

const createActionButton = (): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'lce-button-tiktok';
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
        const targetUrl = await resolveCurrentTikTokMediaUrl();

        if (!targetUrl) {
          const message = 'Impossible de détecter la vidéo TikTok active. Fais défiler puis réessaie.';
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
          const targetUrl = await resolveCurrentTikTokMediaUrl();

          if (!targetUrl) {
            const message = 'Impossible de détecter la vidéo TikTok active. Fais défiler puis réessaie.';
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

    void (async () => {
      const targetUrl = await resolveCurrentTikTokMediaUrl();

      if (!targetUrl) {
        const message = 'Impossible de détecter la vidéo TikTok active. Fais défiler puis réessaie.';
        setButtonState(button, 'error', message);
        showToast('error', message);
        resetButtonStateLater(button);
        return;
      }

      openComposeModal({
        initialUrl: targetUrl,
        sourceLabel: 'TikTok',
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
    })();
  });

  return button;
};

const removeLegacyFloatingButton = (): void => {
  const floatingButton = document.getElementById('lce-tiktok-floating-button');

  if (floatingButton) {
    floatingButton.remove();
  }
};

const removeInlineButton = (): void => {
  const slot = document.getElementById(INLINE_SLOT_ID);

  if (slot) {
    slot.remove();
  }
};

const isElementVisible = (element: HTMLElement): boolean => {
  const rect = element.getBoundingClientRect();
  return rect.width >= 8 && rect.height >= 8 && rect.bottom > 0 && rect.top < window.innerHeight;
};

const getAnchorsInRoot = (root: ParentNode): HTMLElement[] => {
  const seen = new Set<HTMLElement>();
  const anchors: HTMLElement[] = [];

  for (const selector of ACTION_ANCHOR_SELECTORS) {
    for (const node of Array.from(root.querySelectorAll<HTMLElement>(selector))) {
      if (seen.has(node)) {
        continue;
      }

      seen.add(node);
      anchors.push(node);
    }
  }

  return anchors;
};

const scoreContainer = (container: HTMLElement, anchorCount: number): number => {
  const rect = container.getBoundingClientRect();
  const rightBias = rect.left > window.innerWidth * 0.52 ? 1 : 0;
  const sizePenalty = rect.width * rect.height;
  return anchorCount * 100_000 + rightBias * 10_000 - sizePenalty;
};

const resolveActionContainer = (): HTMLElement | null => {
  const anchors = getAnchorsInRoot(document).filter((node) => isElementVisible(node));

  if (anchors.length === 0) {
    return null;
  }

  const candidates = new Map<HTMLElement, number>();

  for (const anchor of anchors) {
    let node: HTMLElement | null = anchor;

    while (node && node !== document.body) {
      const rect = node.getBoundingClientRect();

      // La colonne d'actions TikTok est étroite : 22 px sur le fil, 29 px sur
      // la page vidéo. Un seuil à 40 px écartait la bonne colonne et retombait
      // sur la rangée qui l'englobe (le bouton finissait à côté des icônes) ;
      // les libellés des boutons ne font qu'une vingtaine de pixels de large.
      if (rect.width >= 16 && rect.height >= 80 && rect.width <= 480 && rect.height <= window.innerHeight) {
        const anchorCount = getAnchorsInRoot(node).filter((candidate) => isElementVisible(candidate)).length;

        if (anchorCount >= 2) {
          const score = scoreContainer(node, anchorCount);
          const prev = candidates.get(node);

          if (typeof prev !== 'number' || score > prev) {
            candidates.set(node, score);
          }
        }
      }

      node = node.parentElement;
    }
  }

  const sorted = [...candidates.entries()].sort((a, b) => b[1] - a[1]);
  return sorted[0]?.[0] || null;
};

type ActionLayout = 'vertical' | 'horizontal';

const detectActionLayout = (container: HTMLElement): ActionLayout => {
  const actionAnchors = getAnchorsInRoot(container).filter((node) => isElementVisible(node)).slice(0, 8);

  if (actionAnchors.length >= 2) {
    let minX = Number.POSITIVE_INFINITY;
    let maxX = Number.NEGATIVE_INFINITY;
    let minY = Number.POSITIVE_INFINITY;
    let maxY = Number.NEGATIVE_INFINITY;

    for (const anchor of actionAnchors) {
      const rect = anchor.getBoundingClientRect();
      const centerX = rect.left + rect.width / 2;
      const centerY = rect.top + rect.height / 2;

      minX = Math.min(minX, centerX);
      maxX = Math.max(maxX, centerX);
      minY = Math.min(minY, centerY);
      maxY = Math.max(maxY, centerY);
    }

    const spreadX = maxX - minX;
    const spreadY = maxY - minY;

    if (spreadY > spreadX * 1.2) {
      return 'vertical';
    }

    if (spreadX > spreadY * 1.2) {
      return 'horizontal';
    }
  }

  const containerStyle = window.getComputedStyle(container);
  const containerFlexDirection = containerStyle.flexDirection.toLowerCase();

  if (containerFlexDirection.includes('column')) {
    return 'vertical';
  }

  if (containerFlexDirection.includes('row')) {
    return 'horizontal';
  }

  const containerRect = container.getBoundingClientRect();
  return containerRect.height > containerRect.width ? 'vertical' : 'horizontal';
};

const upsertInlineButton = (container: HTMLElement): void => {
  ensureStyles();
  let slot = document.getElementById(INLINE_SLOT_ID) as HTMLDivElement | null;
  let button = document.getElementById(INLINE_BUTTON_ID) as HTMLButtonElement | null;

  if (!slot) {
    slot = document.createElement('div');
    slot.id = INLINE_SLOT_ID;
    slot.className = 'lce-tiktok-slot';
  }

  if (!button) {
    button = createActionButton();
    button.id = INLINE_BUTTON_ID;
    slot.prepend(button);
  }

  if (!slot.contains(button)) {
    slot.prepend(button);
  }

  const legacyLabel = slot.querySelector<HTMLElement>('.lce-tiktok-slot-label');
  if (legacyLabel) {
    legacyLabel.remove();
  }

  const firstChild = container.firstElementChild;
  const horizontalHost =
    firstChild instanceof HTMLDivElement && firstChild !== slot ? firstChild : container;
  const actionLayout = detectActionLayout(container);
  slot.classList.remove('is-vertical', 'is-horizontal');
  slot.classList.add(actionLayout === 'vertical' ? 'is-vertical' : 'is-horizontal');

  if (actionLayout === 'horizontal') {
    if (slot.parentElement !== horizontalHost || horizontalHost.firstElementChild !== slot) {
      horizontalHost.prepend(slot);
    }
  } else {
    // Colonne verticale : le bouton se glisse entre l'avatar (première div) et
    // la première action native (deuxième div), pour ouvrir la pile d'actions
    // sans passer sous la pastille musicale.
    const avatar = container.firstElementChild === slot ? slot.nextElementSibling : container.firstElementChild;

    if (avatar instanceof HTMLElement) {
      if (slot.previousElementSibling !== avatar || slot.parentElement !== container) {
        container.insertBefore(slot, avatar.nextElementSibling);
      }
    } else if (slot.parentElement !== container) {
      container.appendChild(slot);
    }
  }

  // Le balayage repasse sur le bouton à chaque mutation de la page : le libellé
  // est rappelé sans retoucher le contenu, pour ne pas reconstruire le logo.
  setButtonLabel(button, DEFAULT_BUTTON_TITLE);
};

const scanTikTokPage = async (): Promise<void> => {
  removeLegacyFloatingButton();

  const isReady = await hasExtensionAuth();

  if (!isReady) {
    removeInlineButton();
    return;
  }

  const actionContainer = resolveActionContainer();
  latestActionContainer = actionContainer;

  const domTarget = resolveDomTikTokTarget(actionContainer, false);
  syncActiveTikTokTarget(domTarget);

  if (!actionContainer) {
    removeInlineButton();
    return;
  }

  upsertInlineButton(actionContainer);
};

const isGetActiveMediaUrlMessage = (value: unknown): value is { type: string } => {
  if (!value || typeof value !== 'object') {
    return false;
  }

  const payload = value as { type?: unknown };
  return payload.type === GET_ACTIVE_MEDIA_URL_TYPE;
};

const registerActiveMediaUrlListener = (): void => {
  const runtime = readRuntime();

  if (!runtime || !runtime.onMessage || typeof runtime.onMessage.addListener !== 'function') {
    return;
  }

  try {
    runtime.onMessage.addListener(
      (message: unknown, _sender: browser.runtime.MessageSender, sendResponse: (response?: unknown) => void) => {
        if (!isGetActiveMediaUrlMessage(message)) {
          return;
        }

        void (async () => {
          try {
            const url = await resolveCurrentTikTokMediaUrl();

            sendResponse({
              ok: true,
              url,
            });
          } catch {
            sendResponse({
              ok: false,
              url: null,
            });
          }
        })();

        return true;
      },
    );
  } catch (error) {
    markRuntimeContextInvalidatedIfNeeded(error);
    // Ignore runtime invalidation while the extension is reloading.
  }
};

void initInpageTheme();
registerActiveMediaUrlListener();
registerToastListener();
startObservedScanner(scanTikTokPage);
