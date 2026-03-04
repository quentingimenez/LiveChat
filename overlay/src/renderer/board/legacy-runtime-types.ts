export interface BoardRuntimeState {
  config: unknown;
  bindings: unknown;
  items: unknown[];
  total: number;
  selectedId: string | null;
  selectedItem: unknown;
  captureItemId: string | null;
  capturePendingAccelerator: string | null;
  capturePendingDisplay: string;
  resolveAddDialog: unknown;
  resolveRenameDialog: unknown;
  resolveDeleteDialog: unknown;
  search: string;
  searchDebounceTimeoutId: number | null;
  itemsLoadRequestId: number;
  previewMediaKey: string | null;
  previewMessageAutosaveTimeoutId: number | null;
  previewMessageEditor: unknown;
  previewLayoutRafId: number | null;
  previewLayoutObserver: ResizeObserver | null;
}

export interface BoardRuntimeNodes {
  statusNode: unknown;
  countPillNode: unknown;
  itemsListNode: unknown;
  previewStageNode: unknown;
  selectedTitleNode: unknown;
  selectedMetaNode: unknown;
  searchForm: unknown;
  searchInput: unknown;
  stopPlaybackButton: unknown;
  refreshButton: unknown;
  addLinkButton: unknown;
  captureOverlay: unknown;
  captureCurrentNode: unknown;
  captureCancelButton: unknown;
  captureSaveButton: unknown;
  addOverlay: unknown;
  addForm: unknown;
  addUrlInput: unknown;
  addTitleInput: unknown;
  addMessageInput: unknown;
  addRefreshInput: unknown;
  addCancelButton: unknown;
  renameOverlay: unknown;
  renameForm: unknown;
  renameInput: unknown;
  renameMessageInput: unknown;
  renameCancelButton: unknown;
  deleteOverlay: unknown;
  deleteMessage: unknown;
  deleteCancelButton: unknown;
  deleteConfirmButton: unknown;
}
