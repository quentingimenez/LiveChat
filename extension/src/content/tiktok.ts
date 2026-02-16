const STYLE_ID = 'lce-tiktok-style';
const BUTTON_ATTRIBUTE = 'data-lce-tiktok-button';
const ACTION_ITEM_ATTRIBUTE = 'data-lce-tiktok-action-item';
const FLOATING_BUTTON_ID = 'lce-tiktok-floating-button';
const DEFAULT_BUTTON_TITLE = 'Envoyer ce TikTok vers LiveChat';
const GET_ACTIVE_MEDIA_URL_TYPE = 'lce/get-active-media-url';
const TIKTOK_SYNC_ACTIVE_ITEM_TYPE = 'lce/tiktok-sync-active-item';
const TIKTOK_GET_CAPTURED_URL_TYPE = 'lce/tiktok-get-captured-url';
const MEDIA_LINK_SELECTOR = 'a[href*="/video/"], a[href*="/photo/"]';
const LIKE_ICON_SELECTOR = [
  '[data-e2e="browse-like-icon"]',
  '[data-e2e="like-icon"]',
  '[data-e2e="video-like-icon"]',
  '[class*="like-icon"]',
  '[class*="like-count"]',
].join(', ');
const COMMENT_ICON_SELECTOR = ['[data-e2e*="comment-icon"]', '[class*="comment-icon"]', '[class*="comment-count"]'].join(
  ', ',
);
const SHARE_ICON_SELECTOR = ['[data-e2e*="share-icon"]', '[class*="share-icon"]', '[class*="share-count"]'].join(', ');
const ACTION_ICON_SELECTOR = `${LIKE_ICON_SELECTOR}, ${COMMENT_ICON_SELECTOR}, ${SHARE_ICON_SELECTOR}`;
const ACTION_INTERACTIVE_SELECTOR = 'button, [role="button"]';
const ACTIVE_SYNC_HEARTBEAT_MS = 5000;

type ButtonState = 'idle' | 'loading' | 'success' | 'error';
const BUTTON_TEXT_BY_STATE: Record<ButtonState, string> = {
  idle: 'LC',
  loading: '...',
  success: 'OK',
  error: 'ER',
};

interface MediaAnchor {
  url: string;
  centerX: number;
  centerY: number;
}

const buttonResetTimers = new WeakMap<HTMLButtonElement, number>();
let lastSyncedItemId: string | null = null;
let lastSyncedUrl: string | null = null;
let lastSyncAt = 0;

const isRuntimeAvailable = (): boolean => {
  try {
    return typeof chrome !== 'undefined' && !!chrome.runtime && typeof chrome.runtime.id === 'string';
  } catch {
    return false;
  }
};

