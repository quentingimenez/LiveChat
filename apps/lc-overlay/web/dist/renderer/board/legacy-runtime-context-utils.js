export function createBoardLegacyRuntimeContextUtils() {
    function createRuntimeContextHelpers(params) {
        function normalizeMediaKind(value) {
            return params.boardUtils.normalizeMediaKind(value);
        }
        function toSafeDateLabel(value) {
            return params.boardUtils.toSafeDateLabel(value);
        }
        function toCardTitle(item) {
            return params.boardUtils.toCardTitle(item);
        }
        function toMessagePreview(value, maxLength = 120) {
            return params.boardUtils.toMessagePreview(value, maxLength);
        }
        function isHttpUrl(value) {
            return params.boardUtils.isHttpUrl(value);
        }
        function buildAuthedUrl(pathname, options = {}) {
            return params.previewMediaUtils.buildAuthedUrl({
                config: params.state.config,
                pathname,
                options,
            });
        }
        function applyPreviewVolume(root = params.previewStageNode) {
            params.previewMediaUtils.applyPreviewVolume({
                root,
                linearVolume: params.state.config?.volume,
                gamma: params.volumeCurveGamma,
            });
        }
        function findSelectedItem() {
            const resolved = params.selectionUtils.resolveSelectedItemForPreview({
                items: params.state.items,
                selectedId: params.state.selectedId,
                selectedItem: params.state.selectedItem,
            });
            params.state.selectedItem = resolved.syncedSelectedItem;
            return resolved.selectedItem;
        }
        function getItemShortcuts(itemId) {
            return params.bindingsUtils.getItemShortcuts(params.state.bindings, itemId);
        }
        function resolvePreviewMessageItemTitle(itemId) {
            const selected = findSelectedItem();
            if (selected && `${selected.id || ''}` === `${itemId || ''}`) {
                return `${selected.title || ''}`.trim();
            }
            const inList = (Array.isArray(params.state.items) ? params.state.items : []).find((entry) => `${entry?.id || ''}` === `${itemId || ''}`);
            return `${inList?.title || ''}`.trim();
        }
        function isCaptureUiReady() {
            return params.dialogUtils.isCaptureUiReady({
                captureOverlay: params.captureOverlay,
                captureCurrentNode: params.captureCurrentNode,
                captureCancelButton: params.captureCancelButton,
                captureSaveButton: params.captureSaveButton,
            });
        }
        return {
            normalizeMediaKind,
            toSafeDateLabel,
            toCardTitle,
            toMessagePreview,
            isHttpUrl,
            buildAuthedUrl,
            applyPreviewVolume,
            findSelectedItem,
            getItemShortcuts,
            resolvePreviewMessageItemTitle,
            isCaptureUiReady,
        };
    }
    return {
        createRuntimeContextHelpers,
    };
}
