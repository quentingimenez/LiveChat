function resolveJobId(jobId) {
    if (typeof jobId !== 'string' || jobId.trim() === '') {
        return 'unknown-job';
    }
    return jobId;
}
export function createOverlayLegacyMediaRenderUtils() {
    async function renderMedia(params) {
        const mediaPayload = params.payload?.media;
        if (!mediaPayload) {
            return;
        }
        const mediaUrl = params.buildAuthorizedMediaUrl(mediaPayload.url);
        params.setActiveObjectUrl(null);
        const element = params.createMediaElement(mediaPayload.kind);
        params.attachMediaDiagnostics(element, {
            mediaUrl,
            mediaKind: mediaPayload.kind,
            jobId: resolveJobId(params.payload?.jobId),
        });
        element.src = mediaUrl;
        const targetContainer = params.ensureMediaFrame() || params.mediaLayer;
        if (!(targetContainer instanceof HTMLElement)) {
            return;
        }
        const attached = params.attachMediaElement({
            targetContainer,
            element,
            mediaKind: mediaPayload.kind,
            onMediaReady: params.scheduleMediaHeaderLayout,
        });
        if (!attached) {
            return;
        }
        params.scheduleMediaHeaderLayout();
        params.applyVolume();
        if (mediaPayload.kind === 'image') {
            return;
        }
        const playableElement = element;
        params.setActivePlayableElement(playableElement);
        params.applyMediaStartOffset(playableElement, params.resolveStartOffsetSec(mediaPayload));
        playableElement.addEventListener('playing', () => {
            params.resumeCountdown();
            params.setPlaybackState('playing');
        });
        playableElement.addEventListener('pause', () => {
            if (playableElement.ended) {
                return;
            }
            params.pauseCountdown();
            params.setPlaybackState('paused');
        });
        try {
            await playableElement.play();
        }
        catch (error) {
            params.logAutoplayError('Autoplay failed:', error);
            params.pauseCountdown();
            params.setPlaybackState('paused');
        }
        playableElement.addEventListener('ended', () => {
            params.clearOverlay();
        }, { once: true });
    }
    return {
        renderMedia,
    };
}
