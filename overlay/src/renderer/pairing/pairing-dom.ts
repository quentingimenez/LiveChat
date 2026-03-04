export type PairingStatusKind = '' | 'success' | 'error';

export interface PairingDom {
  form: HTMLFormElement;
  serverUrlInput: HTMLInputElement;
  pairingCodeInput: HTMLInputElement;
  setStatus(message: string, kind?: PairingStatusKind): void;
  setBusy(busy: boolean): void;
}

function getRequiredElement<TElement extends Element>(
  root: Document,
  id: string,
  isExpectedElement: (element: Element) => element is TElement
): TElement {
  const element = root.getElementById(id);

  if (!element || !isExpectedElement(element)) {
    throw new Error(`Missing required element: #${id}`);
  }

  return element;
}

export function createPairingDom(root: Document): PairingDom {
  const form = getRequiredElement(root, 'pairing-form', (element): element is HTMLFormElement => {
    return element instanceof HTMLFormElement;
  });

  const statusNode = getRequiredElement(root, 'status', (element): element is HTMLElement => {
    return element instanceof HTMLElement;
  });

  const submitButton = getRequiredElement(root, 'submit-button', (element): element is HTMLButtonElement => {
    return element instanceof HTMLButtonElement;
  });

  const serverUrlInput = getRequiredElement(root, 'server-url', (element): element is HTMLInputElement => {
    return element instanceof HTMLInputElement;
  });

  const pairingCodeInput = getRequiredElement(root, 'pairing-code', (element): element is HTMLInputElement => {
    return element instanceof HTMLInputElement;
  });

  return {
    form,
    serverUrlInput,
    pairingCodeInput,
    setStatus(message: string, kind: PairingStatusKind = ''): void {
      statusNode.textContent = message;
      statusNode.className = kind;
    },
    setBusy(busy: boolean): void {
      submitButton.disabled = busy;
    }
  };
}
