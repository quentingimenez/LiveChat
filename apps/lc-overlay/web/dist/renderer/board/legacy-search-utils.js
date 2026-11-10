function toSafeDelayMs(value) {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
        return 0;
    }
    return Math.max(0, Math.floor(value));
}
function toTimerId(value) {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
        return null;
    }
    return value;
}
export function createBoardLegacySearchUtils() {
    function normalizeSearchQuery(value) {
        return `${value || ''}`.trim();
    }
    function clearDebounceTimer(timerId, clearTimer) {
        const normalizedTimerId = toTimerId(timerId);
        if (normalizedTimerId === null) {
            return null;
        }
        clearTimer(normalizedTimerId);
        return null;
    }
    function scheduleDebouncedTask(params) {
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
