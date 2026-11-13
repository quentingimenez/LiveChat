function toSafeNumber(value) {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
        return 0;
    }
    return value;
}
export function createOverlayLegacyMediaFrameUtils() {
    function computeHeaderShift(params) {
        const mediaWidth = Math.max(0, Math.round(toSafeNumber(params.mediaWidth)));
        const leftWidth = Math.max(0, Math.round(toSafeNumber(params.leftWidth)));
        const rightWidth = Math.max(0, Math.round(toSafeNumber(params.rightWidth)));
        const minGapPx = Math.max(0, Math.round(toSafeNumber(params.minGapPx ?? 14)));
        const overflowPx = mediaWidth > 0 ? leftWidth + rightWidth + minGapPx - mediaWidth : 0;
        if (overflowPx <= 0) {
            return {
                leftShiftPx: 0,
                rightShiftPx: 0,
                unresolvedPx: 0
            };
        }
        // Room between the frame and the screen edge on each side: unbounded
        // when unknown. A frame anchored to a corner has none on that side, so
        // the other side takes what it can and the rest stays unresolved.
        const leftRoomPx = roomPx(params.leftRoomPx);
        const rightRoomPx = roomPx(params.rightRoomPx);
        const rightShiftPx = Math.min(Math.floor(overflowPx / 2), rightRoomPx);
        const leftShiftPx = Math.min(overflowPx - rightShiftPx, leftRoomPx);
        const finalRightShiftPx = Math.min(overflowPx - leftShiftPx, rightRoomPx);
        return {
            leftShiftPx,
            rightShiftPx: finalRightShiftPx,
            unresolvedPx: overflowPx - leftShiftPx - finalRightShiftPx
        };
    }
    function roomPx(value) {
        if (typeof value !== 'number' || Number.isNaN(value)) {
            return Number.POSITIVE_INFINITY;
        }
        return Math.max(0, Math.floor(value));
    }
    function createMediaFrameNodes(params) {
        const frame = document.createElement('div');
        frame.className = 'overlay-media-frame';
        frame.style.setProperty('--header-left-shift', '0px');
        frame.style.setProperty('--header-right-shift', '0px');
        const header = document.createElement('div');
        header.className = 'overlay-media-header';
        const headerLeft = document.createElement('div');
        headerLeft.className = 'overlay-media-header-left';
        const headerRight = document.createElement('div');
        headerRight.className = 'overlay-media-header-right';
        header.appendChild(headerLeft);
        header.appendChild(headerRight);
        const content = document.createElement('div');
        content.className = 'overlay-media-content';
        const footer = document.createElement('div');
        footer.className = 'overlay-media-footer';
        footer.style.display = 'none';
        frame.appendChild(header);
        frame.appendChild(content);
        frame.appendChild(footer);
        ensureCountdownLayerParent(params.countdownLayer, headerRight);
        return {
            frame,
            content,
            headerLeft,
            headerRight,
            footer
        };
    }
    function ensureCountdownLayerParent(countdownLayer, headerRight) {
        if (!(countdownLayer instanceof HTMLElement)) {
            return;
        }
        if (!(headerRight instanceof HTMLElement)) {
            return;
        }
        if (countdownLayer.parentElement !== headerRight) {
            headerRight.appendChild(countdownLayer);
        }
    }
    return {
        computeHeaderShift,
        createMediaFrameNodes,
        ensureCountdownLayerParent
    };
}
