import { resolveIngestTargetUrl, normalizeApiUrl, toApiOriginPattern } from './url';

const SETTINGS_STORAGE_KEY = 'lce.settings.v1';
const DRAFT_STORAGE_KEY = 'lce.compose-draft.v1';

export const DEFAULT_AUTHOR_NAME = 'LiveChat Extension';

export interface ExtensionSettings {
  apiUrl: string;
  ingestToken: string;
  guildId: string;
  authorName: string;
  authorImage: string | null;
}

export interface ComposeDraft {
  url: string;
  text: string;
  forceRefresh: boolean;
  source: string;
  createdAt: number;
}

interface SettingsValidationSuccess {
  ok: true;
  value: ExtensionSettings;
}

interface SettingsValidationFailure {
  ok: false;
  message: string;
}

export type SettingsValidationResult = SettingsValidationSuccess | SettingsValidationFailure;

export interface PermissionTransitionResult {
  granted: boolean;
  pattern: string;
  removedPrevious: boolean;
  reason?: string;
}

const asNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

const getSessionStorageArea = (): chrome.storage.StorageArea => {
  return chrome.storage.session || chrome.storage.local;
};

export const normalizeSettingsInput = (input: Partial<ExtensionSettings>): SettingsValidationResult => {
  const rawApiUrl = asNonEmptyString(input.apiUrl);
  if (!rawApiUrl) {
    return {
      ok: false,
      message: 'API_URL est obligatoire.'
    };
  }

  let apiUrl: string;
  try {
    apiUrl = normalizeApiUrl(rawApiUrl);
  } catch (error) {
    return {
      ok: false,
      message: error instanceof Error ? error.message : 'API_URL invalide.'
    };
  }

  const ingestToken = asNonEmptyString(input.ingestToken);
  if (!ingestToken) {
    return {
      ok: false,
      message: 'INGEST_API_TOKEN est obligatoire.'
    };
  }

  const guildId = asNonEmptyString(input.guildId);
  if (!guildId) {
    return {
      ok: false,
      message: 'guildId est obligatoire.'
    };
  }

  const authorName = asNonEmptyString(input.authorName) || DEFAULT_AUTHOR_NAME;
  const authorImage = asNonEmptyString(input.authorImage) || null;

  return {
    ok: true,
    value: {
      apiUrl,
      ingestToken,
      guildId,
      authorName,
      authorImage
    }
  };
};

export const isSettingsComplete = (value: ExtensionSettings | null): value is ExtensionSettings => {
  if (!value) {
    return false;
  }

  return !!(
    asNonEmptyString(value.apiUrl) &&
    asNonEmptyString(value.ingestToken) &&
    asNonEmptyString(value.guildId) &&
    asNonEmptyString(value.authorName)
  );
};

export const getSettings = async (): Promise<ExtensionSettings | null> => {
  const payload = await chrome.storage.local.get(SETTINGS_STORAGE_KEY);
  const candidate = payload[SETTINGS_STORAGE_KEY] as Partial<ExtensionSettings> | undefined;

  if (!candidate) {
    return null;
  }

  const normalized = normalizeSettingsInput(candidate);
  return normalized.ok ? normalized.value : null;
};

export const saveSettings = async (settings: ExtensionSettings): Promise<void> => {
  await chrome.storage.local.set({
    [SETTINGS_STORAGE_KEY]: settings
  });
};

export const clearSettings = async (): Promise<void> => {
  await chrome.storage.local.remove(SETTINGS_STORAGE_KEY);
};

export const getComposeDraft = async (): Promise<ComposeDraft | null> => {
  const payload = await getSessionStorageArea().get(DRAFT_STORAGE_KEY);
  const candidate = payload[DRAFT_STORAGE_KEY] as Partial<ComposeDraft> | undefined;

  if (!candidate) {
    return null;
  }

  const normalizedUrl = resolveIngestTargetUrl(`${candidate.url || ''}`);
  if (!normalizedUrl) {
    return null;
  }

  return {
    url: normalizedUrl,
    text: `${candidate.text || ''}`,
    forceRefresh: !!candidate.forceRefresh,
    source: asNonEmptyString(candidate.source) || 'unknown',
    createdAt:
      typeof candidate.createdAt === 'number' && Number.isFinite(candidate.createdAt)
        ? Math.floor(candidate.createdAt)
        : Date.now()
  };
};

export const setComposeDraft = async (draft: ComposeDraft): Promise<void> => {
  const normalizedUrl = resolveIngestTargetUrl(draft.url);

  if (!normalizedUrl) {
    return;
  }

  await getSessionStorageArea().set({
    [DRAFT_STORAGE_KEY]: {
      ...draft,
      url: normalizedUrl
    }
  });
};

export const clearComposeDraft = async (): Promise<void> => {
  await getSessionStorageArea().remove(DRAFT_STORAGE_KEY);
};

export const hasApiHostPermission = async (apiUrl: string): Promise<boolean> => {
  const pattern = toApiOriginPattern(apiUrl);

  return chrome.permissions.contains({
    origins: [pattern]
  });
};

export const requestApiHostPermission = async (apiUrl: string): Promise<boolean> => {
  const pattern = toApiOriginPattern(apiUrl);

  return chrome.permissions.request({
    origins: [pattern]
  });
};

export const removeApiHostPermission = async (apiUrl: string): Promise<boolean> => {
  const pattern = toApiOriginPattern(apiUrl);

  return chrome.permissions.remove({
    origins: [pattern]
  });
};

export const ensureApiPermissionTransition = async (
  previousApiUrl: string | null,
  nextApiUrl: string,
): Promise<PermissionTransitionResult> => {
  const nextPattern = toApiOriginPattern(nextApiUrl);

  const alreadyGranted = await chrome.permissions.contains({
    origins: [nextPattern]
  });

  let granted = alreadyGranted;

  if (!alreadyGranted) {
    granted = await chrome.permissions.request({
      origins: [nextPattern]
    });
  }

  if (!granted) {
    return {
      granted: false,
      pattern: nextPattern,
      removedPrevious: false,
      reason: 'permission_denied'
    };
  }

  let removedPrevious = false;

  if (previousApiUrl) {
    const previousPattern = toApiOriginPattern(previousApiUrl);

    if (previousPattern !== nextPattern) {
      removedPrevious = await chrome.permissions.remove({
        origins: [previousPattern]
      });
    }
  }

  return {
    granted: true,
    pattern: nextPattern,
    removedPrevious
  };
};
