function getRequiredElement(root, id, isExpectedElement) {
    const element = root.getElementById(id);
    if (!element || !isExpectedElement(element)) {
        throw new Error(`Missing required element: #${id}`);
    }
    return element;
}
export function createPairingDom(root) {
    const form = getRequiredElement(root, 'pairing-form', (element) => {
        return element instanceof HTMLFormElement;
    });
    const statusNode = getRequiredElement(root, 'status', (element) => {
        return element instanceof HTMLElement;
    });
    const submitButton = getRequiredElement(root, 'submit-button', (element) => {
        return element instanceof HTMLButtonElement;
    });
    const serverUrlInput = getRequiredElement(root, 'server-url', (element) => {
        return element instanceof HTMLInputElement;
    });
    const pairingCodeInput = getRequiredElement(root, 'pairing-code', (element) => {
        return element instanceof HTMLInputElement;
    });
    return {
        form,
        serverUrlInput,
        pairingCodeInput,
        setStatus(message, kind = '') {
            statusNode.textContent = message;
            statusNode.className = kind;
        },
        setBusy(busy) {
            submitButton.disabled = busy;
        }
    };
}
