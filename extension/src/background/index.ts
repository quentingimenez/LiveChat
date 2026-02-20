import { sendToIngest } from '../lib/ingestClient';
import {
  MESSAGE_TYPES,
  isGetComposeStateRequest,
  isGetAuthStateRequest,
  isSendComposeRequest,
  isSendQuickRequest,
  isTikTokGetCapturedUrlRequest,
  isTikTokSyncActiveItemRequest,
  type ActiveMediaUrlResponse,
  type ActionResponse,
  type ComposeStateResponse,
  type ShowToastMessage,
  type ToastLevel,
} from '../lib/messages';
import {
  clearComposeDraft,
  getComposeDraft,
  getSettings,
  isSettingsComplete,
  setComposeDraft,
  type ComposeDraft,
} from '../lib/settings';
import { resolveIngestTargetUrl, resolveUrlFromContextCandidates } from '../lib/url';

const MENU_ID_QUICK = 'lce-context-send-quick';
const MENU_ID_COMPOSE = 'lce-context-send-compose';

const SUPPORTED_DOCUMENT_PATTERNS = [
  'https://www.youtube.com/*',
  'https://m.youtube.com/*',
  'https://www.tiktok.com/*',
  'https://x.com/*',
  'https://twitter.com/*'
];

const TIKTOK_WEB_REQUEST_PATTERNS = ['*://www.tiktok.com/aweme/v100/play/*', '*://*.tiktok.com/video/tos/*'] as const;
const TIKTOK_CAPTURE_STORAGE_PREFIX = 'lce:tiktok:capture:';
const TIKTOK_CAPTURE_MAX_ITEMS = 16;

interface TikTokCaptureRecord {
  itemId: string | null;
  pageUrl: string | null;
  mediaUrl: string | null;
  playUrl: string | null;
  ts: number;
}

interface TikTokCaptureState {
  activeItemId: string | null;
  latest: TikTokCaptureRecord | null;
  byItemId: Record<string, TikTokCaptureRecord>;
  updatedAt: number;
}

const tiktokCaptureStateCache = new Map<number, TikTokCaptureState>();

const createContextMenus = (): void => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: MENU_ID_QUICK,
      title: 'Envoyer rapidement vers LiveChat',
      contexts: ['page', 'link', 'video'],
      documentUrlPatterns: SUPPORTED_DOCUMENT_PATTERNS,
    });

    chrome.contextMenus.create({
      id: MENU_ID_COMPOSE,
      title: 'Envoyer vers LiveChat avec texte',
      contexts: ['page', 'link', 'video'],
      documentUrlPatterns: SUPPORTED_DOCUMENT_PATTERNS,
    });
  });
};

const trimToNonEmpty = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

const isTikTokHostname = (hostname: string): boolean => {
  const normalizedHost = hostname.toLowerCase();
  return normalizedHost === 'tiktok.com' || normalizedHost.endsWith('.tiktok.com');
};

const normalizeTikTokItemId = (value: unknown): string | null => {
  const normalized = trimToNonEmpty(value);

  if (!normalized) {
    return null;
  }

  return /^\d{15,22}$/.test(normalized) ? normalized : null;
};

const normalizeTikTokPageUrl = (value: unknown): string | null => {
  const candidate = trimToNonEmpty(value);

  if (!candidate) {
    return null;
  }

  const normalized = resolveIngestTargetUrl(candidate);

  if (!normalized) {
    return null;
  }

  try {
    const parsed = new URL(normalized);

    if (!isTikTokHostname(parsed.hostname)) {
      return null;
    }

    return /\/(?:video|photo)\/\d{15,22}/i.test(parsed.pathname) ? normalized : null;
  } catch {
    return null;
  }
};

const normalizeTikTokPlayUrl = (value: unknown): string | null => {
  const candidate = trimToNonEmpty(value);

  if (!candidate) {
    return null;
  }

  try {
    const parsed = new URL(candidate);

    if (parsed.hostname.toLowerCase() !== 'www.tiktok.com') {
      return null;
    }

    return /^\/aweme\/v100\/play\//i.test(parsed.pathname) ? parsed.toString() : null;
  } catch {
    return null;
  }
};

