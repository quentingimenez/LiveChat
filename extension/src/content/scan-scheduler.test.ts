import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { startObservedScanner } from './scan-scheduler';

// Ce que ce fichier protège : les content scripts réagissent à un flux continu
// de mutations du DOM. Sans budget, un scan par frame (~60/s) sature le thread
// principal ; avec un budget mal foutu, le dernier état du DOM n'est jamais
// scanné. Les tests pilotent des timers et des frames simulés pour mesurer le
// nombre de scans réellement déclenchés, pas une valeur recopiée du module.

const FRAME_MS = 16;
const SECOND_MS = 1000;
const BUDGET_MS = 150;
const URL_POLL_MS = 700;

type RafDriver = {
  request: (callback: FrameRequestCallback) => number;
  cancel: (id: number) => void;
  flush: () => void;
  pending: () => number;
};

const createRafDriver = (): RafDriver => {
  const pending = new Map<number, FrameRequestCallback>();
  let nextId = 1;

  return {
    request(callback) {
      const id = nextId++;
      pending.set(id, callback);
      return id;
    },
    cancel(id) {
      pending.delete(id);
    },
    flush() {
      // Un callback inscrit pendant le flush attend la frame suivante.
      for (const [id, callback] of [...pending.entries()]) {
        if (!pending.has(id)) {
          continue;
        }
        pending.delete(id);
        callback(0);
      }
    },
    pending() {
      return pending.size;
    },
  };
};

type Environment = {
  raf: RafDriver;
  observers: FakeMutationObserver[];
  location: { href: string };
  clearTimeoutSpy: ReturnType<typeof vi.fn>;
  clearIntervalSpy: ReturnType<typeof vi.fn>;
  emitMutations: (count: number, before?: () => void) => void;
  setBody: (available: boolean) => void;
};

class FakeMutationObserver {
  readonly callback: MutationCallback;
  disconnected = false;
  // API minimale pour satisfaire le type DOM ; le test n'en a pas besoin.
  readonly takeRecords: MutationRecord[] = [];

  constructor(callback: MutationCallback) {
    this.callback = callback;
  }

  observe(): void {
    // Le vrai MutationObserver observe document.documentElement ; le stub n'a
    // pas besoin de réellement suivre le DOM, les tests émettent les mutations.
  }

  disconnect(): void {
    this.disconnected = true;
  }
}

const installEnvironment = (): Environment => {
  const raf = createRafDriver();
  const observers: FakeMutationObserver[] = [];
  const location = { href: 'https://www.youtube.com/watch?v=first' };
  const setTimeoutSpy = vi.fn((callback: () => void, delay?: number): number =>
    setTimeout(callback, delay) as unknown as number,
  );
  const clearTimeoutSpy = vi.fn((id: number): void =>
    clearTimeout(id as unknown as ReturnType<typeof setTimeout>),
  );
  const setIntervalSpy = vi.fn((callback: () => void, delay?: number): number =>
    setInterval(callback, delay) as unknown as number,
  );
  const clearIntervalSpy = vi.fn((id: number): void =>
    clearInterval(id as unknown as ReturnType<typeof setInterval>),
  );
  const body = {} as HTMLElement;
  const documentElement = {} as HTMLElement;
  const documentStub = {
    body: body as HTMLElement | null,
    documentElement,
  };

  vi.stubGlobal('window', {
    location,
    setTimeout: setTimeoutSpy,
    clearTimeout: clearTimeoutSpy,
    setInterval: setIntervalSpy,
    clearInterval: clearIntervalSpy,
    requestAnimationFrame: raf.request,
    cancelAnimationFrame: raf.cancel,
  });
  vi.stubGlobal('document', documentStub);
  vi.stubGlobal('MutationObserver', class extends FakeMutationObserver {
    constructor(callback: MutationCallback) {
      super(callback);
      observers.push(this);
    }
  });

  return {
    raf,
    observers,
    location,
    clearTimeoutSpy,
    clearIntervalSpy,
    emitMutations(count, before) {
      before?.();
      for (const observer of observers) {
        for (let index = 0; index < count; index += 1) {
          observer.callback([], observer as unknown as MutationObserver);
        }
      }
    },
    setBody(available) {
      documentStub.body = available ? body : null;
    },
  };
};

