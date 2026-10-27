interface RenderEmptyPreviewStateParams {
  previewStageNode: unknown;
  selectedTitleNode: unknown;
  selectedMetaNode: unknown;
  createEmptyStateNode: (params: { text: string }) => Node;
  emptyText: string;
  emptyMetaText: string;
}

interface ShouldResetPreviewMessageEditorParams {
  previewMessageEditor: unknown;
  selectedItemId: unknown;
}

interface BuildPreviewMediaContextParams {
  selectedItem: unknown;
  normalizeMediaKind: (value: unknown) => string;
  buildAuthedUrl: (pathname: string) => URL;
}

interface PreviewMediaContext {
  mediaKind: string;
  mediaUrl: string;
  mediaKey: string;
}

export interface BoardLegacyPreviewRenderStateUtils {
  renderEmptyPreviewState(params: RenderEmptyPreviewStateParams): void;
  shouldResetPreviewMessageEditor(params: ShouldResetPreviewMessageEditorParams): boolean;
  buildPreviewMediaContext(params: BuildPreviewMediaContextParams): PreviewMediaContext;
}

function toTrimmedString(value: unknown): string {
  return `${value || ''}`.trim();
}

function toRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

export function createBoardLegacyPreviewRenderStateUtils(): BoardLegacyPreviewRenderStateUtils {
  function renderEmptyPreviewState(params: RenderEmptyPreviewStateParams): void {
    if (params.previewStageNode instanceof HTMLElement) {
      params.previewStageNode.innerHTML = '';
      params.previewStageNode.classList.remove('preview-stage-visual');
      params.previewStageNode.appendChild(
        params.createEmptyStateNode({
          text: params.emptyText,
        }),
      );
    }

    if (params.selectedTitleNode instanceof HTMLElement) {
      params.selectedTitleNode.textContent = '';
    }

    if (params.selectedMetaNode instanceof HTMLElement) {
      params.selectedMetaNode.textContent = params.emptyMetaText;
    }
  }

  function shouldResetPreviewMessageEditor(params: ShouldResetPreviewMessageEditorParams): boolean {
    const editorRecord = toRecord(params.previewMessageEditor);
    const editorItemId = toTrimmedString(editorRecord.itemId);
    const selectedItemId = toTrimmedString(params.selectedItemId);

    if (!editorItemId) {
      return false;
    }

    return editorItemId !== selectedItemId;
  }

  function buildPreviewMediaContext(params: BuildPreviewMediaContextParams): PreviewMediaContext {
    const selectedRecord = toRecord(params.selectedItem);
    const itemId = toTrimmedString(selectedRecord.id);
    const mediaAssetId = toTrimmedString(selectedRecord.mediaAssetId);
    const mediaRecord = toRecord(selectedRecord.media);
    const mediaKind = params.normalizeMediaKind(mediaRecord.kind);
    const mediaUrl = params.buildAuthedUrl(`/overlay/meme-board/media/${mediaAssetId}`).toString();
    const mediaKey = `${itemId}|${mediaAssetId}|${mediaKind}|${mediaUrl}`;

    return {
      mediaKind,
      mediaUrl,
      mediaKey,
    };
  }

  return {
    renderEmptyPreviewState,
    shouldResetPreviewMessageEditor,
    buildPreviewMediaContext,
  };
}
