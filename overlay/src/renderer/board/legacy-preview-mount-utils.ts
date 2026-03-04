interface PreviewMessageEditorState {
  itemId: string;
  inputNode: unknown;
  lastSavedMessage: string;
  saving: boolean;
  inFlightSavePromise: Promise<boolean> | null;
}

interface CreatedPreviewMessageEditor {
  rootNode: Node;
  inputNode: unknown;
  initialMessage: string;
}

interface CreatePreviewMediaNodeParams {
  mediaKind: string;
  mediaUrl: string;
  title: string;
  onMediaReady: () => void;
}

interface CreatePreviewMessageEditorParams {
  initialMessage: string;
  onInput: (inputNode: unknown) => void;
  onBlur: (inputNode: unknown) => void;
}

interface MountPreviewMediaParams {
  previewStageNode: unknown;
  currentPreviewMediaKey: string | null;
  nextPreviewMediaKey: string;
  mediaKind: string;
  mediaUrl: string;
  title: string;
  selectedItemId: string;
  initialMessage: string;
  getPreviewMessageEditor: () => PreviewMessageEditorState | null;
  setPreviewMessageEditor: (nextEditor: PreviewMessageEditorState) => void;
  setPreviewMediaKey: (nextKey: string) => void;
  schedulePreviewLayoutRefresh: () => void;
  schedulePreviewMessageAutosave: (delayMs?: number) => void;
  createPreviewMediaNode: (params: CreatePreviewMediaNodeParams) => Node;
  createPreviewMessageEditor: (params: CreatePreviewMessageEditorParams) => CreatedPreviewMessageEditor;
  createPreviewControls: (handlers: unknown) => Node;
  createPreviewControlHandlers: () => unknown;
}

export interface BoardLegacyPreviewMountUtils {
  mountPreviewMediaIfNeeded(params: MountPreviewMediaParams): void;
}

export function createBoardLegacyPreviewMountUtils(): BoardLegacyPreviewMountUtils {
  function mountPreviewMediaIfNeeded(params: MountPreviewMediaParams): void {
    if (params.currentPreviewMediaKey === params.nextPreviewMediaKey) {
      return;
    }

    params.setPreviewMediaKey(params.nextPreviewMediaKey);

    if (!(params.previewStageNode instanceof HTMLElement)) {
      return;
    }

    params.previewStageNode.innerHTML = '';
    params.previewStageNode.classList.toggle('preview-stage-visual', params.mediaKind !== 'audio');

    const mediaNode = params.createPreviewMediaNode({
      mediaKind: params.mediaKind,
      mediaUrl: params.mediaUrl,
      title: params.title,
      onMediaReady: params.schedulePreviewLayoutRefresh,
    });

    const messageEditor = params.createPreviewMessageEditor({
      initialMessage: params.initialMessage,
      onInput: (inputNode) => {
        const editor = params.getPreviewMessageEditor();
        if (!editor || editor.inputNode !== inputNode) {
          return;
        }

        params.schedulePreviewMessageAutosave();
      },
      onBlur: (inputNode) => {
        const editor = params.getPreviewMessageEditor();
        if (!editor || editor.inputNode !== inputNode) {
          return;
        }

        params.schedulePreviewMessageAutosave(220);
      },
    });

    params.setPreviewMessageEditor({
      itemId: `${params.selectedItemId || ''}`.trim(),
      inputNode: messageEditor.inputNode,
      lastSavedMessage: `${messageEditor.initialMessage || ''}`.trim(),
      saving: false,
      inFlightSavePromise: null,
    });

    const controlsNode = params.createPreviewControls(params.createPreviewControlHandlers());

    params.previewStageNode.appendChild(mediaNode);
    params.previewStageNode.appendChild(messageEditor.rootNode);
    params.previewStageNode.appendChild(controlsNode);
  }

  return {
    mountPreviewMediaIfNeeded,
  };
}
