import {
  DEFAULT_AUTHOR_NAME,
  ensureApiPermissionTransition,
  getSettings,
  hasApiHostPermission,
  normalizeSettingsInput,
  requestApiHostPermission,
  saveSettings,
  type ExtensionSettings,
} from '../lib/settings';
import { normalizeApiUrl } from '../lib/url';
import '../styles/options.css';

interface PairingConsumeSuccessResponse {
  apiBaseUrl?: unknown;
  ingestApiToken?: unknown;
  guildId?: unknown;
  authorName?: unknown;
  authorImage?: unknown;
}

interface PairingConsumeErrorResponse {
  error?: unknown;
  message?: unknown;
}

type PairingConsumeResponse = PairingConsumeSuccessResponse & PairingConsumeErrorResponse;

const pairingForm = document.getElementById('pairing-form') as HTMLFormElement;
const manualForm = document.getElementById('options-form') as HTMLFormElement;
const pairingCodeInput = document.getElementById('pairing-code') as HTMLInputElement;
const pairButton = document.getElementById('pair-button') as HTMLButtonElement;
const apiUrlInput = document.getElementById('api-url') as HTMLInputElement;
const manualApiUrlInput = document.getElementById('api-url-manual') as HTMLInputElement;
const ingestTokenInput = document.getElementById('ingest-token') as HTMLInputElement;
const guildIdInput = document.getElementById('guild-id') as HTMLInputElement;
const authorNameInput = document.getElementById('author-name') as HTMLInputElement;
const authorImageInput = document.getElementById('author-image') as HTMLInputElement;
const permissionStateNode = document.getElementById('permission-state') as HTMLParagraphElement;
const saveButton = document.getElementById('save-button') as HTMLButtonElement;
const testButton = document.getElementById('test-button') as HTMLButtonElement;
const statusNode = document.getElementById('options-status') as HTMLParagraphElement;

const STATUS_CLASS_MAP = {
  info: 'status-info',
  success: 'status-success',
  error: 'status-error',
  warning: 'status-warning',
} as const;

const asNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized || null;
};

const parseJsonBody = <T>(rawBody: string): T | null => {
  const normalized = rawBody.trim();

  if (!normalized) {
    return null;
  }

  try {
    return JSON.parse(normalized) as T;
  } catch {
    return null;
  }
};

const setStatus = (message: string, variant: keyof typeof STATUS_CLASS_MAP): void => {
  statusNode.textContent = message;
  statusNode.classList.remove('hidden', 'status-info', 'status-success', 'status-error', 'status-warning');
  statusNode.classList.add(STATUS_CLASS_MAP[variant]);
};

const clearStatus = (): void => {
  statusNode.textContent = '';
  statusNode.classList.add('hidden');
};

const withBusyState = (busy: boolean, action: 'save' | 'test' | 'pair' = 'save'): void => {
  saveButton.disabled = busy;
  testButton.disabled = busy;
  pairButton.disabled = busy;
  saveButton.textContent = busy && action === 'save' ? 'Sauvegarde...' : 'Sauvegarder';
  testButton.textContent = busy && action === 'test' ? 'Test...' : 'Tester la config';
  pairButton.textContent = busy && action === 'pair' ? 'Appairage...' : 'Récupérer la configuration';
};

const getApiUrlFromInputs = (preferred: 'pairing' | 'manual'): string => {
  if (preferred === 'pairing') {
    return asNonEmptyString(apiUrlInput.value) || asNonEmptyString(manualApiUrlInput.value) || '';
  }

  return asNonEmptyString(manualApiUrlInput.value) || asNonEmptyString(apiUrlInput.value) || '';
};

const setApiUrlInputs = (value: string): void => {
  apiUrlInput.value = value;
  manualApiUrlInput.value = value;
};

const applySettingsToForm = (settings: ExtensionSettings): void => {
  setApiUrlInputs(settings.apiUrl);
  ingestTokenInput.value = settings.ingestToken;
  guildIdInput.value = settings.guildId;
  authorNameInput.value = settings.authorName;
  authorImageInput.value = settings.authorImage || '';
};

