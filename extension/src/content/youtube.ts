const STYLE_ID = 'lce-youtube-style';
const BUTTON_ATTRIBUTE = 'data-lce-youtube-button';
const LEGACY_FLOATING_BUTTON_ID = 'lce-youtube-floating-button';
const SHORTS_FLOATING_BUTTON_ID = 'lce-youtube-shorts-floating-button';
const WATCH_FLOATING_BUTTON_ID = 'lce-youtube-watch-floating-button';
const WATCH_SLOT_ATTRIBUTE = 'data-lce-youtube-watch-slot';
const DEFAULT_BUTTON_TITLE = 'Envoyer la vidéo YouTube vers LiveChat';

const WATCH_TARGET_SELECTORS = [
  'ytd-watch-metadata #top-level-buttons-computed',
  'ytd-watch-metadata #actions-inner',
  'ytd-watch-metadata #actions',
  '#above-the-fold #top-level-buttons-computed',
  'ytd-menu-renderer #top-level-buttons-computed',
] as const;
const SHORTS_TARGET_SELECTORS = [
  'ytd-reel-player-overlay-renderer #actions',
  'ytd-reel-video-renderer #actions',
  'ytd-reel-video-renderer #actions-inner',
  '#shorts-container #actions',
] as const;

type ButtonVariant = 'watch' | 'shorts';
type ButtonState = 'idle' | 'loading' | 'success' | 'error';

const BUTTON_TEXT_BY_STATE: Record<ButtonState, string> = {
  idle: 'LC',
  loading: '...',
  success: 'OK',
  error: 'ER',
};

const buttonResetTimers = new WeakMap<HTMLButtonElement, number>();

const inpageStyles = `
.lce-button-youtube {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 42px;
  height: 42px;
  border: 1px solid rgba(255, 255, 255, 0.22);
  border-radius: 999px;
  padding: 0;
  font-family: "Roboto", "Arial", sans-serif;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.02em;
  cursor: pointer;
  pointer-events: auto;
  background: #0f0f0f;
  color: #fff;
  transition: transform 120ms ease, background-color 180ms ease, color 180ms ease, box-shadow 180ms ease;
}
.lce-button-youtube:hover {
  background: #272727;
}
.lce-button-youtube:disabled {
  opacity: 1;
}
.lce-button-youtube-watch {
  margin-left: 18px;
  margin-right: 14px;
  flex: 0 0 auto;
  position: relative;
  z-index: 2;
}
.lce-button-youtube-shorts {
  width: 48px;
  height: 48px;
}
.lce-button-youtube-shorts-floating {
  position: fixed;
  z-index: 2147483646;
  pointer-events: auto !important;
}
.lce-button-youtube-watch-floating {
  position: fixed;
  z-index: 2147483646;
  pointer-events: auto !important;
}
.lce-youtube-shorts-slot {
  display: flex;
  justify-content: center;
  width: 100%;
  margin-bottom: 12px;
  pointer-events: auto;
  z-index: 3;
}
.lce-youtube-watch-slot {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-left: 8px;
  margin-right: 8px;
  flex: 0 0 auto;
  pointer-events: auto !important;
}
.lce-button-youtube.is-loading {
  background: #fed54a;
  color: #0f0f0f;
  animation: lce-youtube-pulse 0.95s ease-in-out infinite;
}
.lce-button-youtube.is-success {
  background: #30d158;
  color: #0f0f0f;
  animation: lce-youtube-pulse-success 0.9s ease-in-out 2;
}
.lce-button-youtube.is-error {
  background: #ff453a;
  color: #fff;
  animation: lce-youtube-pulse-error 0.9s ease-in-out 2;
}
@keyframes lce-youtube-pulse {
  0% { transform: scale(1); box-shadow: 0 0 0 0 rgba(254, 213, 74, 0.45); }
  70% { transform: scale(1.06); box-shadow: 0 0 0 10px rgba(254, 213, 74, 0); }
  100% { transform: scale(1); box-shadow: 0 0 0 0 rgba(254, 213, 74, 0); }
}
@keyframes lce-youtube-pulse-success {
  0% { transform: scale(1); box-shadow: 0 0 0 0 rgba(48, 209, 88, 0.45); }
  70% { transform: scale(1.08); box-shadow: 0 0 0 12px rgba(48, 209, 88, 0); }
  100% { transform: scale(1); box-shadow: 0 0 0 0 rgba(48, 209, 88, 0); }
}
@keyframes lce-youtube-pulse-error {
  0% { transform: scale(1); box-shadow: 0 0 0 0 rgba(255, 69, 58, 0.45); }
  70% { transform: scale(1.08); box-shadow: 0 0 0 12px rgba(255, 69, 58, 0); }
  100% { transform: scale(1); box-shadow: 0 0 0 0 rgba(255, 69, 58, 0); }
}
`;

const ensureStyles = (): void => {
  if (document.getElementById(STYLE_ID)) {
    return;
  }

  const styleNode = document.createElement('style');
  styleNode.id = STYLE_ID;
  styleNode.textContent = inpageStyles;
  document.head.appendChild(styleNode);
};

