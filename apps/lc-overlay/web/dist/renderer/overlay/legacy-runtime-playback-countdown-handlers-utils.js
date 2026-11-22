export function createOverlayLegacyRuntimePlaybackCountdownHandlersUtils() {
    function createRuntimePlaybackCountdownHandlers(params) {
        function clearTimer() {
            if (params.state.resetTimer === null) {
                return;
            }
            params.clearTimeout(params.state.resetTimer);
            params.state.resetTimer = null;
        }
        function clearCountdownTimer() {
            if (params.state.countdownState.timer === null) {
                return;
            }
            params.clearInterval(params.state.countdownState.timer);
            params.state.countdownState.timer = null;
        }
        function clearPlaybackSyncTimer() {
            if (params.state.playbackSyncTimer === null) {
                return;
            }
            params.clearInterval(params.state.playbackSyncTimer);
            params.state.playbackSyncTimer = null;
        }
        function getRemainingCountdownMs() {
            return params.countdownUtils.getRemainingCountdownMs({
                countdownState: params.state.countdownState,
                normalizeRemainingCountdownMs: params.playbackUtils.normalizeRemainingCountdownMs,
            });
        }
        function emitPlaybackState(state) {
            params.playbackSessionUtils.emitPlaybackState({
                activePlayback: params.state.activePlayback,
                state,
                getRemainingCountdownMs,
                reportPlaybackState: params.reportPlaybackState,
                logDebug: params.logDebug,
            });
        }
        function getDerivedPlaybackState() {
            return params.playbackSessionUtils.getDerivedPlaybackState({
                activePlayback: params.state.activePlayback,
                activePlayableElement: params.state.activePlayableElement,
                countdownPaused: params.state.countdownState.paused,
                derivePlaybackState: (nextParams) => params.playbackUtils.derivePlaybackState(nextParams),
            });
        }
        function setPlaybackState(state) {
            params.playbackSessionUtils.setPlaybackState({
                state,
                getActivePlayback: () => params.state.activePlayback,
                emitPlaybackState,
            });
        }
        function startPlaybackSession(jobId) {
            params.playbackSessionUtils.startPlaybackSession({
                jobId,
                normalizeJobId: params.playbackUtils.normalizeJobId,
                createActivePlayback: params.playbackUtils.createActivePlayback,
                setActivePlayback: (nextPlayback) => {
                    params.state.activePlayback = nextPlayback;
                },
                getActivePlayback: () => params.state.activePlayback,
                clearPlaybackSyncTimer,
                setPlaybackSyncTimer: (nextTimer) => {
                    params.state.playbackSyncTimer = nextTimer;
                },
                setInterval: params.setInterval,
                getDerivedPlaybackState,
                emitPlaybackState,
            });
        }
        function endPlaybackSession(reason) {
            params.playbackSessionUtils.endPlaybackSession({
                reason,
                getActivePlayback: () => params.state.activePlayback,
                setActivePlayback: (nextPlayback) => {
                    params.state.activePlayback = nextPlayback;
                },
                emitPlaybackState,
                reportPlaybackStop: params.reportPlaybackStop,
                clearPlaybackSyncTimer,
                logDebug: params.logDebug,
            });
        }
        function renderCountdown() {
            params.countdownUtils.renderCountdown({
                countdownLayer: params.nodes.countdownLayer,
                countdownState: params.state.countdownState,
                normalizeRemainingCountdownMs: params.playbackUtils.normalizeRemainingCountdownMs,
                formatRemainingTime: params.utils.formatRemainingTime,
                scheduleMediaHeaderLayout: params.scheduleMediaHeaderLayout,
            });
        }
        function updateCountdown() {
            params.countdownUtils.updateCountdown({
                countdownState: params.state.countdownState,
                tickCountdown: params.playbackUtils.tickCountdown,
                renderCountdown,
                clearCountdownTimer,
                isActiveMediaStillRunning: params.playbackUtils.isActiveMediaStillRunning,
                activePlayableElement: params.state.activePlayableElement,
                shouldClearOverlayAfterCountdown: params.playbackUtils.shouldClearOverlayAfterCountdown,
                clearOverlay,
                nowMs: params.nowMs,
            });
        }
        function startCountdown(durationSec, options = {}) {
            params.countdownUtils.startCountdown({
                durationSec,
                countdownLayer: params.nodes.countdownLayer,
                options,
                countdownState: params.state.countdownState,
                clearCountdownTimer,
                renderCountdown,
                setInterval: params.setInterval,
                updateCountdown,
                nowMs: params.nowMs,
            });
        }
        function pauseCountdown() {
            params.countdownUtils.pauseCountdown({
                countdownState: params.state.countdownState,
                updateCountdown,
            });
        }
        function resumeCountdown() {
            params.countdownUtils.resumeCountdown({
                countdownState: params.state.countdownState,
                nowMs: params.nowMs,
            });
        }
        // Lets the countdown clear the overlay when it reaches zero, whatever
        // the media is doing (a video extended past its own end).
        function setCountdownAutoClear(autoClear) {
            params.state.countdownState.autoClear = autoClear === true;
        }
        function clearCountdown() {
            params.countdownUtils.clearCountdown({
                countdownState: params.state.countdownState,
                clearCountdownTimer,
                countdownLayer: params.nodes.countdownLayer,
                scheduleMediaHeaderLayout: params.scheduleMediaHeaderLayout,
            });
        }
        // `options.reason` is echoed in the playback-stop report (see
        // `stopReportReason`); callers ending a playback normally pass none.
        function clearOverlay(options) {
            endPlaybackSession(options && typeof options.reason === 'string' ? options.reason : null);
            clearTimer();
            clearCountdown();
            params.clearMediaHeaderLayoutRaf();
            // A stop removes the media from the view; pausing and dropping the
            // source first keeps the audio (and the Windows media session) from
            // outliving the overlay. The flag tells the renderer's `pause`
            // listener that this is not a user pause to share with the guild.
            params.state.releasingMedia = true;
            try {
                if (typeof params.overlayResetUtils.releaseMediaElement === 'function') {
                    params.overlayResetUtils.releaseMediaElement(params.state.activePlayableElement);
                }
                if (typeof params.overlayResetUtils.releaseMediaElements === 'function') {
                    params.overlayResetUtils.releaseMediaElements(params.nodes.mediaLayer);
                }
            }
            finally {
                params.state.releasingMedia = false;
            }
            params.state.activePlayableElement = null;
            params.state.activeMediaRelayout = null;
            params.state.activeObjectUrl = params.overlayResetUtils.releaseObjectUrl(params.state.activeObjectUrl);
            params.resetMediaFrameState();
            params.overlayResetUtils.resetOverlayLayers({
                mediaLayer: params.nodes.mediaLayer,
                textLayer: params.nodes.textLayer,
                overlayRoot: params.nodes.overlayRoot,
                countdownLayer: params.nodes.countdownLayer,
            });
        }
        return {
            startPlaybackSession,
            startCountdown,
            clearCountdown,
            pauseCountdown,
            resumeCountdown,
            setPlaybackState,
            getRemainingCountdownMs,
            setCountdownAutoClear,
            clearOverlay,
        };
    }
    return {
        createRuntimePlaybackCountdownHandlers,
    };
}
