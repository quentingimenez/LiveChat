const MODAL_STYLE_ID = 'lce-compose-modal-style';
const MODAL_OVERLAY_ID = 'lce-compose-modal-overlay';

export interface ComposeSubmitPayload {
  url: string;
  text: string;
  forceRefresh: boolean;
  saveToBoard: boolean;
}

export interface ComposeSubmitResult {
  ok: boolean;
  message: string;
}

export interface OpenComposeModalParams {
  initialUrl: string;
  sourceLabel: string;
  onSubmit: (payload: ComposeSubmitPayload) => Promise<ComposeSubmitResult>;
  onSuccess?: (message: string) => void;
  onError?: (message: string) => void;
}

interface ComposeModalRefs {
  overlay: HTMLDivElement;
  form: HTMLFormElement;
  sourceNode: HTMLSpanElement;
  urlInput: HTMLInputElement;
  textInput: HTMLTextAreaElement;
  forceRefreshInput: HTMLInputElement;
  saveToBoardInput: HTMLInputElement;
  submitButton: HTMLButtonElement;
  cancelButton: HTMLButtonElement;
  closeButton: HTMLButtonElement;
  statusNode: HTMLParagraphElement;
}

let modalRefs: ComposeModalRefs | null = null;
let activeParams: OpenComposeModalParams | null = null;
let modalBusy = false;
let keydownListenerRegistered = false;

const ensureStyles = (): void => {
  if (document.getElementById(MODAL_STYLE_ID)) {
    return;
  }

  const styleNode = document.createElement('style');
  styleNode.id = MODAL_STYLE_ID;
  styleNode.textContent = `
.lce-compose-modal-overlay {
  position: fixed;
  inset: 0;
  z-index: 2147483647;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.56);
  padding: 16px;
}
.lce-compose-modal-overlay.is-hidden {
  display: none;
}
.lce-compose-modal {
  width: min(520px, 100%);
  max-height: min(90vh, 760px);
  overflow: auto;
  border-radius: 14px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  background: #11161d;
  color: #f4f7fb;
  box-shadow: 0 22px 46px rgba(0, 0, 0, 0.5);
  font-family: "Segoe UI", "Helvetica Neue", Arial, sans-serif;
}
.lce-compose-modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 16px 18px 12px;
}
.lce-compose-modal-title {
  margin: 0;
  font-size: 17px;
  font-weight: 700;
}
.lce-compose-modal-source {
  margin: 4px 0 0;
  font-size: 12px;
  color: #a9b4c2;
}
.lce-compose-modal-close {
  border: none;
  border-radius: 999px;
  width: 30px;
  height: 30px;
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
  font-size: 16px;
  line-height: 1;
  cursor: pointer;
}
.lce-compose-modal-close:hover {
  background: rgba(255, 255, 255, 0.18);
}
.lce-compose-modal-form {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 0 18px 18px;
}
.lce-compose-modal-label {
  font-size: 12px;
  font-weight: 600;
  color: #cfdae7;
}
.lce-compose-modal-input,
.lce-compose-modal-textarea {
  width: 100%;
  border-radius: 10px;
  border: 1px solid rgba(255, 255, 255, 0.18);
  background: rgba(8, 11, 16, 0.78);
  color: #f3f7fd;
  padding: 10px 11px;
  font-size: 14px;
  line-height: 1.4;
  box-sizing: border-box;
}
.lce-compose-modal-textarea {
  resize: vertical;
  min-height: 92px;
}
.lce-compose-modal-input:focus,
.lce-compose-modal-textarea:focus {
  outline: 2px solid rgba(64, 153, 255, 0.5);
  border-color: rgba(64, 153, 255, 0.9);
}
.lce-compose-modal-checkbox {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #d4dde8;
}
.lce-compose-modal-status {
  margin: 6px 0 0;
  border-radius: 9px;
  padding: 8px 10px;
  font-size: 13px;
}
.lce-compose-modal-status.is-hidden {
  display: none;
}
.lce-compose-modal-status-error {
  background: rgba(255, 69, 58, 0.2);
  border: 1px solid rgba(255, 69, 58, 0.45);
  color: #ffb7b2;
}
.lce-compose-modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 9px;
  margin-top: 6px;
}
.lce-compose-modal-button {
  border-radius: 10px;
  border: 1px solid transparent;
  min-height: 36px;
  padding: 0 14px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}
.lce-compose-modal-button-cancel {
  background: rgba(255, 255, 255, 0.08);
  color: #eaf0f7;
}
.lce-compose-modal-button-cancel:hover {
  background: rgba(255, 255, 255, 0.14);
}
.lce-compose-modal-button-submit {
  background: linear-gradient(135deg, #1f7aff, #35a2ff);
  color: #f8fbff;
}
.lce-compose-modal-button-submit:hover {
  filter: brightness(1.04);
}
.lce-compose-modal-button:disabled {
  cursor: default;
  opacity: 0.75;
  filter: none;
}
body.lce-compose-modal-open {
  overflow: hidden !important;
}
`;
  document.head.appendChild(styleNode);
};

