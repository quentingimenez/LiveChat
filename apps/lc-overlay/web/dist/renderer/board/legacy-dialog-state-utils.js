export function createBoardLegacyDialogStateUtils() {
    function hideDialog(overlayNode) {
        if (overlayNode instanceof HTMLElement) {
            overlayNode.classList.add('hidden');
        }
    }
    function closeDialogWithResolver(params) {
        hideDialog(params.overlayNode);
        const resolver = params.getResolver();
        if (typeof resolver !== 'function') {
            return;
        }
        params.setResolver(null);
        resolver(params.value);
    }
    function createResolverPromise(setResolver) {
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