const normalizeTikTokMediaUrl = (value: unknown): string | null => {
  const candidate = trimToNonEmpty(value);

  if (!candidate) {
    return null;
  }

  try {
    const parsed = new URL(candidate);

    if (!parsed.hostname.toLowerCase().includes('tiktok.com')) {
      return null;
    }

    return /\/video\/tos\//i.test(parsed.pathname) ? parsed.toString() : null;
  } catch {
    return null;
  }
};

const extractTikTokItemIdFromUrl = (value: unknown): string | null => {
  const candidate = trimToNonEmpty(value);

  if (!candidate) {
    return null;
  }

  try {
    const parsed = new URL(candidate);
    const pathMatch = parsed.pathname.match(/\/(?:video|photo)\/(\d{15,22})/i);
    const pathItemId = normalizeTikTokItemId(pathMatch?.[1]);

    if (pathItemId) {
      return pathItemId;
    }

    const fromQuery = normalizeTikTokItemId(parsed.searchParams.get('item_id'));

    if (fromQuery) {
      return fromQuery;
    }
  } catch {
    // Ignore malformed URLs.
  }

  const fallbackMatch = candidate.match(/\b(\d{15,22})\b/);
  return normalizeTikTokItemId(fallbackMatch?.[1]);
};

const getTikTokCaptureStorageKey = (tabId: number): string => `${TIKTOK_CAPTURE_STORAGE_PREFIX}${tabId}`;

const createEmptyTikTokCaptureState = (): TikTokCaptureState => ({
  activeItemId: null,
  latest: null,
  byItemId: {},
  updatedAt: Date.now(),
});

const sanitizeTikTokCaptureRecord = (value: unknown): TikTokCaptureRecord | null => {
  if (!value || typeof value !== 'object') {
    return null;
  }

  const record = value as Record<string, unknown>;
  const itemId = normalizeTikTokItemId(record.itemId);
  const pageUrl = normalizeTikTokPageUrl(record.pageUrl);
  const mediaUrl = normalizeTikTokMediaUrl(record.mediaUrl);
  const playUrl = normalizeTikTokPlayUrl(record.playUrl);
  const ts = typeof record.ts === 'number' && Number.isFinite(record.ts) ? record.ts : Date.now();

  if (!itemId && !pageUrl && !mediaUrl && !playUrl) {
    return null;
  }

  return {
    itemId,
    pageUrl,
    mediaUrl,
    playUrl,
    ts,
  };
};

const sanitizeTikTokCaptureState = (value: unknown): TikTokCaptureState | null => {
  if (!value || typeof value !== 'object') {
    return null;
  }

  const state = value as Record<string, unknown>;
  const byItemIdRaw = state.byItemId && typeof state.byItemId === 'object' ? (state.byItemId as Record<string, unknown>) : {};
  const byItemId: Record<string, TikTokCaptureRecord> = {};

  for (const [key, entryRaw] of Object.entries(byItemIdRaw)) {
    const sanitizedEntry = sanitizeTikTokCaptureRecord(entryRaw);

    if (!sanitizedEntry || !sanitizedEntry.itemId) {
      continue;
    }

    byItemId[key] = sanitizedEntry;
  }

  const latest = sanitizeTikTokCaptureRecord(state.latest);
  const activeItemId = normalizeTikTokItemId(state.activeItemId);
  const updatedAt = typeof state.updatedAt === 'number' && Number.isFinite(state.updatedAt) ? state.updatedAt : Date.now();

  return {
    activeItemId,
    latest,
    byItemId,
    updatedAt,
  };
};

const trimTikTokCaptureItems = (byItemId: Record<string, TikTokCaptureRecord>): Record<string, TikTokCaptureRecord> => {
  const ordered = Object.entries(byItemId)
    .sort(([, left], [, right]) => right.ts - left.ts)
    .slice(0, TIKTOK_CAPTURE_MAX_ITEMS);

  return Object.fromEntries(ordered);
};

