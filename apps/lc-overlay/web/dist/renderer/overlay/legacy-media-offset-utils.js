export function createOverlayLegacyMediaOffsetUtils(options) {
    const { getStartOffsetSec, logWarning } = options;
    function applyMediaStartOffset(element, startOffsetSec) {
        if (!(element instanceof HTMLMediaElement)) {
            return;
        }
        const resolvedOffset = getStartOffsetSec(startOffsetSec);
        if (resolvedOffset === null) {
            return;
        }
        const seekToOffset = () => {
            const durationSec = Number.isFinite(element.duration) && element.duration > 0 ? element.duration : null;
            const targetSec = durationSec === null ? resolvedOffset : Math.min(resolvedOffset, Math.max(0, durationSec - 0.05));
            if (!Number.isFinite(targetSec) || targetSec <= 0) {
                return;
            }
            try {
                element.currentTime = targetSec;
            }
            catch (error) {
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
