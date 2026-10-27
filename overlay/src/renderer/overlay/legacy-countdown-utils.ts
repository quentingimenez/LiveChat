interface OverlayCountdownState {
  timer: number | null;
  remainingMs: number | null;
  lastTickAtMs: number | null;
  paused: boolean;
  autoClear: boolean;
}

interface GetRemainingCountdownMsParams {
  countdownState: OverlayCountdownState;
  normalizeRemainingCountdownMs: (value: unknown) => number | null;
}

interface RenderCountdownParams {
  countdownLayer: unknown;
  countdownState: OverlayCountdownState;
  normalizeRemainingCountdownMs: (value: unknown) => number | null;
  formatRemainingTime: (remainingMs: number) => string;
  scheduleMediaHeaderLayout: () => void;
}

interface UpdateCountdownParams {
  countdownState: OverlayCountdownState;
  tickCountdown: (params: {
    countdownRemainingMs: number;
    countdownLastTickAtMs: number | null;
    countdownPaused: boolean;
    nowMs: number;
  }) => {
    countdownRemainingMs: number;
    countdownLastTickAtMs: number;
  };
  renderCountdown: () => void;
  clearCountdownTimer: () => void;
  isActiveMediaStillRunning: (playableElement: unknown) => boolean;
  activePlayableElement: unknown;
  shouldClearOverlayAfterCountdown: (params: {
    countdownAutoClear: boolean;
    isActiveMediaStillRunning: boolean;
  }) => boolean;
  clearOverlay: () => void;
  nowMs: () => number;
}

interface StartCountdownParams {
  durationSec: unknown;
  countdownLayer: unknown;
  options: {
    autoClear?: boolean;
  };
  countdownState: OverlayCountdownState;
  clearCountdownTimer: () => void;
  renderCountdown: () => void;
  setInterval: (callback: () => void, delayMs: number) => number;
  updateCountdown: () => void;
  nowMs: () => number;
}

interface PauseCountdownParams {
  countdownState: OverlayCountdownState;
  updateCountdown: () => void;
}

interface ResumeCountdownParams {
  countdownState: OverlayCountdownState;
  nowMs: () => number;
}

interface ClearCountdownParams {
  countdownState: OverlayCountdownState;
  clearCountdownTimer: () => void;
  countdownLayer: unknown;
  scheduleMediaHeaderLayout: () => void;
}

export interface OverlayLegacyCountdownUtils {
  getRemainingCountdownMs(params: GetRemainingCountdownMsParams): number | null;
  renderCountdown(params: RenderCountdownParams): void;
  updateCountdown(params: UpdateCountdownParams): void;
  startCountdown(params: StartCountdownParams): void;
  pauseCountdown(params: PauseCountdownParams): void;
  resumeCountdown(params: ResumeCountdownParams): void;
  clearCountdown(params: ClearCountdownParams): void;
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

export function createOverlayLegacyCountdownUtils(): OverlayLegacyCountdownUtils {
  function getRemainingCountdownMs(params: GetRemainingCountdownMsParams): number | null {
    return params.normalizeRemainingCountdownMs(params.countdownState.remainingMs);
  }

  function renderCountdown(params: RenderCountdownParams): void {
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

  function updateCountdown(params: UpdateCountdownParams): void {
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
      if (
        params.shouldClearOverlayAfterCountdown({
          countdownAutoClear: params.countdownState.autoClear,
          isActiveMediaStillRunning,
        })
      ) {
        params.clearOverlay();
      }
    }
  }

  function startCountdown(params: StartCountdownParams): void {
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

  function pauseCountdown(params: PauseCountdownParams): void {
    if (!isFiniteNumber(params.countdownState.remainingMs) || params.countdownState.paused) {
      return;
    }

    params.updateCountdown();
    params.countdownState.paused = true;
  }

  function resumeCountdown(params: ResumeCountdownParams): void {
    if (!isFiniteNumber(params.countdownState.remainingMs) || !params.countdownState.paused) {
      return;
    }

    params.countdownState.paused = false;
    params.countdownState.lastTickAtMs = params.nowMs();
  }

  function clearCountdown(params: ClearCountdownParams): void {
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
