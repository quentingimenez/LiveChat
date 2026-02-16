const STYLE_ID = 'lce-tiktok-style';
const FLOATING_BUTTON_ID = 'lce-tiktok-floating-button';
const DEFAULT_BUTTON_TITLE = 'Envoyer ce TikTok vers LiveChat';
const GET_ACTIVE_MEDIA_URL_TYPE = 'lce/get-active-media-url';

type ButtonState = 'idle' | 'loading' | 'success' | 'error';

const BUTTON_TEXT_BY_STATE: Record<ButtonState, string> = {
  idle: 'LC',
  loading: '...',
  success: 'OK',
  error: 'ER',
};

const buttonResetTimers = new WeakMap<HTMLButtonElement, number>();

const inpageStyles = `
.lce-button-tiktok {
  position: fixed;
  right: 22px;
  bottom: 24px;
  width: 56px;
  height: 56px;
  border: 2px solid rgba(255, 255, 255, 0.18);
  border-radius: 999px;
  background: rgba(22, 24, 35, 0.92);
  color: #fff;
  font-family: "TikTokDisplayFont", "ProximaNova", "Segoe UI", sans-serif;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.02em;
  cursor: pointer;
  z-index: 2147483646;
  pointer-events: auto;
  transition: transform 120ms ease, background-color 180ms ease, color 180ms ease, box-shadow 180ms ease;
}
.lce-button-tiktok:hover {
  background: rgba(22, 24, 35, 1);
}
.lce-button-tiktok:disabled {
  opacity: 1;
}
.lce-button-tiktok.is-loading {
  background: #fed54a;
  color: #0f0f0f;
}
.lce-button-tiktok.is-success {
  background: #25f4ee;
  color: #0f0f0f;
}
.lce-button-tiktok.is-error {
  background: #fe2c55;
  color: #fff;
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

const normalizeTikTokMediaUrl = (rawUrl: string, base?: string): string | null => {
  let parsed: URL;

  try {
    parsed = new URL(rawUrl, base);
  } catch {
    return null;
  }

  if (!parsed.hostname.toLowerCase().includes('tiktok.com')) {
    return null;
  }

  const namedMediaMatch = parsed.pathname.match(/^\/@([^/]+)\/(video|photo)\/(\d+)/i);

  if (namedMediaMatch) {
    const handle = namedMediaMatch[1];
    const mediaType = namedMediaMatch[2].toLowerCase();
    const mediaId = namedMediaMatch[3];
    return `https://www.tiktok.com/@${handle}/${mediaType}/${mediaId}`;
  }

  const genericMediaMatch = parsed.pathname.match(/\/(video|photo)\/(\d+)/i);

  if (genericMediaMatch) {
    const mediaType = genericMediaMatch[1].toLowerCase();
    const mediaId = genericMediaMatch[2];
    return `https://www.tiktok.com/${mediaType}/${mediaId}`;
  }

  return null;
};

const getCurrentTikTokMediaUrl = (): string | null => {
  return normalizeTikTokMediaUrl(window.location.href, window.location.href);
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

const createFloatingButton = (): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'lce-button-tiktok';
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
        const targetUrl = getCurrentTikTokMediaUrl();

        if (!targetUrl) {
          setButtonState(button, 'error', 'Ouvre une URL TikTok vidéo/photo pour envoyer.');
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

const removeFloatingButton = (): void => {
  const floatingButton = document.getElementById(FLOATING_BUTTON_ID);

  if (floatingButton) {
    floatingButton.remove();
  }
};

const upsertFloatingButton = (): void => {
  let floatingButton = document.getElementById(FLOATING_BUTTON_ID) as HTMLButtonElement | null;

  if (!floatingButton) {
    floatingButton = createFloatingButton();
    floatingButton.id = FLOATING_BUTTON_ID;
    document.body.appendChild(floatingButton);
  }

  floatingButton.title = DEFAULT_BUTTON_TITLE;
};

const scanTikTokPage = (): void => {
  const mediaUrl = getCurrentTikTokMediaUrl();

  if (!mediaUrl) {
    removeFloatingButton();
    return;
  }

  upsertFloatingButton();
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
      (message: unknown, _sender: chrome.runtime.MessageSender, sendResponse: (response?: unknown) => void) => {
        if (!isGetActiveMediaUrlMessage(message)) {
          return;
        }

        sendResponse({
          ok: true,
          url: getCurrentTikTokMediaUrl(),
        });
      },
    );
  } catch {
    // Ignore runtime invalidation while the extension is reloading.
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

registerActiveMediaUrlListener();
startObservedScanner(scanTikTokPage);
