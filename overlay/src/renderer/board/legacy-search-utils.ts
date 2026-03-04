interface ScheduleDebouncedTaskParams {
  delayMs: number;
  setTimer: (callback: () => void, delayMs: number) => number;
  runTask: () => void;
}

export interface BoardLegacySearchUtils {
  normalizeSearchQuery(value: unknown): string;
  clearDebounceTimer(timerId: unknown, clearTimer: (timerId: number) => void): number | null;
  scheduleDebouncedTask(params: ScheduleDebouncedTaskParams): number;
}

function toSafeDelayMs(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return 0;
  }

  return Math.max(0, Math.floor(value));
}

function toTimerId(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return null;
  }

  return value;
}

export function createBoardLegacySearchUtils(): BoardLegacySearchUtils {
  function normalizeSearchQuery(value: unknown): string {
    return `${value || ''}`.trim();
  }

  function clearDebounceTimer(timerId: unknown, clearTimer: (timerId: number) => void): number | null {
    const normalizedTimerId = toTimerId(timerId);
    if (normalizedTimerId === null) {
      return null;
    }

    clearTimer(normalizedTimerId);
    return null;
  }

  function scheduleDebouncedTask(params: ScheduleDebouncedTaskParams): number {
    return params.setTimer(() => {
      params.runTask();
    }, toSafeDelayMs(params.delayMs));
  }

  return {
    normalizeSearchQuery,
    clearDebounceTimer,
    scheduleDebouncedTask
  };
}
