import type { BoardLegacyBindingsUtils } from './legacy-bindings-utils';
import type { BoardLegacyDialogUtils } from './legacy-dialog-utils';
import type { BoardLegacyPreviewLayoutUtils } from './legacy-preview-layout-utils';
import type { BoardLegacyPreviewMediaUtils } from './legacy-preview-media-utils';
import type { BoardLegacyRuntimeContextUtils } from './legacy-runtime-context-utils';
import type { BoardLegacySearchUtils } from './legacy-search-utils';
import type { BoardLegacySelectionUtils } from './legacy-selection-utils';
import type { BoardLegacyStatusUtils } from './legacy-status-utils';
import type { BoardLegacyUtils } from './legacy-utils';
import type { BoardRuntimeNodes, BoardRuntimeState } from './legacy-runtime-types';

interface RuntimeFoundationParams {
  state: BoardRuntimeState;
  nodes: BoardRuntimeNodes;
  searchUtils: BoardLegacySearchUtils;
  statusUtils: BoardLegacyStatusUtils;
  previewLayoutUtils: BoardLegacyPreviewLayoutUtils;
  runtimeContextUtils: BoardLegacyRuntimeContextUtils;
  boardUtils: BoardLegacyUtils;
  previewMediaUtils: BoardLegacyPreviewMediaUtils;
  selectionUtils: BoardLegacySelectionUtils;
  bindingsUtils: BoardLegacyBindingsUtils;
  dialogUtils: BoardLegacyDialogUtils;
  volumeCurveGamma: number;
  previewMediaMinHeightPx: number;
  previewSideLayoutBreakpointPx: number;
  setTimeout: (callback: () => void, delayMs: number) => number;
  clearTimeout: (timeoutId: number) => void;
  requestAnimationFrame: (callback: () => void) => number;
}

export interface BoardRuntimeContextHelpers {
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

interface RuntimeFoundation {
  setStatus(message: string, variant?: 'success' | 'error', options?: { autoDismissMs?: number }): void;
  clearStatus(): void;
  clearSearchDebounce(): void;
  clearPreviewMessageAutosave(): void;
  schedulePreviewLayoutRefresh(): void;
  refreshPreviewLayoutObserverTargets(): void;
  resetPreviewMessageEditor(): void;
  runtimeContextHelpers: BoardRuntimeContextHelpers;
}

export interface BoardLegacyRuntimeFoundationUtils {
  createRuntimeFoundation(params: RuntimeFoundationParams): RuntimeFoundation;
}

export function createBoardLegacyRuntimeFoundationUtils(): BoardLegacyRuntimeFoundationUtils {
  function createRuntimeFoundation(params: RuntimeFoundationParams): RuntimeFoundation {
    const statusController = params.statusUtils.createStatusController({
      statusNode: params.nodes.statusNode,
      setTimer: params.setTimeout,
      clearTimer: params.clearTimeout,
    });

    function setStatus(
      message: string,
      variant: 'success' | 'error' = 'success',
      options: { autoDismissMs?: number } = {}
    ): void {
      statusController.setStatus(message, variant, options);
    }

    function clearStatus(): void {
      statusController.clearStatus();
    }

    function clearSearchDebounce(): void {
      params.state.searchDebounceTimeoutId = params.searchUtils.clearDebounceTimer(
        params.state.searchDebounceTimeoutId,
        params.clearTimeout
      );
    }

    function clearPreviewMessageAutosave(): void {
      if (params.state.previewMessageAutosaveTimeoutId === null) {
        return;
      }

      params.clearTimeout(params.state.previewMessageAutosaveTimeoutId);
      params.state.previewMessageAutosaveTimeoutId = null;
    }

    function fitPreviewMediaToStage(): void {
      params.previewLayoutUtils.fitPreviewMediaToStage({
        previewStageNode: params.nodes.previewStageNode,
        minHeightPx: params.previewMediaMinHeightPx,
        sideLayoutBreakpointPx: params.previewSideLayoutBreakpointPx,
      });
    }

    function schedulePreviewLayoutRefresh(): void {
      if (params.state.previewLayoutRafId !== null) {
        return;
      }

      params.state.previewLayoutRafId = params.requestAnimationFrame(() => {
        params.state.previewLayoutRafId = null;
        fitPreviewMediaToStage();
      });
    }

    function refreshPreviewLayoutObserverTargets(): void {
      params.previewLayoutUtils.refreshPreviewLayoutObserverTargets({
        previewLayoutObserver: params.state.previewLayoutObserver,
        previewStageNode: params.nodes.previewStageNode,
      });
    }

    function resetPreviewMessageEditor(): void {
      clearPreviewMessageAutosave();
      params.state.previewMessageEditor = null;
    }

    const runtimeContextHelpers = params.runtimeContextUtils.createRuntimeContextHelpers({
      boardUtils: params.boardUtils,
      previewMediaUtils: params.previewMediaUtils,
      selectionUtils: params.selectionUtils,
      bindingsUtils: params.bindingsUtils,
      dialogUtils: params.dialogUtils,
      state: params.state,
      previewStageNode: params.nodes.previewStageNode,
      volumeCurveGamma: params.volumeCurveGamma,
      captureOverlay: params.nodes.captureOverlay,
      captureCurrentNode: params.nodes.captureCurrentNode,
      captureCancelButton: params.nodes.captureCancelButton,
      captureSaveButton: params.nodes.captureSaveButton,
    });

    return {
      setStatus,
      clearStatus,
      clearSearchDebounce,
      clearPreviewMessageAutosave,
      schedulePreviewLayoutRefresh,
      refreshPreviewLayoutObserverTargets,
      resetPreviewMessageEditor,
      runtimeContextHelpers,
    };
  }

  return {
    createRuntimeFoundation,
  };
}
