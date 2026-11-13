function isFiniteNumber(value) {
    return typeof value === 'number' && Number.isFinite(value);
}
export function createOverlayLegacyCountdownUtils() {
    function getRemainingCountdownMs(params) {
        return params.normalizeRemainingCountdownMs(params.countdownState.remainingMs);
    }
    function renderCountdown(params) {
        if (!(params.countdownLayer instanceof HTMLElement)) {
            return;
        }
        const remainingMs = getRemainingCountdownMs({
            countdownState: params.countdownState,
            normalizeRemainingCountdownMs: params.normalizeRemainingCountdownMs,
        });
        if (remainingMs === null) {
            params.countdownLayer.textContent = '';
            params.countdownLayer.style.display = 'none';
            params.scheduleMediaHeaderLayout();
            return;
        }
        const remainingDisplay = params.formatRemainingTime(remainingMs);
        params.countdownLayer.textContent = remainingDisplay;
        params.countdownLayer.style.display = 'flex';
        params.scheduleMediaHeaderLayout();
    }
    function updateCountdown(params) {
        if (!isFiniteNumber(params.countdownState.remainingMs)) {
            return;
        }
        const nextCountdownState = params.tickCountdown({
            countdownRemainingMs: params.countdownState.remainingMs,
            countdownLastTickAtMs: params.countdownState.lastTickAtMs,
            countdownPaused: params.countdownState.paused,
            nowMs: params.nowMs(),
        });
        params.countdownState.remainingMs = nextCountdownState.countdownRemainingMs;
        params.countdownState.lastTickAtMs = nextCountdownState.countdownLastTickAtMs;
        params.renderCountdown();
        if (params.countdownState.remainingMs <= 0) {
            params.clearCountdownTimer();
            const isActiveMediaStillRunning = params.isActiveMediaStillRunning(params.activePlayableElement);
            if (params.shouldClearOverlayAfterCountdown({
                countdownAutoClear: params.countdownState.autoClear,
                isActiveMediaStillRunning,
            })) {
                params.clearOverlay();
            }
        }
    }
    function startCountdown(params) {
        if (!(params.countdownLayer instanceof HTMLElement) || !isFiniteNumber(params.durationSec) || params.durationSec <= 0) {
            return;
        }
        params.clearCountdownTimer();
        params.countdownState.remainingMs = params.durationSec * 1000;
        params.countdownState.lastTickAtMs = params.nowMs();
        params.countdownState.paused = false;
        params.countdownState.autoClear = params.options.autoClear === true;
        params.renderCountdown();
        params.countdownState.timer = params.setInterval(() => {
            params.updateCountdown();
        }, 200);
    }
    function pauseCountdown(params) {
        if (!isFiniteNumber(params.countdownState.remainingMs) || params.countdownState.paused) {
            return;
        }
        params.updateCountdown();
        params.countdownState.paused = true;
    }
    function resumeCountdown(params) {
        if (!isFiniteNumber(params.countdownState.remainingMs) || !params.countdownState.paused) {
            return;
        }
        params.countdownState.paused = false;
        params.countdownState.lastTickAtMs = params.nowMs();
    }
    function clearCountdown(params) {
        params.clearCountdownTimer();
        params.countdownState.remainingMs = null;
        params.countdownState.lastTickAtMs = null;
        params.countdownState.paused = false;
        params.countdownState.autoClear = false;
        if (params.countdownLayer instanceof HTMLElement) {
            params.countdownLayer.textContent = '';
            params.countdownLayer.style.display = 'none';
        }
        params.scheduleMediaHeaderLayout();
    }
    return {
        getRemainingCountdownMs,
        renderCountdown,
        updateCountdown,
        startCountdown,
        pauseCountdown,
        resumeCountdown,
        clearCountdown,
    };
}
