// Gap kept between the header chrome and the screen edge when the header
// spills past a frame anchored near that edge.
const HEADER_SCREEN_GAP_PX = 8;
export function createOverlayLegacyMediaFrameLayoutUtils() {
    function clearMediaHeaderLayoutRaf(params) {
        if (params.state.mediaHeaderLayoutRaf === null) {
            return;
        }
        if (typeof params.cancelAnimationFrame === 'function') {
            params.cancelAnimationFrame(params.state.mediaHeaderLayoutRaf);
        }
        else {
            params.clearTimeout(params.state.mediaHeaderLayoutRaf);
        }
        params.state.mediaHeaderLayoutRaf = null;
    }
    function syncMediaHeaderLayout(params) {
        if (!(params.state.activeMediaFrame instanceof HTMLElement) ||
            !(params.state.activeMediaHeaderLeft instanceof HTMLElement) ||
            !(params.state.activeMediaHeaderRight instanceof HTMLElement)) {
            return;
        }
        const frame = params.state.activeMediaFrame;
        const frameRect = frame.getBoundingClientRect();
        const mediaNode = params.state.activeMediaContent?.firstElementChild;
        const mediaWidth = mediaNode instanceof HTMLElement
            ? Math.max(0, Math.round(mediaNode.getBoundingClientRect().width))
            : Math.max(0, Math.round(frameRect.width));
        // Measure the author chip at its natural width, without the clamp a
        // previous pass may have set.
        frame.style.removeProperty('--header-left-max-width');
        const leftWidth = params.state.activeMediaHeaderLeft.scrollWidth;
        const viewportWidth = frame.ownerDocument.documentElement.clientWidth;
        const shift = params.computeHeaderShift({
            mediaWidth,
            leftWidth,
            rightWidth: params.state.activeMediaHeaderRight.scrollWidth,
            minGapPx: 14,
            leftRoomPx: frameRect.left - HEADER_SCREEN_GAP_PX,
            rightRoomPx: viewportWidth - frameRect.right - HEADER_SCREEN_GAP_PX,
        });
        frame.style.setProperty('--header-left-shift', `${shift.leftShiftPx}px`);
        frame.style.setProperty('--header-right-shift', `${shift.rightShiftPx}px`);
        if (shift.unresolvedPx > 0) {
            // No room left on either side: shorten the author name (ellipsis)
            // rather than overlap the countdown or leave the screen.
            frame.style.setProperty('--header-left-max-width', `${Math.max(0, leftWidth - shift.unresolvedPx)}px`);
        }
    }
    function scheduleMediaHeaderLayout(params) {
        if (params.state.mediaHeaderLayoutRaf !== null) {
            return;
        }
        if (typeof params.requestAnimationFrame === 'function') {
            params.state.mediaHeaderLayoutRaf = params.requestAnimationFrame(() => {
                params.state.mediaHeaderLayoutRaf = null;
                params.syncMediaHeaderLayout();
            });
            return;
        }
        params.state.mediaHeaderLayoutRaf = params.setTimeout(() => {
            params.state.mediaHeaderLayoutRaf = null;
            params.syncMediaHeaderLayout();
        }, 0);
    }
    function ensureMediaFrame(params) {
        if (!(params.mediaLayer instanceof HTMLElement)) {
            return null;
        }
        if (params.state.activeMediaFrame instanceof HTMLElement &&
            params.state.activeMediaContent instanceof HTMLElement &&
            params.state.activeMediaHeaderLeft instanceof HTMLElement &&
            params.state.activeMediaHeaderRight instanceof HTMLElement) {
            params.ensureCountdownLayerParent(params.countdownLayer, params.state.activeMediaHeaderRight);
            return params.state.activeMediaContent;
        }
        const nextMediaFrameNodes = params.createMediaFrameNodes({
            countdownLayer: params.countdownLayer,
        });
        params.mediaLayer.appendChild(nextMediaFrameNodes.frame);
        params.state.activeMediaFrame = nextMediaFrameNodes.frame;
        params.state.activeMediaContent = nextMediaFrameNodes.content;
        params.state.activeMediaHeaderLeft = nextMediaFrameNodes.headerLeft;
        params.state.activeMediaHeaderRight = nextMediaFrameNodes.headerRight;
        params.state.activeMediaFooter = nextMediaFrameNodes.footer;
        params.scheduleMediaHeaderLayout();
        return nextMediaFrameNodes.content;
    }
    function resetMediaFrameState(params) {
        params.state.activeMediaFrame = null;
        params.state.activeMediaContent = null;
        params.state.activeMediaHeaderLeft = null;
        params.state.activeMediaHeaderRight = null;
        params.state.activeMediaFooter = null;
        params.state.mediaHeaderLayoutRaf = null;
    }
    return {
        clearMediaHeaderLayoutRaf,
        syncMediaHeaderLayout,
        scheduleMediaHeaderLayout,
        ensureMediaFrame,
        resetMediaFrameState,
    };
}
