interface CloseDialogWithResolverParams<T> {
  overlayNode: unknown;
  value: T;
  getResolver: () => unknown;
  setResolver: (nextResolver: unknown) => void;
}

export interface BoardLegacyDialogStateUtils {
  hideDialog(overlayNode: unknown): void;
  closeDialogWithResolver<T>(params: CloseDialogWithResolverParams<T>): void;
  createResolverPromise<T>(
    setResolver: (nextResolver: ((value: T) => void) | null) => void,
  ): Promise<T>;
}

export function createBoardLegacyDialogStateUtils(): BoardLegacyDialogStateUtils {
  function hideDialog(overlayNode: unknown): void {
    if (overlayNode instanceof HTMLElement) {
      overlayNode.classList.add('hidden');
    }
  }

  function closeDialogWithResolver<T>(params: CloseDialogWithResolverParams<T>): void {
    hideDialog(params.overlayNode);

    const resolver = params.getResolver();
    if (typeof resolver !== 'function') {
      return;
    }

    params.setResolver(null);
    (resolver as (value: T) => void)(params.value);
  }

  function createResolverPromise<T>(
    setResolver: (nextResolver: ((value: T) => void) | null) => void,
  ): Promise<T> {
    return new Promise((resolve) => {
      setResolver(resolve);
    });
  }

  return {
    hideDialog,
    closeDialogWithResolver,
    createResolverPromise,
  };
}
