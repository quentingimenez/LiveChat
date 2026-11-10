function isFiniteNumber(value) {
    return typeof value === 'number' && Number.isFinite(value);
}
export function createOverlayLegacyPlaybackUtils() {
    function normalizeJobId(jobId) {
        if (typeof jobId !== 'string') {
            return null;
        }
        const normalized = jobId.trim();
        return normalized ? normalized : null;
    }
    function createActivePlayback(jobId) {
        return {
            jobId,
            state: 'playing'
        };
    }
    function normalizeRemainingCountdownMs(value) {
        if (!isFiniteNumber(value)) {
            return null;
        }
        return Math.max(0, Math.round(value));
    }
    function derivePlaybackState(params) {
        if (params.countdownPaused) {
            return 'paused';
        }
        if (params.playablePaused === true && params.playableEnded !== true) {
            return 'paused';
        }
        if (params.playablePaused === false && params.playableEnded !== true) {
            return 'playing';
        }
        return params.activePlaybackState;
    }
    function tickCountdown(params) {
        let countdownRemainingMs = Math.max(0, params.countdownRemainingMs);
        const nowMs = isFiniteNumber(params.nowMs) ? params.nowMs : Date.now();
        if (!params.countdownPaused && isFiniteNumber(params.countdownLastTickAtMs)) {
            const elapsedMs = Math.max(0, nowMs - params.countdownLastTickAtMs);
            countdownRemainingMs = Math.max(0, countdownRemainingMs - elapsedMs);
        }
        return {
            countdownRemainingMs,
            countdownLastTickAtMs: nowMs
        };
    }
    function isActiveMediaStillRunning(playableElement) {
        if (!(playableElement instanceof HTMLMediaElement)) {
            return false;
        }
        return !playableElement.ended;
    }
    function shouldClearOverlayAfterCountdown(params) {
        return params.countdownAutoClear || !params.isActiveMediaStillRunning;
    }
    return {
        normalizeJobId,
        createActivePlayback,
        normalizeRemainingCountdownMs,
        derivePlaybackState,
        tickCountdown,
        isActiveMediaStillRunning,
        shouldClearOverlayAfterCountdown
    };
}
