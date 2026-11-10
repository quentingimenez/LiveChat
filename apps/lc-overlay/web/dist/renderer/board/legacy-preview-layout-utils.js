function toFiniteNumber(value, fallback = 0) {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
        return fallback;
    }
    return value;
}
function resolveStageNode(value) {
    return value instanceof HTMLElement ? value : null;
}
export function createBoardLegacyPreviewLayoutUtils() {
    function fitPreviewMediaToStage(params) {
        const previewStageNode = resolveStageNode(params.previewStageNode);
        if (!previewStageNode) {
            return;
        }
        const mediaNode = previewStageNode.querySelector('.preview-media');
        if (!(mediaNode instanceof HTMLElement)) {
            return;
        }
        if (mediaNode instanceof HTMLAudioElement) {
            mediaNode.style.removeProperty('max-height');
            return;
        }
        const stageHeight = previewStageNode.clientHeight;
        if (!Number.isFinite(stageHeight) || stageHeight <= 0) {
            return;
        }
        const children = Array.from(previewStageNode.children).filter((node) => node instanceof HTMLElement);
        const computedStyle = window.getComputedStyle(previewStageNode);
        const parsedGap = Number.parseFloat(computedStyle.rowGap || computedStyle.gap || '0');
        const rowGap = Number.isFinite(parsedGap) ? parsedGap : 0;
        const parsedPaddingTop = Number.parseFloat(computedStyle.paddingTop || '0');
        const parsedPaddingBottom = Number.parseFloat(computedStyle.paddingBottom || '0');
        const verticalPadding = toFiniteNumber(parsedPaddingTop, 0) + toFiniteNumber(parsedPaddingBottom, 0);
        const isSideLayout = window.matchMedia(`(max-width: ${params.sideLayoutBreakpointPx}px)`).matches;
        let nonMediaHeight = 0;
        for (const childNode of children) {
            if (childNode === mediaNode) {
                continue;
            }
            nonMediaHeight += childNode.getBoundingClientRect().height;
        }
        const totalGaps = rowGap * Math.max(0, children.length - 1);
        const rawAvailableHeight = isSideLayout
            ? Math.floor(stageHeight - verticalPadding)
            : Math.floor(stageHeight - nonMediaHeight - totalGaps - verticalPadding);
        const availableHeight = Math.max(0, rawAvailableHeight);
        const minHeightPx = Math.max(0, Math.floor(toFiniteNumber(params.minHeightPx, 0)));
        const effectiveMaxHeight = availableHeight >= minHeightPx ? availableHeight : minHeightPx;
        const nextMaxHeight = `${effectiveMaxHeight}px`;
        if (mediaNode.style.maxHeight !== nextMaxHeight) {
            mediaNode.style.maxHeight = nextMaxHeight;
        }
    }
    function refreshPreviewLayoutObserverTargets(params) {
        const previewLayoutObserver = params.previewLayoutObserver;
        if (typeof ResizeObserver !== 'function' || !(previewLayoutObserver instanceof ResizeObserver)) {
            return;
        }
        const previewStageNode = resolveStageNode(params.previewStageNode);
        if (!previewStageNode) {
            return;
        }
        previewLayoutObserver.disconnect();
        previewLayoutObserver.observe(previewStageNode);
        const trackedChildren = previewStageNode.querySelectorAll('.preview-media, .preview-message-editor, .preview-controls');
        trackedChildren.forEach((node) => {
            if (node instanceof HTMLElement) {
                previewLayoutObserver.observe(node);
            }
        });
    }
    return {
        fitPreviewMediaToStage,
        refreshPreviewLayoutObserverTargets,
    };
}
