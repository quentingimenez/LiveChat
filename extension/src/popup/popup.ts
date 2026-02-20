import { MESSAGE_TYPES, type ActionResponse, type ComposeStateResponse } from '../lib/messages';
import '../styles/popup.css';

const form = document.getElementById('compose-form') as HTMLFormElement;
const urlInput = document.getElementById('compose-url') as HTMLInputElement;
const textInput = document.getElementById('compose-text') as HTMLTextAreaElement;
const forceRefreshInput = document.getElementById('compose-force-refresh') as HTMLInputElement;
const saveToBoardInput = document.getElementById('compose-save-to-board') as HTMLInputElement;
const sendButton = document.getElementById('send-button') as HTMLButtonElement;
const settingsWarning = document.getElementById('settings-warning') as HTMLParagraphElement;
const statusNode = document.getElementById('popup-status') as HTMLParagraphElement;
const openOptionsButton = document.getElementById('open-options') as HTMLButtonElement;

const STATUS_CLASS_MAP = {
  info: 'status-info',
  success: 'status-success',
  error: 'status-error',
  warning: 'status-warning',
} as const;

const isComposeStateResponse = (value: unknown): value is ComposeStateResponse => {
  if (!value || typeof value !== 'object') {
    return false;
  }

  const payload = value as Record<string, unknown>;
  return typeof payload.url === 'string' && typeof payload.hasSettings === 'boolean';
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

const setBusy = (busy: boolean): void => {
  sendButton.disabled = busy;
  sendButton.textContent = busy ? 'Envoi...' : 'Envoyer';
};

const updateSettingsWarning = (message: string | null): void => {
  if (!message) {
    settingsWarning.textContent = '';
    settingsWarning.classList.add('hidden');
    return;
  }

  settingsWarning.textContent = message;
  settingsWarning.classList.remove('hidden');
};

const loadComposeState = async (): Promise<void> => {
  clearStatus();

  try {
    const state = (await chrome.runtime.sendMessage({
      type: MESSAGE_TYPES.GET_COMPOSE_STATE,
    })) as ComposeStateResponse;

    if (!isComposeStateResponse(state)) {
      updateSettingsWarning('Impossible de charger l’état initial du formulaire.');
      sendButton.disabled = true;
      return;
    }

    urlInput.value = state.url;
    textInput.value = state.text;
    forceRefreshInput.checked = state.forceRefresh;
    saveToBoardInput.checked = state.saveToBoard;

    if (!state.hasSettings) {
      updateSettingsWarning(state.settingsError || 'Configuration incomplète. Ouvre les options.');
      sendButton.disabled = true;
      return;
    }

    sendButton.disabled = false;
    updateSettingsWarning(null);
  } catch (error) {
    updateSettingsWarning(error instanceof Error ? error.message : 'Erreur de communication avec le service worker.');
    sendButton.disabled = true;
  }
};

form.addEventListener('submit', (event) => {
  event.preventDefault();

  void (async () => {
    clearStatus();

    const url = urlInput.value.trim();
    if (!url) {
      setStatus('URL obligatoire.', 'error');
      return;
    }

    setBusy(true);

    try {
      const response = (await chrome.runtime.sendMessage({
        type: MESSAGE_TYPES.SEND_COMPOSE,
        url,
        text: textInput.value,
        forceRefresh: forceRefreshInput.checked,
        saveToBoard: saveToBoardInput.checked,
      })) as ActionResponse;

      if (!response || typeof response.ok !== 'boolean') {
        setStatus('Réponse invalide du service worker.', 'error');
        return;
      }

      if (response.ok) {
        setStatus(response.message, 'success');
        textInput.value = '';
        forceRefreshInput.checked = false;
        saveToBoardInput.checked = false;
      } else {
        setStatus(response.message, 'error');
      }
    } catch (error) {
      setStatus(error instanceof Error ? error.message : 'Erreur réseau.', 'error');
    } finally {
      setBusy(false);
    }
  })();
});

openOptionsButton.addEventListener('click', () => {
  void chrome.runtime.openOptionsPage();
});

void loadComposeState();
