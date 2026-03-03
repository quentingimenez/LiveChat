const STYLE_ID = 'lce-youtube-style';
const BUTTON_ATTRIBUTE = 'data-lce-youtube-button';
const LEGACY_FLOATING_BUTTON_ID = 'lce-youtube-floating-button';
const SHORTS_FLOATING_BUTTON_ID = 'lce-youtube-shorts-floating-button';
const WATCH_FLOATING_BUTTON_ID = 'lce-youtube-watch-floating-button';
const WATCH_SLOT_ATTRIBUTE = 'data-lce-youtube-watch-slot';
const DEFAULT_BUTTON_TITLE = 'Envoyer la vidéo YouTube vers LiveChat';
const GET_AUTH_STATE_TYPE = 'lce/get-auth-state';
const AUTH_STATUS_CACHE_MS = 1500;

interface LongPressComposeOptions {
  button: HTMLButtonElement;
  onSubmit: (text: string) => Promise<void>;
  holdDurationMs?: number;
  title?: string;
  placeholder?: string;
  submitLabel?: string;
  cancelLabel?: string;
}

interface LongPressComposeBinding {
  consumeSuppressedClick: () => boolean;
  closePopover: () => void;
}

const attachLongPressCompose = (() => {
  const composeStyleId = 'lce-compose-popover-style';
  const defaultHoldDurationMs = 520;
  const moveCancelThresholdPx = 12;
  let activePopoverClose: (() => void) | null = null;

  const ensureComposePopoverStyles = (): void => {
    if (document.getElementById(composeStyleId)) {
      return;
    }

    const styleNode = document.createElement('style');
    styleNode.id = composeStyleId;
    styleNode.textContent = `
.lce-compose-popover {
  position: fixed;
  z-index: 2147483647;
  width: min(320px, calc(100vw - 20px));
  max-height: min(280px, calc(100vh - 24px));
  border-radius: 12px;
  border: 1px solid rgba(255, 255, 255, 0.18);
  padding: 10px;
  background: rgba(15, 15, 15, 0.96);
  backdrop-filter: blur(10px);
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.5);
  color: #f5f5f5;
  font-family: "Segoe UI", "Helvetica Neue", sans-serif;
  pointer-events: auto;
}
.lce-compose-popover-title {
  margin: 0 0 8px;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.02em;
  color: rgba(255, 255, 255, 0.96);
}
.lce-compose-popover-textarea {
  width: 100%;
  min-height: 78px;
  max-height: 120px;
  resize: vertical;
  border-radius: 8px;
  border: 1px solid rgba(255, 255, 255, 0.22);
  background: rgba(0, 0, 0, 0.28);
  color: #fff;
  font-family: inherit;
  font-size: 12px;
  line-height: 1.4;
  padding: 8px 9px;
  outline: none;
}
.lce-compose-popover-textarea:focus {
  border-color: #66b3ff;
  box-shadow: 0 0 0 2px rgba(102, 179, 255, 0.22);
}
.lce-compose-popover-textarea::placeholder {
  color: rgba(255, 255, 255, 0.56);
}
.lce-compose-popover-error {
  min-height: 16px;
  margin-top: 6px;
  font-size: 11px;
  line-height: 1.3;
  color: #ff8a80;
}
.lce-compose-popover-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 8px;
}
.lce-compose-popover-button {
  border: none;
  border-radius: 999px;
  padding: 6px 12px;
  font-family: inherit;
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
}
.lce-compose-popover-button-cancel {
  background: rgba(255, 255, 255, 0.16);
  color: #fff;
}
.lce-compose-popover-button-submit {
  background: #1d9bf0;
  color: #fff;
}
.lce-compose-popover-button:disabled {
  opacity: 0.65;
  cursor: default;
}
`;
    document.head.appendChild(styleNode);
  };

  const composeClamp = (value: number, min: number, max: number): number => {
    return Math.min(max, Math.max(min, value));
  };

  const toErrorMessage = (error: unknown): string => {
    if (error instanceof Error && error.message.trim()) {
      return error.message.trim();
    }

    if (typeof error === 'string' && error.trim()) {
      return error.trim();
    }

    return 'Erreur pendant l’envoi.';
  };

  const closeActivePopover = (): void => {
    if (!activePopoverClose) {
      return;
    }

    const close = activePopoverClose;
    activePopoverClose = null;
    close();
  };

  const openComposePopover = (options: LongPressComposeOptions): void => {
    ensureComposePopoverStyles();
    closeActivePopover();

    const popover = document.createElement('div');
    popover.className = 'lce-compose-popover';
    popover.setAttribute('role', 'dialog');
    popover.setAttribute('aria-modal', 'false');
    popover.setAttribute('aria-label', options.title || 'Envoyer vers LiveChat avec texte');

    const title = document.createElement('p');
    title.className = 'lce-compose-popover-title';
    title.textContent = options.title || 'Envoyer vers LiveChat avec texte';

    const textarea = document.createElement('textarea');
    textarea.className = 'lce-compose-popover-textarea';
    textarea.placeholder = options.placeholder || 'Ajouter un texte (optionnel)';
    textarea.spellcheck = true;

    const errorNode = document.createElement('div');
    errorNode.className = 'lce-compose-popover-error';

    const actions = document.createElement('div');
    actions.className = 'lce-compose-popover-actions';

    const cancelButton = document.createElement('button');
    cancelButton.type = 'button';
    cancelButton.className = 'lce-compose-popover-button lce-compose-popover-button-cancel';
    cancelButton.textContent = options.cancelLabel || 'Annuler';

    const submitButton = document.createElement('button');
    submitButton.type = 'button';
    submitButton.className = 'lce-compose-popover-button lce-compose-popover-button-submit';
    submitButton.textContent = options.submitLabel || 'Envoyer';

    actions.append(cancelButton, submitButton);
    popover.append(title, textarea, errorNode, actions);
    document.body.appendChild(popover);

    let closed = false;
    let busy = false;

    const setBusy = (nextBusy: boolean): void => {
      busy = nextBusy;
      textarea.disabled = nextBusy;
      cancelButton.disabled = nextBusy;
      submitButton.disabled = nextBusy;
      submitButton.textContent = nextBusy ? 'Envoi...' : options.submitLabel || 'Envoyer';
    };

    const reposition = (): void => {
      if (!options.button.isConnected || !popover.isConnected) {
        close();
        return;
      }

      const rect = options.button.getBoundingClientRect();
      const width = popover.offsetWidth || 320;
      const maxLeft = Math.max(8, window.innerWidth - width - 8);
      const left = composeClamp(rect.left + rect.width / 2 - width / 2, 8, maxLeft);
      const topAbove = rect.top - popover.offsetHeight - 10;
      const topBelow = rect.bottom + 10;
      const maxTop = Math.max(8, window.innerHeight - popover.offsetHeight - 8);
      const top = topAbove >= 8 ? topAbove : composeClamp(topBelow, 8, maxTop);

      popover.style.left = `${left}px`;
      popover.style.top = `${top}px`;
    };

    const close = (): void => {
      if (closed) {
        return;
      }

      closed = true;
      if (activePopoverClose === close) {
        activePopoverClose = null;
      }

      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
      document.removeEventListener('pointerdown', handleDocumentPointerDown, true);
      document.removeEventListener('keydown', handleDocumentKeyDown, true);
      popover.remove();
    };

    const handleDocumentPointerDown = (event: PointerEvent): void => {
      const target = event.target as Node | null;

      if (!target || (popover.contains(target) || options.button.contains(target))) {
        return;
      }

      close();
    };

    const handleDocumentKeyDown = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape') {
        return;
      }

      event.preventDefault();
      close();
    };

    const submit = (): void => {
      if (busy) {
        return;
      }

      void (async () => {
        setBusy(true);
        errorNode.textContent = '';

        try {
          await options.onSubmit(textarea.value);
          close();
        } catch (error) {
          errorNode.textContent = toErrorMessage(error);
        } finally {
          if (!closed) {
            setBusy(false);
          }
        }
      })();
    };

    cancelButton.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      close();
    });

    submitButton.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      submit();
    });

    textarea.addEventListener('keydown', (event) => {
      if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
        event.preventDefault();
        submit();
        return;
      }

      if (event.key !== 'Escape') {
        return;
      }

      event.preventDefault();
      close();
    });

    popover.addEventListener('pointerdown', (event) => {
      event.stopPropagation();
    });

    popover.addEventListener('click', (event) => {
      event.stopPropagation();
    });

    activePopoverClose = close;
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    document.addEventListener('pointerdown', handleDocumentPointerDown, true);
    document.addEventListener('keydown', handleDocumentKeyDown, true);

    reposition();
    window.requestAnimationFrame(() => {
      if (!closed) {
        textarea.focus();
      }
    });
  };

  return (options: LongPressComposeOptions): LongPressComposeBinding => {
    const holdDurationMs = Math.max(250, options.holdDurationMs ?? defaultHoldDurationMs);
    let pressTimer: number | null = null;
    let isPointerDown = false;
    let pointerId: number | null = null;
    let startX = 0;
    let startY = 0;
    let suppressClicksUntil = 0;

    const clearPressTimer = (): void => {
      if (pressTimer === null) {
        return;
      }

      window.clearTimeout(pressTimer);
      pressTimer = null;
    };

    const cancelPress = (): void => {
      isPointerDown = false;
      pointerId = null;
      clearPressTimer();
    };

    options.button.addEventListener('pointerdown', (event) => {
      if (event.pointerType === 'mouse' && event.button !== 0) {
        return;
      }

      if (options.button.disabled) {
        return;
      }

      isPointerDown = true;
      pointerId = event.pointerId;
      startX = event.clientX;
      startY = event.clientY;

      clearPressTimer();
      pressTimer = window.setTimeout(() => {
        pressTimer = null;

        if (!isPointerDown || options.button.disabled) {
          return;
        }

        suppressClicksUntil = Date.now() + 1200;
        openComposePopover(options);
      }, holdDurationMs);
    });

    options.button.addEventListener('pointermove', (event) => {
      if (!isPointerDown || pointerId !== event.pointerId || pressTimer === null) {
        return;
      }

      const dx = Math.abs(event.clientX - startX);
      const dy = Math.abs(event.clientY - startY);

      if (dx > moveCancelThresholdPx || dy > moveCancelThresholdPx) {
        clearPressTimer();
      }
    });

    options.button.addEventListener('pointerup', (event) => {
      if (pointerId !== event.pointerId) {
        return;
      }

      cancelPress();
    });

    options.button.addEventListener('pointercancel', () => {
      cancelPress();
    });

    options.button.addEventListener('pointerleave', (event) => {
      if (event.pointerType === 'mouse') {
        cancelPress();
      }
    });

    options.button.addEventListener('contextmenu', (event) => {
      if (Date.now() > suppressClicksUntil) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();
    });

    return {
      consumeSuppressedClick: () => {
        if (Date.now() > suppressClicksUntil) {
          return false;
        }

        suppressClicksUntil = 0;
        return true;
      },
      closePopover: () => {
        closeActivePopover();
      },
    };
  };
})();

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
  border: none;
  border-radius: 999px;
  padding: 0;
  font-family: "Roboto", "Arial", sans-serif;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.02em;
  cursor: pointer;
  pointer-events: auto;
  background: #272727;
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
  margin-left: 9px;
  margin-right: 7px;
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
  margin-left: 4px;
  margin-right: 4px;
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

const sendCompose = async (url: string, text: string): Promise<{ ok: boolean; message: string }> => {
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
      url,
      text,
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
        const targetUrl = button.dataset.targetUrl || getCurrentYoutubeUrl();

        if (!targetUrl) {
          const message = 'Impossible de détecter l’URL YouTube courante.';
          setButtonState(button, 'error', message);
          showToast('error', message);
          resetButtonStateLater(button);
          throw new Error(message);
        }

        const response = await sendCompose(targetUrl, text);
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

  button.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();

    if (longPressCompose.consumeSuppressedClick()) {
      return;
    }

    longPressCompose.closePopover();

    void (async () => {
      if (button.disabled) {
        return;
      }

      button.disabled = true;
      setButtonState(button, 'loading', 'Envoi en cours...');

      try {
        const targetUrl = button.dataset.targetUrl || getCurrentYoutubeUrl();

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

  document.addEventListener('fullscreenchange', queueScan);
  window.addEventListener('resize', queueScan);

  queueScan();
};

registerToastListener();
startObservedScanner(scanYoutubeTargets);
