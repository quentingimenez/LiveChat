export function createOverlayLegacyRuntimeRenderHandlersUtils() {
    function createRuntimeRenderHandlers(params) {
        function isTextVisible() {
            return params.state.overlayConfig.showText !== false;
        }
        function applyMediaHeaderAuthor(payload, options = { enabled: false }) {
            params.textUtils.applyMediaHeaderAuthor({
                container: params.state.mediaFrameState.activeMediaHeaderLeft,
                payload,
                showText: isTextVisible(),
                enabled: options.enabled === true,
                onLayout: params.scheduleMediaHeaderLayout,
            });
        }
        function applyMediaFooterText(payload, options = { enabled: false }) {
            params.textUtils.applyMediaFooterText({
                container: params.state.mediaFrameState.activeMediaFooter,
                payload,
                showText: isTextVisible(),
                enabled: options.enabled === true,
            });
        }
        function applyOverlayInfo(payload, options = {
            showAuthorInline: false,
            attachTextToMedia: false,
        }) {
            params.textUtils.applyOverlayInfo({
                layer: params.nodes.textLayer,
                payload,
                showText: isTextVisible(),
                showAuthorInline: options.showAuthorInline === true,
                attachTextToMedia: options.attachTextToMedia === true,
            });
        }
        function resolveVolume() {
            const payloadVolume = params.state.activeVolume;
            if (typeof payloadVolume === 'number' && Number.isFinite(payloadVolume) && payloadVolume >= 0 && payloadVolume <= 1) {
                return payloadVolume;
            }
            return params.state.overlayConfig.volume;
        }
        function applyVolume() {
            params.mediaDomUtils.applyMediaVolume({
                root: params.nodes.mediaLayer,
                perceptualGain: params.utils.toPerceptualGain(resolveVolume(), params.constants.volumeCurveGamma),
            });
        }
        function ensureTwitterWidgets() {
            return params.twitterWidgetsUtils.ensureTwitterWidgets();
        }
        function renderTweetCard(payload) {
            return params.tweetCardPlaybackUtils.renderTweetCard({
                payload,
                createTweetCardModel: params.tweetCardUtils.createTweetCardModel,
                createTweetCardDom: (nextParams) => params.tweetCardRendererUtils.createTweetCardDom(nextParams),
                ensureMediaFrame: params.ensureMediaFrame,
                mediaLayer: params.nodes.mediaLayer,
                scheduleMediaHeaderLayout: params.scheduleMediaHeaderLayout,
                ensureTwitterWidgets: (() => ensureTwitterWidgets()),
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
        async function renderMedia(payload) {
            await params.mediaRenderUtils.renderMedia({
                payload: payload,
                setActiveObjectUrl: (nextObjectUrl) => {
                    params.state.activeObjectUrl = nextObjectUrl;
                },
                buildAuthorizedMediaUrl: (url) => params.mediaDomUtils.buildAuthorizedMediaUrl(url, params.state.overlayConfig.clientToken),
                createMediaElement: params.mediaDomUtils.createMediaElement,
                attachMediaDiagnostics: (element, diagnosticsParams) => params.diagnosticsUtils.attachMediaDiagnostics(element, diagnosticsParams),
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
        async function onPlay(payload) {
            params.state.activeVolume = typeof payload?.volume === 'number' ? payload.volume : null;
            const isLocalTest = payload?.test === true;
            await params.playFlowUtils.runPlayFlow({
                payload: payload,
                clearOverlay: params.clearOverlay,
                startPlaybackSession: params.startPlaybackSession,
                applyOverlayInfo,
                renderTweetCard,
                renderMedia,
                applyMediaHeaderAuthor,
                applyMediaFooterText,
                logRenderError: params.logError,
                reportError: (errorPayload) => {
                    if (!isLocalTest && typeof params.reportError === 'function') {
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
        function onSettings(nextSettings) {
            params.state.overlayConfig = params.bootstrapUtils.applyOverlaySettings({
                currentConfig: params.state.overlayConfig,
                nextSettings,
                overlayRoot: params.nodes.overlayRoot,
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
