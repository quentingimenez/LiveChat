import { resolveIngestTargetUrl, normalizeApiUrl, toApiOriginPattern } from './url';
import { toNonEmptyString } from './utils';

const SETTINGS_STORAGE_KEY = 'lce.settings.v1';
const DRAFT_STORAGE_KEY = 'lce.compose-draft.v1';

/**
 * Réglages strictement techniques : l'auteur d'un envoi (pseudo, avatar) est
 * résolu côté serveur à partir du profil du membre lié au client d'ingestion,
 * l'extension ne le connaît ni ne le transmet. Les anciennes versions stockaient
 * `authorName`/`authorImage` : ces clés sont ignorées puis effacées au chargement.
 */
export interface ExtensionSettings {
  apiUrl: string;
  ingestToken: string;
  guildId: string;
}

export interface ComposeDraft {
  url: string;
  text: string;
  forceRefresh: boolean;
  saveToBoard: boolean;
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

const getSessionStorageArea = (): browser.storage.StorageArea => {
  return browser.storage.session || browser.storage.local;
};

export const normalizeSettingsInput = (input: Partial<ExtensionSettings>): SettingsValidationResult => {
  const rawApiUrl = toNonEmptyString(input.apiUrl);
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

  const ingestToken = toNonEmptyString(input.ingestToken);
  if (!ingestToken) {
    return {
      ok: false,
      message: 'INGEST_API_TOKEN est obligatoire.'
    };
  }

  const guildId = toNonEmptyString(input.guildId);
  if (!guildId) {
    return {
      ok: false,
      message: 'guildId est obligatoire.'
    };
  }

  return {
    ok: true,
    value: {
      apiUrl,
      ingestToken,
      guildId
    }
  };
};

export const isSettingsComplete = (value: ExtensionSettings | null): value is ExtensionSettings => {
  if (!value) {
    return false;
  }

  return !!(
    toNonEmptyString(value.apiUrl) &&
    toNonEmptyString(value.ingestToken) &&
    toNonEmptyString(value.guildId)
  );
};

export const getSettings = async (): Promise<ExtensionSettings | null> => {
  const payload = await browser.storage.local.get(SETTINGS_STORAGE_KEY);
  const stored: unknown = payload[SETTINGS_STORAGE_KEY];

  if (!stored || typeof stored !== 'object') {
    return null;
  }

  const normalized = normalizeSettingsInput(stored as Partial<ExtensionSettings>);
  if (!normalized.ok) {
    return null;
  }

  // Les versions précédentes stockaient un pseudo et un avatar : ils ne sont
  // plus lus, on les efface dès le premier chargement pour ne pas les laisser
  // traîner dans le navigateur.
  if ('authorName' in stored || 'authorImage' in stored) {
    await saveSettings(normalized.value);
  }

  return normalized.value;
};

export const saveSettings = async (settings: ExtensionSettings): Promise<void> => {
  await browser.storage.local.set({
    [SETTINGS_STORAGE_KEY]: settings
  });
};

export const clearSettings = async (): Promise<void> => {
  await browser.storage.local.remove(SETTINGS_STORAGE_KEY);
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
    saveToBoard: !!candidate.saveToBoard,
    source: toNonEmptyString(candidate.source) || 'unknown',
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

  return browser.permissions.contains({
    origins: [pattern]
  });
};

export const requestApiHostPermission = async (apiUrl: string): Promise<boolean> => {
  const pattern = toApiOriginPattern(apiUrl);

  return browser.permissions.request({
    origins: [pattern]
  });
};

export const removeApiHostPermission = async (apiUrl: string): Promise<boolean> => {
  const pattern = toApiOriginPattern(apiUrl);

  return browser.permissions.remove({
    origins: [pattern]
  });
};

/**
 * Retire l'ancien domaine autorisé quand l'URL de l'API change. Ne demande
 * aucune permission : l'appelant doit déjà avoir obtenu l'accès au nouveau
 * domaine dans le même geste utilisateur.
 */
export const removePreviousApiHostPermission = async (
  previousApiUrl: string | null,
  nextApiUrl: string,
): Promise<boolean> => {
  if (!previousApiUrl) {
    return false;
  }

  const previousPattern = toApiOriginPattern(previousApiUrl);
  const nextPattern = toApiOriginPattern(nextApiUrl);

  if (previousPattern === nextPattern) {
    return false;
  }

  return browser.permissions.remove({
    origins: [previousPattern]
  });
};

/**
 * Demande l'accès au domaine puis retire l'ancien s'il a changé.
 *
 * `permissions.request` est volontairement la première opération de la
 * fonction : Firefox refuse l'appel si un `await` a consommé le geste
 * utilisateur (« may only be called from a user input handler »).
 */
export const ensureApiPermissionTransition = async (nextApiUrl: string): Promise<PermissionTransitionResult> => {
  const nextPattern = toApiOriginPattern(nextApiUrl);

  const granted = await browser.permissions.request({
    origins: [nextPattern]
  });

  if (!granted) {
    return {
      granted: false,
      pattern: nextPattern,
      removedPrevious: false,
      reason: 'permission_denied'
    };
  }

  const previousSettings = await getSettings();

  const removedPrevious = await removePreviousApiHostPermission(previousSettings?.apiUrl || null, nextApiUrl);

  return {
    granted: true,
    pattern: nextPattern,
    removedPrevious
  };
};