const getTikTokCaptureState = async (tabId: number): Promise<TikTokCaptureState> => {
  const fromCache = tiktokCaptureStateCache.get(tabId);

  if (fromCache) {
    return fromCache;
  }

  const storageKey = getTikTokCaptureStorageKey(tabId);

  try {
    const stored = await chrome.storage.session.get(storageKey);
    const parsed = sanitizeTikTokCaptureState(stored[storageKey]);

    if (parsed) {
      tiktokCaptureStateCache.set(tabId, parsed);
      return parsed;
    }
  } catch {
    // Ignore storage read errors.
  }

  const emptyState = createEmptyTikTokCaptureState();
  tiktokCaptureStateCache.set(tabId, emptyState);
  return emptyState;
};

const setTikTokCaptureState = async (tabId: number, nextState: TikTokCaptureState): Promise<void> => {
  const storageKey = getTikTokCaptureStorageKey(tabId);
  tiktokCaptureStateCache.set(tabId, nextState);

  try {
    await chrome.storage.session.set({
      [storageKey]: nextState,
    });
  } catch {
    // Ignore storage write errors.
  }
};

const mergeTikTokCaptureRecord = (
  base: TikTokCaptureRecord | null,
  patch: {
    itemId?: string | null;
    pageUrl?: string | null;
    mediaUrl?: string | null;
    playUrl?: string | null;
  },
): TikTokCaptureRecord | null => {
  const merged: TikTokCaptureRecord = {
    itemId: patch.itemId !== undefined ? patch.itemId : base?.itemId || null,
    pageUrl: patch.pageUrl !== undefined ? patch.pageUrl : base?.pageUrl || null,
    mediaUrl: patch.mediaUrl !== undefined ? patch.mediaUrl : base?.mediaUrl || null,
    playUrl: patch.playUrl !== undefined ? patch.playUrl : base?.playUrl || null,
    ts: Date.now(),
  };

  if (!merged.itemId && !merged.pageUrl && !merged.mediaUrl && !merged.playUrl) {
    return null;
  }

  return merged;
};

const upsertTikTokCapture = async (
  tabId: number,
  patch: {
    itemId?: string | null;
    pageUrl?: string | null;
    mediaUrl?: string | null;
    playUrl?: string | null;
    activeItemId?: string | null;
  },
): Promise<void> => {
  const currentState = await getTikTokCaptureState(tabId);
  const nextByItemId: Record<string, TikTokCaptureRecord> = { ...currentState.byItemId };
  const nextState: TikTokCaptureState = {
    activeItemId: patch.activeItemId !== undefined ? patch.activeItemId : currentState.activeItemId,
    latest: currentState.latest,
    byItemId: nextByItemId,
    updatedAt: Date.now(),
  };

  const normalizedItemId =
    normalizeTikTokItemId(patch.itemId) ||
    extractTikTokItemIdFromUrl(patch.pageUrl) ||
    extractTikTokItemIdFromUrl(patch.playUrl) ||
    extractTikTokItemIdFromUrl(patch.mediaUrl) ||
    null;

  const normalizedPageUrl = patch.pageUrl !== undefined ? normalizeTikTokPageUrl(patch.pageUrl) : undefined;
  const normalizedMediaUrl = patch.mediaUrl !== undefined ? normalizeTikTokMediaUrl(patch.mediaUrl) : undefined;
  const normalizedPlayUrl = patch.playUrl !== undefined ? normalizeTikTokPlayUrl(patch.playUrl) : undefined;

  const hasRecordPatch =
    patch.itemId !== undefined ||
    patch.pageUrl !== undefined ||
    patch.mediaUrl !== undefined ||
    patch.playUrl !== undefined;

  if (hasRecordPatch) {
    const baseRecord = normalizedItemId ? nextByItemId[normalizedItemId] || currentState.latest : currentState.latest;
    const mergedRecord = mergeTikTokCaptureRecord(baseRecord || null, {
      itemId: normalizedItemId,
      pageUrl: normalizedPageUrl,
      mediaUrl: normalizedMediaUrl,
      playUrl: normalizedPlayUrl,
    });

    if (mergedRecord) {
      nextState.latest = mergedRecord;

      if (mergedRecord.itemId) {
        nextByItemId[mergedRecord.itemId] = mergedRecord;
      }
    }
  }

  nextState.byItemId = trimTikTokCaptureItems(nextByItemId);
  await setTikTokCaptureState(tabId, nextState);
};

