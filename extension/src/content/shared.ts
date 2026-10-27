import {
  MESSAGE_TYPES,
  isShowToastMessage,
  type ActionResponse,
  type SendQuickRequestMessage,
  type SendSource,
  type ToastLevel,
} from '../lib/messages';
import { resolveIngestTargetUrl } from '../lib/url';
import { inpageStyles } from '../styles/inpage.css';

const STYLE_ID = 'lce-inpage-style';
const TOAST_CONTAINER_ID = 'lce-toast-container';

let toastHideTimeout: number | null = null;
let toastListenerRegistered = false;

const ensureStyles = () => {
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

export const showToast = (level: ToastLevel, message: string): void => {
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

export const showActionResponse = (response: ActionResponse): void => {
  if (response.ok) {
    showToast('success', response.message);
    return;
  }

  showToast('error', response.message);
};

export const registerToastListener = (): void => {
  if (toastListenerRegistered) {
    return;
  }

  chrome.runtime.onMessage.addListener((message: unknown) => {
    if (!isShowToastMessage(message)) {
      return;
    }

    showToast(message.level, message.message);
  });

  toastListenerRegistered = true;
};

export const sendQuickToBackground = async (url: string, source: SendSource): Promise<ActionResponse> => {
  const normalizedUrl = resolveIngestTargetUrl(url, window.location.href);

  if (!normalizedUrl) {
    return {
      ok: false,
      jobId: null,
      message: 'URL invalide ou non supportée pour envoi.'
    };
  }

  const message: SendQuickRequestMessage = {
    type: MESSAGE_TYPES.SEND_QUICK,
    url: normalizedUrl,
    source,
  };

  try {
    const response = (await chrome.runtime.sendMessage(message)) as ActionResponse;

    if (!response || typeof response.ok !== 'boolean') {
      return {
        ok: false,
        jobId: null,
        message: 'Réponse invalide du service worker.'
      };
    }

    return response;
  } catch (error) {
    return {
      ok: false,
      jobId: null,
      message: error instanceof Error ? error.message : 'Échec de communication avec le service worker.'
    };
  }
};

interface CreateActionButtonParams {
  label: string;
  title: string;
  onClick: () => Promise<void>;
  floating?: boolean;
}

export const createActionButton = (params: CreateActionButtonParams): HTMLButtonElement => {
  ensureStyles();

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'lce-button';

  if (params.floating) {
    button.classList.add('lce-button-floating');
  }

  button.textContent = params.label;
  button.title = params.title;

  button.addEventListener('click', async (event) => {
    event.preventDefault();
    event.stopPropagation();

    if (button.disabled) {
      return;
    }

    button.disabled = true;

    try {
      await params.onClick();
    } finally {
      window.setTimeout(() => {
        button.disabled = false;
      }, 250);
    }
  });

  return button;
};

export const startObservedScanner = (scan: () => void): void => {
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
