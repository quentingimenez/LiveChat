function resolveMediaFlags(activePlayableElement) {
    if (!(activePlayableElement instanceof HTMLMediaElement)) {
        return {
            paused: null,
            ended: null,
        };
    }
    return {
        paused: typeof activePlayableElement.paused === 'boolean' ? activePlayableElement.paused : null,
        ended: typeof activePlayableElement.ended === 'boolean' ? activePlayableElement.ended : null,
    };
}
export function createOverlayLegacyPlaybackSessionUtils() {
    function emitPlaybackState(params) {
        if (!params.activePlayback || typeof params.reportPlaybackState !== 'function') {
            return;
        }
        const payload = {
            jobId: params.activePlayback.jobId,
            state: params.state,
            remainingMs: params.getRemainingCountdownMs(),
        };
        params.reportPlaybackState(payload);
        params.logDebug(`[OVERLAY] playback-state sent (jobId: ${payload.jobId}, state: ${payload.state}, remainingMs: ${payload.remainingMs === null ? 'null' : payload.remainingMs})`);
    }
    function getDerivedPlaybackState(params) {
        if (!params.activePlayback) {
            return null;
        }
        const mediaFlags = resolveMediaFlags(params.activePlayableElement);
        return params.derivePlaybackState({
            activePlaybackState: params.activePlayback.state,
            countdownPaused: params.countdownPaused,
            playablePaused: mediaFlags.paused,
            playableEnded: mediaFlags.ended,
        });
    }
    function setPlaybackState(params) {
        const activePlayback = params.getActivePlayback();
        if (!activePlayback) {
            return;
        }
        if (activePlayback.state !== params.state) {
            activePlayback.state = params.state;
        }
        params.emitPlaybackState(activePlayback.state);
    }
    function startPlaybackSession(params) {
        const normalizedJobId = params.normalizeJobId(params.jobId);
        if (!normalizedJobId) {
            params.setActivePlayback(null);
            params.clearPlaybackSyncTimer();
            return;
        }
        params.setActivePlayback(params.createActivePlayback(normalizedJobId));
        params.emitPlaybackState('playing');
        params.clearPlaybackSyncTimer();
        const playbackSyncTimer = params.setInterval(() => {
            const activePlayback = params.getActivePlayback();
            if (!activePlayback) {
                return;
            }
            const derivedState = params.getDerivedPlaybackState();
            if (derivedState && activePlayback.state !== derivedState) {
                activePlayback.state = derivedState;
            }
            params.emitPlaybackState(activePlayback.state);
        }, 1000);
        params.setPlaybackSyncTimer(playbackSyncTimer);
    }
    function endPlaybackSession(params) {
        const activePlayback = params.getActivePlayback();
        if (!activePlayback) {
            return;
        }
        const endedJobId = activePlayback.jobId;
        params.emitPlaybackState('ended');
        if (typeof params.reportPlaybackStop === 'function') {
            params.reportPlaybackStop({
                jobId: endedJobId,
            });
            params.logDebug(`[OVERLAY] playback-stop sent (jobId: ${endedJobId})`);
        }
        params.setActivePlayback(null);
        params.clearPlaybackSyncTimer();
    }
    return {
        emitPlaybackState,
        getDerivedPlaybackState,
        setPlaybackState,
        startPlaybackSession,
        endPlaybackSession,
    };
}