const resolveTikTokCapturedUrlForTab = async (
  tabId: number,
  domUrlCandidate?: string | null,
): Promise<string | null> => {
  const state = await getTikTokCaptureState(tabId);
  const normalizedDomUrl = normalizeTikTokPageUrl(domUrlCandidate);
  const domItemId = extractTikTokItemIdFromUrl(normalizedDomUrl || domUrlCandidate);

  if (domItemId) {
    const domRecord = state.byItemId[domItemId];

    if (domRecord) {
      const normalizedDomRecordPageUrl = normalizeTikTokPageUrl(domRecord.pageUrl);

      if (normalizedDomRecordPageUrl) {
        return normalizedDomRecordPageUrl;
      }
    }
  }

  if (state.activeItemId) {
    const activeRecord = state.byItemId[state.activeItemId];

    if (activeRecord) {
      const normalizedActiveRecordPageUrl = normalizeTikTokPageUrl(activeRecord.pageUrl);

      if (normalizedActiveRecordPageUrl) {
        return normalizedActiveRecordPageUrl;
      }
    }
  }

  if (normalizedDomUrl) {
    return normalizedDomUrl;
  }

  if (state.latest) {
    const normalizedLatestPageUrl = normalizeTikTokPageUrl(state.latest.pageUrl);

    if (normalizedLatestPageUrl) {
      return normalizedLatestPageUrl;
    }
  }

  return null;
};

const captureTikTokWebRequest = (details: chrome.webRequest.WebResponseCacheDetails): void => {
  if (details.tabId < 0) {
    return;
  }

  const isPlayRequest = !!normalizeTikTokPlayUrl(details.url);
  const isMediaRequest = !!normalizeTikTokMediaUrl(details.url);

  if (!isPlayRequest && !isMediaRequest) {
    return;
  }

  const itemId = extractTikTokItemIdFromUrl(details.url);

  void upsertTikTokCapture(details.tabId, {
    itemId,
    playUrl: isPlayRequest ? details.url : undefined,
    mediaUrl: isMediaRequest ? details.url : undefined,
  });
};

const captureTikTokRedirect = (details: chrome.webRequest.WebRedirectionResponseDetails): void => {
  if (details.tabId < 0) {
    return;
  }

  const playUrl = normalizeTikTokPlayUrl(details.url);
  const redirectMediaUrl = normalizeTikTokMediaUrl(details.redirectUrl);

  if (!playUrl && !redirectMediaUrl) {
    return;
  }

  const itemId = extractTikTokItemIdFromUrl(details.url) || extractTikTokItemIdFromUrl(details.redirectUrl);

  void upsertTikTokCapture(details.tabId, {
    itemId,
    playUrl: playUrl || undefined,
    mediaUrl: redirectMediaUrl || undefined,
  });
};

const responseFromIngestResult = (result: Awaited<ReturnType<typeof sendToIngest>>): ActionResponse => {
  if (result.ok) {
    return {
      ok: true,
      jobId: result.jobId,
      message: result.message,
    };
  }

  return {
    ok: false,
    jobId: null,
    message: result.error.message,
    errorCode: result.error.code,
  };
};

const sendToastToTab = async (tabId: number | undefined, level: ToastLevel, message: string): Promise<void> => {
  if (typeof tabId !== 'number') {
    return;
  }

  const toastMessage: ShowToastMessage = {
    type: MESSAGE_TYPES.SHOW_TOAST,
    level,
    message,
  };

  try {
    await chrome.tabs.sendMessage(tabId, toastMessage);
  } catch {
    // Ignore if no content script is available on the tab.
  }
};

