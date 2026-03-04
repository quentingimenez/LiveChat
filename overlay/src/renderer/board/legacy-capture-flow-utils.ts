interface CaptureState {
  captureItemId: string | null;
  capturePendingAccelerator: string | null;
  capturePendingDisplay: string;
}

interface RefreshCaptureUiParams {
  isCaptureUiReady: boolean;
  captureCurrentNode: unknown;
  captureSaveButton: unknown;
  state: CaptureState;
}

interface BeginCaptureForItemParams {
  itemId: string;
  captureOverlay: unknown;
  setCaptureState: (nextState: CaptureState) => void;
  refreshCaptureUi: () => void;
  setStatusSuccess: (message: string) => void;
}

interface EndCaptureParams {
  captureOverlay: unknown;
  setCaptureState: (nextState: CaptureState) => void;
  refreshCaptureUi: () => void;
}

interface CommitCapturedShortcutParams {
  state: CaptureState;
  bindings: unknown;
  assignShortcutForItem: (
    bindings: unknown,
    itemId: string,
    accelerator: string,
  ) => {
    nextBindings: Record<string, string>;
  };
  persistBindings: (nextBindings: Record<string, string>) => Promise<void>;
  endCapture: () => void;
  renderList: () => void;
  formatAcceleratorDisplay: (accelerator: string) => string;
  setStatusSuccess: (message: string) => void;
  setStatusError: (message: string) => void;
}

export interface BoardLegacyCaptureFlowUtils {
  refreshCaptureUi(params: RefreshCaptureUiParams): void;
  beginCaptureForItem(params: BeginCaptureForItemParams): void;
  endCapture(params: EndCaptureParams): void;
  commitCapturedShortcut(params: CommitCapturedShortcutParams): Promise<void>;
}

function toTrimmedString(value: unknown): string {
  return `${value || ''}`.trim();
}

function toErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  if (typeof error === 'string') {
    return error;
  }

  return `${error}`;
}

export function createBoardLegacyCaptureFlowUtils(): BoardLegacyCaptureFlowUtils {
  function refreshCaptureUi(params: RefreshCaptureUiParams): void {
    if (!params.isCaptureUiReady) {
      return;
    }

    if (!(params.captureCurrentNode instanceof HTMLElement) || !(params.captureSaveButton instanceof HTMLButtonElement)) {
      return;
    }

    params.captureCurrentNode.textContent = params.state.capturePendingDisplay || 'Aucune combinaison détectée.';
    params.captureSaveButton.disabled = !params.state.capturePendingAccelerator;
  }

  function beginCaptureForItem(params: BeginCaptureForItemParams): void {
    params.setCaptureState({
      captureItemId: toTrimmedString(params.itemId) || null,
      capturePendingAccelerator: null,
      capturePendingDisplay: '',
    });

    params.refreshCaptureUi();

    if (params.captureOverlay instanceof HTMLElement) {
      params.captureOverlay.classList.remove('hidden');
    }

    params.setStatusSuccess('Capture clavier active : fais la combinaison puis clique "Arrêter et enregistrer" (Esc pour annuler).');
  }

  function endCapture(params: EndCaptureParams): void {
    params.setCaptureState({
      captureItemId: null,
      capturePendingAccelerator: null,
      capturePendingDisplay: '',
    });

    params.refreshCaptureUi();

    if (params.captureOverlay instanceof HTMLElement) {
      params.captureOverlay.classList.add('hidden');
    }
  }

  async function commitCapturedShortcut(params: CommitCapturedShortcutParams): Promise<void> {
    const itemId = toTrimmedString(params.state.captureItemId);
    const accelerator = toTrimmedString(params.state.capturePendingAccelerator);

    if (!itemId) {
      params.setStatusError('Aucun mème cible pour ce raccourci.');
      return;
    }

    if (!accelerator) {
      params.setStatusError('Aucune combinaison valide détectée.');
      return;
    }

    const { nextBindings } = params.assignShortcutForItem(params.bindings, itemId, accelerator);

    try {
      await params.persistBindings(nextBindings);
      params.endCapture();
      params.renderList();
      params.setStatusSuccess(`Raccourci assigné : ${params.formatAcceleratorDisplay(accelerator)}`);
    } catch (error) {
      params.setStatusError(toErrorMessage(error) || "Impossible d'assigner ce raccourci.");
    }
  }

  return {
    refreshCaptureUi,
    beginCaptureForItem,
    endCapture,
    commitCapturedShortcut,
  };
}
