export type ToastLevel = 'success' | 'error' | 'info';
export type SendSource = 'youtube' | 'tiktok' | 'twitter' | 'context-menu' | 'popup';

export const MESSAGE_TYPES = {
  SEND_QUICK: 'lce/send-quick',
  SEND_COMPOSE: 'lce/send-compose',
  GET_COMPOSE_STATE: 'lce/get-compose-state',
  GET_ACTIVE_MEDIA_URL: 'lce/get-active-media-url',
  TIKTOK_SYNC_ACTIVE_ITEM: 'lce/tiktok-sync-active-item',
  TIKTOK_GET_CAPTURED_URL: 'lce/tiktok-get-captured-url',
  SHOW_TOAST: 'lce/show-toast'
} as const;

export interface SendQuickRequestMessage {
  type: (typeof MESSAGE_TYPES)['SEND_QUICK'];
  url: string;
  source: SendSource;
}

export interface SendComposeRequestMessage {
  type: (typeof MESSAGE_TYPES)['SEND_COMPOSE'];
  url: string;
  text?: string;
  forceRefresh?: boolean;
}

export interface GetComposeStateRequestMessage {
  type: (typeof MESSAGE_TYPES)['GET_COMPOSE_STATE'];
}

export interface GetActiveMediaUrlRequestMessage {
  type: (typeof MESSAGE_TYPES)['GET_ACTIVE_MEDIA_URL'];
}

export interface TikTokSyncActiveItemRequestMessage {
  type: (typeof MESSAGE_TYPES)['TIKTOK_SYNC_ACTIVE_ITEM'];
  itemId: string | null;
  url: string | null;
}

export interface TikTokGetCapturedUrlRequestMessage {
  type: (typeof MESSAGE_TYPES)['TIKTOK_GET_CAPTURED_URL'];
}

export interface ShowToastMessage {
  type: (typeof MESSAGE_TYPES)['SHOW_TOAST'];
  level: ToastLevel;
  message: string;
}

export type BackgroundRequestMessage =
  | SendQuickRequestMessage
  | SendComposeRequestMessage
  | GetComposeStateRequestMessage
  | GetActiveMediaUrlRequestMessage
  | TikTokSyncActiveItemRequestMessage
  | TikTokGetCapturedUrlRequestMessage;

export interface ActionResponse {
  ok: boolean;
  message: string;
  jobId: string | null;
  errorCode?: string;
}

export interface ComposeStateResponse {
  ok: boolean;
  message?: string;
  url: string;
  text: string;
  forceRefresh: boolean;
  hasSettings: boolean;
  settingsError: string | null;
  draftSource: string | null;
}

export interface ActiveMediaUrlResponse {
  ok: boolean;
  url: string | null;
}

const isRecord = (value: unknown): value is Record<string, unknown> => {
  return typeof value === 'object' && value !== null;
};

const asTrimmedString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

const asNullableTrimmedString = (value: unknown): string | null => {
  if (value === null || value === undefined) {
    return null;
  }

  return asTrimmedString(value);
};

export const isSendQuickRequest = (value: unknown): value is SendQuickRequestMessage => {
  if (!isRecord(value) || value.type !== MESSAGE_TYPES.SEND_QUICK) {
    return false;
  }

  return !!asTrimmedString(value.url);
};

export const isSendComposeRequest = (value: unknown): value is SendComposeRequestMessage => {
  if (!isRecord(value) || value.type !== MESSAGE_TYPES.SEND_COMPOSE) {
    return false;
  }

  return !!asTrimmedString(value.url);
};

export const isGetComposeStateRequest = (value: unknown): value is GetComposeStateRequestMessage => {
  return isRecord(value) && value.type === MESSAGE_TYPES.GET_COMPOSE_STATE;
};

export const isGetActiveMediaUrlRequest = (value: unknown): value is GetActiveMediaUrlRequestMessage => {
  return isRecord(value) && value.type === MESSAGE_TYPES.GET_ACTIVE_MEDIA_URL;
};

export const isTikTokSyncActiveItemRequest = (value: unknown): value is TikTokSyncActiveItemRequestMessage => {
  if (!isRecord(value) || value.type !== MESSAGE_TYPES.TIKTOK_SYNC_ACTIVE_ITEM) {
    return false;
  }

  const itemId = asNullableTrimmedString(value.itemId);
  const url = asNullableTrimmedString(value.url);

  return (
    (itemId === null || typeof itemId === 'string') &&
    (url === null || typeof url === 'string')
  );
};

export const isTikTokGetCapturedUrlRequest = (value: unknown): value is TikTokGetCapturedUrlRequestMessage => {
  return isRecord(value) && value.type === MESSAGE_TYPES.TIKTOK_GET_CAPTURED_URL;
};

export const isBackgroundRequestMessage = (value: unknown): value is BackgroundRequestMessage => {
  return (
    isSendQuickRequest(value) ||
    isSendComposeRequest(value) ||
    isGetComposeStateRequest(value) ||
    isGetActiveMediaUrlRequest(value) ||
    isTikTokSyncActiveItemRequest(value) ||
    isTikTokGetCapturedUrlRequest(value)
  );
};

export const isShowToastMessage = (value: unknown): value is ShowToastMessage => {
  if (!isRecord(value) || value.type !== MESSAGE_TYPES.SHOW_TOAST) {
    return false;
  }

  const message = asTrimmedString(value.message);
  if (!message) {
    return false;
  }

  return value.level === 'success' || value.level === 'error' || value.level === 'info';
};
