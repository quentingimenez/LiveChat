interface PreviewMediaContext {
  mediaKey: string | null;
  mediaKind: string | null;
  mediaUrl: string | null;
}

interface PreviewSelectedItemLike {
  id?: unknown;
  title?: unknown;
  message?: unknown;
  media?: {
    kind?: unknown;
  };
}

interface PreviewTextTarget {
  textContent: string;
}

interface RenderPreviewParams {
  findSelectedItem: () => unknown;
  getPreviewMessageEditor: () => unknown;
  hasPendingPreviewMessageChanges: (editor?: unknown) => boolean;
  clearPreviewMessageAutosave: () => void;
  savePreviewMessage: (options: {
    editor?: unknown;
    waitForInFlight?: boolean;
    keepalive?: boolean;
    silent?: boolean;
  }) => Promise<boolean>;
  resetPreviewMessageEditor: () => void;
  setPreviewMediaKey: (nextKey: string | null) => void;
  renderEmptyPreviewState: (params: {
    previewStageNode: unknown;
    selectedTitleNode: unknown;
    selectedMetaNode: unknown;
    createEmptyStateNode: (message: string) => HTMLElement;
    emptyText: string;
    emptyMetaText: string;
  }) => void;
  previewStageNode: unknown;
  selectedTitleNode: unknown;
  selectedMetaNode: PreviewTextTarget;
  createEmptyStateNode: (message: string) => HTMLElement;
  refreshPreviewLayoutObserverTargets: () => void;
  shouldResetPreviewMessageEditor: (params: { previewMessageEditor: unknown; selectedItemId: unknown }) => boolean;
  buildPreviewMediaContext: (params: {
    selectedItem: unknown;
    normalizeMediaKind: (value: unknown) => string;
    buildAuthedUrl: (pathname: unknown, options?: unknown) => string;
  }) => PreviewMediaContext;
  getPreviewMediaKey: () => string | null;
  mountPreviewMediaIfNeeded: (params: unknown) => void;
  setPreviewMessageEditor: (nextEditor: unknown) => void;
  schedulePreviewLayoutRefresh: () => void;
  schedulePreviewMessageAutosave: (delayMs?: number) => void;
  createPreviewMediaNode: (params: {
    mediaKind: string | null;
    mediaUrl: string | null;
    title: string;
  }) => HTMLElement;
  createPreviewMessageEditor: (params: {
    selectedItemId: string;
    initialMessage: string;
    onInput: () => void;
  }) => unknown;
  createPreviewControls: (params: unknown) => HTMLElement;
  createPreviewControlHandlers: () => unknown;
  applyPreviewVolume: () => void;
  getItemShortcuts: (itemId: unknown) => string[];
  toCardTitle: (item: unknown) => string;
  toMessagePreview: (value: unknown, maxLength?: number) => string;
  buildPreviewMetaText: (params: {
    mediaKind: unknown;
    hasMessage: boolean;
    shortcuts: string[];
  }) => string;
  normalizeMediaKind: (value: unknown) => string;
  buildAuthedUrl: (pathname: unknown, options?: unknown) => string;
}

export interface BoardLegacyPreviewRenderOrchestrationUtils {
  renderPreview(params: RenderPreviewParams): void;
}

function toItemRecord(value: unknown): PreviewSelectedItemLike {
  if (!value || typeof value !== 'object') {
    return {};
  }

  return value as PreviewSelectedItemLike;
}

function toItemId(value: unknown): string {
  return `${value || ''}`;
}

export function createBoardLegacyPreviewRenderOrchestrationUtils(): BoardLegacyPreviewRenderOrchestrationUtils {
  function renderPreview(params: RenderPreviewParams): void {
    const selectedItem = params.findSelectedItem();

    if (!selectedItem) {
      if (params.hasPendingPreviewMessageChanges()) {
        const detachedEditor = params.getPreviewMessageEditor();
        params.clearPreviewMessageAutosave();
        void params.savePreviewMessage({
          editor: detachedEditor,
          waitForInFlight: false,
          keepalive: true,
          silent: true,
        });
      }
      params.resetPreviewMessageEditor();
      params.setPreviewMediaKey(null);
      params.renderEmptyPreviewState({
        previewStageNode: params.previewStageNode,
        selectedTitleNode: params.selectedTitleNode,
        selectedMetaNode: params.selectedMetaNode,
        createEmptyStateNode: params.createEmptyStateNode,
        emptyText: 'Sélectionne un mème pour voir son média et gérer son raccourci.',
        emptyMetaText: 'Aucun élément sélectionné.',
      });
      params.refreshPreviewLayoutObserverTargets();
      return;
    }

    const selectedItemRecord = toItemRecord(selectedItem);

    if (
      params.shouldResetPreviewMessageEditor({
        previewMessageEditor: params.getPreviewMessageEditor(),
        selectedItemId: selectedItemRecord.id,
      })
    ) {
      const detachedEditor = params.getPreviewMessageEditor();
      if (params.hasPendingPreviewMessageChanges(detachedEditor)) {
        params.clearPreviewMessageAutosave();
        void params.savePreviewMessage({
          editor: detachedEditor,
          waitForInFlight: false,
          keepalive: true,
          silent: true,
        });
      }
      params.resetPreviewMessageEditor();
    }

    const previewMedia = params.buildPreviewMediaContext({
      selectedItem,
      normalizeMediaKind: params.normalizeMediaKind,
      buildAuthedUrl: params.buildAuthedUrl,
    });

    params.mountPreviewMediaIfNeeded({
      previewStageNode: params.previewStageNode,
      currentPreviewMediaKey: params.getPreviewMediaKey(),
      nextPreviewMediaKey: previewMedia.mediaKey,
      mediaKind: previewMedia.mediaKind,
      mediaUrl: previewMedia.mediaUrl,
      title: params.toCardTitle(selectedItem),
      selectedItemId: toItemId(selectedItemRecord.id),
      initialMessage: `${selectedItemRecord.message || ''}`.trim(),
      getPreviewMessageEditor: params.getPreviewMessageEditor,
      setPreviewMessageEditor: (nextEditor: unknown) => {
        params.setPreviewMessageEditor(nextEditor);
      },
      setPreviewMediaKey: (nextKey: string | null) => {
        params.setPreviewMediaKey(nextKey);
      },
      schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
      schedulePreviewMessageAutosave: params.schedulePreviewMessageAutosave,
      createPreviewMediaNode: params.createPreviewMediaNode,
      createPreviewMessageEditor: params.createPreviewMessageEditor,
      createPreviewControls: params.createPreviewControls,
      createPreviewControlHandlers: params.createPreviewControlHandlers,
    });
    params.applyPreviewVolume();
    params.refreshPreviewLayoutObserverTargets();
    params.schedulePreviewLayoutRefresh();

    const shortcuts = params.getItemShortcuts(selectedItemRecord.id);
    const hasMessage = params.toMessagePreview(selectedItemRecord.message).length > 0;
    if (params.selectedTitleNode instanceof HTMLElement) {
      params.selectedTitleNode.textContent = `${params.toCardTitle(selectedItem)}`;
    }
    params.selectedMetaNode.textContent = params.buildPreviewMetaText({
      mediaKind: selectedItemRecord.media?.kind,
      hasMessage,
      shortcuts,
    });
  }

  return {
    renderPreview,
  };
}
