const STYLE_ID = 'lce-tiktok-style';
const INLINE_BUTTON_ID = 'lce-tiktok-inline-button';
const INLINE_SLOT_ID = 'lce-tiktok-inline-slot';
const DEFAULT_BUTTON_TITLE = 'Envoyer ce TikTok vers LiveChat';
const GET_ACTIVE_MEDIA_URL_TYPE = 'lce/get-active-media-url';
const GET_AUTH_STATE_TYPE = 'lce/get-auth-state';
const TIKTOK_GET_CAPTURED_URL_TYPE = 'lce/tiktok-get-captured-url';
const TIKTOK_SYNC_ACTIVE_ITEM_TYPE = 'lce/tiktok-sync-active-item';
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
  width: 48px;
  height: 48px;
  border: none;
  border-radius: 999px;
  background: #1F1F1F;
  color: #fff;
  font-family: "TikTokDisplayFont", "ProximaNova", "Segoe UI", sans-serif;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.02em;
  cursor: pointer;
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
.lce-tiktok-slot {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-start;
  margin-top: 0;
  pointer-events: auto;
}
.lce-tiktok-slot.is-vertical {
  margin-top: 10px;
  margin-bottom: 14px;
}
.lce-tiktok-slot.is-horizontal {
  margin-top: 0;
  margin-bottom: 0;
}
.lce-tiktok-slot.is-horizontal .lce-button-tiktok {
  width: 40px;
  height: 40px;
  font-size: 10px;
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

const readRuntime = (): typeof chrome.runtime | null => {
  if (runtimeContextInvalidated) {
    return null;
  }

  try {
    if (typeof chrome === 'undefined') {
      return null;
    }

    return chrome.runtime || null;
  } catch (error) {
    markRuntimeContextInvalidatedIfNeeded(error);
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
    markRuntimeContextInvalidatedIfNeeded(error);
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
    markRuntimeContextInvalidatedIfNeeded(error);
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

const createActionButton = (): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'lce-button-tiktok';
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
        const targetUrl = await resolveCurrentTikTokMediaUrl();

        if (!targetUrl) {
          const message = 'Impossible de détecter la vidéo TikTok active. Fais défiler puis réessaie.';
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

      if (rect.width >= 40 && rect.height >= 80 && rect.width <= 480 && rect.height <= window.innerHeight) {
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

  const horizontalHost = container.firstElementChild instanceof HTMLDivElement ? container.firstElementChild : container;
  const actionLayout = detectActionLayout(container);
  slot.classList.remove('is-vertical', 'is-horizontal');
  slot.classList.add(actionLayout === 'vertical' ? 'is-vertical' : 'is-horizontal');

  if (actionLayout === 'horizontal') {
    if (slot.parentElement !== horizontalHost) {
      horizontalHost.prepend(slot);
    } else if (horizontalHost.firstElementChild !== slot) {
      horizontalHost.prepend(slot);
    }
  } else {
    const containerChildrenWithoutSlot = Array.from(container.children).filter((child) => child !== slot);
    const firstContainerChild = containerChildrenWithoutSlot[0] || null;
    const insertionPoint = firstContainerChild ? firstContainerChild.nextElementSibling : container.firstElementChild;

    if (slot.parentElement !== container) {
      container.insertBefore(slot, insertionPoint);
    } else if (firstContainerChild && slot.previousElementSibling !== firstContainerChild) {
      container.insertBefore(slot, insertionPoint);
    } else if (!firstContainerChild && container.firstElementChild !== slot) {
      container.prepend(slot);
    }
  }

  button.title = DEFAULT_BUTTON_TITLE;
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
      (message: unknown, _sender: chrome.runtime.MessageSender, sendResponse: (response?: unknown) => void) => {
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

registerActiveMediaUrlListener();
registerToastListener();
startObservedScanner(scanTikTokPage);
