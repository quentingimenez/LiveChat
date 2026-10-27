import type { OverlayLegacyBootstrapUtils } from './legacy-bootstrap-utils.js';
import type { OverlayLegacyInlineVideoUtils } from './legacy-inline-video-utils.js';
import type { OverlayLegacyMediaDiagnosticsUtils } from './legacy-media-diagnostics.js';
import type { OverlayLegacyMediaDomUtils } from './legacy-media-dom-utils.js';
import type { OverlayLegacyMediaOffsetUtils } from './legacy-media-offset-utils.js';
import type { OverlayLegacyMediaRenderUtils } from './legacy-media-render-utils.js';
import type { OverlayLegacyTextUtils } from './legacy-overlay-text-utils.js';
import type { OverlayLegacyPlayFlowUtils } from './legacy-play-flow-utils.js';
import type { OverlayLegacyTweetCardPlaybackUtils } from './legacy-tweet-card-playback-utils.js';
import type { OverlayLegacyTweetCardRendererUtils } from './legacy-tweet-card-renderer.js';
import type { OverlayLegacyTweetCardUtils } from './legacy-tweet-card-utils.js';
import type { OverlayLegacyTwitterWidgetsUtils } from './legacy-twitter-widgets-utils.js';
import type { OverlayLegacyUtils } from './legacy-utils.js';
import type { OverlayRuntimeConstants, OverlayRuntimeNodes, OverlayRuntimeState } from './legacy-runtime-types.js';

interface CreateRuntimeRenderHandlersParams {
  nodes: OverlayRuntimeNodes;
  state: OverlayRuntimeState;
  constants: OverlayRuntimeConstants;
  textUtils: OverlayLegacyTextUtils;
  mediaDomUtils: OverlayLegacyMediaDomUtils;
  mediaRenderUtils: OverlayLegacyMediaRenderUtils;
  inlineVideoUtils: OverlayLegacyInlineVideoUtils;
  utils: OverlayLegacyUtils;
  tweetCardPlaybackUtils: OverlayLegacyTweetCardPlaybackUtils;
  tweetCardUtils: OverlayLegacyTweetCardUtils;
  tweetCardRendererUtils: OverlayLegacyTweetCardRendererUtils;
  twitterWidgetsUtils: OverlayLegacyTwitterWidgetsUtils;
  diagnosticsUtils: OverlayLegacyMediaDiagnosticsUtils;
  mediaOffsetUtils: OverlayLegacyMediaOffsetUtils;
  bootstrapUtils: OverlayLegacyBootstrapUtils;
  playFlowUtils: OverlayLegacyPlayFlowUtils;
  ensureMediaFrame: () => HTMLElement | null;
  scheduleMediaHeaderLayout: () => void;
  clearOverlay: () => void;
  startPlaybackSession: (jobId: unknown) => void;
  startCountdown: (durationSec: unknown, options?: { autoClear?: boolean }) => void;
  clearCountdown: () => void;
  resumeCountdown: () => void;
  pauseCountdown: () => void;
  setPlaybackState: (state: string) => void;
  setTimeout: (callback: () => void, delayMs: number) => number;
  reportError: ((payload: { jobId: string; code: string; message: string }) => void) | null;
  logWarning: (message: string, error: unknown) => void;
  logError: (message: string, error: unknown) => void;
}

interface RuntimeRenderHandlers {
  onPlay: (payload: unknown) => Promise<void>;
  onSettings: (settings: unknown) => void;
  ensureTwitterWidgets: () => Promise<unknown>;
}

export interface OverlayLegacyRuntimeRenderHandlersUtils {
  createRuntimeRenderHandlers(params: CreateRuntimeRenderHandlersParams): RuntimeRenderHandlers;
}

