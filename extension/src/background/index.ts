import { sendToIngest } from '../lib/ingestClient';
import { restrictLocalStorageToTrustedContexts } from '../lib/storage-access';
import {
  MESSAGE_TYPES,
  isGetComposeStateRequest,
  isGetAuthStateRequest,
  isSendComposeRequest,
  isSendQuickRequest,
  isTikTokGetCapturedUrlRequest,
  isTikTokSyncActiveItemRequest,
  isGetThemePreferenceRequest,
  type ActiveMediaUrlResponse,
  type ActionResponse,
  type ComposeStateResponse,
  type SendSource,
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
import {
  THEME_STORAGE_KEY,
  normalizeThemePreference,
  readThemePreference,
} from '../lib/theme';
import {
  normalizeTikTokItemId,
  normalizeTikTokPageUrl,
  normalizeTikTokPlayUrl,
  normalizeTikTokMediaUrl,
  extractTikTokItemIdFromUrl,
  createEmptyTikTokCaptureState,
  sanitizeTikTokCaptureState,
  trimTikTokCaptureItems,
  mergeTikTokCaptureRecord,
  type TikTokCaptureRecord,
  type TikTokCaptureState,
} from '../lib/tiktok-capture';
import {
  isIngestTargetAllowedForSource,
  resolveIngestTargetUrl,
  resolveUrlFromContextCandidates,
} from '../lib/url';

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

const tiktokCaptureStateCache = new Map<number, TikTokCaptureState>();

// Au chargement du service worker : le jeton stocké dans storage.local n'est
// plus lisible depuis les content scripts (voir lib/storage-access.ts).
void restrictLocalStorageToTrustedContexts();

const createContextMenus = async (): Promise<void> => {
  // Firefox ne fournit que la variante promesse de `removeAll` : sans l'attente,
  // les deux `create` peuvent être écrasés par le nettoyage asynchrone.
  await browser.contextMenus.removeAll();

  browser.contextMenus.create({
    id: MENU_ID_QUICK,
    title: 'Envoyer rapidement vers LiveChat',
    contexts: ['page', 'link', 'video'],
    documentUrlPatterns: SUPPORTED_DOCUMENT_PATTERNS,
  });

  browser.contextMenus.create({
    id: MENU_ID_COMPOSE,
    title: 'Envoyer vers LiveChat avec texte',
    contexts: ['page', 'link', 'video'],
    documentUrlPatterns: SUPPORTED_DOCUMENT_PATTERNS,
  });
};

const getTikTokCaptureStorageKey = (tabId: number): string => `${TIKTOK_CAPTURE_STORAGE_PREFIX}${tabId}`;

const getTikTokCaptureState = async (tabId: number): Promise<TikTokCaptureState> => {
  const fromCache = tiktokCaptureStateCache.get(tabId);

  if (fromCache) {
    return fromCache;
  }

  const storageKey = getTikTokCaptureStorageKey(tabId);

  try {
    const stored = await browser.storage.session.get(storageKey);
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
    await browser.storage.session.set({
      [storageKey]: nextState,
    });
  } catch {
    // Ignore storage write errors.
  }
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

const captureTikTokWebRequest = (details: browser.webRequest._OnCompletedDetails): void => {
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

const captureTikTokRedirect = (details: browser.webRequest._OnBeforeRedirectDetails): void => {
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
    await browser.tabs.sendMessage(tabId, toastMessage);
  } catch {
    // Ignore if no content script is available on the tab.
  }
};

const sendQuickAction = async (url: string, source?: SendSource): Promise<ActionResponse> => {
  if (!isIngestTargetAllowedForSource(source, url)) {
    return {
      ok: false,
      jobId: null,
      message: 'URL invalide ou non supportée.',
      errorCode: 'INVALID_PAYLOAD',
    };
  }

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
  source?: SendSource;
  text?: string;
  forceRefresh?: boolean;
  saveToBoard?: boolean;
  durationSec?: number;
}): Promise<ActionResponse> => {
  if (!isIngestTargetAllowedForSource(params.source, params.url)) {
    return {
      ok: false,
      jobId: null,
      message: 'URL invalide ou non supportée.',
      errorCode: 'INVALID_PAYLOAD',
    };
  }

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
    durationSec: params.durationSec,
  });

  if (result.ok) {
    await clearComposeDraft();
  }

  return responseFromIngestResult(result);
};

