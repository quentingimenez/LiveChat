import { INVALID_DURATION_MESSAGE, parseDisplayDuration } from '../lib/duration';
import { MESSAGE_TYPES, type ActionResponse, type ComposeStateResponse } from '../lib/messages';
import { initTheme } from '../lib/theme';
import '../styles/popup.css';
import '../styles/theme-light.css';

const form = document.getElementById('compose-form') as HTMLFormElement;
const composeView = document.getElementById('compose-view') as HTMLDivElement;
const setupView = document.getElementById('setup-view') as HTMLElement;
const setupMessage = document.getElementById('setup-message') as HTMLParagraphElement;
const setupOpenOptionsButton = document.getElementById('setup-open-options') as HTMLButtonElement;
const urlInput = document.getElementById('compose-url') as HTMLInputElement;
const textLabel = document.getElementById('compose-text-label') as HTMLLabelElement;
const textInput = document.getElementById('compose-text') as HTMLTextAreaElement;
const forceRefreshInput = document.getElementById('compose-force-refresh') as HTMLInputElement;
const saveToBoardInput = document.getElementById('compose-save-to-board') as HTMLInputElement;
const durationInput = document.getElementById('compose-duration') as HTMLInputElement;
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

const showSetupView = (message: string): void => {
  composeView.classList.add('hidden');
  setupView.classList.remove('hidden');
  setupMessage.textContent = message;
  clearStatus();
  updateSettingsWarning(null);
  sendButton.disabled = true;
};

const showComposeView = (): void => {
  setupView.classList.add('hidden');
  composeView.classList.remove('hidden');
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

// The text is the caption of a play sent now. A save-to-board keeps no text,
// so the field disappears in that mode (and its value is never submitted).
const syncSaveToBoard = (): void => {
  const saving = saveToBoardInput.checked;
  textLabel.hidden = saving;
  textInput.hidden = saving;
  if (saving) {
    textInput.value = '';
  }
};

const loadComposeState = async (): Promise<void> => {
  clearStatus();

  try {
    const state = (await browser.runtime.sendMessage({
      type: MESSAGE_TYPES.GET_COMPOSE_STATE,
    })) as ComposeStateResponse;

    if (!isComposeStateResponse(state)) {
      showSetupView('Impossible de charger l’état initial du formulaire.');
      return;
    }

    urlInput.value = state.url;
    textInput.value = state.text;
    forceRefreshInput.checked = state.forceRefresh;
    saveToBoardInput.checked = state.saveToBoard;
    syncSaveToBoard();

    if (!state.hasSettings) {
      showSetupView(state.settingsError || 'Configuration incomplète. Ouvre les options.');
      return;
    }

    showComposeView();
    sendButton.disabled = false;
    updateSettingsWarning(null);
  } catch (error) {
    showSetupView(error instanceof Error ? error.message : 'Erreur de communication avec le service worker.');
  }
};

saveToBoardInput.addEventListener('change', syncSaveToBoard);

form.addEventListener('submit', (event) => {
  event.preventDefault();

  void (async () => {
    clearStatus();

    const url = urlInput.value.trim();
    if (!url) {
      setStatus('Lien du média obligatoire.', 'error');
      return;
    }

    const duration = parseDisplayDuration(durationInput.value);
    if (!duration.ok) {
      setStatus(INVALID_DURATION_MESSAGE, 'error');
      return;
    }

    setBusy(true);

    try {
      const response = (await browser.runtime.sendMessage({
        type: MESSAGE_TYPES.SEND_COMPOSE,
        url,
        text: textInput.value,
        forceRefresh: forceRefreshInput.checked,
        saveToBoard: saveToBoardInput.checked,
        durationSec: duration.value ?? undefined,
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
        durationInput.value = '';
        syncSaveToBoard();
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

const openOptions = (): void => {
  void browser.runtime.openOptionsPage();
};

openOptionsButton.addEventListener('click', openOptions);
setupOpenOptionsButton.addEventListener('click', openOptions);

// Le thème est déjà posé par theme-guard.js : ici on le garde en phase avec le
// réglage partagé (storage) et avec le thème du navigateur.
void initTheme();
void loadComposeState();