const sendQuickAction = async (url: string): Promise<ActionResponse> => {
  const normalizedUrl = resolveIngestTargetUrl(url);

  if (!normalizedUrl) {
    return {
      ok: false,
      jobId: null,
      message: 'URL invalide ou non supportée.',
      errorCode: 'INVALID_PAYLOAD',
    };
  }

  const result = await sendToIngest({
    mode: 'quick',
    url: normalizedUrl,
  });

  return responseFromIngestResult(result);
};

const sendComposeAction = async (params: {
  url: string;
  text?: string;
  forceRefresh?: boolean;
  saveToBoard?: boolean;
}): Promise<ActionResponse> => {
  const normalizedUrl = resolveIngestTargetUrl(params.url);

  if (!normalizedUrl) {
    return {
      ok: false,
      jobId: null,
      message: 'URL invalide ou non supportée.',
      errorCode: 'INVALID_PAYLOAD',
    };
  }

  const result = await sendToIngest({
    mode: 'compose',
    url: normalizedUrl,
    text: params.text,
    forceRefresh: !!params.forceRefresh,
    saveToBoard: !!params.saveToBoard,
  });

  if (result.ok) {
    await clearComposeDraft();
  }

  return responseFromIngestResult(result);
};

const getActiveTab = async (): Promise<chrome.tabs.Tab | null> => {
  const [activeTab] = await chrome.tabs.query({
    active: true,
    lastFocusedWindow: true,
  });

  return activeTab || null;
};

const resolveActiveMediaUrlFromTab = async (activeTab: chrome.tabs.Tab | null): Promise<string> => {
  if (!activeTab) {
    return '';
  }

  const tabUrl = `${activeTab.url || ''}`;
  const resolvedTabUrl = resolveIngestTargetUrl(tabUrl) || '';
  const fallbackUrl = resolvedTabUrl;

  if (typeof activeTab.id !== 'number') {
    return fallbackUrl;
  }

  let contentCandidate = '';

  try {
    const response = (await chrome.tabs.sendMessage(activeTab.id, {
      type: MESSAGE_TYPES.GET_ACTIVE_MEDIA_URL,
    })) as ActiveMediaUrlResponse;

    if (response?.ok && typeof response.url === 'string' && response.url.trim()) {
      contentCandidate = resolveIngestTargetUrl(response.url) || '';
    }
  } catch {
    // Ignore, content script may not be injected on the active tab.
  }

  return contentCandidate || fallbackUrl;
};

const getActiveTabUrl = async (): Promise<string> => {
  return resolveActiveMediaUrlFromTab(await getActiveTab());
};

const getComposeState = async (): Promise<ComposeStateResponse> => {
  const [draft, settings, activeTabUrl] = await Promise.all([getComposeDraft(), getSettings(), getActiveTabUrl()]);
  const hasSettings = isSettingsComplete(settings);

  return {
    ok: true,
    url: draft?.url || activeTabUrl,
    text: draft?.text || '',
    forceRefresh: draft?.forceRefresh || false,
    saveToBoard: draft?.saveToBoard || false,
    hasSettings,
    settingsError: hasSettings ? null : 'Configuration incomplète. Ouvre les options de l’extension.',
    draftSource: draft?.source || null,
  };
};

const openPopupWithFallback = async (tabId: number | undefined): Promise<void> => {
  try {
    await chrome.action.openPopup();
  } catch {
    await sendToastToTab(tabId, 'info', 'Clique sur l’icône de l’extension pour ouvrir le formulaire.');
  }
};

chrome.runtime.onInstalled.addListener(() => {
  createContextMenus();
});

chrome.runtime.onStartup.addListener(() => {
  createContextMenus();
});

chrome.webRequest.onCompleted.addListener(
  (details) => {
    captureTikTokWebRequest(details);
  },
  { urls: [...TIKTOK_WEB_REQUEST_PATTERNS] },
);

chrome.webRequest.onBeforeRedirect.addListener(
  (details) => {
    captureTikTokRedirect(details);
  },
  { urls: [...TIKTOK_WEB_REQUEST_PATTERNS] },
);

