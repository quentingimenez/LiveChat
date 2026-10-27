type OverlayPlaybackState = 'playing' | 'paused' | 'ended';

interface ActivePlaybackLike {
  jobId: string;
  state: OverlayPlaybackState;
}

interface DerivePlaybackStateParams {
  activePlaybackState: OverlayPlaybackState;
  countdownPaused: boolean;
  playablePaused: boolean | null;
  playableEnded: boolean | null;
}

interface TickCountdownParams {
  countdownRemainingMs: number;
  countdownLastTickAtMs: number | null;
  countdownPaused: boolean;
  nowMs: number;
}

interface TickCountdownResult {
  countdownRemainingMs: number;
  countdownLastTickAtMs: number;
}

interface ShouldClearOverlayAfterCountdownParams {
  countdownAutoClear: boolean;
  isActiveMediaStillRunning: boolean;
}

export interface OverlayLegacyPlaybackUtils {
  normalizeJobId(jobId: unknown): string | null;
  createActivePlayback(jobId: string): ActivePlaybackLike;
  normalizeRemainingCountdownMs(value: unknown): number | null;
  derivePlaybackState(params: DerivePlaybackStateParams): OverlayPlaybackState;
  tickCountdown(params: TickCountdownParams): TickCountdownResult;
  isActiveMediaStillRunning(playableElement: unknown): boolean;
  shouldClearOverlayAfterCountdown(params: ShouldClearOverlayAfterCountdownParams): boolean;
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

export function createOverlayLegacyPlaybackUtils(): OverlayLegacyPlaybackUtils {
  function normalizeJobId(jobId: unknown): string | null {
    if (typeof jobId !== 'string') {
      return null;
    }

    const normalized = jobId.trim();
    return normalized ? normalized : null;
  }

  function createActivePlayback(jobId: string): ActivePlaybackLike {
    return {
      jobId,
      state: 'playing'
    };
  }

  function normalizeRemainingCountdownMs(value: unknown): number | null {
    if (!isFiniteNumber(value)) {
      return null;
    }

    return Math.max(0, Math.round(value));
  }

  function derivePlaybackState(params: DerivePlaybackStateParams): OverlayPlaybackState {
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

  function tickCountdown(params: TickCountdownParams): TickCountdownResult {
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

  function isActiveMediaStillRunning(playableElement: unknown): boolean {
    if (!(playableElement instanceof HTMLMediaElement)) {
      return false;
    }

    return !playableElement.ended;
  }

  function shouldClearOverlayAfterCountdown(params: ShouldClearOverlayAfterCountdownParams): boolean {
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
