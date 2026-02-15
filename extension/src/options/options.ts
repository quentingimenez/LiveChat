import {
  DEFAULT_AUTHOR_NAME,
  ensureApiPermissionTransition,
  getSettings,
  hasApiHostPermission,
  normalizeSettingsInput,
  requestApiHostPermission,
  saveSettings,
} from '../lib/settings';
import { normalizeApiUrl } from '../lib/url';
import '../styles/options.css';

const form = document.getElementById('options-form') as HTMLFormElement;
const apiUrlInput = document.getElementById('api-url') as HTMLInputElement;
const ingestTokenInput = document.getElementById('ingest-token') as HTMLInputElement;
const guildIdInput = document.getElementById('guild-id') as HTMLInputElement;
const authorNameInput = document.getElementById('author-name') as HTMLInputElement;
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

const setStatus = (message: string, variant: keyof typeof STATUS_CLASS_MAP): void => {
  statusNode.textContent = message;
  statusNode.classList.remove('hidden', 'status-info', 'status-success', 'status-error', 'status-warning');
  statusNode.classList.add(STATUS_CLASS_MAP[variant]);
};

const clearStatus = (): void => {
  statusNode.textContent = '';
  statusNode.classList.add('hidden');
};

const withBusyState = (busy: boolean): void => {
  saveButton.disabled = busy;
  testButton.disabled = busy;
  saveButton.textContent = busy ? 'Sauvegarde...' : 'Sauvegarder';
};

const collectSettingsFromForm = () => {
  return {
    apiUrl: apiUrlInput.value,
    ingestToken: ingestTokenInput.value,
    guildId: guildIdInput.value,
    authorName: authorNameInput.value,
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

const loadExistingSettings = async (): Promise<void> => {
  const existingSettings = await getSettings();

  if (!existingSettings) {
    authorNameInput.value = DEFAULT_AUTHOR_NAME;
    await refreshPermissionState();
    return;
  }

  apiUrlInput.value = existingSettings.apiUrl;
  ingestTokenInput.value = existingSettings.ingestToken;
  guildIdInput.value = existingSettings.guildId;
  authorNameInput.value = existingSettings.authorName;

  await refreshPermissionState();
};

form.addEventListener('submit', (event) => {
  event.preventDefault();

  void (async () => {
    clearStatus();
    withBusyState(true);

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
    withBusyState(true);

    try {
      const normalized = normalizeSettingsInput(collectSettingsFromForm());

      if (!normalized.ok) {
        setStatus(normalized.message, 'error');
        return;
      }

      const hasPermission = await hasApiHostPermission(normalized.value.apiUrl);
      const permissionGranted = hasPermission || (await requestApiHostPermission(normalized.value.apiUrl));

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

      const body = await response
        .json()
        .catch(() => null) as
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
