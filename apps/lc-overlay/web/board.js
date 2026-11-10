(() => {
  const boardLegacyRuntimeInputsUtils = window.__boardLegacyRuntimeInputsUtils;
  const runtimeInputs = boardLegacyRuntimeInputsUtils.createRuntimeInputs(document);
  const { state, nodes: runtimeNodes, constants } = runtimeInputs;
  const { statusNode, countPillNode, itemsListNode, previewStageNode, selectedTitleNode, selectedMetaNode } = runtimeNodes;
  const { volumeCurveGamma: VOLUME_CURVE_GAMMA, instantSearchDebounceMs: INSTANT_SEARCH_DEBOUNCE_MS } = constants;
  const { previewMessageAutosaveDebounceMs: PREVIEW_MESSAGE_AUTOSAVE_DEBOUNCE_MS } = constants;
  const { previewMediaMinHeightPx: PREVIEW_MEDIA_MIN_HEIGHT_PX, previewSideLayoutBreakpointPx: PREVIEW_SIDE_LAYOUT_BREAKPOINT_PX } =
    constants;
  const boardLegacyUtils = window.__boardLegacyUtils;
  const boardLegacyBindingsUtils = window.__boardLegacyBindingsUtils;
  const boardLegacyDialogUtils = window.__boardLegacyDialogUtils;
  const boardLegacyPreviewLayoutUtils = window.__boardLegacyPreviewLayoutUtils;
  const boardLegacyPreviewMediaUtils = window.__boardLegacyPreviewMediaUtils;
  const boardLegacyRuntimeContextUtils = window.__boardLegacyRuntimeContextUtils;
  const boardLegacyRuntimeDialogHandlersUtils = window.__boardLegacyRuntimeDialogHandlersUtils;
  const boardLegacyRuntimeFoundationUtils = window.__boardLegacyRuntimeFoundationUtils;
  const boardLegacyRuntimeItemLoadHandlersUtils = window.__boardLegacyRuntimeItemLoadHandlersUtils;
  const boardLegacyRuntimeLifecycleUtils = window.__boardLegacyRuntimeLifecycleUtils;
  const boardLegacyRuntimePreviewListHandlersUtils = window.__boardLegacyRuntimePreviewListHandlersUtils;
  const boardLegacyRuntimeCaptureHandlersUtils = window.__boardLegacyRuntimeCaptureHandlersUtils;
  const boardLegacySearchUtils = window.__boardLegacySearchUtils;
  const boardLegacySelectionUtils = window.__boardLegacySelectionUtils;
  const boardLegacyStatusUtils = window.__boardLegacyStatusUtils;

  const runtimeFoundation = boardLegacyRuntimeFoundationUtils.createRuntimeFoundation({
    state,
    nodes: runtimeNodes,
    searchUtils: boardLegacySearchUtils,
    statusUtils: boardLegacyStatusUtils,
    previewLayoutUtils: boardLegacyPreviewLayoutUtils,
    runtimeContextUtils: boardLegacyRuntimeContextUtils,
    boardUtils: boardLegacyUtils,
    previewMediaUtils: boardLegacyPreviewMediaUtils,
    selectionUtils: boardLegacySelectionUtils,
    bindingsUtils: boardLegacyBindingsUtils,
    dialogUtils: boardLegacyDialogUtils,
    volumeCurveGamma: VOLUME_CURVE_GAMMA,
    previewMediaMinHeightPx: PREVIEW_MEDIA_MIN_HEIGHT_PX,
    previewSideLayoutBreakpointPx: PREVIEW_SIDE_LAYOUT_BREAKPOINT_PX,
    setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
    clearTimeout: (timeoutId) => window.clearTimeout(timeoutId),
    requestAnimationFrame: (callback) => window.requestAnimationFrame(callback),
  });

  const { setStatus, clearStatus, clearSearchDebounce, clearPreviewMessageAutosave, schedulePreviewLayoutRefresh } =
    runtimeFoundation;
  const { refreshPreviewLayoutObserverTargets, resetPreviewMessageEditor, runtimeContextHelpers } = runtimeFoundation;

  const { normalizeMediaKind, toSafeDateLabel, toCardTitle, toMessagePreview, isHttpUrl, buildAuthedUrl } =
    runtimeContextHelpers;
  const { applyPreviewVolume, findSelectedItem, getItemShortcuts, resolvePreviewMessageItemTitle, isCaptureUiReady } =
    runtimeContextHelpers;

  const dialogHandlers = boardLegacyRuntimeDialogHandlersUtils.createRuntimeDialogHandlers({
    state,
    nodes: runtimeNodes,
    isHttpUrl,
    toCardTitle,
    setStatusError: (message) => {
      setStatus(message, 'error');
    },
  });

  const {
    isAddDialogReady,
    closeAddDialog,
    openAddDialog,
    isDeleteDialogReady,
    closeDeleteDialog,
    openDeleteDialog,
    isRenameDialogReady,
    closeRenameDialog,
    openRenameDialog,
    bindBackdropClose,
  } = dialogHandlers;

  let patchItemMetadata = async () => {};
  let applyLocalItemMetadata = () => false;
  let triggerItem = async () => {};
  let renameItem = async () => {};
  let beginCaptureForItem = () => {};
  let clearShortcutForItem = async () => {};
  let deleteItem = async () => {};

  const previewListHandlers = boardLegacyRuntimePreviewListHandlersUtils.createRuntimePreviewListHandlers({
    state,
    nodes: runtimeNodes,
    defaultAutosaveDelayMs: PREVIEW_MESSAGE_AUTOSAVE_DEBOUNCE_MS,
    setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
    clearTimeout: (timeoutId) => window.clearTimeout(timeoutId),
    clearPreviewMessageAutosave,
    resetPreviewMessageEditor,
    schedulePreviewLayoutRefresh,
    refreshPreviewLayoutObserverTargets,
    clearStatus,
    setStatusError: (message) => {
      setStatus(message, 'error');
    },
    findSelectedItem,
    getItemShortcuts,
    toCardTitle,
    toSafeDateLabel,
    toMessagePreview,
    normalizeMediaKind,
    buildAuthedUrl,
    applyPreviewVolume,
    resolvePreviewMessageItemTitle,
    getPatchItemMetadata: () => patchItemMetadata,
    getApplyLocalItemMetadata: () => applyLocalItemMetadata,
    getActionHandlers: () => ({
      triggerItem,
      renameItem,
      beginCaptureForItem,
      clearShortcutForItem,
      deleteItem,
    }),
  });

  const {
    hasPendingPreviewMessageChanges,
    schedulePreviewMessageAutosave,
    savePreviewMessage,
    flushPreviewMessageChanges,
    flushPreviewMessageOnLifecycleExit,
    renderPreview,
    renderList,
  } = previewListHandlers;

  const itemLoadHandlers = boardLegacyRuntimeItemLoadHandlersUtils.createRuntimeItemLoadHandlers({
    state,
    nodes: runtimeNodes,
    buildAuthedUrl,
    clearSearchDebounce,
    flushPreviewMessageChanges,
    clearStatus,
    setStatusSuccess: (message) => {
      setStatus(message, 'success');
    },
    setStatusError: (message) => {
      setStatus(message, 'error');
    },
    renderList,
    openRenameDialog,
    openDeleteDialog,
    instantSearchDebounceMs: INSTANT_SEARCH_DEBOUNCE_MS,
    setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
    setMemeBindings: (payload) => window.livechatOverlay.setMemeBindings(payload),
    triggerMeme: (payload) => window.livechatOverlay.triggerMeme(payload),
    stopMemePlayback: () => window.livechatOverlay.stopMemePlayback(),
  });

  const { loadItemsAndRender, runSearchNow, scheduleInstantSearch, addItemFromLink, persistBindings, stopCurrentPlayback } =
    itemLoadHandlers;
  ({
    patchItemMetadata,
    applyLocalItemMetadata,
    triggerItem,
    clearShortcutForItem,
    renameItem,
    deleteItem,
  } = itemLoadHandlers);

  const captureHandlers = boardLegacyRuntimeCaptureHandlersUtils.createRuntimeCaptureHandlers({
    state,
    nodes: runtimeNodes,
    isCaptureUiReady,
    persistBindings,
    renderList,
    setStatusSuccess: (message) => {
      setStatus(message, 'success');
    },
    setStatusError: (message) => {
      setStatus(message, 'error');
    },
  });

  const { refreshCaptureUi, endCapture, commitCapturedShortcut, formatAcceleratorDisplay, keyEventToDisplay, keyEventToAccelerator } =
    captureHandlers;
  ({ beginCaptureForItem } = captureHandlers);

  const bootstrap = async () => {
    await boardLegacyRuntimeLifecycleUtils.registerWireupAndBootstrap({
      getConfig: () => window.livechatOverlay.getConfig(),
      getMemeBindings: () => window.livechatOverlay.getMemeBindings(),
      onSettings: (listener) => {
        window.livechatOverlay.onSettings(listener);
      },
      state,
      nodes: runtimeNodes,
      keyEventToAccelerator,
      keyEventToDisplay,
      formatAcceleratorDisplay,
      endCapture,
      clearStatus,
      setStatusSuccess: (message) => {
        setStatus(message, 'success');
      },
      closeAddDialog,
      closeRenameDialog,
      closeDeleteDialog,
      refreshCaptureUi,
      commitCapturedShortcut,
      isCaptureUiReady,
      isAddDialogReady,
      isRenameDialogReady,
      isDeleteDialogReady,
      isHttpUrl,
      setStatusError: (message) => {
        setStatus(message, 'error');
      },
      bindBackdropClose,
      openAddDialog,
      addItemFromLink,
      runSearchNow,
      scheduleInstantSearch,
      flushPreviewMessageChanges: () => flushPreviewMessageChanges(),
      loadItemsAndRender,
      stopCurrentPlayback,
      flushPreviewMessageOnLifecycleExit,
      schedulePreviewLayoutRefresh,
      applyPreviewVolume,
      refreshPreviewLayoutObserverTargets,
    });
  };

  bootstrap().catch((error) => {
    setStatus(error?.message || 'Initialisation mème board impossible.', 'error');
  });
})();
