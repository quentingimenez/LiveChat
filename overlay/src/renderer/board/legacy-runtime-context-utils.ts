interface RuntimeStateLike {
  config: unknown;
  items: unknown[];
  selectedId: string | null;
  selectedItem: unknown;
  bindings: unknown;
}

interface CreateRuntimeContextHelpersParams {
  boardUtils: {
    normalizeMediaKind(value: unknown): string;
    toSafeDateLabel(value: unknown): string;
    toCardTitle(item: unknown): string;
    toMessagePreview(value: unknown, maxLength?: number): string;
    isHttpUrl(value: unknown): boolean;
  };
  previewMediaUtils: {
    buildAuthedUrl(params: { config: unknown; pathname: string; options?: Record<string, unknown> }): URL;
    applyPreviewVolume(params: { root: unknown; linearVolume: unknown; gamma: number }): void;
  };
  selectionUtils: {
    resolveSelectedItemForPreview(params: {
      items: unknown;
      selectedId: unknown;
      selectedItem: unknown;
    }): {
      selectedItem: unknown;
      syncedSelectedItem: unknown;
    };
  };
  bindingsUtils: {
    getItemShortcuts(bindings: unknown, itemId: unknown): string[];
  };
  dialogUtils: {
    isCaptureUiReady(nodes: {
      captureOverlay: unknown;
      captureCurrentNode: unknown;
      captureCancelButton: unknown;
      captureSaveButton: unknown;
    }): boolean;
  };
  state: RuntimeStateLike;
  previewStageNode: unknown;
  volumeCurveGamma: number;
  captureOverlay: unknown;
  captureCurrentNode: unknown;
  captureCancelButton: unknown;
  captureSaveButton: unknown;
}

interface RuntimeContextHelpers {
  normalizeMediaKind(value: unknown): string;
  toSafeDateLabel(value: unknown): string;
  toCardTitle(item: unknown): string;
  toMessagePreview(value: unknown, maxLength?: number): string;
  isHttpUrl(value: unknown): boolean;
  buildAuthedUrl(pathname: string, options?: Record<string, unknown>): URL;
  applyPreviewVolume(root?: unknown): void;
  findSelectedItem(): unknown;
  getItemShortcuts(itemId: unknown): string[];
  resolvePreviewMessageItemTitle(itemId: unknown): string;
  isCaptureUiReady(): boolean;
}

export interface BoardLegacyRuntimeContextUtils {
  createRuntimeContextHelpers(params: CreateRuntimeContextHelpersParams): RuntimeContextHelpers;
}

export function createBoardLegacyRuntimeContextUtils(): BoardLegacyRuntimeContextUtils {
  function createRuntimeContextHelpers(params: CreateRuntimeContextHelpersParams): RuntimeContextHelpers {
    function normalizeMediaKind(value: unknown): string {
      return params.boardUtils.normalizeMediaKind(value);
    }

    function toSafeDateLabel(value: unknown): string {
      return params.boardUtils.toSafeDateLabel(value);
    }

    function toCardTitle(item: unknown): string {
      return params.boardUtils.toCardTitle(item);
    }

    function toMessagePreview(value: unknown, maxLength = 120): string {
      return params.boardUtils.toMessagePreview(value, maxLength);
    }

    function isHttpUrl(value: unknown): boolean {
      return params.boardUtils.isHttpUrl(value);
    }

    function buildAuthedUrl(pathname: string, options: Record<string, unknown> = {}): URL {
      return params.previewMediaUtils.buildAuthedUrl({
        config: params.state.config,
        pathname,
        options,
      });
    }

    function applyPreviewVolume(root = params.previewStageNode): void {
      params.previewMediaUtils.applyPreviewVolume({
        root,
        linearVolume: (params.state.config as { volume?: unknown } | null)?.volume,
        gamma: params.volumeCurveGamma,
      });
    }

    function findSelectedItem(): unknown {
      const resolved = params.selectionUtils.resolveSelectedItemForPreview({
        items: params.state.items,
        selectedId: params.state.selectedId,
        selectedItem: params.state.selectedItem,
      });
      params.state.selectedItem = resolved.syncedSelectedItem;
      return resolved.selectedItem;
    }

    function getItemShortcuts(itemId: unknown): string[] {
      return params.bindingsUtils.getItemShortcuts(params.state.bindings, itemId);
    }

    function resolvePreviewMessageItemTitle(itemId: unknown): string {
      const selected = findSelectedItem() as { id?: unknown; title?: unknown } | null;
      if (selected && `${selected.id || ''}` === `${itemId || ''}`) {
        return `${selected.title || ''}`.trim();
      }

      const inList = (Array.isArray(params.state.items) ? params.state.items : []).find(
        (entry) => `${(entry as { id?: unknown } | null)?.id || ''}` === `${itemId || ''}`,
      ) as { title?: unknown } | undefined;

      return `${inList?.title || ''}`.trim();
    }

    function isCaptureUiReady(): boolean {
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