chrome.tabs.onRemoved.addListener((tabId) => {
  tiktokCaptureStateCache.delete(tabId);
  const storageKey = getTikTokCaptureStorageKey(tabId);
  void chrome.storage.session.remove(storageKey);
});

chrome.contextMenus.onClicked.addListener((info, tab) => {
  void (async () => {
    const targetUrl = resolveUrlFromContextCandidates({
      linkUrl: info.linkUrl,
      srcUrl: info.srcUrl,
      pageUrl: info.pageUrl,
      tabUrl: tab?.url,
    });

    if (!targetUrl) {
      await sendToastToTab(tab?.id, 'error', 'Impossible de déterminer l’URL à envoyer.');
      return;
    }

    if (info.menuItemId === MENU_ID_QUICK) {
      const response = await sendQuickAction(targetUrl);
      await sendToastToTab(tab?.id, response.ok ? 'success' : 'error', response.message);
      return;
    }

    if (info.menuItemId === MENU_ID_COMPOSE) {
      const draft: ComposeDraft = {
        url: targetUrl,
        text: '',
        forceRefresh: false,
        saveToBoard: false,
        source: 'context-menu',
        createdAt: Date.now(),
      };

      await setComposeDraft(draft);
      await openPopupWithFallback(tab?.id);
    }
  })();
});

chrome.runtime.onMessage.addListener((message: unknown, sender, sendResponse) => {
  void (async () => {
    if (isSendQuickRequest(message)) {
      if (message.source === 'tiktok') {
        const normalizedTikTokPageUrl = normalizeTikTokPageUrl(message.url);

        if (normalizedTikTokPageUrl) {
          sendResponse(await sendQuickAction(normalizedTikTokPageUrl));
          return;
        }

        sendResponse({
          ok: false,
          jobId: null,
          message: 'URL TikTok invalide. Ouvre directement une URL /video ou /photo puis réessaie.',
          errorCode: 'INVALID_PAYLOAD',
        } satisfies ActionResponse);
        return;
      }

      sendResponse(await sendQuickAction(message.url));
      return;
    }

    if (isSendComposeRequest(message)) {
      sendResponse(
        await sendComposeAction({
          url: message.url,
          text: message.text,
          forceRefresh: message.forceRefresh,
          saveToBoard: message.saveToBoard,
        }),
      );
      return;
    }

    if (isGetComposeStateRequest(message)) {
      sendResponse(await getComposeState());
      return;
    }

    if (isGetAuthStateRequest(message)) {
      const settings = await getSettings();
      sendResponse({
        ok: true,
        hasSettings: isSettingsComplete(settings),
      });
      return;
    }

    if (isTikTokSyncActiveItemRequest(message)) {
      const senderTabId = typeof sender?.tab?.id === 'number' ? sender.tab.id : null;

      if (senderTabId !== null) {
        const normalizedItemId = normalizeTikTokItemId(message.itemId);
        const normalizedPageUrl = normalizeTikTokPageUrl(message.url);

        await upsertTikTokCapture(senderTabId, {
          activeItemId: normalizedItemId,
          itemId: normalizedItemId || undefined,
          pageUrl: normalizedPageUrl || undefined,
        });
      }

      sendResponse({ ok: true });
      return;
    }

    if (isTikTokGetCapturedUrlRequest(message)) {
      const senderTabId = typeof sender?.tab?.id === 'number' ? sender.tab.id : null;

      if (senderTabId === null) {
        sendResponse({
          ok: false,
          url: null,
        });
        return;
      }

      const capturedUrl = await resolveTikTokCapturedUrlForTab(senderTabId, message.domUrl ?? null);

      sendResponse({
        ok: !!capturedUrl,
        url: capturedUrl,
      });
      return;
    }

    sendResponse({
      ok: false,
      jobId: null,
      message: 'Message runtime non supporté.',
      errorCode: 'UNKNOWN',
    } satisfies ActionResponse);
  })();

  return true;
});
