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
}

const form = document.getElementById('options-form') as HTMLFormElement;
const pairingCodeInput = document.getElementById('pairing-code') as HTMLInputElement;
const deviceNameInput = document.getElementById('device-name') as HTMLInputElement;
const pairButton = document.getElementById('pair-button') as HTMLButtonElement;
const apiUrlInput = document.getElementById('api-url') as HTMLInputElement;
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

const applySettingsToForm = (settings: ExtensionSettings): void => {
  apiUrlInput.value = settings.apiUrl;
  ingestTokenInput.value = settings.ingestToken;
  guildIdInput.value = settings.guildId;
  authorNameInput.value = settings.authorName;
  authorImageInput.value = settings.authorImage || '';
};

const collectSettingsFromForm = (): Partial<ExtensionSettings> => {
  return {
    apiUrl: apiUrlInput.value,
    ingestToken: ingestTokenInput.value,
    guildId: guildIdInput.value,
    authorName: authorNameInput.value,
    authorImage: authorImageInput.value,
  };
};

const refreshPermissionState = async (): Promise<void> => {
  const rawApiUrl = apiUrlInput.value.trim();

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

const resolvePairingApiUrl = async (): Promise<string> => {
  const rawApiUrl = apiUrlInput.value.trim();

  if (rawApiUrl) {
    return normalizeApiUrl(rawApiUrl);
  }

  const existingSettings = await getSettings();

  if (existingSettings?.apiUrl) {
    apiUrlInput.value = existingSettings.apiUrl;
    return existingSettings.apiUrl;
  }

  throw new Error('API_URL est requis pour récupérer la configuration avec un code.');
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

pairButton.addEventListener('click', () => {
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
      const apiUrl = await resolvePairingApiUrl();
      const permissionGranted = await ensureApiPermission(apiUrl);

      if (!permissionGranted) {
        setStatus('Permission réseau refusée pour ce domaine API.', 'error');
        return;
      }

      const response = await fetch(`${apiUrl}/ingest/pair/consume`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          code: pairingCode,
          deviceName: asNonEmptyString(deviceNameInput.value) || undefined,
        }),
      });

      const body = (await response.json().catch(() => null)) as
        | (PairingConsumeSuccessResponse & PairingConsumeErrorResponse)
        | null;

      if (!response.ok) {
        if (response.status === 404 && body?.error === 'pairing_code_invalid_or_expired') {
          setStatus('Code invalide ou expiré. Regénère un code avec /overlay-code.', 'error');
          return;
        }

        if (response.status === 400 && body?.error === 'invalid_payload') {
          setStatus('Payload d’appairage invalide. Vérifie le code saisi.', 'error');
          return;
        }

        setStatus(`Échec de l’appairage (${response.status}).`, 'error');
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

form.addEventListener('submit', (event) => {
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
  void refreshPermissionState();
});

void loadExistingSettings();
