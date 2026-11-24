// Aide de test (non expédiée) : monte un content script de site dans un Window
// happy-dom isolé. Aucun comportement de production n'est défini ici.
import { Window } from 'happy-dom';
import { vi } from 'vitest';

export type Site = 'twitter' | 'youtube' | 'tiktok';

export interface SiteFixture {
  win: Window;
  doc: Document;
  buttons: HTMLButtonElement[];
  onMessage: ReturnType<typeof vi.fn>;
  runtimeSend: ReturnType<typeof vi.fn>;
  dispose: () => void;
}

const SITE_SETUP: Record<Site, { url: string; html: string; marker: string }> = {
  twitter: {
    url: 'https://twitter.com/x/status/123',
    html: '<article data-testid="tweet"><div role="group"></div></article>',
    marker: '[data-lce-twitter-button]',
  },
  youtube: {
    url: 'https://www.youtube.com/shorts/abc',
    html: '<reel-action-bar-view-model></reel-action-bar-view-model>',
    marker: '[data-lce-youtube-button]',
  },
  tiktok: {
    url: 'https://www.tiktok.com/@u/video/123',
    html: '<div class="action-col" data-item-id="123456"><div data-e2e="browse-share-icon"></div><div data-e2e="browse-comment-icon"></div><div data-e2e="browse-like-icon"></div></div>',
    marker: '#lce-tiktok-inline-button',
  },
};

/** Vue assouplie sur le Window happy-dom (types DOM incompatibles avec TS lib.dom). */
interface LooseWindow {
  setInterval: (fn: () => void, ms?: number) => number;
  setTimeout: (fn: () => void, ms?: number) => number;
  clearTimeout: (id: number) => void;
  requestAnimationFrame: (cb: () => void) => number;
  MutationObserver: unknown;
  Element: { prototype: { getBoundingClientRect: () => DOMRect } };
  [key: string]: unknown;
}

interface LooseDocument {
  createElement: (tag: string, ...rest: unknown[]) => HTMLElement;
  querySelector: <T>(selector: string) => T | null;
}

/**
 * Importe un script de site, capture le bouton injecté et les écouteurs runtime.
 * Les timers du Window délèguent aux timers globaux (donc à ceux de vi.useFakeTimers).
 */
export const loadSite = async (site: Site): Promise<SiteFixture> => {
  vi.resetModules();
  const setup = SITE_SETUP[site];
  const win = new Window({ url: setup.url });
  const w = win as unknown as LooseWindow;
  const doc = win.document as unknown as LooseDocument;
  win.document.body.innerHTML = setup.html;

  const rect = {
    x: 100, y: 100, left: 100, right: 130, top: 100, bottom: 300,
    width: 30, height: 200, toJSON: () => ({}),
  } as DOMRect;
  w.Element.prototype.getBoundingClientRect = () => rect;

  const buttons: HTMLButtonElement[] = [];
  const origCreate = win.document.createElement.bind(win.document) as LooseDocument['createElement'];
  doc.createElement = (tag: string, ...rest: unknown[]) => {
    const element = origCreate(tag, ...rest);
    if (tag.toLowerCase() === 'button') {
      buttons.push(element as HTMLButtonElement);
    }
    return element;
  };

  // Le scanner de fond est désactivé : le scan initial, synchrone, suffit.
  // Les callbacks enregistrés sont conservés pour les libérer en fin de test.
  const intervals: Array<() => void> = [];
  w.setInterval = (fn: () => void) => {
    intervals.push(fn);
    return intervals.length;
  };
  w.setTimeout = (fn: () => void, ms?: number) =>
    (globalThis.setTimeout as (f: () => void, t: number) => number)(fn, ms ?? 0);
  w.clearTimeout = (id: number) =>
    (globalThis.clearTimeout as (t: number) => void)(id);

  const onMessage = vi.fn();
  const runtimeSend = vi.fn(async (message: { type: string }) => {
    if (message.type === 'lce/get-auth-state') {
      return { ok: true, hasSettings: true };
    }
    if (message.type === 'lce/theme:get') {
      return { ok: true, preference: 'dark' };
    }
    return { ok: true, message: 'ok' };
  });

  vi.stubGlobal('window', win);
  vi.stubGlobal('document', win.document);
  vi.stubGlobal('Event', win.Event);
  vi.stubGlobal('MouseEvent', win.MouseEvent);
  vi.stubGlobal('KeyboardEvent', win.KeyboardEvent);
  vi.stubGlobal('MutationObserver', w.MutationObserver);
  for (const key of [
    'HTMLElement', 'HTMLDivElement', 'HTMLAnchorElement', 'HTMLVideoElement', 'Element',
  ] as const) {
    vi.stubGlobal(key, w[key]);
  }
  vi.stubGlobal('browser', {
    runtime: { sendMessage: runtimeSend, onMessage: { addListener: onMessage } },
  });

  const loaders: Record<Site, () => Promise<unknown>> = {
    twitter: () => import('./twitter'),
    youtube: () => import('./youtube'),
    tiktok: () => import('./tiktok'),
  };
  await loaders[site]();
  // Le scan initial passe par requestAnimationFrame puis par la vérification
  // asynchrone de l'autorisation : on attend quelques frames.
  for (let i = 0; i < 6; i += 1) {
    await new Promise<void>((resolve) => w.requestAnimationFrame(() => resolve()));
    for (let j = 0; j < 12; j += 1) {
      await Promise.resolve();
    }
  }
  const button = doc.querySelector<HTMLButtonElement>(setup.marker);
  if (button) {
    buttons.push(button);
  }

  return {
    win,
    doc: win.document as unknown as Document,
    buttons,
    onMessage,
    runtimeSend,
    dispose: () => {
      intervals.forEach((fn) => {
        try {
          fn();
        } catch {
          // Le DOM a déjà été démonté : le callback n'a plus rien à nettoyer.
        }
      });
      vi.useRealTimers();
      vi.unstubAllGlobals();
      win.happyDOM.cancelAsync();
    },
  };
};

/** Événement marqué isTrusted : la page ne peut pas en fabriquer. */
export const trusted = (event: unknown): Event => {
  Object.defineProperty(event, 'isTrusted', { value: true, configurable: true });
  return event as Event;
};

/** PointerEvent synthétique (absent de happy-dom). */
export const pointer = (
  win: Window,
  type: string,
  coords: { x: number; y: number; pointerId?: number; pointerType?: string; button?: number },
): Event => {
  const event = new win.MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: coords.x,
    clientY: coords.y,
    button: coords.button ?? 0,
  });
  Object.defineProperty(event, 'pointerId', { value: coords.pointerId ?? 1 });
  Object.defineProperty(event, 'pointerType', { value: coords.pointerType ?? 'mouse' });
  return trusted(event);
};

export const sites: Site[] = ['twitter', 'youtube', 'tiktok'];
