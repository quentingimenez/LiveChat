// Caractérisation du maintien pour composer : délai, annulations, seuil de
// déplacement, déclenchement unique et retrait de la bulle après envoi.
// Les trois sites partagent ce comportement ; la table évite trois copies.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { loadSite, pointer, sites, trusted, type SiteFixture } from './testSupport';

const fixtures: SiteFixture[] = [];
afterEach(() => {
  while (fixtures.length) {
    fixtures.pop()!.dispose();
  }
});

const load = async (site: (typeof sites)[number]): Promise<SiteFixture> => {
  const fixture = await loadSite(site);
  fixtures.push(fixture);
  expect(fixture.buttons.some((button) => button.className.includes('lce-button'))).toBe(true);
  return fixture;
};

const popover = (doc: Document): HTMLElement | null =>
  doc.querySelector('.lce-compose-popover');

describe.each(sites)('maintien pour composer — %s', (site) => {
  it('ouvre la bulle après le délai de maintien, pas avant', async () => {
    vi.useFakeTimers();
    const { win, doc, buttons } = await load(site);
    const button = buttons.find((candidate) => candidate.className.includes('lce-button'))!;

    button.dispatchEvent(pointer(win, 'pointerdown', { x: 100, y: 100 }));
    await vi.advanceTimersByTimeAsync(400);
    expect(popover(doc)).toBeNull();

    await vi.advanceTimersByTimeAsync(200);
    expect(popover(doc)).not.toBeNull();
    expect(popover(doc)!.querySelector('.lce-compose-popover-textarea')).not.toBeNull();

    button.dispatchEvent(pointer(win, 'pointerup', { x: 100, y: 100 }));
  });

  it('annule sur pointerup et pointercancel avant le délai', async () => {
    vi.useFakeTimers();
    const { win, doc, buttons } = await load(site);
    const button = buttons.find((candidate) => candidate.className.includes('lce-button'))!;

    button.dispatchEvent(pointer(win, 'pointerdown', { x: 100, y: 100 }));
    await vi.advanceTimersByTimeAsync(100);
    button.dispatchEvent(pointer(win, 'pointerup', { x: 100, y: 100 }));
    await vi.advanceTimersByTimeAsync(1000);
    expect(popover(doc)).toBeNull();

    button.dispatchEvent(pointer(win, 'pointerdown', { x: 100, y: 100 }));
    await vi.advanceTimersByTimeAsync(100);
    button.dispatchEvent(pointer(win, 'pointercancel', { x: 100, y: 100 }));
    await vi.advanceTimersByTimeAsync(1000);
    expect(popover(doc)).toBeNull();
  });

  it('annule quand le pointeur se déplace au-delà du seuil', async () => {
    vi.useFakeTimers();
    const { win, doc, buttons } = await load(site);
    const button = buttons.find((candidate) => candidate.className.includes('lce-button'))!;

    // 13 px > seuil de 12 : le maintien est annulé.
    button.dispatchEvent(pointer(win, 'pointerdown', { x: 100, y: 100 }));
    button.dispatchEvent(pointer(win, 'pointermove', { x: 113, y: 100 }));
    await vi.advanceTimersByTimeAsync(1000);
    expect(popover(doc)).toBeNull();

    // 12 px = seuil : le maintien n'est pas annulé.
    button.dispatchEvent(pointer(win, 'pointerdown', { x: 100, y: 100 }));
    button.dispatchEvent(pointer(win, 'pointermove', { x: 112, y: 100 }));
    await vi.advanceTimersByTimeAsync(1000);
    expect(popover(doc)).not.toBeNull();
    button.dispatchEvent(pointer(win, 'pointerup', { x: 113, y: 100 }));
  });

  it('ignore un pointerdown non trusted', async () => {
    vi.useFakeTimers();
    const { win, doc, buttons } = await load(site);
    const button = buttons.find((candidate) => candidate.className.includes('lce-button'))!;
    const fake = new win.MouseEvent('pointerdown', { bubbles: true, clientX: 100, clientY: 100 });
    Object.defineProperty(fake, 'pointerId', { value: 1 });
    Object.defineProperty(fake, 'pointerType', { value: 'mouse' });

    button.dispatchEvent(fake as unknown as Event);
    await vi.advanceTimersByTimeAsync(1000);
    expect(popover(doc)).toBeNull();
  });

  it('n’ouvre qu’une seule bulle après deux maintiens consécutifs', async () => {
    vi.useFakeTimers();
    const { win, doc, buttons } = await load(site);
    const button = buttons.find((candidate) => candidate.className.includes('lce-button'))!;

    button.dispatchEvent(pointer(win, 'pointerdown', { x: 100, y: 100 }));
    await vi.advanceTimersByTimeAsync(600);
    button.dispatchEvent(pointer(win, 'pointerup', { x: 100, y: 100 }));
    button.dispatchEvent(pointer(win, 'pointerdown', { x: 100, y: 100 }));
    await vi.advanceTimersByTimeAsync(600);
    button.dispatchEvent(pointer(win, 'pointerup', { x: 100, y: 100 }));

    expect(doc.querySelectorAll('.lce-compose-popover')).toHaveLength(1);
  });

  it('retire la bulle avec le bouton Annuler', async () => {
    vi.useFakeTimers();
    const { win, doc, buttons } = await load(site);
    const button = buttons.find((candidate) => candidate.className.includes('lce-button'))!;

    button.dispatchEvent(pointer(win, 'pointerdown', { x: 100, y: 100 }));
    await vi.advanceTimersByTimeAsync(600);
    button.dispatchEvent(pointer(win, 'pointerup', { x: 100, y: 100 }));
    const cancel = popover(doc)!.querySelector('.lce-compose-popover-button-cancel') as HTMLButtonElement;
    cancel.dispatchEvent(trusted(new win.MouseEvent('click', { bubbles: true })) as Event);
    expect(popover(doc)).toBeNull();
  });

  it('ignore un clic non trusted sur le bouton Envoyer', async () => {
    vi.useFakeTimers();
    const { win, doc, buttons, runtimeSend } = await load(site);
    const button = buttons.find((candidate) => candidate.className.includes('lce-button'))!;

    button.dispatchEvent(pointer(win, 'pointerdown', { x: 100, y: 100 }));
    await vi.advanceTimersByTimeAsync(600);
    button.dispatchEvent(pointer(win, 'pointerup', { x: 100, y: 100 }));
    const submit = popover(doc)!.querySelector('.lce-compose-popover-button-submit') as HTMLButtonElement;
    submit.dispatchEvent(new win.MouseEvent('click', { bubbles: true }) as unknown as Event);
    await vi.advanceTimersByTimeAsync(20);

    expect(popover(doc)).not.toBeNull();
    expect(runtimeSend.mock.calls.some((call: unknown[]) =>
      (call[0] as { type: string }).type === 'lce/send-compose',
    )).toBe(false);
  });
});

