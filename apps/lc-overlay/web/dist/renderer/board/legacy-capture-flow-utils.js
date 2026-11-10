function toTrimmedString(value) {
    return `${value || ''}`.trim();
}
function toErrorMessage(error) {
    if (error instanceof Error && error.message) {
        return error.message;
    }
    if (typeof error === 'string') {
        return error;
    }
    return `${error}`;
}
export function createBoardLegacyCaptureFlowUtils() {
    function refreshCaptureUi(params) {
        if (!params.isCaptureUiReady) {
            return;
        }
        if (!(params.captureCurrentNode instanceof HTMLElement) || !(params.captureSaveButton instanceof HTMLButtonElement)) {
            return;
        }
        params.captureCurrentNode.textContent = params.state.capturePendingDisplay || 'Aucune combinaison détectée.';
        params.captureSaveButton.disabled = !params.state.capturePendingAccelerator;
    }
    function beginCaptureForItem(params) {
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
    function endCapture(params) {
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
    async function commitCapturedShortcut(params) {
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
        }
        catch (error) {
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