const collectSettingsFromForm = (): Partial<ExtensionSettings> => {
  return {
    apiUrl: getApiUrlFromInputs('manual'),
    ingestToken: ingestTokenInput.value,
    guildId: guildIdInput.value,
    authorName: authorNameInput.value,
    authorImage: authorImageInput.value,
  };
};

const refreshPermissionState = async (): Promise<void> => {
  const rawApiUrl = getApiUrlFromInputs('pairing').trim();

  if (!rawApiUrl) {
    permissionStateNode.textContent = 'Autorisation domaine: non configuré.';
    return;
  }

  let normalizedApiUrl: string;

  try {
    normalizedApiUrl = normalizeApiUrl(rawApiUrl);
  } catch {
    permissionStateNode.textContent = 'Autorisation domaine: URL invalide.';
    return;
  }

  const hasPermission = await hasApiHostPermission(normalizedApiUrl);
  permissionStateNode.textContent = hasPermission
    ? `Autorisation domaine: accordée (${new URL(normalizedApiUrl).origin})`
    : `Autorisation domaine: manquante (${new URL(normalizedApiUrl).origin})`;
};

const resolvePairingApiUrl = (): string => {
  const rawApiUrl = asNonEmptyString(getApiUrlFromInputs('pairing'));

  if (!rawApiUrl) {
    throw new Error('API_URL obligatoire avant appairage (URL racine du bot).');
  }

  return normalizeApiUrl(rawApiUrl);
};

const ensureApiPermission = async (apiUrl: string): Promise<boolean> => {
  const hasPermission = await hasApiHostPermission(apiUrl);
  return hasPermission || (await requestApiHostPermission(apiUrl));
};

const loadExistingSettings = async (): Promise<void> => {
  const existingSettings = await getSettings();

  if (!existingSettings) {
    authorNameInput.value = DEFAULT_AUTHOR_NAME;
    await refreshPermissionState();
    return;
  }

  applySettingsToForm(existingSettings);
  await refreshPermissionState();
};

const getPairingFailureMessage = (params: {
  status: number;
  endpoint: string;
  body: PairingConsumeResponse | null;
}) => {
  const { status, endpoint, body } = params;
  const remoteError = asNonEmptyString(body?.error) || asNonEmptyString(body?.message);

  if (status === 404 && body?.error === 'pairing_code_invalid_or_expired') {
    return 'Code invalide ou expiré. Regénère un code avec /pair-code.';
  }

  if (status === 404) {
    return `Endpoint introuvable (${endpoint}). Vérifie que le bot est à jour et que API_URL est la racine (sans /overlay ni /ingest).`;
  }

  if (status === 403) {
    return `Échec appairage (403): requête refusée avant le bot ou par un proxy. Vérifie API_URL racine et autorise POST ${endpoint}.`;
  }

  if (status === 401) {
    return `Échec appairage (401): accès non autorisé sur ${endpoint}. Vérifie la route exposée côté reverse-proxy.`;
  }

  if (status === 400 && body?.error === 'invalid_payload') {
    return 'Payload d’appairage invalide. Vérifie le code saisi.';
  }

  if (remoteError) {
    return `Échec de l’appairage (${status}): ${remoteError}`;
  }

  return `Échec de l’appairage (${status}) sur ${endpoint}.`;
};