const inpageStyles = `
.lce-tiktok-action-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  margin-bottom: 12px;
}
.lce-button-tiktok {
  width: 48px;
  height: 48px;
  border: 1px solid rgba(255, 255, 255, 0.18);
  border-radius: 999px;
  background: rgba(22, 24, 35, 0.9);
  color: #fff;
  font-family: "TikTokDisplayFont", "ProximaNova", "Segoe UI", sans-serif;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.02em;
  cursor: pointer;
  transition: transform 120ms ease, background-color 180ms ease, color 180ms ease, box-shadow 180ms ease;
}
.lce-button-tiktok-floating {
  position: fixed;
  right: 22px;
  bottom: 24px;
  width: 56px;
  height: 56px;
  z-index: 2147483646;
  border-width: 2px;
  pointer-events: auto;
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
  animation: lce-tiktok-pulse 0.95s ease-in-out infinite;
}
.lce-button-tiktok.is-success {
  background: #25f4ee;
  color: #0f0f0f;
  animation: lce-tiktok-pulse-success 0.9s ease-in-out 2;
}
.lce-button-tiktok.is-error {
  background: #fe2c55;
  color: #fff;
  animation: lce-tiktok-pulse-error 0.9s ease-in-out 2;
}
.lce-tiktok-action-label {
  color: #fff;
  font-family: "TikTokDisplayFont", "ProximaNova", "Segoe UI", sans-serif;
  font-size: 12px;
  font-weight: 600;
  line-height: 1;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.45);
}
@keyframes lce-tiktok-pulse {
  0% { transform: scale(1); box-shadow: 0 0 0 0 rgba(37, 244, 238, 0.35); }
  70% { transform: scale(1.05); box-shadow: 0 0 0 10px rgba(37, 244, 238, 0); }
  100% { transform: scale(1); box-shadow: 0 0 0 0 rgba(37, 244, 238, 0); }
}
@keyframes lce-tiktok-pulse-success {
  0% { transform: scale(1); box-shadow: 0 0 0 0 rgba(37, 244, 238, 0.45); }
  70% { transform: scale(1.07); box-shadow: 0 0 0 12px rgba(37, 244, 238, 0); }
  100% { transform: scale(1); box-shadow: 0 0 0 0 rgba(37, 244, 238, 0); }
}
@keyframes lce-tiktok-pulse-error {
  0% { transform: scale(1); box-shadow: 0 0 0 0 rgba(254, 44, 85, 0.45); }
  70% { transform: scale(1.07); box-shadow: 0 0 0 12px rgba(254, 44, 85, 0); }
  100% { transform: scale(1); box-shadow: 0 0 0 0 rgba(254, 44, 85, 0); }
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

const extractMediaItemId = (url: string | null | undefined): string | null => {
  if (!url) {
    return null;
  }

  const match = url.match(/\/(?:video|photo)\/(\d{15,22})/i);
  return match?.[1] || null;
};

const resolveCapturedTikTokUrl = async (): Promise<string | null> => {
  if (!isRuntimeAvailable()) {
    return null;
  }

  try {
    const response = (await chrome.runtime.sendMessage({
      type: TIKTOK_GET_CAPTURED_URL_TYPE,
    })) as { ok?: unknown; url?: unknown };

    if (!response || response.ok !== true || typeof response.url !== 'string' || !response.url.trim()) {
      return null;
    }

    const normalized = normalizeTikTokMediaUrl(response.url, window.location.href);
    return normalized || response.url;
  } catch {
    return null;
  }
};

const syncActiveMediaToBackground = (url: string | null): void => {
  if (!isRuntimeAvailable()) {
    return;
  }

  const itemId = extractMediaItemId(url);
  const now = Date.now();
  const hasChanged = itemId !== lastSyncedItemId || (url || null) !== lastSyncedUrl;
  const heartbeatElapsed = now - lastSyncAt >= ACTIVE_SYNC_HEARTBEAT_MS;

  if (!hasChanged && !heartbeatElapsed) {
    return;
  }

  lastSyncedItemId = itemId;
  lastSyncedUrl = url || null;
  lastSyncAt = now;

  try {
    void chrome.runtime
      .sendMessage({
        type: TIKTOK_SYNC_ACTIVE_ITEM_TYPE,
        itemId,
        url: url || null,
      })
      .catch(() => {
        // Ignore sync errors when the service worker is restarting.
      });
  } catch {
    // Ignore synchronous runtime invalidation errors.
  }
};

const sendQuick = async (url: string): Promise<{ ok: boolean; message: string }> => {
  if (!isRuntimeAvailable()) {
    return {
      ok: false,
      message: 'Contexte extension invalide. Recharge la page TikTok puis réessaie.',
    };
  }

  try {
    const response = (await chrome.runtime.sendMessage({
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

const computeVisibleArea = (rect: DOMRect): number => {
  const visibleWidth = Math.max(0, Math.min(rect.right, window.innerWidth) - Math.max(rect.left, 0));
  const visibleHeight = Math.max(0, Math.min(rect.bottom, window.innerHeight) - Math.max(rect.top, 0));
  return visibleWidth * visibleHeight;
};

const pickNearestMediaUrlToPoint = (
  anchors: MediaAnchor[],
  x: number,
  y: number,
  maxScore = Number.POSITIVE_INFINITY,
): string | null => {
  if (anchors.length === 0) {
    return null;
  }

  let bestMatch: { url: string; score: number } | null = null;

  for (const anchor of anchors) {
    const deltaX = Math.abs(anchor.centerX - x);
    const deltaY = Math.abs(anchor.centerY - y);
    const outOfViewportPenalty =
      anchor.centerX < -120 || anchor.centerX > window.innerWidth + 120 || anchor.centerY < -120 || anchor.centerY > window.innerHeight + 120
        ? 24_000
        : 0;
    const score = deltaY * 3 + deltaX + outOfViewportPenalty;

    if (!bestMatch || score < bestMatch.score) {
      bestMatch = { url: anchor.url, score };
    }
  }

  if (!bestMatch || bestMatch.score > maxScore) {
    return null;
  }

  return bestMatch.url;
};

const isAnchorNearVideoRect = (anchor: MediaAnchor, rect: DOMRect): boolean => {
  const paddingX = Math.max(220, rect.width * 0.42);
  const paddingY = Math.max(180, rect.height * 0.38);

  return (
    anchor.centerX >= rect.left - paddingX &&
    anchor.centerX <= rect.right + paddingX &&
    anchor.centerY >= rect.top - paddingY &&
    anchor.centerY <= rect.bottom + paddingY
  );
};

const resolveMediaUrlFromActiveVideo = (): string | null => {
  const videos = Array.from(document.querySelectorAll<HTMLVideoElement>('video'));

  let bestVideo: { element: HTMLVideoElement; score: number } | null = null;

  for (const video of videos) {
    const rect = video.getBoundingClientRect();

    if (rect.width < 180 || rect.height < 240) {
      continue;
    }

    const visibleArea = computeVisibleArea(rect);

    if (visibleArea < 35_000) {
      continue;
    }

    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height / 2;
    const distanceToViewportCenter = Math.abs(centerX - window.innerWidth / 2) + Math.abs(centerY - window.innerHeight / 2);
    const playingBonus = !video.paused && !video.ended ? 85_000 : 0;
    const score = visibleArea + playingBonus - distanceToViewportCenter * 35;

    if (!bestVideo || score > bestVideo.score) {
      bestVideo = { element: video, score };
    }
  }

  if (!bestVideo) {
    return null;
  }

  const videoRect = bestVideo.element.getBoundingClientRect();
  const videoCenterX = videoRect.left + videoRect.width / 2;
  const videoCenterY = videoRect.top + videoRect.height / 2;
  let scope: HTMLElement | null = bestVideo.element;

  for (let depth = 0; depth < 8 && scope; depth += 1) {
    const scopedAnchors = collectMediaAnchors(scope).filter((anchor) => isAnchorNearVideoRect(anchor, videoRect));
    const scopedUrl = pickNearestMediaUrlToPoint(scopedAnchors, videoCenterX, videoCenterY, 2_200);

    if (scopedUrl) {
      return scopedUrl;
    }

    scope = scope.parentElement;
  }

  const globalNearbyAnchors = collectMediaAnchors(document).filter((anchor) => isAnchorNearVideoRect(anchor, videoRect));
  const globalNearbyUrl = pickNearestMediaUrlToPoint(globalNearbyAnchors, videoCenterX, videoCenterY, 2_400);

  if (globalNearbyUrl) {
    return globalNearbyUrl;
  }

  return null;
};

const resolveMediaUrlFromViewportCenter = (): string | null => {
  const probePoints: Array<{ xRatio: number; yRatio: number }> = [
    { xRatio: 0.5, yRatio: 0.45 },
    { xRatio: 0.5, yRatio: 0.55 },
    { xRatio: 0.5, yRatio: 0.35 },
    { xRatio: 0.5, yRatio: 0.65 },
  ];

  for (const point of probePoints) {
    const probeX = Math.round(window.innerWidth * point.xRatio);
    const probeY = Math.round(window.innerHeight * point.yRatio);
    const startNode = document.elementFromPoint(probeX, probeY);

    let scope: HTMLElement | null =
      startNode instanceof HTMLElement ? startNode : startNode instanceof Element ? startNode.parentElement : null;

    for (let depth = 0; depth < 12 && scope; depth += 1) {
      const scopedAnchors = collectMediaAnchors(scope);
      const scopedUrl = pickNearestMediaUrlToPoint(scopedAnchors, probeX, probeY, 2_200);

      if (scopedUrl) {
        return scopedUrl;
      }

      scope = scope.parentElement;
    }
  }

  return null;
};

const collectMediaAnchors = (root: ParentNode = document): MediaAnchor[] => {
  const anchors = Array.from(root.querySelectorAll<HTMLAnchorElement>(MEDIA_LINK_SELECTOR));
  const mediaAnchors: MediaAnchor[] = [];
  const seenKeys = new Set<string>();

  for (const anchor of anchors) {
    const rawHref = anchor.getAttribute('href') || anchor.href;
    const normalized = normalizeTikTokMediaUrl(rawHref, window.location.href);

    if (!normalized) {
      continue;
    }

    const rect = anchor.getBoundingClientRect();

    if (rect.width < 1 || rect.height < 1) {
      continue;
    }

    const dedupeKey = `${normalized}|${Math.round(rect.left)}|${Math.round(rect.top)}`;

    if (seenKeys.has(dedupeKey)) {
      continue;
    }

    seenKeys.add(dedupeKey);
    mediaAnchors.push({
      url: normalized,
      centerX: rect.left + rect.width / 2,
      centerY: rect.top + rect.height / 2,
    });
  }

  return mediaAnchors;
};

const pickNearestMediaUrl = (actionColumn: HTMLElement, anchors: MediaAnchor[]): string | null => {
  if (anchors.length === 0) {
    return null;
  }

  const rect = actionColumn.getBoundingClientRect();
  const centerX = rect.left + rect.width / 2;
  const centerY = rect.top + rect.height / 2;
  let bestMatch: { url: string; score: number } | null = null;

  for (const anchor of anchors) {
    const deltaX = Math.abs(anchor.centerX - centerX);
    const deltaY = Math.abs(anchor.centerY - centerY);
    const isLikelyVisible = anchor.centerY >= -160 && anchor.centerY <= window.innerHeight + 160;
    const visibilityPenalty = isLikelyVisible ? 0 : 10_000;
    const score = deltaY * 3 + deltaX + visibilityPenalty;

    if (!bestMatch || score < bestMatch.score) {
      bestMatch = { url: anchor.url, score };
    }
  }

  return bestMatch?.url || null;
};

const pickViewportMediaUrl = (anchors: MediaAnchor[]): string | null => {
  if (anchors.length === 0) {
    return null;
  }

  const viewportCenterX = window.innerWidth / 2;
  const viewportCenterY = window.innerHeight / 2;
  let bestMatch: { url: string; score: number } | null = null;

  for (const anchor of anchors) {
    const deltaX = Math.abs(anchor.centerX - viewportCenterX);
    const deltaY = Math.abs(anchor.centerY - viewportCenterY);
    const outOfViewportXPenalty = anchor.centerX < -120 || anchor.centerX > window.innerWidth + 120 ? 12_000 : 0;
    const outOfViewportYPenalty = anchor.centerY < -120 || anchor.centerY > window.innerHeight + 120 ? 18_000 : 0;
    const score = deltaY * 2 + deltaX + outOfViewportXPenalty + outOfViewportYPenalty;

    if (!bestMatch || score < bestMatch.score) {
      bestMatch = { url: anchor.url, score };
    }
  }

  return bestMatch?.url || null;
};

const resolveDocumentMediaFallback = (): string | null => {
  const canonical = document.querySelector<HTMLLinkElement>('link[rel="canonical"]')?.href;
  const normalizedCanonical = canonical ? normalizeTikTokMediaUrl(canonical, window.location.href) : null;

  if (normalizedCanonical) {
    return normalizedCanonical;
  }

  const ogUrl = document.querySelector<HTMLMetaElement>('meta[property="og:url"]')?.content;
  const normalizedOgUrl = ogUrl ? normalizeTikTokMediaUrl(ogUrl, window.location.href) : null;

  if (normalizedOgUrl) {
    return normalizedOgUrl;
  }

  return normalizeTikTokMediaUrl(window.location.href);
};

const resolveCurrentMediaUrl = (anchors: MediaAnchor[] = collectMediaAnchors(document)): string | null => {
  const activeVideoMediaUrl = resolveMediaUrlFromActiveVideo();

  if (activeVideoMediaUrl) {
    return activeVideoMediaUrl;
  }

  const viewportCenterMediaUrl = resolveMediaUrlFromViewportCenter();

  if (viewportCenterMediaUrl) {
    return viewportCenterMediaUrl;
  }

  const viewportMediaUrl = pickViewportMediaUrl(anchors);

  if (viewportMediaUrl) {
    return viewportMediaUrl;
  }

  return resolveDocumentMediaFallback();
};

const resolveMediaUrlForActionColumn = (actionColumn: HTMLElement, globalAnchors: MediaAnchor[]): string | null => {
  let scope: HTMLElement | null = actionColumn;

  for (let depth = 0; depth < 5 && scope; depth += 1) {
    const scopedMediaUrl = pickNearestMediaUrl(actionColumn, collectMediaAnchors(scope));

    if (scopedMediaUrl) {
      return scopedMediaUrl;
    }

    scope = scope.parentElement;
  }

  const nearestGlobalUrl = pickNearestMediaUrl(actionColumn, globalAnchors);

  if (nearestGlobalUrl) {
    return nearestGlobalUrl;
  }

  return resolveCurrentMediaUrl(globalAnchors);
};

const isActionColumnCandidate = (candidate: HTMLElement): boolean => {
  if (candidate.getClientRects().length === 0) {
    return false;
  }

  const rect = candidate.getBoundingClientRect();

  if (rect.width < 20 || rect.width > 260 || rect.height < 72) {
    return false;
  }

  if (rect.bottom < -120 || rect.top > window.innerHeight + 120) {
    return false;
  }

  const hasLike = !!candidate.querySelector(LIKE_ICON_SELECTOR);
  const hasComment = !!candidate.querySelector(COMMENT_ICON_SELECTOR);
  const hasShare = !!candidate.querySelector(SHARE_ICON_SELECTOR);

  if (!hasLike || (!hasComment && !hasShare)) {
    return false;
  }

  const actionButtonsCount = candidate.querySelectorAll(ACTION_INTERACTIVE_SELECTOR).length;
  return actionButtonsCount >= 2;
};

const findActionColumnFromNode = (node: HTMLElement): HTMLElement | null => {
  let current: HTMLElement | null = node;

  for (let depth = 0; depth < 10 && current; depth += 1) {
    const nextParent: HTMLElement | null = current.parentElement;

    if (!nextParent) {
      break;
    }

    if (isActionColumnCandidate(nextParent)) {
      return nextParent;
    }

    current = nextParent;
  }

  return null;
};

const collectActionColumns = (): HTMLElement[] => {
  const actionIconNodes = Array.from(document.querySelectorAll<HTMLElement>(ACTION_ICON_SELECTOR));
  const uniqueColumns = new Set<HTMLElement>();

  for (const iconNode of actionIconNodes) {
    const actionColumn = findActionColumnFromNode(iconNode);

    if (!actionColumn) {
      continue;
    }

    uniqueColumns.add(actionColumn);
  }

  return [...uniqueColumns];
};

const createActionButton = (floating = false): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'lce-button-tiktok';
  if (floating) {
    button.classList.add('lce-button-tiktok-floating');
  }
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
        const actionItem = button.closest<HTMLElement>(`[${ACTION_ITEM_ATTRIBUTE}]`);
        const actionColumn = actionItem?.parentElement;
        const runtimeUrl =
          actionColumn instanceof HTMLElement
            ? resolveMediaUrlForActionColumn(actionColumn, collectMediaAnchors(document))
            : resolveCurrentMediaUrl();
        const targetUrl = runtimeUrl || button.dataset.targetUrl || (await resolveCapturedTikTokUrl());

        if (!targetUrl) {
          setButtonState(button, 'error', 'Impossible de détecter une URL TikTok valide.');
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

const findInsertionPoint = (actionColumn: HTMLElement): Element | null => {
  const children = Array.from(actionColumn.children);

  for (const child of children) {
    if (!(child instanceof HTMLElement)) {
      continue;
    }

    if (child.hasAttribute(ACTION_ITEM_ATTRIBUTE)) {
      continue;
    }

    if (child.querySelector(ACTION_INTERACTIVE_SELECTOR)) {
      return child;
    }
  }

  return actionColumn.firstElementChild;
};

const upsertActionButton = (actionColumn: HTMLElement, targetUrl: string): void => {
  let actionItem = actionColumn.querySelector<HTMLElement>(`[${ACTION_ITEM_ATTRIBUTE}]`);

  if (!actionItem) {
    actionItem = document.createElement('div');
    actionItem.setAttribute(ACTION_ITEM_ATTRIBUTE, '1');
    actionItem.className = 'lce-tiktok-action-item';

    const button = createActionButton();
    button.setAttribute(BUTTON_ATTRIBUTE, '1');

    const label = document.createElement('div');
    label.className = 'lce-tiktok-action-label';
    label.textContent = 'LiveChat';

    actionItem.appendChild(button);
    actionItem.appendChild(label);

    const insertionPoint = findInsertionPoint(actionColumn);

    if (insertionPoint) {
      actionColumn.insertBefore(actionItem, insertionPoint);
    } else {
      actionColumn.appendChild(actionItem);
    }
  }

  const button = actionItem.querySelector<HTMLButtonElement>(`button[${BUTTON_ATTRIBUTE}]`);

  if (button) {
    button.dataset.targetUrl = targetUrl;
    button.title = DEFAULT_BUTTON_TITLE;
  }
};

const removeFloatingButton = (): void => {
  const floatingButton = document.getElementById(FLOATING_BUTTON_ID);

  if (floatingButton) {
    floatingButton.remove();
  }
};

const upsertFloatingButton = (targetUrl: string | null): void => {
  let floatingButton = document.getElementById(FLOATING_BUTTON_ID) as HTMLButtonElement | null;

  if (!floatingButton) {
    floatingButton = createActionButton(true);
    floatingButton.id = FLOATING_BUTTON_ID;
    floatingButton.setAttribute(BUTTON_ATTRIBUTE, '1');
    document.body.appendChild(floatingButton);
  }

  floatingButton.dataset.targetUrl = targetUrl || '';
  floatingButton.title = DEFAULT_BUTTON_TITLE;
};

const isTikTokHomeFeedPath = (): boolean => {
  const path = window.location.pathname.trim();
  return path === '' || path === '/';
};

const scanTikTokTargets = (): void => {
  const actionColumns = collectActionColumns();
  const globalAnchors = collectMediaAnchors(document);
  const fallbackUrl = resolveCurrentMediaUrl(globalAnchors);
  syncActiveMediaToBackground(fallbackUrl);
  let hasInlineButton = false;

  for (const actionColumn of actionColumns) {
    const targetUrl = resolveMediaUrlForActionColumn(actionColumn, globalAnchors);

    if (!targetUrl) {
      continue;
    }

    upsertActionButton(actionColumn, targetUrl);
    hasInlineButton = true;
  }

  if (isTikTokHomeFeedPath()) {
    upsertFloatingButton(fallbackUrl);
    return;
  }

  if (!hasInlineButton) {
    upsertFloatingButton(fallbackUrl);
    return;
  }

  removeFloatingButton();
};

const isGetActiveMediaUrlMessage = (value: unknown): value is { type: string } => {
  if (!value || typeof value !== 'object') {
    return false;
  }

  const payload = value as { type?: unknown };
  return payload.type === GET_ACTIVE_MEDIA_URL_TYPE;
};

const registerActiveMediaUrlListener = (): void => {
  if (!isRuntimeAvailable()) {
    return;
  }

  try {
    chrome.runtime.onMessage.addListener((message: unknown, _sender, sendResponse) => {
      if (!isGetActiveMediaUrlMessage(message)) {
        return;
      }

      sendResponse({
        ok: true,
        url: resolveCurrentMediaUrl(),
      });
    });
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
startObservedScanner(scanTikTokTargets);
