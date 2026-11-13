(() => {
  const overlayLegacyUtils = window.__overlayLegacyUtils;
  const overlayLegacyMediaDomUtils = window.__overlayLegacyMediaDomUtils;
  const overlayLegacyMediaFrameUtils = window.__overlayLegacyMediaFrameUtils;
  const overlayLegacyMediaFrameLayoutUtils = window.__overlayLegacyMediaFrameLayoutUtils;
  const overlayLegacyMediaRenderUtils = window.__overlayLegacyMediaRenderUtils;
  const overlayLegacyPlayFlowUtils = window.__overlayLegacyPlayFlowUtils;
  const overlayLegacyCountdownUtils = window.__overlayLegacyCountdownUtils;
  const overlayLegacyPlaybackUtils = window.__overlayLegacyPlaybackUtils;
  const overlayLegacyPlaybackSessionUtils = window.__overlayLegacyPlaybackSessionUtils;
  const overlayLegacyBootstrapUtils = window.__overlayLegacyBootstrapUtils;
  const overlayLegacyOverlayResetUtils = window.__overlayLegacyOverlayResetUtils;
  const overlayLegacyDiagnosticsUtils = window.__overlayLegacyDiagnosticsUtils;
  const overlayLegacyMediaOffsetUtils = window.__overlayLegacyMediaOffsetUtils;
  const overlayLegacyInlineVideoUtils = window.__overlayLegacyInlineVideoUtils;
  const overlayLegacyTextUtils = window.__overlayLegacyTextUtils;
  const overlayLegacyTwitterWidgetsUtils = window.__overlayLegacyTwitterWidgetsUtils;
  const overlayLegacyTweetCardPlaybackUtils = window.__overlayLegacyTweetCardPlaybackUtils;
  const overlayLegacyTweetCardUtils = window.__overlayLegacyTweetCardUtils;
  const overlayLegacyTweetCardRendererUtils = window.__overlayLegacyTweetCardRendererUtils;
  const overlayLegacyRuntimeInputsUtils = window.__overlayLegacyRuntimeInputsUtils;
  const overlayLegacyRuntimeMediaFrameHandlersUtils = window.__overlayLegacyRuntimeMediaFrameHandlersUtils;
  const overlayLegacyRuntimePlaybackCountdownHandlersUtils = window.__overlayLegacyRuntimePlaybackCountdownHandlersUtils;
  const overlayLegacyRuntimeRenderHandlersUtils = window.__overlayLegacyRuntimeRenderHandlersUtils;

  const runtimeInputs = overlayLegacyRuntimeInputsUtils.createRuntimeInputs(document);
  const { nodes, state, constants } = runtimeInputs;

  const mediaFrameHandlers = overlayLegacyRuntimeMediaFrameHandlersUtils.createRuntimeMediaFrameHandlers({
    nodes,
    state,
    mediaFrameUtils: overlayLegacyMediaFrameUtils,
    mediaFrameLayoutUtils: overlayLegacyMediaFrameLayoutUtils,
    setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
    clearTimeout: (timeoutId) => window.clearTimeout(timeoutId),
    requestAnimationFrame:
      typeof window.requestAnimationFrame === 'function'
        ? (callback) => window.requestAnimationFrame(() => callback())
        : null,
    cancelAnimationFrame:
      typeof window.cancelAnimationFrame === 'function' ? (id) => window.cancelAnimationFrame(id) : null,
  });

  const playbackCountdownHandlers =
    overlayLegacyRuntimePlaybackCountdownHandlersUtils.createRuntimePlaybackCountdownHandlers({
      nodes,
      state,
      utils: overlayLegacyUtils,
      countdownUtils: overlayLegacyCountdownUtils,
      playbackUtils: overlayLegacyPlaybackUtils,
      playbackSessionUtils: overlayLegacyPlaybackSessionUtils,
      overlayResetUtils: overlayLegacyOverlayResetUtils,
      clearMediaHeaderLayoutRaf: mediaFrameHandlers.clearMediaHeaderLayoutRaf,
      resetMediaFrameState: mediaFrameHandlers.resetMediaFrameState,
      scheduleMediaHeaderLayout: mediaFrameHandlers.scheduleMediaHeaderLayout,
      reportPlaybackState: window.livechatOverlay?.reportPlaybackState || null,
      reportPlaybackStop: window.livechatOverlay?.reportPlaybackStop || null,
      setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
      clearTimeout: (timeoutId) => window.clearTimeout(timeoutId),
      setInterval: (callback, delayMs) => window.setInterval(callback, delayMs),
      clearInterval: (intervalId) => window.clearInterval(intervalId),
      nowMs: () => Date.now(),
      logDebug: (message) => {
        console.debug(message);
      },
    });

  const renderHandlers = overlayLegacyRuntimeRenderHandlersUtils.createRuntimeRenderHandlers({
    nodes,
    state,
    constants,
    textUtils: overlayLegacyTextUtils,
    mediaDomUtils: overlayLegacyMediaDomUtils,
    mediaRenderUtils: overlayLegacyMediaRenderUtils,
    inlineVideoUtils: overlayLegacyInlineVideoUtils,
    utils: overlayLegacyUtils,
    tweetCardPlaybackUtils: overlayLegacyTweetCardPlaybackUtils,
    tweetCardUtils: overlayLegacyTweetCardUtils,
    tweetCardRendererUtils: overlayLegacyTweetCardRendererUtils,
    twitterWidgetsUtils: overlayLegacyTwitterWidgetsUtils,
    diagnosticsUtils: overlayLegacyDiagnosticsUtils,
    mediaOffsetUtils: overlayLegacyMediaOffsetUtils,
    bootstrapUtils: overlayLegacyBootstrapUtils,
    playFlowUtils: overlayLegacyPlayFlowUtils,
    ensureMediaFrame: mediaFrameHandlers.ensureMediaFrame,
    scheduleMediaHeaderLayout: mediaFrameHandlers.scheduleMediaHeaderLayout,
    clearOverlay: playbackCountdownHandlers.clearOverlay,
    startPlaybackSession: playbackCountdownHandlers.startPlaybackSession,
    startCountdown: playbackCountdownHandlers.startCountdown,
    clearCountdown: playbackCountdownHandlers.clearCountdown,
    resumeCountdown: playbackCountdownHandlers.resumeCountdown,
    pauseCountdown: playbackCountdownHandlers.pauseCountdown,
    setPlaybackState: playbackCountdownHandlers.setPlaybackState,
    setTimeout: (callback, delayMs) => setTimeout(() => callback(), delayMs),
    reportError: window.livechatOverlay?.reportError || null,
    logWarning: (message, error) => {
      console.warn(message, error?.message || error);
    },
    logError: (message, error) => {
      console.error(message, error);
    },
  });

  // Publish the caption footer height on the media frame so the stylesheet can
  // shrink the media to the space left on screen (see --overlay-media-max-height
  // in styles/overlay/base.css). Only the caption box is resized-observed: its
  // size does not depend on the media size, so updating the variable can never
  // feed back into another observation (no ResizeObserver loop error).
  const watchMediaFooterHeight = () => {
    const mediaLayer = nodes.mediaLayer;
    if (
      !(mediaLayer instanceof HTMLElement) ||
      typeof ResizeObserver !== 'function' ||
      typeof MutationObserver !== 'function'
    ) {
      return;
    }

    let observedCaption = null;

    const syncFooterHeight = () => {
      const frame = mediaLayer.querySelector('.overlay-media-frame');
      if (!(frame instanceof HTMLElement)) {
        return;
      }
      const footer = frame.querySelector('.overlay-media-footer');
      let footerHeightPx = 0;
      if (footer instanceof HTMLElement && footer.style.display !== 'none') {
        const marginTopPx = Number.parseFloat(window.getComputedStyle(footer).marginTop) || 0;
        footerHeightPx = Math.ceil(footer.getBoundingClientRect().height + marginTopPx);
      }
      const nextValue = `${footerHeightPx}px`;
      if (frame.style.getPropertyValue('--overlay-media-footer-height') !== nextValue) {
        frame.style.setProperty('--overlay-media-footer-height', nextValue);
      }
    };

    const resizeObserver = new ResizeObserver(() => syncFooterHeight());

    const mutationObserver = new MutationObserver(() => {
      const caption = mediaLayer.querySelector('.overlay-media-footer-meta');
      if (caption !== observedCaption) {
        if (observedCaption) {
          resizeObserver.unobserve(observedCaption);
        }
        observedCaption = caption instanceof HTMLElement ? caption : null;
        if (observedCaption) {
          resizeObserver.observe(observedCaption);
        }
      }
      syncFooterHeight();
    });
    mutationObserver.observe(mediaLayer, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['style'],
    });
  };

  watchMediaFooterHeight();

  const bootstrap = async () => {
    await overlayLegacyBootstrapUtils.bootstrapOverlay({
      getConfig: () => window.livechatOverlay.getConfig(),
      getCurrentConfig: () => state.overlayConfig,
      overlayRoot: nodes.overlayRoot,
      setConfig: (nextConfig) => {
        state.overlayConfig = nextConfig;
      },
      registerOnPlay: (handler) => {
        window.livechatOverlay.onPlay(handler);
      },
      registerOnStop: (handler) => {
        window.livechatOverlay.onStop(handler);
      },
      registerOnSettings: (handler) => {
        window.livechatOverlay.onSettings(handler);
      },
      onPlay: renderHandlers.onPlay,
      onStop: () => playbackCountdownHandlers.clearOverlay(),
      onSettings: renderHandlers.onSettings,
      addWindowResizeListener: (handler) => {
        window.addEventListener('resize', handler);
      },
      scheduleMediaHeaderLayout: mediaFrameHandlers.scheduleMediaHeaderLayout,
      ensureTwitterWidgets: renderHandlers.ensureTwitterWidgets,
      rendererReady: () => {
        window.livechatOverlay.rendererReady();
      },
    });
  };

  bootstrap().catch((error) => {
    console.error('Overlay bootstrap failed:', error);
  });
})();
