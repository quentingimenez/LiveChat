import { Window } from 'happy-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { OpenComposeModalParams } from './composeModalYoutube';

/**
 * Ce que ce fichier protège : une page ne doit pas pouvoir envoyer un
 * compose via un `submit` ou un clic synthétique. Le formulaire neutralise
 * tous les `submit` (même trusted), le bouton n'est plus `type="submit"`, et
 * l'envoi passe uniquement par un clic ou un clavier trusted.
 */
const window = new Window({ url: 'https://www.youtube.com/watch?v=abc123' });

const modalModules = {
  YouTube: () => import('./composeModalYoutube'),
  Twitter: () => import('./composeModalTwitter'),
  TikTok: () => import('./composeModalTiktok'),
} as const;

type ModalName = keyof typeof modalModules;

const flush = async (): Promise<void> => {
  await new Promise((resolve) => setTimeout(resolve, 0));
};

const trusted = (event: unknown): Event => {
  Object.defineProperty(event as object, 'isTrusted', { value: true });
  return event as Event;
};

const dispatch = (target: EventTarget, event: unknown): void => {
  target.dispatchEvent(event as Event);
};

type ComposeModalModule = {
  openComposeModal: (params: OpenComposeModalParams) => void;
};

const loadModal = async (name: ModalName): Promise<ComposeModalModule> => {
  vi.resetModules();
  return (await modalModules[name]()) as ComposeModalModule;
};

const openModal = async (
  name: ModalName,
  onSubmit: OpenComposeModalParams['onSubmit'],
): Promise<{
  form: HTMLFormElement;
  submitButton: HTMLButtonElement;
  urlInput: HTMLInputElement;
}> => {
  const modal = await loadModal(name);

  modal.openComposeModal({
    initialUrl: 'https://www.youtube.com/watch?v=abc123',
    sourceLabel: name,
    onSubmit,
  });

  const form = window.document.querySelector('[data-lce-compose-form]') as unknown as HTMLFormElement | null;
  const submitButton = window.document.querySelector(
    '[data-lce-compose-submit]',
  ) as unknown as HTMLButtonElement | null;
  const urlInput = window.document.querySelector('#lce-compose-modal-url') as unknown as HTMLInputElement | null;

  if (!form || !submitButton || !urlInput) {
    throw new Error('Modal non initialisée');
  }

  return { form, submitButton, urlInput };
};

const send = vi.fn(async () => ({ ok: true, message: 'ok' }));

beforeEach(() => {
  vi.stubGlobal('window', window);
  vi.stubGlobal('document', window.document);
  vi.stubGlobal('Event', window.Event);
  vi.stubGlobal('MouseEvent', window.MouseEvent);
  vi.stubGlobal('KeyboardEvent', window.KeyboardEvent);
  window.document.body.innerHTML = '';
  send.mockClear();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe.each(Object.keys(modalModules) as ModalName[])('modal %s', (name) => {
  it('ignore un submit émis par requestSubmit', async () => {
    const { form } = await openModal(name, send);

    form.requestSubmit();
    await flush();

    expect(send).not.toHaveBeenCalled();
  });

  it('ignore un submit trusted forgé par la page', async () => {
    const { form } = await openModal(name, send);

    dispatch(
      form,
      trusted(
        new window.Event('submit', {
          bubbles: true,
          cancelable: true,
        }),
      ),
    );
    await flush();

    expect(send).not.toHaveBeenCalled();
  });

  it('ignore un clic synthétique sur le bouton Envoyer', async () => {
    const { submitButton } = await openModal(name, send);

    submitButton.click();
    await flush();

    expect(submitButton.type).toBe('button');
    expect(send).not.toHaveBeenCalled();
  });

  it('envoie sur un clic trusted du bouton Envoyer', async () => {
    const { submitButton } = await openModal(name, send);

    dispatch(
      submitButton,
      trusted(
        new window.MouseEvent('click', {
          bubbles: true,
          cancelable: true,
        }),
      ),
    );
    await flush();

    expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith(
      expect.objectContaining({
        url: 'https://www.youtube.com/watch?v=abc123',
        saveToBoard: false,
      }),
    );
  });

  it('envoie sur Entrée trusted dans le champ URL', async () => {
    const { urlInput } = await openModal(name, send);

    dispatch(
      urlInput,
      trusted(
        new window.KeyboardEvent('keydown', {
          key: 'Enter',
          bubbles: true,
          cancelable: true,
        }),
      ),
    );
    await flush();

    expect(send).toHaveBeenCalledTimes(1);
  });
});