// L'URL de la vidéo active est résolue différemment sur TikTok (message runtime
// dédié) : le submit réussi n'est caractérisé que là où l'URL vient du DOM/URL
// de la page. Le nettoyage de la bulle après annulation est couvert partout.
describe.each(['twitter', 'youtube'] as const)('envoi depuis la bulle — %s', (site) => {
  it('ferme la bulle et envoie le texte avec un clic trusted', async () => {
    vi.useFakeTimers();
    const { win, doc, buttons, runtimeSend } = await load(site);
    const button = buttons.find((candidate) => candidate.className.includes('lce-button'))!;

    button.dispatchEvent(pointer(win, 'pointerdown', { x: 100, y: 100 }));
    await vi.advanceTimersByTimeAsync(600);
    button.dispatchEvent(pointer(win, 'pointerup', { x: 100, y: 100 }));
    const bubble = popover(doc)!;
    bubble.querySelector('textarea')!.value = 'un message';
    const submit = bubble.querySelector('.lce-compose-popover-button-submit') as HTMLButtonElement;
    submit.dispatchEvent(trusted(new win.MouseEvent('click', { bubbles: true })));
    await vi.advanceTimersByTimeAsync(20);

    expect(popover(doc)).toBeNull();
    expect(runtimeSend.mock.calls.some((call: unknown[]) =>
      (call[0] as { type: string }).type === 'lce/send-compose',
    )).toBe(true);
  });
});
