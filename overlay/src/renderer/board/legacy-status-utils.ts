interface SetStatusOptions {
  autoDismissMs?: number;
}

type StatusVariant = 'success' | 'error';

interface CreateStatusControllerParams {
  statusNode: unknown;
  setTimer: (callback: () => void, delayMs: number) => number;
  clearTimer: (timerId: number) => void;
}

export interface BoardStatusController {
  clearStatusTimer(): void;
  setStatus(message: string, variant?: StatusVariant, options?: SetStatusOptions): void;
  clearStatus(): void;
}

export interface BoardLegacyStatusUtils {
  createStatusController(params: CreateStatusControllerParams): BoardStatusController;
}

function normalizeAutoDismissMs(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return 5000;
  }

  return Math.max(0, Math.floor(value));
}

export function createBoardLegacyStatusUtils(): BoardLegacyStatusUtils {
  function createStatusController(params: CreateStatusControllerParams): BoardStatusController {
    let statusTimeoutId: number | null = null;

    function clearStatusTimer(): void {
      if (statusTimeoutId === null) {
        return;
      }

      params.clearTimer(statusTimeoutId);
      statusTimeoutId = null;
    }

    function clearStatus(): void {
      if (!(params.statusNode instanceof HTMLElement)) {
        clearStatusTimer();
        return;
      }

      clearStatusTimer();
      params.statusNode.textContent = '';
      params.statusNode.classList.add('hidden');
      params.statusNode.classList.remove('error', 'success');
    }

    function setStatus(message: string, variant: StatusVariant = 'success', options: SetStatusOptions = {}): void {
      if (!(params.statusNode instanceof HTMLElement)) {
        return;
      }

      const autoDismissMs = normalizeAutoDismissMs(options?.autoDismissMs);

      clearStatusTimer();

      // Force a hide/show cycle so a repeated toast still appears as a new one.
      params.statusNode.classList.add('hidden');
      params.statusNode.classList.remove('error', 'success');
      void params.statusNode.offsetWidth;

      params.statusNode.textContent = message;
      params.statusNode.classList.remove('hidden');
      params.statusNode.classList.add(variant);

      if (autoDismissMs > 0) {
        statusTimeoutId = params.setTimer(() => {
          clearStatus();
        }, autoDismissMs);
      }
    }

    return {
      clearStatusTimer,
      setStatus,
      clearStatus,
    };
  }

  return {
    createStatusController,
  };
}
