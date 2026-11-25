/**
 * Box of a picture shown at `scalePercent` of its natural size, kept inside
 * `maxWidth` × `maxHeight` with its aspect ratio. `capped` tells whether the
 * display zone shrank it. Also loaded by the Node unit tests.
 */
export function computeDisplayedImageSize({ naturalWidth, naturalHeight, scalePercent, maxWidth, maxHeight }) {
    const natural = [Number(naturalWidth), Number(naturalHeight)];
    const percent = Number(scalePercent);
    if (!(natural[0] > 0) || !(natural[1] > 0) || !(percent > 0)) {
        return { width: 0, height: 0, capped: false };
    }
    const factor = percent / 100;
    let width = natural[0] * factor;
    let height = natural[1] * factor;
    const limitWidth = Number.isFinite(Number(maxWidth)) ? Math.max(0, Number(maxWidth)) : Number.POSITIVE_INFINITY;
    const limitHeight = Number.isFinite(Number(maxHeight)) ? Math.max(0, Number(maxHeight)) : Number.POSITIVE_INFINITY;
    const fit = Math.min(1, limitWidth / width, limitHeight / height);
    width *= fit;
    height *= fit;
    return {
        width: Math.max(1, Math.round(width)),
        height: Math.max(1, Math.round(height)),
        capped: fit < 1
    };
}
function resolveJobId(jobId) {
    if (typeof jobId !== 'string' || jobId.trim() === '') {
        return 'unknown-job';
    }
    return jobId;
}
export function createOverlayLegacyMediaRenderUtils() {
    // Space left in the display zone for the media itself: the zone minus the
    // header (author chip + countdown) and the caption footer.
    function mediaZoneLimits(params, targetContainer) {
        const frame = targetContainer.parentElement;
        const header = frame ? frame.querySelector('.overlay-media-header') : null;
        let headerHeight = 0;
        if (header instanceof HTMLElement) {
            const style = window.getComputedStyle(header);
            headerHeight = header.getBoundingClientRect().height + (Number.parseFloat(style.marginBottom) || 0);
        }
        const footerHeight = frame instanceof HTMLElement
            ? Number.parseFloat(frame.style.getPropertyValue('--overlay-media-footer-height')) || 0
            : 0;
        const layer = params.mediaLayer;
        const width = layer instanceof HTMLElement ? layer.clientWidth : window.innerWidth;
        const height = layer instanceof HTMLElement ? layer.clientHeight : window.innerHeight;
        return { maxWidth: width, maxHeight: Math.max(0, height - headerHeight - footerHeight) };
    }
    // A media with a chosen scale takes natural size × scale / 100, still
    // capped by the zone. Without the field it keeps today's CSS fit.
    //
    // Every kind takes a scale: a still, the picture of a video, and the
    // artwork of a sound. An `audio` element has no size of its own, so its
    // artwork is the element to size.
    function naturalSizeOf(element) {
        if (element instanceof HTMLImageElement) {
            return { width: element.naturalWidth, height: element.naturalHeight };
        }
        if (element instanceof HTMLVideoElement) {
            return { width: element.videoWidth, height: element.videoHeight };
        }
        return null;
    }
    // Returns the closure that re-fits the element, or `null` for a media
    // without a chosen scale. The runtime replays it when the window changes
    // size (display resolution change), else the media keeps the pixel size
    // measured for the old zone.
    function applyDisplayScale(params, element, targetContainer, mediaPayload) {
        const percent = Number(mediaPayload.displayScalePercent);
        if (!Number.isFinite(percent) || percent <= 0) {
            return null;
        }
        const sized = () => {
            const natural = naturalSizeOf(element);
            return Boolean(natural && natural.width > 0 && natural.height > 0);
        };
        const apply = () => {
            if (!element.isConnected) {
                return;
            }
            const natural = naturalSizeOf(element);
            if (!natural || !(natural.width > 0) || !(natural.height > 0)) {
                return;
            }
            const limits = mediaZoneLimits(params, targetContainer);
            const size = computeDisplayedImageSize({
                naturalWidth: natural.width,
                naturalHeight: natural.height,
                scalePercent: percent,
                ...limits
            });
            if (size.width > 0 && size.height > 0) {
                element.style.width = `${size.width}px`;
                element.style.height = `${size.height}px`;
            }
            params.scheduleMediaHeaderLayout();
        };
        if (sized()) {
            // Let the caption footer be laid out before measuring the zone.
            if (typeof window.requestAnimationFrame === 'function') {
                window.requestAnimationFrame(apply);
            }
            else {
                setTimeout(apply, 0);
            }
            return apply;
        }
        // A video reports its size once its metadata is known, a picture once
        // it is loaded.
        element.addEventListener(element instanceof HTMLVideoElement ? 'loadedmetadata' : 'load', apply, { once: true });
        return apply;
    }
    // Returns the closure that re-fits a scaled media to the current zone, so
    // the runtime can replay it after a window resize; `null` when the media
    // has no chosen scale.
    async function renderMedia(params) {
        const mediaPayload = params.payload?.media;
        if (!mediaPayload) {
            return null;
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
            return null;
        }
        let relayout = null;
        // An audio with a picture shows it while it plays. It comes first so
        // the header layout measures it as the media.
        if (mediaPayload.kind === 'audio' && typeof mediaPayload.artworkUrl === 'string' && mediaPayload.artworkUrl !== '') {
            const artwork = document.createElement('img');
            artwork.className = 'overlay-media-artwork';
            artwork.alt = '';
            artwork.addEventListener('load', () => {
                params.scheduleMediaHeaderLayout();
            }, { once: true });
            artwork.addEventListener('error', () => {
                artwork.remove();
                params.scheduleMediaHeaderLayout();
            }, { once: true });
            artwork.src = params.buildAuthorizedMediaUrl(mediaPayload.artworkUrl);
            targetContainer.appendChild(artwork);
            // The artwork is what a scaled sound sizes: the audio element
            // itself stays invisible.
            relayout = applyDisplayScale(params, artwork, targetContainer, mediaPayload);
        }
        const attached = params.attachMediaElement({
            targetContainer,
            element,
            mediaKind: mediaPayload.kind,
            onMediaReady: params.scheduleMediaHeaderLayout,
        });
        if (!attached) {
            return relayout;
        }
        params.scheduleMediaHeaderLayout();
        params.applyVolume();
        if (mediaPayload.kind === 'image') {
            return applyDisplayScale(params, element, targetContainer, mediaPayload);
        }
        // A video takes the scale before it plays: its first frame already has
        // the size the overlay will show.
        if (mediaPayload.kind === 'video') {
            relayout = applyDisplayScale(params, element, targetContainer, mediaPayload);
        }
        const playableElement = element;
        params.setActivePlayableElement(playableElement);
        params.applyMediaStartOffset(playableElement, params.resolveStartOffsetSec(mediaPayload));
        const requestGuildPause = typeof params.requestGuildPause === 'function' ? params.requestGuildPause : null;
        const state = params.state && typeof params.state === 'object' ? params.state : {};
        playableElement.addEventListener('playing', () => {
            params.resumeCountdown();
            params.setPlaybackState('playing');
            if (state.suppressPlayBroadcast === true) {
                state.suppressPlayBroadcast = false;
                return;
            }
            if (state.guildPaused === true && requestGuildPause) {
                state.guildPaused = false;
                requestGuildPause(false);
            }
        });
        playableElement.addEventListener('pause', () => {
            if (playableElement.ended) {
                return;
            }
            params.pauseCountdown();
            params.setPlaybackState('paused');
            if (state.suppressPauseBroadcast === true) {
                state.suppressPauseBroadcast = false;
                return;
            }
            // A stop released the element: do not turn the stop into a guild pause.
            if (state.releasingMedia === true || state.activePlayableElement !== playableElement) {
                return;
            }
            if (state.guildPaused === true) {
                return;
            }
            if (requestGuildPause) {
                requestGuildPause(true);
            }
        });
        const startPaused = params.payload?.paused === true || state.guildPaused === true;
        if (startPaused) {
            state.guildPaused = true;
            params.pauseCountdown();
            params.setPlaybackState('paused');
        }
        else {
            try {
                await playableElement.play();
            }
            catch (error) {
                params.logAutoplayError('Autoplay failed:', error);
                params.pauseCountdown();
                params.setPlaybackState('paused');
            }
        }
        // The playback ends with the media: the overlay leaves the screen as
        // soon as the video (or the sound) reaches its own end, whatever the
        // duration of the job.
        playableElement.addEventListener('ended', () => {
            params.clearOverlay();
        }, { once: true });
        return relayout;
    }
    return {
        renderMedia,
    };
}
