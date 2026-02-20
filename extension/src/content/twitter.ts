const STYLE_ID = 'lce-twitter-style';
const BUTTON_ATTRIBUTE = 'data-lce-twitter-button';
const SLOT_ATTRIBUTE = 'data-lce-twitter-slot';
const FLOATING_BUTTON_ID = 'lce-twitter-floating-button';
const GET_AUTH_STATE_TYPE = 'lce/get-auth-state';
const AUTH_STATUS_CACHE_MS = 1500;

type ButtonState = 'idle' | 'loading' | 'success' | 'error';

const BUTTON_TEXT_BY_STATE: Record<ButtonState, string> = {
  idle: 'LC',
  loading: '...',
  success: 'OK',
  error: 'ER',
};

const buttonResetTimers = new WeakMap<HTMLButtonElement, number>();

const inpageStyles = `
.lce-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border: none;
  border-radius: 999px;
  padding: 0;
  font-family: "Segoe UI", "Helvetica Neue", sans-serif;
  font-size: 11px;
  font-weight: 700;
  cursor: pointer;
  background: transparent;
  color: #72767B;
  transition: background-color 140ms ease, color 140ms ease, transform 120ms ease;
}
.lce-button:hover { background: rgba(29, 155, 240, 0.1); color: #1d9bf0; }
.lce-button:disabled { opacity: 1; }
.lce-button.is-loading {
  background: rgba(29, 155, 240, 0.1);
  color: #1d9bf0;
}
.lce-button.is-success {
  background: #30d158;
  color: #0f0f0f;
}
.lce-button.is-error {
  background: #ff453a;
  color: #fff;
}
.lce-button-floating { position: fixed; right: 20px; bottom: 24px; z-index: 2147483646; }
.lce-twitter-action-slot {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-left: 4px;
  flex: 0 0 auto;
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

type ToastLevel = 'success' | 'error' | 'info';

const TOAST_STYLE_ID = 'lce-toast-shared-style';
const TOAST_CONTAINER_ID = 'lce-toast-container';

let toastHideTimeout: number | null = null;
let toastListenerRegistered = false;
let authStateKnown = false;
let authStateHasSettings = false;
let authStateCheckedAt = 0;
let authStatePromise: Promise<boolean> | null = null;

const ensureToastStyles = (): void => {
  if (document.getElementById(TOAST_STYLE_ID)) {
    return;
  }

  const styleNode = document.createElement('style');
  styleNode.id = TOAST_STYLE_ID;
  styleNode.textContent = `
.lce-toast-container {
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: 2147483647;
  display: flex;
  flex-direction: column;
  gap: 8px;
  pointer-events: none;
}
.lce-toast {
  min-width: 200px;
  max-width: 360px;
  border-radius: 10px;
  padding: 10px 12px;
  font-family: "Segoe UI", "Helvetica Neue", sans-serif;
  font-size: 13px;
  font-weight: 600;
  color: #fff;
  box-shadow: 0 12px 24px rgba(0, 0, 0, 0.3);
}
.lce-toast-success { background: linear-gradient(135deg, #2e7d32, #43a047); }
.lce-toast-error { background: linear-gradient(135deg, #b71c1c, #e53935); }
.lce-toast-info { background: linear-gradient(135deg, #1565c0, #1e88e5); }
`;
  document.head.appendChild(styleNode);
};

const getToastContainer = (): HTMLDivElement => {
  let container = document.getElementById(TOAST_CONTAINER_ID) as HTMLDivElement | null;

  if (!container) {
    container = document.createElement('div');
    container.id = TOAST_CONTAINER_ID;
    container.className = 'lce-toast-container';
    document.body.appendChild(container);
  }

  return container;
};

const showToast = (level: ToastLevel, message: string): void => {
  if (!message || !message.trim()) {
    return;
  }

  ensureToastStyles();

  const container = getToastContainer();
  const toastNode = document.createElement('div');
  toastNode.className = `lce-toast lce-toast-${level}`;
  toastNode.textContent = message;
  container.replaceChildren(toastNode);

  if (toastHideTimeout !== null) {
    window.clearTimeout(toastHideTimeout);
  }

  toastHideTimeout = window.setTimeout(() => {
    toastNode.remove();
  }, 3500);
};

const isShowToastPayload = (value: unknown): value is { type: string; level?: unknown; message?: unknown } => {
  if (!value || typeof value !== 'object') {
    return false;
  }

  return (value as { type?: unknown }).type === 'lce/show-toast';
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

const isAuthStateResponse = (value: unknown): value is { ok: boolean; hasSettings: boolean } => {
  if (!value || typeof value !== 'object') {
    return false;
  }

  const payload = value as Record<string, unknown>;
  return typeof payload.ok === 'boolean' && typeof payload.hasSettings === 'boolean';
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

const normalizeTwitterStatusUrl = (rawUrl: string, base?: string): string | null => {
  let parsed: URL;

  try {
    parsed = new URL(rawUrl, base);
  } catch {
    return null;
  }

  const host = parsed.hostname.toLowerCase();

  if (!['x.com', 'www.x.com', 'twitter.com', 'www.twitter.com'].includes(host)) {
    return null;
  }

  const userStatusMatch = parsed.pathname.match(/^\/([^/]+)\/status\/(\d+)/i);

  if (userStatusMatch) {
    return `${parsed.origin}/${userStatusMatch[1]}/status/${userStatusMatch[2]}`;
  }

  const webStatusMatch = parsed.pathname.match(/^\/i\/web\/status\/(\d+)/i);

  if (webStatusMatch) {
    return `${parsed.origin}/i/web/status/${webStatusMatch[1]}`;
  }

  return null;
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

  button.title = title || 'Envoyer ce tweet vers LiveChat';
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

const createActionButton = (article: HTMLElement): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'lce-button';
  setButtonState(button, 'idle');

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
        const targetUrl = button.dataset.targetUrl || resolveTweetStatusUrl(article);

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
    })();
  });

  return button;
};

const createFloatingButton = (): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'lce-button lce-button-floating';
  setButtonState(button, 'idle');

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
        const targetUrl = button.dataset.targetUrl || normalizeTwitterStatusUrl(window.location.href, window.location.origin);

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
    })();
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

  button.dataset.targetUrl = statusUrl;
  return true;
};

const upsertFloatingButton = (targetUrl: string): void => {
  let floatingButton = document.getElementById(FLOATING_BUTTON_ID) as HTMLButtonElement | null;

  if (!floatingButton) {
    floatingButton = createFloatingButton();
    floatingButton.id = FLOATING_BUTTON_ID;
    document.body.appendChild(floatingButton);
  }

  floatingButton.dataset.targetUrl = targetUrl;
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

const startObservedScanner = (scan: () => void | Promise<void>): void => {
  let scanQueued = false;
  let lastUrl = window.location.href;

  const runScan = () => {
    scanQueued = false;
    void Promise.resolve(scan()).catch(() => {
      // Ignore scanner errors to keep observer alive.
    });
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

  queueScan();
};

registerToastListener();
startObservedScanner(scanTweets);
