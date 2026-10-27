interface OpenAddDialogNodes {
  addOverlay: unknown;
  addUrlInput: unknown;
  addTitleInput: unknown;
  addMessageInput: unknown;
  addRefreshInput: unknown;
}

interface OpenRenameDialogNodes {
  renameInput: unknown;
  renameMessageInput: unknown;
  renameOverlay: unknown;
}

interface OpenDeleteDialogNodes {
  deleteMessage: unknown;
  deleteOverlay: unknown;
  deleteConfirmButton: unknown;
}

export interface AddDialogPayload {
  url: string;
  title: string;
  message: string;
  forceRefresh: boolean;
}

export interface RenameDialogPayload {
  title: string;
  message: string;
}

interface OpenAddDialogParams {
  isReady: boolean;
  nodes: OpenAddDialogNodes;
  isHttpUrl: (value: string) => boolean;
  setStatusError: (message: string) => void;
  closeDialog: (value: AddDialogPayload | null) => void;
  createResolverPromise: () => Promise<AddDialogPayload | null>;
}

interface OpenRenameDialogParams {
  title: string;
  message: string;
  isReady: boolean;
  nodes: OpenRenameDialogNodes;
  closeDialog: (value: RenameDialogPayload | null) => void;
  createResolverPromise: () => Promise<RenameDialogPayload | null>;
}

interface OpenDeleteDialogParams {
  title: string;
  isReady: boolean;
  nodes: OpenDeleteDialogNodes;
  closeDialog: (value: boolean) => void;
  createResolverPromise: () => Promise<boolean>;
}

export interface BoardLegacyDialogFlowUtils {
  openAddDialog(params: OpenAddDialogParams): Promise<AddDialogPayload | null>;
  openRenameDialog(params: OpenRenameDialogParams): Promise<RenameDialogPayload | null>;
  openDeleteDialog(params: OpenDeleteDialogParams): Promise<boolean>;
}

function toTrimmedString(value: unknown): string {
  return `${value || ''}`.trim();
}

export function createBoardLegacyDialogFlowUtils(): BoardLegacyDialogFlowUtils {
  function openAddDialog(params: OpenAddDialogParams): Promise<AddDialogPayload | null> {
    if (!params.isReady) {
      const fallbackUrl = window.prompt('Lien du mème :');

      if (fallbackUrl === null) {
        return Promise.resolve(null);
      }

      const normalizedFallbackUrl = fallbackUrl.trim();

      if (!params.isHttpUrl(normalizedFallbackUrl)) {
        params.setStatusError('Lien invalide. Utilise une URL complète en http(s).');
        return Promise.resolve(null);
      }

      const fallbackTitle = window.prompt('Nom du mème (optionnel) :', '') || '';
      const fallbackMessage = window.prompt('Message overlay (optionnel):', '') || '';

      return Promise.resolve({
        url: normalizedFallbackUrl,
        title: fallbackTitle.trim(),
        message: fallbackMessage.trim(),
        forceRefresh: false,
      });
    }

    const addOverlay = params.nodes.addOverlay;
    const addUrlInput = params.nodes.addUrlInput;
    const addTitleInput = params.nodes.addTitleInput;
    const addMessageInput = params.nodes.addMessageInput;
    const addRefreshInput = params.nodes.addRefreshInput;

    if (
      !(addOverlay instanceof HTMLElement) ||
      !(addUrlInput instanceof HTMLInputElement) ||
      !(addTitleInput instanceof HTMLInputElement) ||
      !(addMessageInput instanceof HTMLTextAreaElement) ||
      !(addRefreshInput instanceof HTMLInputElement)
    ) {
      return Promise.resolve(null);
    }

    params.closeDialog(null);
    addUrlInput.value = '';
    addTitleInput.value = '';
    addMessageInput.value = '';
    addRefreshInput.checked = false;
    addOverlay.classList.remove('hidden');
    addUrlInput.focus();

    return params.createResolverPromise();
  }

  function openRenameDialog(params: OpenRenameDialogParams): Promise<RenameDialogPayload | null> {
    if (!params.isReady) {
      const fallbackTitle = window.prompt('Nom du mème (laisser vide pour enlever le nom) :', params.title);

      if (fallbackTitle === null) {
        return Promise.resolve(null);
      }

      const fallbackMessage = window.prompt(
        'Message overlay (laisser vide pour enlever le message):',
        `${params.message || ''}`.trim(),
      );

      if (fallbackMessage === null) {
        return Promise.resolve(null);
      }

      return Promise.resolve({
        title: fallbackTitle,
        message: fallbackMessage,
      });
    }

    const renameInput = params.nodes.renameInput;
    const renameMessageInput = params.nodes.renameMessageInput;
    const renameOverlay = params.nodes.renameOverlay;

    if (
      !(renameInput instanceof HTMLInputElement) ||
      !(renameMessageInput instanceof HTMLTextAreaElement) ||
      !(renameOverlay instanceof HTMLElement)
    ) {
      return Promise.resolve(null);
    }

    params.closeDialog(null);

    renameInput.value = params.title;
    renameMessageInput.value = toTrimmedString(params.message);
    renameOverlay.classList.remove('hidden');
    renameInput.focus();
    renameInput.select();

    return params.createResolverPromise();
  }

  function openDeleteDialog(params: OpenDeleteDialogParams): Promise<boolean> {
    if (!params.isReady) {
      return Promise.resolve(window.confirm(`Supprimer "${params.title}" de la mème board ?`));
    }

    const deleteMessage = params.nodes.deleteMessage;
    const deleteOverlay = params.nodes.deleteOverlay;
    const deleteConfirmButton = params.nodes.deleteConfirmButton;

    if (
      !(deleteMessage instanceof HTMLElement) ||
      !(deleteOverlay instanceof HTMLElement) ||
      !(deleteConfirmButton instanceof HTMLElement)
    ) {
      return Promise.resolve(false);
    }

    params.closeDialog(false);
    deleteMessage.textContent = `Tu vas supprimer "${params.title}" de la mème board. Cette action est irréversible.`;
    deleteOverlay.classList.remove('hidden');
    deleteConfirmButton.focus();

    return params.createResolverPromise();
  }

  return {
    openAddDialog,
    openRenameDialog,
    openDeleteDialog,
  };
}
