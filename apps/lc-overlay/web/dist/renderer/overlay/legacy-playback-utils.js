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
    // Reason echoed in the playback-stop report for an `overlay://stop`:
    // only a stop on this device (`local`) is told apart, every other stop
    // is a plain end of playback.
    function stopReportReason(stopPayload) {
        return stopPayload && stopPayload.reason === 'local' ? 'local' : null;
    }
    // Seconds of the job left to fill once a video has played through, or 0.
    // A job outlasts its video when the server gave a very short clip a
    // minimum display time, or when the sender chose a longer duration: the
    // clip is then extended (see `resolveVideoEndAction`) instead of leaving
    // the screen at its first end. Unknown media durations never extend.
    function videoExtensionSec(payload) {
        const media = payload && payload.media;
        if (!media || media.kind !== 'video' || !isFiniteNumber(media.durationSec) || media.durationSec <= 0) {
            return 0;
        }
        if (!isFiniteNumber(payload.durationSec)) {
            return 0;
        }
        const offsetSec = isFiniteNumber(media.startOffsetSec) && media.startOffsetSec > 0 ? media.startOffsetSec : 0;
        const clipSec = Math.max(0, media.durationSec - offsetSec);
        return Math.max(0, payload.durationSec - clipSec);
    }
    // What to do when a video reaches its end: `clear` the overlay (job
    // over, or nothing to extend), `replay` a silent clip from its start, or
    // `hold` the last frame of a clip with sound (its sound plays once).
    // The server rounds durations up to whole seconds, so less than
    // `minRemainingMs` left is not worth another pass.
    function resolveVideoEndAction(params) {
        if (params.extend !== true) {
            return 'clear';
        }
        const minRemainingMs = isFiniteNumber(params.minRemainingMs) ? params.minRemainingMs : 250;
        if (!isFiniteNumber(params.remainingMs) || params.remainingMs < minRemainingMs) {
            return 'clear';
        }
        return params.hasAudio === true ? 'hold' : 'replay';
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
        stopReportReason,
        videoExtensionSec,
        resolveVideoEndAction,
        isActiveMediaStillRunning,
        shouldClearOverlayAfterCountdown
    };
}
