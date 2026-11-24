// Budget de fréquence des rescans du DOM dans les content scripts.
//
// MutationObserver + requestAnimationFrame seuls déclenchent un scan par frame,
// soit ~60 scans/s sur un onglet actif, pour des rafales de mutations souvent
// sans rapport avec les boutons injectés. On impose une fenêtre minimale de
// 150 ms entre deux scans (6,7 scans/s max) : c'est assez réactif pour l'œil
// (réaction < 150 ms) et assez bas pour ne pas saturer le thread principal.
// Une mutation tombée dans la fenêtre reprogramme un scan final, donc le
// dernier état du DOM est toujours traité (pas de perte de la fin de rafale).

const SCAN_MIN_INTERVAL_MS = 150;
const URL_POLL_MS = 700;

// Horloge monotone : `Date.now()` peut reculer (correction NTP, changement
// manuel d'heure) et ferait alors dériver la fenêtre de budget, au point de
// retarder — voire perdre — le scan final d'une rafale. `performance.now()`
// ne recule pas ; le repli n'existe que pour les moteurs sans `performance`.
const monotonicNow = (): number =>
  typeof performance !== 'undefined' && typeof performance.now === 'function'
    ? performance.now()
    : Date.now();

/**
 * Déclencheur additionnel (YouTube : plein écran et redimensionnement).
 * Il passe par le même budget que les mutations : il est donc appliqué au plus
 * tard `SCAN_MIN_INTERVAL_MS` plus tard qu'avec un scan immédiat.
 */
export type ScannerTrigger = { target: EventTarget; event: string };

/**
 * Démarre le scanner observé et retourne une fonction de nettoyage qui
 * débranche l'observateur, les déclencheurs additionnels, le timer de budget
 * et le timer de détection d'URL.
 */
export const startObservedScanner = (
  scan: () => void | Promise<void>,
  extraTriggers: readonly ScannerTrigger[] = [],
): (() => void) => {
  let stopped = false;
  let lastUrl = window.location.href;
  let pendingRaf: number | null = null;
  let budgetTimer: number | null = null;
  let lastScanAt: number | null = null;

  const scheduleFrame = (): void => {
    if (stopped || pendingRaf !== null || !document.body) {
      return;
    }
    pendingRaf = window.requestAnimationFrame(() => {
      pendingRaf = null;
      if (stopped || !document.body) {
        return;
      }

      lastScanAt = monotonicNow();
      try {
        void Promise.resolve(scan()).catch(() => {
          // Ignore scanner errors to keep observer alive.
        });
      } catch {
        // Ignore scanner errors to keep observer alive.
      }
    });
  };

  const queueScan = (): void => {
    if (stopped || !document.body) {
      return;
    }

    const wait =
      lastScanAt === null ? 0 : SCAN_MIN_INTERVAL_MS - (monotonicNow() - lastScanAt);
    if (wait <= 0) {
      // La fenêtre est écoulée : une seule frame suffit, elle lira l'état
      // courant du DOM même si plusieurs mutations se sont empilées.
      scheduleFrame();
      return;
    }

    // Dans la fenêtre : un seul timer, reprogrammé au plus tôt une fois la
    // fenêtre écoulée. Toutes les mutations de la rafale partagent ce timer,
    // ce qui donne un scan final sans multiplier les frames.
    if (budgetTimer !== null) {
      return;
    }
    budgetTimer = window.setTimeout(() => {
      budgetTimer = null;
      const remaining =
        lastScanAt === null ? 0 : SCAN_MIN_INTERVAL_MS - (monotonicNow() - lastScanAt);
      if (remaining <= 0) {
        scheduleFrame();
      } else {
        queueScan();
      }
    }, wait);
  };

  let observer = document.body ? new MutationObserver(() => queueScan()) : null;
  observer?.observe(document.documentElement, {
    subtree: true,
    childList: true,
  });

  const urlTimer = window.setInterval(() => {
    const href = window.location.href;
    // L'URL peut changer avant que document.body existe : on mémorise la
    // nouvelle valeur pour que l'initialisation rattrape le changement.
    if (href !== lastUrl) {
      lastUrl = href;
      queueScan();
    } else if (!observer && document.body) {
      // Reprise après un démarrage sans body (content script injecté tôt).
      observer = new MutationObserver(() => queueScan());
      observer.observe(document.documentElement, {
        subtree: true,
        childList: true,
      });
      queueScan();
    }
  }, URL_POLL_MS);

  for (const { target, event } of extraTriggers) {
    target.addEventListener(event, queueScan);
  }

  queueScan();

  return () => {
    stopped = true;
    observer?.disconnect();
    for (const { target, event } of extraTriggers) {
      target.removeEventListener(event, queueScan);
    }
    if (pendingRaf !== null) {
      window.cancelAnimationFrame(pendingRaf);
      pendingRaf = null;
    }
    window.clearTimeout(budgetTimer ?? 0);
    budgetTimer = null;
    window.clearInterval(urlTimer);
  };
};