const normalizeYoutubeUrl = (rawUrl: string): string | null => {
  let parsed: URL;

  try {
    parsed = new URL(rawUrl);
  } catch {
    return null;
  }

  const host = parsed.hostname.toLowerCase();

  if (host === 'youtu.be') {
    const shortId = parsed.pathname.replace(/^\//, '').trim();
    return shortId ? `https://www.youtube.com/watch?v=${shortId}` : null;
  }

  if (!['www.youtube.com', 'youtube.com', 'm.youtube.com'].includes(host)) {
    return null;
  }

  if (parsed.pathname.startsWith('/shorts/')) {
    const shortId = parsed.pathname.split('/').filter(Boolean)[1];
    return shortId ? `https://www.youtube.com/shorts/${shortId}` : null;
  }

  const videoId = parsed.searchParams.get('v');

  if (!videoId) {
    return null;
  }

  const output = new URL('https://www.youtube.com/watch');
  output.searchParams.set('v', videoId);

  const timestamp = parsed.searchParams.get('t');

  if (timestamp) {
    output.searchParams.set('t', timestamp);
  }

  return output.toString();
};

const readRuntime = (): typeof chrome.runtime | null => {
  try {
    if (typeof chrome === 'undefined') {
      return null;
    }

    return chrome.runtime || null;
  } catch {
    return null;
  }
};

const sendQuick = async (url: string): Promise<{ ok: boolean; message: string }> => {
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
      url,
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

const clearButtonResetTimer = (button: HTMLButtonElement): void => {
  const activeTimer = buttonResetTimers.get(button);

  if (typeof activeTimer === 'number') {
    window.clearTimeout(activeTimer);
    buttonResetTimers.delete(button);
  }
};

const setButtonState = (button: HTMLButtonElement, state: ButtonState, title?: string): void => {
  clearButtonResetTimer(button);
  button.classList.remove('is-loading', 'is-success', 'is-error');
  button.textContent = BUTTON_TEXT_BY_STATE[state];

  if (state === 'loading') {
    button.classList.add('is-loading');
  } else if (state === 'success') {
    button.classList.add('is-success');
  } else if (state === 'error') {
    button.classList.add('is-error');
  }

  button.title = title || DEFAULT_BUTTON_TITLE;
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

const createActionButton = (variant: ButtonVariant): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = `lce-button-youtube ${variant === 'shorts' ? 'lce-button-youtube-shorts' : 'lce-button-youtube-watch'}`;
  button.textContent = BUTTON_TEXT_BY_STATE.idle;
  button.title = DEFAULT_BUTTON_TITLE;

  button.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();

    void (async () => {
      if (button.disabled) {
        return;
      }

      button.disabled = true;
      setButtonState(button, 'loading', 'Envoi en cours...');

      try {
        const targetUrl = button.dataset.targetUrl || getCurrentYoutubeUrl();

        if (!targetUrl) {
          setButtonState(button, 'error', 'Impossible de détecter l’URL YouTube courante.');
          resetButtonStateLater(button);
          return;
        }

        const response = await sendQuick(targetUrl);
        setButtonState(button, response.ok ? 'success' : 'error', response.message);
        resetButtonStateLater(button);
      } finally {
        window.setTimeout(() => {
          button.disabled = false;
        }, 300);
      }
    })();
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
  return hitTestableCount * 1_000_000 - areaPenalty;
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
      'ytd-reel-player-overlay-renderer #actions button, ytd-reel-video-renderer #actions button, #shorts-container #actions button',
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

const upsertShortsFloatingButton = (targetUrl: string, container: HTMLElement | null): void => {
  let button = document.getElementById(SHORTS_FLOATING_BUTTON_ID) as HTMLButtonElement | null;

  if (!button) {
    button = createActionButton('shorts');
    button.id = SHORTS_FLOATING_BUTTON_ID;
    button.setAttribute(BUTTON_ATTRIBUTE, '1');
    document.body.appendChild(button);
  }

  button.dataset.targetUrl = targetUrl;
  applyVariantClass(button, 'shorts');
  button.classList.add('lce-button-youtube-shorts-floating');

  if (!isStateClass(button)) {
    button.textContent = BUTTON_TEXT_BY_STATE.idle;
    button.title = DEFAULT_BUTTON_TITLE;
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
  const existingSlots = Array.from(document.querySelectorAll<HTMLElement>(`[${WATCH_SLOT_ATTRIBUTE}]`));
  let slot =
    existingSlots.find((candidate) => candidate.parentElement === container) || existingSlots[0] || null;
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
  button.dataset.targetUrl = targetUrl;
  applyVariantClass(button, 'watch');
  button.classList.remove('lce-button-youtube-watch-floating');

  if (!isStateClass(button)) {
    button.textContent = BUTTON_TEXT_BY_STATE.idle;
    button.title = DEFAULT_BUTTON_TITLE;
  }

  if (!slot.contains(button)) {
    slot.appendChild(button);
  }

  if (slot.parentElement !== container) {
    container.insertBefore(slot, container.firstElementChild);
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

const scanYoutubeTargets = (): void => {
  removeLegacyFloatingButton();

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

const startObservedScanner = (scan: () => void): void => {
  let scanQueued = false;
  let lastUrl = window.location.href;

  const runScan = () => {
    scanQueued = false;
    scan();
  };

  const queueScan = () => {
    if (scanQueued) {
      return;
    }

    scanQueued = true;
    window.requestAnimationFrame(runScan);
  };

  const observer = new MutationObserver(() => {
    queueScan();
  });

  observer.observe(document.documentElement, {
    subtree: true,
    childList: true,
  });

  window.setInterval(() => {
    if (window.location.href !== lastUrl) {
      lastUrl = window.location.href;
      queueScan();
    }
  }, 700);

  document.addEventListener('fullscreenchange', queueScan);
  window.addEventListener('resize', queueScan);

  queueScan();
};

startObservedScanner(scanYoutubeTargets);
