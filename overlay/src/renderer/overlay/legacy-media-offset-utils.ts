interface CreateOverlayLegacyMediaOffsetUtilsOptions {
  getStartOffsetSec(value: unknown): number | null;
  logWarning(message: string, error?: unknown): void;
}

export interface OverlayLegacyMediaOffsetUtils {
  applyMediaStartOffset(element: unknown, startOffsetSec: unknown): void;
}

export function createOverlayLegacyMediaOffsetUtils(
  options: CreateOverlayLegacyMediaOffsetUtilsOptions
): OverlayLegacyMediaOffsetUtils {
  const { getStartOffsetSec, logWarning } = options;

  function applyMediaStartOffset(element: unknown, startOffsetSec: unknown): void {
    if (!(element instanceof HTMLMediaElement)) {
      return;
    }

    const resolvedOffset = getStartOffsetSec(startOffsetSec);
    if (resolvedOffset === null) {
      return;
    }

    const seekToOffset = (): void => {
      const durationSec = Number.isFinite(element.duration) && element.duration > 0 ? element.duration : null;
      const targetSec = durationSec === null ? resolvedOffset : Math.min(resolvedOffset, Math.max(0, durationSec - 0.05));

      if (!Number.isFinite(targetSec) || targetSec <= 0) {
        return;
      }

      try {
        element.currentTime = targetSec;
      } catch (error) {
        logWarning('Media offset seek failed:', error);
      }
    };

    if (typeof element.readyState === 'number' && element.readyState >= 1) {
      seekToOffset();
      return;
    }

    element.addEventListener('loadedmetadata', seekToOffset, {
      once: true
    });
  }

  return {
    applyMediaStartOffset
  };
}
