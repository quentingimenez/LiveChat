interface ActivePlaybackLike {
  jobId: string;
  state: string;
}

interface EmitPlaybackStateParams {
  activePlayback: ActivePlaybackLike | null;
  state: string;
  getRemainingCountdownMs: () => number | null;
  reportPlaybackState: ((payload: { jobId: string; state: string; remainingMs: number | null }) => void) | null;
  logDebug: (message: string) => void;
}

interface DerivePlaybackStateParams {
  activePlayback: ActivePlaybackLike | null;
  activePlayableElement: unknown;
  countdownPaused: boolean;
  derivePlaybackState: (params: {
    activePlaybackState: string;
    countdownPaused: boolean;
    playablePaused: boolean | null;
    playableEnded: boolean | null;
  }) => string;
}

interface SetPlaybackStateParams {
  state: string;
  getActivePlayback: () => ActivePlaybackLike | null;
  emitPlaybackState: (state: string) => void;
}

interface StartPlaybackSessionParams {
  jobId: unknown;
  normalizeJobId: (jobId: unknown) => string | null;
  createActivePlayback: (jobId: string) => ActivePlaybackLike;
  setActivePlayback: (nextPlayback: ActivePlaybackLike | null) => void;
  getActivePlayback: () => ActivePlaybackLike | null;
  clearPlaybackSyncTimer: () => void;
  setPlaybackSyncTimer: (nextTimer: number | null) => void;
  setInterval: (callback: () => void, delayMs: number) => number;
  getDerivedPlaybackState: () => string | null;
  emitPlaybackState: (state: string) => void;
}

interface EndPlaybackSessionParams {
  getActivePlayback: () => ActivePlaybackLike | null;
  setActivePlayback: (nextPlayback: ActivePlaybackLike | null) => void;
  emitPlaybackState: (state: string) => void;
  reportPlaybackStop: ((payload: { jobId: string }) => void) | null;
  clearPlaybackSyncTimer: () => void;
  logDebug: (message: string) => void;
}

export interface OverlayLegacyPlaybackSessionUtils {
  emitPlaybackState(params: EmitPlaybackStateParams): void;
  getDerivedPlaybackState(params: DerivePlaybackStateParams): string | null;
  setPlaybackState(params: SetPlaybackStateParams): void;
  startPlaybackSession(params: StartPlaybackSessionParams): void;
  endPlaybackSession(params: EndPlaybackSessionParams): void;
}

function resolveMediaFlags(activePlayableElement: unknown): { paused: boolean | null; ended: boolean | null } {
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

export function createOverlayLegacyPlaybackSessionUtils(): OverlayLegacyPlaybackSessionUtils {
  function emitPlaybackState(params: EmitPlaybackStateParams): void {
    if (!params.activePlayback || typeof params.reportPlaybackState !== 'function') {
      return;
    }

    const payload = {
      jobId: params.activePlayback.jobId,
      state: params.state,
      remainingMs: params.getRemainingCountdownMs(),
    };

    params.reportPlaybackState(payload);
    params.logDebug(
      `[OVERLAY] playback-state sent (jobId: ${payload.jobId}, state: ${payload.state}, remainingMs: ${
        payload.remainingMs === null ? 'null' : payload.remainingMs
      })`,
    );
  }

  function getDerivedPlaybackState(params: DerivePlaybackStateParams): string | null {
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

  function setPlaybackState(params: SetPlaybackStateParams): void {
    const activePlayback = params.getActivePlayback();
    if (!activePlayback) {
      return;
    }

    if (activePlayback.state !== params.state) {
      activePlayback.state = params.state;
    }

    params.emitPlaybackState(activePlayback.state);
  }

  function startPlaybackSession(params: StartPlaybackSessionParams): void {
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

  function endPlaybackSession(params: EndPlaybackSessionParams): void {
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