pairingForm.addEventListener('submit', (event) => {
  event.preventDefault();

  void (async () => {
    clearStatus();
    withBusyState(true, 'pair');

    try {
      const pairingCode = asNonEmptyString(pairingCodeInput.value)?.toUpperCase();

      if (!pairingCode) {
        setStatus('Code unique obligatoire.', 'error');
        return;
      }

      const previousSettings = await getSettings();
      const apiUrl = resolvePairingApiUrl();
      const endpoint = `${apiUrl}/ingest/pair/consume`;
      const permissionGranted = await ensureApiPermission(apiUrl);

      if (!permissionGranted) {
        setStatus('Permission réseau refusée pour ce domaine API.', 'error');
        return;
      }

      const response = await fetch(endpoint, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          code: pairingCode,
        }),
      });

      const rawBody = await response.text();
      const body = parseJsonBody<PairingConsumeResponse>(rawBody);

      if (!response.ok) {
        setStatus(
          getPairingFailureMessage({
            status: response.status,
            endpoint,
            body,
          }),
          'error',
        );
        return;
      }

      const normalized = normalizeSettingsInput({
        apiUrl: asNonEmptyString(body?.apiBaseUrl) || apiUrl,
        ingestToken: asNonEmptyString(body?.ingestApiToken) || '',
        guildId: asNonEmptyString(body?.guildId) || '',
        authorName: asNonEmptyString(body?.authorName) || DEFAULT_AUTHOR_NAME,
        authorImage: asNonEmptyString(body?.authorImage) || '',
      });

      if (!normalized.ok) {
        setStatus(`Réponse appairage invalide: ${normalized.message}`, 'error');
        return;
      }

      const permissionTransition = await ensureApiPermissionTransition(previousSettings?.apiUrl || null, normalized.value.apiUrl);

      if (!permissionTransition.granted) {
        setStatus('Autorisation du domaine API refusée.', 'error');
        return;
      }

      await saveSettings(normalized.value);
      applySettingsToForm(normalized.value);
      pairingCodeInput.value = '';
      setStatus('Appairage réussi: configuration récupérée et sauvegardée.', 'success');
      await refreshPermissionState();
    } catch (error) {
      setStatus(error instanceof Error ? error.message : 'Échec appairage.', 'error');
    } finally {
      withBusyState(false);
    }
  })();
});

manualForm.addEventListener('submit', (event) => {
  event.preventDefault();

  void (async () => {
    clearStatus();
    withBusyState(true, 'save');

    try {
      const previousSettings = await getSettings();
      const normalized = normalizeSettingsInput(collectSettingsFromForm());

      if (!normalized.ok) {
        setStatus(normalized.message, 'error');
        return;
      }

      const permissionTransition = await ensureApiPermissionTransition(previousSettings?.apiUrl || null, normalized.value.apiUrl);

      if (!permissionTransition.granted) {
        setStatus('Autorisation du domaine API refusée.', 'error');
        return;
      }

      await saveSettings(normalized.value);
      setStatus('Configuration sauvegardée.', 'success');
      await refreshPermissionState();
    } catch (error) {
      setStatus(error instanceof Error ? error.message : 'Erreur de sauvegarde.', 'error');
    } finally {
      withBusyState(false);
    }
  })();
});

testButton.addEventListener('click', () => {
  void (async () => {
    clearStatus();
    withBusyState(true, 'test');

    try {
      const normalized = normalizeSettingsInput(collectSettingsFromForm());

      if (!normalized.ok) {
        setStatus(normalized.message, 'error');
        return;
      }

      const permissionGranted = await ensureApiPermission(normalized.value.apiUrl);

      if (!permissionGranted) {
        setStatus('Permission réseau refusée pour ce domaine API.', 'error');
        return;
      }

      const response = await fetch(`${normalized.value.apiUrl}/ingest`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${normalized.value.ingestToken}`,
        },
        body: JSON.stringify({}),
      });

      const body = (await response.json().catch(() => null)) as
        | {
            error?: string;
          }
        | null;

      if (response.status === 400 && body?.error === 'invalid_payload') {
        setStatus('Configuration valide: serveur joignable, token accepté, endpoint /ingest actif.', 'success');
        return;
      }

      if (response.status === 401 || body?.error === 'unauthorized') {
        setStatus('Token ingest invalide (401 unauthorized).', 'error');
        return;
      }

      if (response.status === 503 || body?.error === 'ingest_api_disabled') {
        setStatus('Le bot répond mais /ingest est désactivé (503).', 'warning');
        return;
      }

      setStatus(`Serveur joignable, réponse inattendue (${response.status}).`, 'warning');
      await refreshPermissionState();
    } catch (error) {
      setStatus(error instanceof Error ? error.message : 'Échec du test réseau.', 'error');
    } finally {
      withBusyState(false);
    }
  })();
});

apiUrlInput.addEventListener('input', () => {
  manualApiUrlInput.value = apiUrlInput.value;
  void refreshPermissionState();
});

manualApiUrlInput.addEventListener('input', () => {
  apiUrlInput.value = manualApiUrlInput.value;
  void refreshPermissionState();
});

void loadExistingSettings();
