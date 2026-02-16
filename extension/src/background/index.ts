import { sendToIngest } from '../lib/ingestClient';
import {
  MESSAGE_TYPES,
  isGetComposeStateRequest,
  isSendComposeRequest,
  isSendQuickRequest,
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

  const fallbackUrl = resolveIngestTargetUrl(`${activeTab.url || ''}`) || '';

  if (typeof activeTab.id !== 'number') {
    return fallbackUrl;
  }

  try {
    const response = (await chrome.tabs.sendMessage(activeTab.id, {
      type: MESSAGE_TYPES.GET_ACTIVE_MEDIA_URL,
    })) as ActiveMediaUrlResponse;

    if (!response?.ok || typeof response.url !== 'string' || !response.url.trim()) {
      return fallbackUrl;
    }

    return resolveIngestTargetUrl(response.url) || fallbackUrl;
  } catch {
    return fallbackUrl;
  }
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
        source: 'context-menu',
        createdAt: Date.now(),
      };

      await setComposeDraft(draft);
      await openPopupWithFallback(tab?.id);
    }
  })();
});

chrome.runtime.onMessage.addListener((message: unknown, _sender, sendResponse) => {
  void (async () => {
    if (isSendQuickRequest(message)) {
      sendResponse(await sendQuickAction(message.url));
      return;
    }

    if (isSendComposeRequest(message)) {
      sendResponse(
        await sendComposeAction({
          url: message.url,
          text: message.text,
          forceRefresh: message.forceRefresh,
        }),
      );
      return;
    }

    if (isGetComposeStateRequest(message)) {
      sendResponse(await getComposeState());
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
