function resolveJobId(jobId) {
    if (typeof jobId !== 'string' || jobId.trim() === '') {
        return 'unknown-job';
    }
    return jobId;
}
function hasStandaloneMedia(payload) {
    return Boolean(payload?.media);
}
function shouldAutoClearByTimer(payload, hasMedia) {
    if (!hasMedia) {
        return true;
    }
    return payload?.media?.kind === 'image';
}
export function createOverlayLegacyPlayFlowUtils() {
    async function runPlayFlow(params) {
        params.clearOverlay();
        params.startPlaybackSession(params.payload?.jobId || null);
        const hasMedia = hasStandaloneMedia(params.payload);
        if (!hasMedia) {
            params.applyOverlayInfo(params.payload, {
                showAuthorInline: true,
                attachTextToMedia: false,
            });
        }
        const autoClearByTimer = shouldAutoClearByTimer(params.payload, hasMedia);
        try {
            if (!params.renderTweetCard(params.payload)) {
                await params.renderMedia(params.payload);
            }
            params.applyMediaHeaderAuthor(params.payload, {
                enabled: hasMedia,
            });
            params.applyMediaFooterText(params.payload, {
                enabled: hasMedia,
            });
            if (hasMedia) {
                params.applyOverlayInfo(params.payload, {
                    showAuthorInline: false,
                    attachTextToMedia: true,
                });
            }
        }
        catch (error) {
            params.logRenderError('Media render failed:', error);
            params.reportError({
                jobId: resolveJobId(params.payload?.jobId),
                code: 'render_failed',
                message: error instanceof Error && typeof error.message === 'string' ? error.message : 'unknown_render_error',
            });
            params.clearOverlay();
            return;
        }
        if (params.payload?.durationSec && params.payload.hold === true) {
            // Layout preview template: the countdown is shown frozen and the
            // card stays until the shell sends overlay://stop.
            params.startCountdown(params.payload.durationSec, { autoClear: false });
            params.pauseCountdown();
            return;
        }
        if (params.payload?.durationSec) {
            params.startCountdown(params.payload.durationSec, { autoClear: autoClearByTimer });
            if (autoClearByTimer) {
                const resetTimer = params.setTimeout(() => {
                    params.clearOverlay();
                }, Number(params.payload.durationSec) * 1000 + 100);
                params.setResetTimer(resetTimer);
            }
            return;
        }
        params.clearCountdown();
    }
    return {
        runPlayFlow,
    };
}