const clearStatus = (): void => {
  if (!modalRefs) {
    return;
  }

  modalRefs.statusNode.className = 'lce-compose-modal-status is-hidden';
  modalRefs.statusNode.textContent = '';
};

const setErrorStatus = (message: string): void => {
  if (!modalRefs) {
    return;
  }

  modalRefs.statusNode.className = 'lce-compose-modal-status lce-compose-modal-status-error';
  modalRefs.statusNode.textContent = message;
};

const setBusy = (busy: boolean): void => {
  modalBusy = busy;

  if (!modalRefs) {
    return;
  }

  modalRefs.submitButton.disabled = busy;
  modalRefs.cancelButton.disabled = busy;
  modalRefs.closeButton.disabled = busy;
  modalRefs.urlInput.disabled = busy;
  modalRefs.textInput.disabled = busy;
  modalRefs.forceRefreshInput.disabled = busy;
  modalRefs.saveToBoardInput.disabled = busy;
  modalRefs.submitButton.textContent = busy ? 'Envoi...' : 'Envoyer';
};

const isVisible = (): boolean => {
  if (!modalRefs) {
    return false;
  }

  return !modalRefs.overlay.classList.contains('is-hidden');
};

const closeModal = (force = false): void => {
  if (!modalRefs) {
    return;
  }

  if (modalBusy && !force) {
    return;
  }

  modalRefs.overlay.classList.add('is-hidden');
  modalRefs.overlay.setAttribute('aria-hidden', 'true');
  document.body.classList.remove('lce-compose-modal-open');
  clearStatus();
  activeParams = null;
};

const onSubmit = (): void => {
  if (!modalRefs || !activeParams || modalBusy) {
    return;
  }

  const { onSubmit: submit, onSuccess, onError } = activeParams;
  const payload: ComposeSubmitPayload = {
    url: modalRefs.urlInput.value.trim(),
    text: modalRefs.textInput.value,
    forceRefresh: modalRefs.forceRefreshInput.checked,
    saveToBoard: modalRefs.saveToBoardInput.checked,
  };

  if (!payload.url) {
    setErrorStatus('URL obligatoire.');
    return;
  }

  clearStatus();
  setBusy(true);
  closeModal(true);

  void (async () => {
    try {
      const response = await submit(payload);

      if (!response || typeof response.ok !== 'boolean' || typeof response.message !== 'string') {
        const message = 'Réponse invalide du service worker.';
        onError?.(message);
        return;
      }

      if (!response.ok) {
        onError?.(response.message);
        return;
      }

      onSuccess?.(response.message);
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Erreur de communication avec le service worker.';
      onError?.(message);
    } finally {
      setBusy(false);
    }
  })();
};

