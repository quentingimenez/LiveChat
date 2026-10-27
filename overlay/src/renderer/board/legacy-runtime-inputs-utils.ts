import type { BoardRuntimeNodes, BoardRuntimeState } from './legacy-runtime-types.js';

interface BoardRuntimeConstants {
  volumeCurveGamma: number;
  instantSearchDebounceMs: number;
  previewMessageAutosaveDebounceMs: number;
  previewMediaMinHeightPx: number;
  previewSideLayoutBreakpointPx: number;
}

interface BoardRuntimeInputs {
  nodes: BoardRuntimeNodes;
  state: BoardRuntimeState;
  constants: BoardRuntimeConstants;
}

export interface BoardLegacyRuntimeInputsUtils {
  createRuntimeInputs(documentRef: Document): BoardRuntimeInputs;
}

function queryNode(documentRef: Document, id: string): HTMLElement | null {
  return documentRef.getElementById(id);
}

export function createBoardLegacyRuntimeInputsUtils(): BoardLegacyRuntimeInputsUtils {
  function createRuntimeInputs(documentRef: Document): BoardRuntimeInputs {
    const nodes: BoardRuntimeNodes = {
      statusNode: queryNode(documentRef, 'status'),
      countPillNode: queryNode(documentRef, 'count-pill'),
      itemsListNode: queryNode(documentRef, 'items-list'),
      previewStageNode: queryNode(documentRef, 'preview-stage'),
      selectedTitleNode: queryNode(documentRef, 'selected-title'),
      selectedMetaNode: queryNode(documentRef, 'selected-meta'),
      searchForm: queryNode(documentRef, 'search-form'),
      searchInput: queryNode(documentRef, 'search-input'),
      stopPlaybackButton: queryNode(documentRef, 'stop-playback-button'),
      refreshButton: queryNode(documentRef, 'refresh-button'),
      addLinkButton: queryNode(documentRef, 'add-link-button'),
      captureOverlay: queryNode(documentRef, 'capture-overlay'),
      captureCurrentNode: queryNode(documentRef, 'capture-current'),
      captureCancelButton: queryNode(documentRef, 'capture-cancel'),
      captureSaveButton: queryNode(documentRef, 'capture-save'),
      addOverlay: queryNode(documentRef, 'add-overlay'),
      addForm: queryNode(documentRef, 'add-form'),
      addUrlInput: queryNode(documentRef, 'add-url-input'),
      addTitleInput: queryNode(documentRef, 'add-title-input'),
      addMessageInput: queryNode(documentRef, 'add-message-input'),
      addRefreshInput: queryNode(documentRef, 'add-refresh-input'),
      addCancelButton: queryNode(documentRef, 'add-cancel'),
      renameOverlay: queryNode(documentRef, 'rename-overlay'),
      renameForm: queryNode(documentRef, 'rename-form'),
      renameInput: queryNode(documentRef, 'rename-input'),
      renameMessageInput: queryNode(documentRef, 'rename-message-input'),
      renameCancelButton: queryNode(documentRef, 'rename-cancel'),
      deleteOverlay: queryNode(documentRef, 'delete-overlay'),
      deleteMessage: queryNode(documentRef, 'delete-message'),
      deleteCancelButton: queryNode(documentRef, 'delete-cancel'),
      deleteConfirmButton: queryNode(documentRef, 'delete-confirm'),
    };

    const state: BoardRuntimeState = {
      config: null,
      bindings: {},
      items: [],
      total: 0,
      selectedId: null,
      selectedItem: null,
      captureItemId: null,
      capturePendingAccelerator: null,
      capturePendingDisplay: '',
      resolveAddDialog: null,
      resolveRenameDialog: null,
      resolveDeleteDialog: null,
      search: '',
      searchDebounceTimeoutId: null,
      itemsLoadRequestId: 0,
      previewMediaKey: null,
      previewMessageAutosaveTimeoutId: null,
      previewMessageEditor: null,
      previewLayoutRafId: null,
      previewLayoutObserver: null,
    };

    const constants: BoardRuntimeConstants = {
      volumeCurveGamma: 2.2,
      instantSearchDebounceMs: 180,
      previewMessageAutosaveDebounceMs: 700,
      previewMediaMinHeightPx: 160,
      previewSideLayoutBreakpointPx: 1093,
    };

    return {
      nodes,
      state,
      constants,
    };
  }

  return {
    createRuntimeInputs,
  };
}
