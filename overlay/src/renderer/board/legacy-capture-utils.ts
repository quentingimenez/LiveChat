interface ResolveCaptureKeyInputParams {
  key: unknown;
  accelerator: unknown;
  display: unknown;
  currentPendingAccelerator: unknown;
  currentPendingDisplay: unknown;
  formatAcceleratorDisplay: (accelerator: string) => string;
}

interface CaptureKeyResolutionBase {
  nextPendingAccelerator: string | null;
  nextPendingDisplay: string;
}

interface CaptureKeyResolutionCancelled extends CaptureKeyResolutionBase {
  kind: 'cancelled';
}

interface CaptureKeyResolutionPending extends CaptureKeyResolutionBase {
  kind: 'pending';
  statusMessage: string;
}

interface CaptureKeyResolutionDetected extends CaptureKeyResolutionBase {
  kind: 'detected';
  statusMessage: string;
}

type CaptureKeyResolution = CaptureKeyResolutionCancelled | CaptureKeyResolutionPending | CaptureKeyResolutionDetected;

export interface BoardLegacyCaptureUtils {
  resolveCaptureKeyInput(params: ResolveCaptureKeyInputParams): CaptureKeyResolution;
}

function toTrimmedString(value: unknown): string {
  return `${value || ''}`.trim();
}

export function createBoardLegacyCaptureUtils(): BoardLegacyCaptureUtils {
  function resolveCaptureKeyInput(params: ResolveCaptureKeyInputParams): CaptureKeyResolution {
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