const ensureModal = (): ComposeModalRefs => {
  if (modalRefs) {
    return modalRefs;
  }

  ensureStyles();

  const overlay = document.createElement('div');
  overlay.id = MODAL_OVERLAY_ID;
  overlay.className = 'lce-compose-modal-overlay is-hidden';
  overlay.setAttribute('role', 'presentation');
  overlay.setAttribute('aria-hidden', 'true');
  overlay.innerHTML = `
<section class="lce-compose-modal" role="dialog" aria-modal="true" aria-labelledby="lce-compose-modal-title">
  <header class="lce-compose-modal-header">
    <div>
      <h2 id="lce-compose-modal-title" class="lce-compose-modal-title">Envoyer vers LiveChat</h2>
      <p class="lce-compose-modal-source">Source: <span data-lce-compose-source>-</span></p>
    </div>
    <button type="button" class="lce-compose-modal-close" data-lce-compose-close aria-label="Fermer">x</button>
  </header>
  <form class="lce-compose-modal-form" data-lce-compose-form>
    <label class="lce-compose-modal-label" for="lce-compose-modal-url">URL media</label>
    <input id="lce-compose-modal-url" class="lce-compose-modal-input" type="url" required placeholder="https://..." />

    <label class="lce-compose-modal-label" for="lce-compose-modal-text">Texte (optionnel)</label>
    <textarea
      id="lce-compose-modal-text"
      class="lce-compose-modal-textarea"
      rows="4"
      placeholder="Ton message LiveChat"
    ></textarea>

    <label class="lce-compose-modal-checkbox">
      <input id="lce-compose-modal-force-refresh" type="checkbox" />
      Ignorer le cache serveur (forceRefresh)
    </label>

    <label class="lce-compose-modal-checkbox">
      <input id="lce-compose-modal-save-to-board" type="checkbox" />
      Sauvegarder dans la Meme Board (saveToBoard)
    </label>

    <p class="lce-compose-modal-status is-hidden" data-lce-compose-status></p>

    <div class="lce-compose-modal-actions">
      <button type="button" class="lce-compose-modal-button lce-compose-modal-button-cancel" data-lce-compose-cancel>
        Annuler
      </button>
      <button type="submit" class="lce-compose-modal-button lce-compose-modal-button-submit" data-lce-compose-submit>
        Envoyer
      </button>
    </div>
  </form>
</section>
`;

  document.body.appendChild(overlay);

  const form = overlay.querySelector<HTMLFormElement>('[data-lce-compose-form]');
  const sourceNode = overlay.querySelector<HTMLSpanElement>('[data-lce-compose-source]');
  const urlInput = overlay.querySelector<HTMLInputElement>('#lce-compose-modal-url');
  const textInput = overlay.querySelector<HTMLTextAreaElement>('#lce-compose-modal-text');
  const forceRefreshInput = overlay.querySelector<HTMLInputElement>('#lce-compose-modal-force-refresh');
  const saveToBoardInput = overlay.querySelector<HTMLInputElement>('#lce-compose-modal-save-to-board');
  const submitButton = overlay.querySelector<HTMLButtonElement>('[data-lce-compose-submit]');
  const cancelButton = overlay.querySelector<HTMLButtonElement>('[data-lce-compose-cancel]');
  const closeButton = overlay.querySelector<HTMLButtonElement>('[data-lce-compose-close]');
  const statusNode = overlay.querySelector<HTMLParagraphElement>('[data-lce-compose-status]');

  if (
    !form ||
    !sourceNode ||
    !urlInput ||
    !textInput ||
    !forceRefreshInput ||
    !saveToBoardInput ||
    !submitButton ||
    !cancelButton ||
    !closeButton ||
    !statusNode
  ) {
    overlay.remove();
    throw new Error('Impossible d’initialiser la modal LiveChat.');
  }

  overlay.addEventListener('click', (event) => {
    if (event.target === overlay) {
      closeModal();
    }
  });

  closeButton.addEventListener('click', () => {
    closeModal();
  });

  cancelButton.addEventListener('click', () => {
    closeModal();
  });

  form.addEventListener('submit', (event) => {
    event.preventDefault();
    onSubmit();
  });

  if (!keydownListenerRegistered) {
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape' && isVisible()) {
        closeModal();
      }
    });
    keydownListenerRegistered = true;
  }

  modalRefs = {
    overlay,
    form,
    sourceNode,
    urlInput,
    textInput,
    forceRefreshInput,
    saveToBoardInput,
    submitButton,
    cancelButton,
    closeButton,
    statusNode,
  };

  return modalRefs;
};

export const openComposeModal = (params: OpenComposeModalParams): void => {
  const refs = ensureModal();
  activeParams = params;

  refs.sourceNode.textContent = params.sourceLabel;
  refs.urlInput.value = params.initialUrl;
  refs.textInput.value = '';
  refs.forceRefreshInput.checked = false;
  refs.saveToBoardInput.checked = false;

  clearStatus();
  setBusy(false);

  refs.overlay.classList.remove('is-hidden');
  refs.overlay.setAttribute('aria-hidden', 'false');
  document.body.classList.add('lce-compose-modal-open');

  window.setTimeout(() => {
    refs.textInput.focus();
  }, 0);
};