export function createOverlayLegacyRuntimeRenderHandlersUtils(): OverlayLegacyRuntimeRenderHandlersUtils {
  function createRuntimeRenderHandlers(params: CreateRuntimeRenderHandlersParams): RuntimeRenderHandlers {
    function isTextVisible(): boolean {
      return params.state.overlayConfig.showText !== false;
    }

    function applyMediaHeaderAuthor(payload: unknown, options: { enabled: boolean } = { enabled: false }): void {
      params.textUtils.applyMediaHeaderAuthor({
        container: params.state.mediaFrameState.activeMediaHeaderLeft,
        payload,
        showText: isTextVisible(),
        enabled: options.enabled === true,
        onLayout: params.scheduleMediaHeaderLayout,
      });
    }

    function applyMediaFooterText(payload: unknown, options: { enabled: boolean } = { enabled: false }): void {
      params.textUtils.applyMediaFooterText({
        container: params.state.mediaFrameState.activeMediaFooter,
        payload,
        showText: isTextVisible(),
        enabled: options.enabled === true,
      });
    }

    function applyOverlayInfo(
      payload: unknown,
      options: { showAuthorInline: boolean; attachTextToMedia: boolean } = {
        showAuthorInline: false,
        attachTextToMedia: false,
      },
    ): void {
      params.textUtils.applyOverlayInfo({
        layer: params.nodes.textLayer,
        payload,
        showText: isTextVisible(),
        showAuthorInline: options.showAuthorInline === true,
        attachTextToMedia: options.attachTextToMedia === true,
      });
    }

    function applyVolume(): void {
      params.mediaDomUtils.applyMediaVolume({
        root: params.nodes.mediaLayer,
        perceptualGain: params.utils.toPerceptualGain(params.state.overlayConfig.volume, params.constants.volumeCurveGamma),
      });
    }

    function ensureTwitterWidgets(): Promise<unknown> {
      return params.twitterWidgetsUtils.ensureTwitterWidgets();
    }

    function renderTweetCard(payload: unknown): boolean {
      return params.tweetCardPlaybackUtils.renderTweetCard({
        payload,
        createTweetCardModel: params.tweetCardUtils.createTweetCardModel,
        createTweetCardDom: (nextParams) =>
          params.tweetCardRendererUtils.createTweetCardDom(
            nextParams as Parameters<(typeof params.tweetCardRendererUtils)['createTweetCardDom']>[0]
          ),
        ensureMediaFrame: params.ensureMediaFrame,
        mediaLayer: params.nodes.mediaLayer,
        scheduleMediaHeaderLayout: params.scheduleMediaHeaderLayout,
        ensureTwitterWidgets: (() => ensureTwitterWidgets()) as Parameters<
          (typeof params.tweetCardPlaybackUtils)['renderTweetCard']
        >[0]['ensureTwitterWidgets'],
        configureInlineVideoLooping: params.inlineVideoUtils.configureInlineVideoLooping,
        playVideoWithRetry: async (video) => {
          params.inlineVideoUtils.playVideoWithRetry(video, { attempts: 4, delayMs: 250 });
        },
        applyVolume,
        ensureInlineVideoAudioFallback: async (videos) => {
          params.inlineVideoUtils.ensureInlineVideoAudioFallback(videos, applyVolume);
        },
        setTimer: params.setTimeout,
        logWarning: params.logWarning,
      });
    }

    async function renderMedia(payload: unknown): Promise<void> {
      await params.mediaRenderUtils.renderMedia({
        payload: payload as unknown as Parameters<(typeof params.mediaRenderUtils)['renderMedia']>[0]['payload'],
        setActiveObjectUrl: (nextObjectUrl) => {
          params.state.activeObjectUrl = nextObjectUrl;
        },
        buildAuthorizedMediaUrl: (url) =>
          params.mediaDomUtils.buildAuthorizedMediaUrl(url, params.state.overlayConfig.clientToken),
        createMediaElement: params.mediaDomUtils.createMediaElement,
        attachMediaDiagnostics: (element, diagnosticsParams) =>
          params.diagnosticsUtils.attachMediaDiagnostics(
            element,
            diagnosticsParams as Parameters<(typeof params.diagnosticsUtils)['attachMediaDiagnostics']>[1]
          ),
        ensureMediaFrame: params.ensureMediaFrame,
        mediaLayer: params.nodes.mediaLayer,
        attachMediaElement: params.mediaDomUtils.attachMediaElement,
        scheduleMediaHeaderLayout: params.scheduleMediaHeaderLayout,
        applyVolume,
        setActivePlayableElement: (element) => {
          params.state.activePlayableElement = element;
        },
        applyMediaStartOffset: params.mediaOffsetUtils.applyMediaStartOffset,
        resolveStartOffsetSec: params.utils.resolveStartOffsetSec,
        resumeCountdown: params.resumeCountdown,
        pauseCountdown: params.pauseCountdown,
        setPlaybackState: params.setPlaybackState,
        clearOverlay: params.clearOverlay,
        logAutoplayError: params.logError,
      });
    }

    async function onPlay(payload: unknown): Promise<void> {
      await params.playFlowUtils.runPlayFlow({
        payload: payload as unknown as Parameters<(typeof params.playFlowUtils)['runPlayFlow']>[0]['payload'],
        clearOverlay: params.clearOverlay,
        startPlaybackSession: params.startPlaybackSession,
        applyOverlayInfo,
        renderTweetCard,
        renderMedia,
        applyMediaHeaderAuthor,
        applyMediaFooterText,
        logRenderError: params.logError,
        reportError: (errorPayload) => {
          if (typeof params.reportError === 'function') {
            params.reportError(errorPayload);
          }
        },
        startCountdown: params.startCountdown,
        setResetTimer: (nextResetTimer) => {
          params.state.resetTimer = nextResetTimer;
        },
        setTimeout: params.setTimeout,
        clearCountdown: params.clearCountdown,
      });
    }

    function onSettings(nextSettings: unknown): void {
      params.state.overlayConfig = params.bootstrapUtils.applyOverlaySettings({
        currentConfig: params.state.overlayConfig,
        nextSettings,
        onHideText: () => {
          if (params.nodes.textLayer instanceof HTMLElement) {
            params.nodes.textLayer.innerHTML = '';
            params.nodes.textLayer.style.display = 'none';
          }
          applyMediaHeaderAuthor(null, {
            enabled: false,
          });
          applyMediaFooterText(null, {
            enabled: false,
          });
        },
        applyVolume,
      });
    }

    return {
      onPlay,
      onSettings,
      ensureTwitterWidgets,
    };
  }

  return {
    createRuntimeRenderHandlers,
  };
}