describe('planificateur de scan observé', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00Z'));
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('coalesce une rafale de 20 mutations en un seul scan', () => {
    const env = installEnvironment();
    const scan = vi.fn();
    const stop = startObservedScanner(scan);
    env.raf.flush();
    scan.mockClear();

    env.emitMutations(20);
    vi.advanceTimersByTime(BUDGET_MS);
    env.raf.flush();

    expect(scan).toHaveBeenCalledTimes(1);
    stop();
  });

  it('mesure le budget : au plus 7 scans par seconde sous un flux continu', () => {
    const env = installEnvironment();
    const scan = vi.fn();
    const stop = startObservedScanner(scan);
    env.raf.flush();
    scan.mockClear();

    const frames = Math.floor(SECOND_MS / FRAME_MS);
    for (let frame = 0; frame < frames; frame += 1) {
      env.emitMutations(3);
      vi.advanceTimersByTime(FRAME_MS);
      env.raf.flush();
    }
    const scansAfter = scan.mock.calls.length;

    // Référence : l'ancien planificateur local, un scan par frame. Ce driver
    // est indépendant du module testé ; il sert uniquement de mesure "avant".
    const legacyRaf = createRafDriver();
    let legacyQueued = false;
    let legacyScans = 0;
    const legacyQueue = (): void => {
      if (legacyQueued) {
        return;
      }
      legacyQueued = true;
      legacyRaf.request(() => {
        legacyQueued = false;
        legacyScans += 1;
      });
    };
    for (let frame = 0; frame < frames; frame += 1) {
      legacyQueue();
      vi.advanceTimersByTime(FRAME_MS);
      legacyRaf.flush();
    }

    console.log(
      `[B-07] scans sur ${frames * FRAME_MS} ms de mutations continues : ` +
        `avant = ${legacyScans} (~${Math.round((legacyScans * SECOND_MS) / (frames * FRAME_MS))}/s), ` +
        `après = ${scansAfter} (~${Math.round((scansAfter * SECOND_MS) / (frames * FRAME_MS))}/s)`,
    );

    expect(legacyScans).toBeGreaterThan(40);
    expect(scansAfter).toBeLessThanOrEqual(7);
    expect(scansAfter).toBeGreaterThan(1);
    stop();
  });

  it('garde la fenêtre de budget quand l’horloge système recule', () => {
    const env = installEnvironment();
    const scan = vi.fn();
    const stop = startObservedScanner(scan);
    env.raf.flush();
    scan.mockClear();

    // Un scan a lieu (fenêtre écoulée) : la mutation suivante tombe donc dans
    // la fenêtre de budget.
    vi.advanceTimersByTime(BUDGET_MS);
    env.emitMutations(1);
    env.raf.flush();
    expect(scan).toHaveBeenCalledTimes(1);

    // Deuxième mutation dans la fenêtre, puis l'heure système recule de 5 s
    // (correction NTP). Le scan final doit arriver dans la fenêtre, pas 5 s
    // plus tard : la fenêtre s'appuie sur une horloge monotone.
    env.emitMutations(1);
    vi.setSystemTime(new Date(Date.now() - 5000));
    vi.advanceTimersByTime(BUDGET_MS);
    env.raf.flush();

    expect(scan).toHaveBeenCalledTimes(2);
    stop();
  });

  it('scanne le dernier état du DOM après une rafale tombée dans la fenêtre', () => {
    const env = installEnvironment();
    const seen: string[] = [];
    let domState = 'initial';
    const scan = vi.fn(() => {
      seen.push(domState);
    });
    const stop = startObservedScanner(scan);
    env.raf.flush();

    env.emitMutations(1, () => {
      domState = 'intermediaire';
    });
    vi.advanceTimersByTime(FRAME_MS);
    env.raf.flush();
    env.emitMutations(1, () => {
      domState = 'final';
    });
    vi.advanceTimersByTime(BUDGET_MS);
    env.raf.flush();

    expect(seen).toEqual(['initial', 'final']);
    stop();
  });

  it('planifie un scan quand window.location.href change', () => {
    const env = installEnvironment();
    const scan = vi.fn();
    const stop = startObservedScanner(scan);
    env.raf.flush();

    env.location.href = 'https://www.youtube.com/watch?v=second';
    vi.advanceTimersByTime(URL_POLL_MS);
    env.raf.flush();
    expect(scan).toHaveBeenCalledTimes(2);

    // Sans changement d'URL, le timer dédié ne déclenche rien.
    vi.advanceTimersByTime(URL_POLL_MS);
    env.raf.flush();
    expect(scan).toHaveBeenCalledTimes(2);
    stop();
  });

  it('avale les erreurs synchrones et les rejections sans casser l’observateur', () => {
    const env = installEnvironment();
    const scan = vi
      .fn<() => void | Promise<void>>()
      .mockImplementationOnce(() => {
        throw new Error('panne synchrone');
      })
      .mockImplementationOnce(() => Promise.reject(new Error('panne asynchrone')))
      .mockImplementation(() => undefined);
    const stop = startObservedScanner(scan);

    expect(() => {
      env.raf.flush();
    }).not.toThrow();
    expect(scan).toHaveBeenCalledTimes(1);

    env.emitMutations(1);
    vi.advanceTimersByTime(BUDGET_MS);
    env.raf.flush();
    env.emitMutations(1);
    vi.advanceTimersByTime(BUDGET_MS);
    env.raf.flush();

    // L'observateur a survécu aux deux erreurs : un troisième scan a lieu.
    expect(scan).toHaveBeenCalledTimes(3);
    stop();
  });

  it('arrête tout scan et débranche l’observateur après nettoyage', () => {
    const env = installEnvironment();
    const scan = vi.fn();
    const stop = startObservedScanner(scan);
    env.raf.flush();
    expect(scan).toHaveBeenCalledTimes(1);

    stop();

    expect(env.observers[0]?.disconnected).toBe(true);
    expect(env.clearTimeoutSpy).toHaveBeenCalled();
    expect(env.clearIntervalSpy).toHaveBeenCalled();
    env.emitMutations(20);
    vi.advanceTimersByTime(SECOND_MS * 5);
    env.raf.flush();
    expect(scan).toHaveBeenCalledTimes(1);
  });

  it('ne scanne pas hors document.body disponible', () => {
    const env = installEnvironment();
    env.setBody(false);
    const scan = vi.fn();
    const stop = startObservedScanner(scan);
    env.raf.flush();
    vi.advanceTimersByTime(BUDGET_MS);
    env.raf.flush();
    expect(scan).not.toHaveBeenCalled();
    expect(env.observers).toHaveLength(0);

    // Dès que le corps existe, le timer d'URL rattrape l'initialisation.
    env.setBody(true);
    vi.advanceTimersByTime(URL_POLL_MS);
    env.raf.flush();
    expect(scan).toHaveBeenCalledTimes(1);
    expect(env.observers).toHaveLength(1);
    stop();
  });

  it('anti-retour : plus aucune copie locale du planificateur dans les sites', () => {
    const source = (name: string): string =>
      readFileSync(new URL(`./${name}`, import.meta.url), 'utf8');

    for (const name of ['twitter.ts', 'youtube.ts', 'tiktok.ts', 'shared.ts']) {
      expect(source(name), `${name} définit encore son propre planificateur`).not.toMatch(
        /const\s+startObservedScanner\s*=\s*\(/,
      );
    }

    for (const name of ['twitter.ts', 'youtube.ts', 'tiktok.ts']) {
      expect(source(name), `${name} n'importe pas le planificateur partagé`).toMatch(
        /import\s*\{[^}]*startObservedScanner[^}]*\}\s*from\s*'\.\/scan-scheduler(?:\.js|\.ts)?'/,
      );
    }
  });
});
