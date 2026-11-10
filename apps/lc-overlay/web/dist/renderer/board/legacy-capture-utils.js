function toTrimmedString(value) {
    return `${value || ''}`.trim();
}
export function createBoardLegacyCaptureUtils() {
    function resolveCaptureKeyInput(params) {
        const key = toTrimmedString(params.key);
        if (key === 'Escape') {
            return {
                kind: 'cancelled',
                nextPendingAccelerator: null,
                nextPendingDisplay: ''
            };
        }
        const accelerator = toTrimmedString(params.accelerator);
        const display = toTrimmedString(params.display);
        const pendingAccelerator = toTrimmedString(params.currentPendingAccelerator);
        const pendingDisplay = toTrimmedString(params.currentPendingDisplay);
        if (!accelerator) {
            return {
                kind: 'pending',
                nextPendingAccelerator: pendingAccelerator || null,
                nextPendingDisplay: pendingAccelerator ? pendingDisplay : display || pendingDisplay,
                statusMessage: 'Combinaison en cours... ajoute une touche non-modificatrice pour valider.'
            };
        }
        const formattedAccelerator = params.formatAcceleratorDisplay(accelerator) || accelerator;
        return {
            kind: 'detected',
            nextPendingAccelerator: accelerator,
            nextPendingDisplay: formattedAccelerator,
            statusMessage: `Combinaison détectée : ${formattedAccelerator}. Clique "Arrêter et enregistrer".`
        };
    }
    return {
        resolveCaptureKeyInput
    };
}
