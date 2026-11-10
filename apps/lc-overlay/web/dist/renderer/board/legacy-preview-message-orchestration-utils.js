export function createBoardLegacyPreviewMessageOrchestrationUtils() {
    function createPreviewMessageHandlers(params) {
        function clearPreviewMessageAutosave() {
            const timeoutId = params.getPreviewMessageAutosaveTimeoutId();
            if (timeoutId !== null) {
                params.clearTimeout(timeoutId);
                params.setPreviewMessageAutosaveTimeoutId(null);
            }
        }
        function hasPendingPreviewMessageChanges(editor = params.getPreviewMessageEditor()) {
            return params.previewMessageUtils.hasPendingPreviewMessageChanges(editor);
        }
        function buildPersistenceParams(editor, options = {}) {
            return {
                editor,
                keepalive: options.keepalive === true,
                silent: options.silent === true,
                resolveTitleForItem: params.resolvePreviewMessageItemTitle,
                patchItemMetadata: params.patchItemMetadata,
                onSaved: ({ itemId, nextTitle, nextMessage }) => {
                    params.applyLocalItemMetadata(itemId, nextTitle, nextMessage);
                    params.renderList();
                },
                setStatusError: params.setStatusError,
                scheduleAutosave: (delayMs) => {
                    schedulePreviewMessageAutosave(delayMs);
                },
            };
        }
        function schedulePreviewMessageAutosave(delayMs = params.defaultAutosaveDelayMs) {
            clearPreviewMessageAutosave();
            if (!params.getPreviewMessageEditor()) {
                return;
            }
            const timeoutId = params.setTimeout(() => {
                params.setPreviewMessageAutosaveTimeoutId(null);
                void savePreviewMessage();
            }, Math.max(0, Math.floor(delayMs)));
            params.setPreviewMessageAutosaveTimeoutId(timeoutId);
        }
        function savePreviewMessage(options = {}) {
            const editor = options.editor || params.getPreviewMessageEditor();
            return params.previewMessageUtils.savePreviewMessage({
                ...buildPersistenceParams(editor, options),
                waitForInFlight: options.waitForInFlight !== false,
            });
        }
        function flushPreviewMessageChanges(options = {}) {
            const editor = options.editor || params.getPreviewMessageEditor();
            return params.previewMessageUtils.flushPreviewMessageChanges({
                ...buildPersistenceParams(editor, options),
                clearAutosave: clearPreviewMessageAutosave,
            });
        }
        function flushPreviewMessageOnLifecycleExit() {
            const editor = params.getPreviewMessageEditor();
            if (!hasPendingPreviewMessageChanges(editor)) {
                return;
            }
            void flushPreviewMessageChanges({
                editor,
                keepalive: true,
                silent: true,
            });
        }
        return {
            clearPreviewMessageAutosave,
            hasPendingPreviewMessageChanges,
            schedulePreviewMessageAutosave,
            savePreviewMessage,
            flushPreviewMessageChanges,
            flushPreviewMessageOnLifecycleExit,
        };
    }
    return {
        createPreviewMessageHandlers,
    };
}
