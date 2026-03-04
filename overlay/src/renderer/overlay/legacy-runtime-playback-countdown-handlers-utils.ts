import type { OverlayLegacyCountdownUtils } from './legacy-countdown-utils.js';
import type { OverlayLegacyOverlayResetUtils } from './legacy-overlay-reset-utils.js';
import type { OverlayLegacyPlaybackSessionUtils } from './legacy-playback-session-utils.js';
import type { OverlayLegacyPlaybackUtils } from './legacy-playback-utils.js';
import type { OverlayLegacyUtils } from './legacy-utils.js';
import type { OverlayRuntimeNodes, OverlayRuntimeState } from './legacy-runtime-types.js';

interface CreateRuntimePlaybackCountdownHandlersParams {
  nodes: OverlayRuntimeNodes;
  state: OverlayRuntimeState;
  utils: OverlayLegacyUtils;
  countdownUtils: OverlayLegacyCountdownUtils;
  playbackUtils: OverlayLegacyPlaybackUtils;
  playbackSessionUtils: OverlayLegacyPlaybackSessionUtils;
  overlayResetUtils: OverlayLegacyOverlayResetUtils;
  clearMediaHeaderLayoutRaf: () => void;
  resetMediaFrameState: () => void;
  scheduleMediaHeaderLayout: () => void;
  reportPlaybackState: ((payload: { jobId: string; state: string; remainingMs: number | null }) => void) | null;
  reportPlaybackStop: ((payload: { jobId: string }) => void) | null;
  setTimeout: (callback: () => void, delayMs: number) => number;
  clearTimeout: (timeoutId: number) => void;
  setInterval: (callback: () => void, delayMs: number) => number;
  clearInterval: (intervalId: number) => void;
  nowMs: () => number;
  logDebug: (message: string) => void;
}

interface RuntimePlaybackCountdownHandlers {
  startPlaybackSession: (jobId: unknown) => void;
  startCountdown: (durationSec: unknown, options?: { autoClear?: boolean }) => void;
  clearCountdown: () => void;
  pauseCountdown: () => void;
  resumeCountdown: () => void;
  setPlaybackState: (state: string) => void;
  clearOverlay: () => void;
}

export interface OverlayLegacyRuntimePlaybackCountdownHandlersUtils {
  createRuntimePlaybackCountdownHandlers(
    params: CreateRuntimePlaybackCountdownHandlersParams,
  ): RuntimePlaybackCountdownHandlers;
}

export function createOverlayLegacyRuntimePlaybackCountdownHandlersUtils(): OverlayLegacyRuntimePlaybackCountdownHandlersUtils {
  function createRuntimePlaybackCountdownHandlers(
    params: CreateRuntimePlaybackCountdownHandlersParams,
  ): RuntimePlaybackCountdownHandlers {
    function clearTimer(): void {
      if (params.state.resetTimer === null) {
        return;
      }

      params.clearTimeout(params.state.resetTimer);
      params.state.resetTimer = null;
    }

    function clearCountdownTimer(): void {
      if (params.state.countdownState.timer === null) {
        return;
      }

      params.clearInterval(params.state.countdownState.timer);
      params.state.countdownState.timer = null;
    }

    function clearPlaybackSyncTimer(): void {
      if (params.state.playbackSyncTimer === null) {
        return;
      }

      params.clearInterval(params.state.playbackSyncTimer);
      params.state.playbackSyncTimer = null;
    }

    function getRemainingCountdownMs(): number | null {
      return params.countdownUtils.getRemainingCountdownMs({
        countdownState: params.state.countdownState,
        normalizeRemainingCountdownMs: params.playbackUtils.normalizeRemainingCountdownMs,
      });
    }

    function emitPlaybackState(state: string): void {
      params.playbackSessionUtils.emitPlaybackState({
        activePlayback: params.state.activePlayback,
        state,
        getRemainingCountdownMs,
        reportPlaybackState: params.reportPlaybackState,
        logDebug: params.logDebug,
      });
    }

    function getDerivedPlaybackState(): string | null {
      return params.playbackSessionUtils.getDerivedPlaybackState({
        activePlayback: params.state.activePlayback,
        activePlayableElement: params.state.activePlayableElement,
        countdownPaused: params.state.countdownState.paused,
        derivePlaybackState: (nextParams) =>
          params.playbackUtils.derivePlaybackState(
            nextParams as Parameters<(typeof params.playbackUtils)['derivePlaybackState']>[0],
          ),
      });
    }

    function setPlaybackState(state: string): void {
      params.playbackSessionUtils.setPlaybackState({
        state,
        getActivePlayback: () => params.state.activePlayback,
        emitPlaybackState,
      });
    }

    function startPlaybackSession(jobId: unknown): void {
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

    function endPlaybackSession(): void {
      params.playbackSessionUtils.endPlaybackSession({
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

    function renderCountdown(): void {
      params.countdownUtils.renderCountdown({
        countdownLayer: params.nodes.countdownLayer,
        countdownState: params.state.countdownState,
        normalizeRemainingCountdownMs: params.playbackUtils.normalizeRemainingCountdownMs,
        formatRemainingTime: params.utils.formatRemainingTime,
        scheduleMediaHeaderLayout: params.scheduleMediaHeaderLayout,
      });
    }

    function updateCountdown(): void {
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

    function startCountdown(durationSec: unknown, options: { autoClear?: boolean } = {}): void {
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

    function pauseCountdown(): void {
      params.countdownUtils.pauseCountdown({
        countdownState: params.state.countdownState,
        updateCountdown,
      });
    }

    function resumeCountdown(): void {
      params.countdownUtils.resumeCountdown({
        countdownState: params.state.countdownState,
        nowMs: params.nowMs,
      });
    }

    function clearCountdown(): void {
      params.countdownUtils.clearCountdown({
        countdownState: params.state.countdownState,
        clearCountdownTimer,
        countdownLayer: params.nodes.countdownLayer,
        scheduleMediaHeaderLayout: params.scheduleMediaHeaderLayout,
      });
    }

    function clearOverlay(): void {
      endPlaybackSession();
      clearTimer();
      clearCountdown();
      params.clearMediaHeaderLayoutRaf();
      params.state.activePlayableElement = null;
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
      clearOverlay,
    };
  }

  return {
    createRuntimePlaybackCountdownHandlers,
  };
}
