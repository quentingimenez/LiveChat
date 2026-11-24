// Caractérisation de la modale montée : état aria, champ durée tel que vu par
// l'utilisateur (invariant ex-test ?raw), validation, payload soumis,
// événements non trusted et fermeture. Une table évite trois copies.
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { INVALID_DURATION_MESSAGE } from '../lib/duration';
import type { OpenComposeModalParams } from './composeModal';

type Site = 'twitter' | 'youtube' | 'tiktok';
const sites: Site[] = ['twitter', 'youtube', 'tiktok'];

const trusted = (event: unknown): Event => {
  Object.defineProperty(event as object, 'isTrusted', { value: true });
  return event as Event;
};

const dispatch = (target: EventTarget, event: unknown): void => {
  target.dispatchEvent(event as Event);
};

type ModalFixture = {
  win: Window;
  doc: Window['document'];
  overlay: Element;
  form: HTMLFormElement;
  duration: HTMLInputElement;
  text: HTMLTextAreaElement;
  status: Element;
  submitButton: HTMLButtonElement;
  onSubmit: ReturnType<typeof vi.fn>;
  reopen: () => void;
};

const mounted: Window[] = [];
afterEach(() => {
  while (mounted.length) {
    mounted.pop()!.happyDOM.cancelAsync();
  }
  vi.unstubAllGlobals();
});

const openModal = async (site: Site): Promise<ModalFixture> => {
  vi.resetModules();
  const win = new Window({ url: 'https://www.youtube.com/watch?v=abc' });
  mounted.push(win);
  vi.stubGlobal('window', win);
  vi.stubGlobal('document', win.document);
  vi.stubGlobal('Event', win.Event);
  vi.stubGlobal('MouseEvent', win.MouseEvent);
  vi.stubGlobal('KeyboardEvent', win.KeyboardEvent);
  vi.stubGlobal('setTimeout', win.setTimeout.bind(win));
  vi.stubGlobal('clearTimeout', win.clearTimeout.bind(win));
  const onSubmit = vi.fn(async () => ({ ok: true, message: 'Envoyé' }));
  const loaders: Record<Site, () => Promise<{
    openComposeModal: (params: OpenComposeModalParams) => void;
  }>> = {
    twitter: () => import('./composeModalTwitter'),
    youtube: () => import('./composeModalYoutube'),
    tiktok: () => import('./composeModalTiktok'),
  };
  const mod = await loaders[site]();
  const open = () => mod.openComposeModal({
    initialUrl: 'https://www.youtube.com/watch?v=abc',
    sourceLabel: 'YouTube',
    onSubmit,
    onSuccess: () => {},
    onError: () => {},
  });
  open();
  const doc = win.document;
  const query = <T>(selector: string): T => {
    const found = doc.querySelector(selector) as unknown as T | null;
    if (!found) {
      throw new Error(`Élément absent : ${selector}`);
    }
    return found;
  };
  const overlay = query<Element>('#lce-compose-modal-overlay');
  return {
    win,
    doc,
    overlay,
    form: query<HTMLFormElement>('[data-lce-compose-form]'),
    duration: query<HTMLInputElement>('#lce-compose-modal-duration'),
    text: query<HTMLTextAreaElement>('#lce-compose-modal-text'),
    status: query<Element>('[data-lce-compose-status]'),
    submitButton: query<HTMLButtonElement>('[data-lce-compose-submit]'),
    onSubmit,
    reopen: () => {
      dispatch(
        query<HTMLButtonElement>('[data-lce-compose-cancel]'),
        trusted(new win.MouseEvent('click', { bubbles: true })),
      );
      open();
    },
  };
};

describe.each(sites)('modale de composition — %s', (site) => {
  it('s’ouvre visible avec le dialogue aria et le champ durée attendu', async () => {
    const { overlay, doc, duration } = await openModal(site);
    expect(overlay.classList.contains('is-hidden')).toBe(false);
    expect(overlay.getAttribute('aria-hidden')).toBe('false');
    expect(doc.querySelector('[role="dialog"]')).not.toBeNull();
    expect(doc.body.classList.contains('lce-compose-modal-open')).toBe(true);

    // L'invariant défendu par le test ?raw, constaté sur le DOM monté.
    expect(duration.id).toBe('lce-compose-modal-duration');
    expect(duration.getAttribute('type')).toBe('text');
    expect(duration.getAttribute('inputmode')).toBe('numeric');
    expect(duration.hasAttribute('min')).toBe(false);
    expect(duration.hasAttribute('max')).toBe(false);
    expect(duration.hasAttribute('step')).toBe(false);
  });

  it('affiche le message de durée invalide sans soumettre', async () => {
    const { duration, status, onSubmit, win, submitButton } = await openModal(site);
    duration.value = 'abc';
    dispatch(submitButton, trusted(new win.MouseEvent('click', { bubbles: true })));
    expect(status.textContent).toBe(INVALID_DURATION_MESSAGE);
    expect(status.className).toContain('lce-compose-modal-status-error');
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('soumet le payload avec la durée parsed quand la durée est valide', async () => {
    const { duration, text, onSubmit, win, submitButton, overlay } = await openModal(site);
    duration.value = '42';
    text.value = 'mon message';
    dispatch(submitButton, trusted(new win.MouseEvent('click', { bubbles: true })));
    await Promise.resolve();
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][0]).toMatchObject({
      url: 'https://www.youtube.com/watch?v=abc',
      text: 'mon message',
      durationSec: 42,
      forceRefresh: false,
      saveToBoard: false,
    });
    expect(overlay.classList.contains('is-hidden')).toBe(true);
  });

  it('ignore un submit non trusted', async () => {
    const { form, onSubmit, duration, win, submitButton } = await openModal(site);
    duration.value = '10';
    dispatch(form, new win.Event('submit', { bubbles: true, cancelable: true }));
    dispatch(submitButton, new win.MouseEvent('click', { bubbles: true }));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('se ferme avec Échap et se rouvre avec un état réinitialisé', async () => {
    const { doc, duration, overlay, reopen, win } = await openModal(site);
    dispatch(doc as unknown as EventTarget, new win.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(overlay.classList.contains('is-hidden')).toBe(true);
    expect(overlay.getAttribute('aria-hidden')).toBe('true');
    expect(doc.body.classList.contains('lce-compose-modal-open')).toBe(false);

    duration.value = '99';
    reopen();
    expect(overlay.classList.contains('is-hidden')).toBe(false);
    expect(duration.value).toBe('');
  });
});

// Le champ du popup n'est pas monté par les tests de l'extension (popup.js est
// une entrée HTML autonome). On constate son invariant sur le DOM issu de
// popup.html, sans import ?raw : le fichier est lu puis monté par happy-dom.
describe('champ durée du popup', () => {
  it('reste un champ texte numérique sans contrainte native', async () => {
    const html = await readFile(new URL('../../popup.html', import.meta.url), 'utf8');
    const win = new Window({ url: 'moz-extension://popup/popup.html' });
    mounted.push(win);
    win.document.body.innerHTML = html;
    const field = win.document.getElementById('compose-duration') as HTMLInputElement | null;
    expect(field).not.toBeNull();
    expect(field!.getAttribute('type')).toBe('text');
    expect(field!.getAttribute('inputmode')).toBe('numeric');
    expect(field!.hasAttribute('min')).toBe(false);
    expect(field!.hasAttribute('max')).toBe(false);
    expect(field!.hasAttribute('step')).toBe(false);
  });
});
