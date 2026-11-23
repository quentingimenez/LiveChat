export function createOverlayLegacyRuntimeRenderHandlersUtils() {
    function createRuntimeRenderHandlers(params) {
        // A guild may set its own text display for its media (`showText` in
        // the `play`); otherwise the device setting applies.
        function isTextVisible() {
            if (typeof params.state.activeShowText === 'boolean') {
                return params.state.activeShowText;
            }
            return params.state.overlayConfig.showText !== false;
        }
        // Media sound muted for every guild or for the playing one: read from
        // the settings at each call, so a change applies to the current media.
        function isMediaSoundMuted() {
            const config = params.state.overlayConfig || {};
            if (config.mediaMuted === true) {
                return true;
            }
            const guilds = Array.isArray(config.mediaMutedGuilds) ? config.mediaMutedGuilds : [];
            return typeof params.state.activeGuildId === 'string' && guilds.includes(params.state.activeGuildId);
        }
        // The playing guild's own zone, else the device one.
        function applyActiveLayout() {
            params.bootstrapUtils.applyOverlayLayout({
                root: params.nodes.overlayRoot,
                layout: params.state.activeLayout || params.state.overlayConfig.layout,
            });
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
        // The playing guild's own volume, from the live device settings.
        // `null` when the settings do not describe it (unknown id, older
        // payload): the play's own effective volume then remains the source.
        function resolveGuildVolume() {
            const config = params.state.overlayConfig || {};
            const guildVolumes = config.guildVolumes;
            const guildId = params.state.activeGuildId;
            if (typeof guildId !== 'string' || !guildVolumes || typeof guildVolumes !== 'object') {
                return null;
            }
            const value = guildVolumes[guildId];
            return typeof value === 'number' && Number.isFinite(value) ? params.utils.clamp01(value) : null;
        }
        // Effective volume, recomputed at each call so a general or guild
        // volume change applies to the media already playing. Falls back to
        // the play's effective volume, then to the general setting.
        function resolveVolume() {
            if (isMediaSoundMuted()) {
                return 0;
            }
            const config = params.state.overlayConfig || {};
            const general = typeof config.volume === 'number' && Number.isFinite(config.volume)
                ? params.utils.clamp01(config.volume)
                : null;
            const guild = resolveGuildVolume();
            if (general !== null && guild !== null) {
                return general * guild;
            }
            const payloadVolume = params.state.activeVolume;
            if (typeof payloadVolume === 'number' && Number.isFinite(payloadVolume) && payloadVolume >= 0 && payloadVolume <= 1) {
                return payloadVolume;
            }
            return general !== null ? general : 1;
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
        // A scaled media is sized in pixels for the zone at render time: when
        // the window is resized (display resolution change), replay the re-fit
        // so it follows the new screen instead of keeping the old one.
        function relayoutActiveMedia() {
            if (typeof params.state.activeMediaRelayout !== 'function') {
                return false;
            }
            params.state.activeMediaRelayout();
            return true;
        }
        async function renderMedia(payload) {
            const relayout = await params.mediaRenderUtils.renderMedia({
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
                videoExtensionSec: params.playbackUtils.videoExtensionSec,
                resolveVideoEndAction: params.playbackUtils.resolveVideoEndAction,
                videoHasAudioTrack: params.inlineVideoUtils.videoHasAudioTrack,
                getRemainingCountdownMs: params.getRemainingCountdownMs,
                setCountdownAutoClear: params.setCountdownAutoClear,
                logAutoplayError: params.logError,
                state: params.state,
                requestGuildPause: params.requestGuildPause,
            });
            params.state.activeMediaRelayout = typeof relayout === 'function' ? relayout : null;
        }
        async function onPlay(payload) {
            params.state.activeVolume = typeof payload?.volume === 'number' ? payload.volume : null;
            params.state.activeGuildId = typeof payload?.guildId === 'string' ? payload.guildId : null;
            params.state.activeShowText = typeof payload?.showText === 'boolean' ? payload.showText : null;
            params.state.activeLayout = payload?.layout && typeof payload.layout === 'object' ? payload.layout : null;
            applyActiveLayout();
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
                pauseCountdown: params.pauseCountdown,
                setResetTimer: (nextResetTimer) => {
                    params.state.resetTimer = nextResetTimer;
                },
                setTimeout: params.setTimeout,
                clearCountdown: params.clearCountdown,
                isCountdownPaused: params.isCountdownPaused,
            });
        }
        function onSettings(nextSettings) {
            params.state.overlayConfig = params.bootstrapUtils.applyOverlaySettings({
                currentConfig: params.state.overlayConfig,
                nextSettings,
                overlayRoot: params.nodes.overlayRoot,
                onHideText: () => {
                    // The playing guild shows its text whatever the device says.
                    if (params.state.activeShowText === true) {
                        return;
                    }
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
            // `applyOverlaySettings` applied the volume before the new
            // settings were stored: apply it again so a media sound switch
            // takes effect on the current media.
            applyVolume();
            // The playing guild keeps its own zone.
            if (params.state.activeLayout) {
                applyActiveLayout();
            }
            // The zone may have moved: re-fit the header chips in it, and the
            // scaled media sized in pixels for the previous zone.
            relayoutActiveMedia();
            params.scheduleMediaHeaderLayout();
        }
        return {
            onPlay,
            onSettings,
            ensureTwitterWidgets,
            relayoutActiveMedia,
        };
    }
    return {
        createRuntimeRenderHandlers,
    };
}