const getActiveTab = async (): Promise<browser.tabs.Tab | null> => {
  const [activeTab] = await browser.tabs.query({
    active: true,
    lastFocusedWindow: true,
  });

  return activeTab || null;
};

const resolveActiveMediaUrlFromTab = async (activeTab: browser.tabs.Tab | null): Promise<string> => {
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
    const response = (await browser.tabs.sendMessage(activeTab.id, {
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
    await browser.action.openPopup();
  } catch {
    await sendToastToTab(tabId, 'info', 'Clique sur l’icône de l’extension pour ouvrir le formulaire.');
  }
};

browser.runtime.onInstalled.addListener(() => {
  void createContextMenus();
});

browser.runtime.onStartup.addListener(() => {
  void createContextMenus();
});

// Le thème vit dans storage.local (TRUSTED_CONTEXTS) : le service worker est
// le seul à le lire et pousse les changements aux content scripts, qui ne
// peuvent plus s'abonner directement à onChanged sur cette area.
browser.storage.onChanged.addListener((changes, areaName) => {
  if (areaName !== 'local' || !(THEME_STORAGE_KEY in changes)) {
    return;
  }

  const preference = normalizeThemePreference(changes[THEME_STORAGE_KEY]?.newValue);
  void broadcastThemePreference(preference);
});

const broadcastThemePreference = async (preference: string): Promise<void> => {
  if (typeof browser.tabs?.query !== 'function') {
    return;
  }

  try {
    const tabs = await browser.tabs.query({});
    for (const tab of tabs) {
      if (typeof tab.id !== 'number') {
        continue;
      }

      // Pas de filtre d'URL : le manifeste est la seule source de vérité des
      // injections (youtube.com nu, *.youtube.com, youtu.be, *.x.com…). Un
      // onglet sans content script rejette simplement le message, on l'ignore.
      void Promise.resolve(
        browser.tabs.sendMessage(tab.id, {
          type: MESSAGE_TYPES.THEME_PREFERENCE_CHANGED,
          preference,
        }),
      ).catch(() => undefined);
    }
  } catch {
    // Onglets inaccessibles : le prochain démarrage de content script relira
    // la préférence via GET_THEME_PREFERENCE.
  }
};

browser.webRequest.onCompleted.addListener(
  (details) => {
    captureTikTokWebRequest(details);
  },
  { urls: [...TIKTOK_WEB_REQUEST_PATTERNS] },
);

browser.webRequest.onBeforeRedirect.addListener(
  (details) => {
    captureTikTokRedirect(details);
  },
  { urls: [...TIKTOK_WEB_REQUEST_PATTERNS] },
);

browser.tabs.onRemoved.addListener((tabId) => {
  tiktokCaptureStateCache.delete(tabId);
  const storageKey = getTikTokCaptureStorageKey(tabId);
  void browser.storage.session.remove(storageKey);
});

browser.contextMenus.onClicked.addListener((info, tab) => {
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

browser.runtime.onMessage.addListener((message: unknown, sender, sendResponse) => {
  void (async () => {
    if (isSendQuickRequest(message)) {
      if (message.source === 'tiktok') {
        const normalizedTikTokPageUrl = normalizeTikTokPageUrl(message.url);

        if (normalizedTikTokPageUrl) {
          sendResponse(await sendQuickAction(normalizedTikTokPageUrl, message.source));
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

      sendResponse(await sendQuickAction(message.url, message.source));
      return;
    }

    if (isSendComposeRequest(message)) {
      sendResponse(
        await sendComposeAction({
          url: message.url,
          source: message.source,
          text: message.text,
          forceRefresh: message.forceRefresh,
          saveToBoard: message.saveToBoard,
          durationSec: message.durationSec,
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

    if (isGetThemePreferenceRequest(message)) {
      sendResponse({ preference: await readThemePreference() });
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
