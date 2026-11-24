// Caractérisation des toasts injectés : conteneur unique, remplacement de
// l'ancien message, niveau, retrait après délai, et filtrage des messages
// runtime reçus. Commun aux trois sites, testé une seule fois par table.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { loadSite, sites, trusted, type SiteFixture } from './testSupport';

const fixtures: SiteFixture[] = [];
afterEach(() => {
  while (fixtures.length) {
    fixtures.pop()!.dispose();
  }
});

/** Diffuse à tous les listeners runtime : seul celui des toasts réagit à ce type. */
const showToast = (fixture: SiteFixture, level: string, message: string): void => {
  for (const call of fixture.onMessage.mock.calls) {
    const listener = call[0];
    if (typeof listener === 'function') {
      (listener as (payload: unknown) => void)({ type: 'lce/show-toast', level, message });
    }
  }
};

const loadWithButton = async (site: (typeof sites)[number]): Promise<SiteFixture> => {
  const fixture = await loadSite(site);
  fixtures.push(fixture);
  expect(fixture.buttons.some((button) => button.className.includes('lce-button'))).toBe(true);
  return fixture;
};

describe.each(sites)('toast injecté — %s', (site) => {
  it('crée un conteneur unique et remplace le toast précédent', async () => {
    const { doc, buttons, win } = await loadWithButton(site);
    showToast(fixtures[fixtures.length - 1], 'success', 'premier');

    const containers = doc.querySelectorAll('#lce-toast-container');
    expect(containers).toHaveLength(1);
    expect(containers[0].textContent).toBe('premier');

    // Une action produit un autre toast : même conteneur, un seul enfant.
    const button = buttons.find((candidate) => candidate.className.includes('lce-button'))!;
    button.dispatchEvent(trusted(new win.MouseEvent('click', { bubbles: true })) as Event);
    await Promise.resolve();
    await Promise.resolve();
    expect(doc.querySelectorAll('#lce-toast-container')).toHaveLength(1);
    expect(doc.querySelectorAll('#lce-toast-container .lce-toast')).toHaveLength(1);
  });

  it('classe le toast par niveau et le retire après 3,5 s', async () => {
    vi.useFakeTimers();
    const { doc } = await loadWithButton(site);
    const fixture = fixtures[fixtures.length - 1];
    showToast(fixture, 'error', 'échec');

    let toast = doc.querySelector('#lce-toast-container .lce-toast')!;
    expect(toast.textContent).toBe('échec');
    expect(toast.className).toContain('lce-toast-error');

    vi.advanceTimersByTime(3400);
    expect(doc.querySelector('#lce-toast-container .lce-toast')).not.toBeNull();
    vi.advanceTimersByTime(200);
    expect(doc.querySelector('#lce-toast-container .lce-toast')).toBeNull();

    showToast(fixture, 'info', 'nouveau');
    toast = doc.querySelector('#lce-toast-container .lce-toast')!;
    expect(toast.className).toContain('lce-toast-info');
    expect(doc.querySelectorAll('#lce-toast-container')).toHaveLength(1);
  });

  it('accepte un payload show-toast valide et ignore un payload étranger', async () => {
    const { doc } = await loadWithButton(site);
    const fixture = fixtures[fixtures.length - 1];
    showToast(fixture, 'success', 'bravo');
    let toast = doc.querySelector('#lce-toast-container .lce-toast');
    expect(toast!.textContent).toBe('bravo');
    expect(toast!.className).toContain('lce-toast-success');

    const listener = fixture.onMessage.mock.calls
      .map((call: unknown[]) => call[0])
      .find((candidate: unknown) => typeof candidate === 'function') as (payload: unknown) => void;
    listener({ type: 'autre-message', message: 'ignoré' });
    listener({ type: 'lce/show-toast', message: '' });
    listener({ type: 'lce/show-toast', message: '   ' });
    expect(doc.querySelector('#lce-toast-container .lce-toast')!.textContent).toBe('bravo');
  });
});

