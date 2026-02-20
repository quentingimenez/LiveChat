const STYLE_ID = 'lce-twitter-style';
const TOAST_CONTAINER_ID = 'lce-twitter-toast-container';
const BUTTON_ATTRIBUTE = 'data-lce-twitter-button';
const SLOT_ATTRIBUTE = 'data-lce-twitter-slot';
const FLOATING_BUTTON_ID = 'lce-twitter-floating-button';

let toastHideTimeout: number | null = null;
let toastListenerRegistered = false;

const inpageStyles = `
.lce-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 36px;
  border: none;
  border-radius: 999px;
  padding: 0 16px;
  font-family: "Segoe UI", "Helvetica Neue", sans-serif;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  background: transparent;
  color: #72767B;
  transition: background-color 140ms ease, color 140ms ease;
}
.lce-button:hover { background: rgba(29, 155, 240, 0.1); color: #1d9bf0; }
.lce-button-floating { position: fixed; right: 20px; bottom: 24px; z-index: 2147483646; }
.lce-twitter-action-slot {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-left: 6px;
  flex: 0 0 auto;
}
.lce-toast-container { position: fixed; right: 16px; bottom: 16px; z-index: 2147483647; }
.lce-toast { border-radius: 10px; padding: 10px 12px; font-family: "Segoe UI", sans-serif; font-size: 13px; font-weight: 600; color: #fff; }
.lce-toast-success { background: linear-gradient(135deg, #2e7d32, #43a047); }
.lce-toast-error { background: linear-gradient(135deg, #b71c1c, #e53935); }
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

const showToast = (level: 'success' | 'error', message: string): void => {
  ensureStyles();

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

const registerToastListener = (): void => {
  if (toastListenerRegistered) {
    return;
  }

  chrome.runtime.onMessage.addListener((message: unknown) => {
    if (!message || typeof message !== 'object') {
      return;
    }

    const payload = message as { type?: unknown; level?: unknown; message?: unknown };

    if (payload.type !== 'lce/show-toast') {
      return;
    }

    if (typeof payload.message !== 'string' || !payload.message.trim()) {
      return;
    }

    const level = payload.level === 'success' ? 'success' : payload.level === 'error' ? 'error' : 'success';
    showToast(level, payload.message);
  });

  toastListenerRegistered = true;
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
  try {
    const response = (await chrome.runtime.sendMessage({
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
  button.textContent = 'LiveChat';
  button.title = 'Envoyer ce tweet vers LiveChat';

  button.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();

    void (async () => {
      if (button.disabled) {
        return;
      }

      button.disabled = true;

      try {
        const targetUrl = button.dataset.targetUrl || resolveTweetStatusUrl(article);

        if (!targetUrl) {
          showToast('error', 'Impossible de détecter l’URL du tweet.');
          return;
        }

        const response = await sendQuick(targetUrl);
        showToast(response.ok ? 'success' : 'error', response.message);
      } finally {
        window.setTimeout(() => {
          button.disabled = false;
        }, 250);
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
  button.textContent = 'LiveChat';
  button.title = 'Envoyer ce tweet vers LiveChat';

  button.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();

    void (async () => {
      if (button.disabled) {
        return;
      }

      button.disabled = true;

      try {
        const targetUrl = button.dataset.targetUrl || normalizeTwitterStatusUrl(window.location.href, window.location.origin);

        if (!targetUrl) {
          showToast('error', 'Impossible de détecter l’URL du tweet.');
          return;
        }

        const response = await sendQuick(targetUrl);
        showToast(response.ok ? 'success' : 'error', response.message);
      } finally {
        window.setTimeout(() => {
          button.disabled = false;
        }, 250);
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

const scanTweets = (): void => {
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

  queueScan();
};

registerToastListener();
startObservedScanner(scanTweets);
