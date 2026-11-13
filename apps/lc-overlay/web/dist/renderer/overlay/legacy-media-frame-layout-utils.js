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
        const mediaNode = params.state.activeMediaContent?.firstElementChild;
        const mediaWidth = mediaNode instanceof HTMLElement
            ? Math.max(0, Math.round(mediaNode.getBoundingClientRect().width))
            : Math.max(0, Math.round(params.state.activeMediaFrame.getBoundingClientRect().width));
        const shift = params.computeHeaderShift({
            mediaWidth,
            leftWidth: params.state.activeMediaHeaderLeft.scrollWidth,
            rightWidth: params.state.activeMediaHeaderRight.scrollWidth,
            minGapPx: 14,
        });
        params.state.activeMediaFrame.style.setProperty('--header-left-shift', `${shift.leftShiftPx}px`);
        params.state.activeMediaFrame.style.setProperty('--header-right-shift', `${shift.rightShiftPx}px`);
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
