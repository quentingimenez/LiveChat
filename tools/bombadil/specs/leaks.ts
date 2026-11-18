// Resource-leak properties for the long nightly runs (bombadil-nightly.yml).
//
// Bombadil reads Chrome's counters without forcing a garbage collection, so
// listeners and nodes of detached views stay counted until the GC runs. On
// 10-minute calibration runs (2026-11-18) the counters grew by up to:
//
// | target | dom_nodes in 10 s | js_event_listeners in 10 s |
// | --- | --- | --- |
// | board  | +23,054 | +2,833 |
// | admin  | +27,193 | +549   |
//
// while a forced GC brought the board back to 150-211 listeners and
// 570-1,228 nodes after 400 random clicks (no real leak). The limits below are
// about three times the growth seen over 2 minutes: they catch a runaway leak
// (a timer or subscription that piles up), not a slow one.

// Bombadil 0.7.10 ships this module but not in its package `exports`: the
// types are mapped in tsconfig.json `paths`.
import { noResourceLeak } from '@antithesishq/bombadil/browser/extras/resources';

const windowMillis = 120_000;

export function leakProperties(limits: { domNodes: number; listeners: number }) {
  return {
    noDomNodeLeak: noResourceLeak({ metric: 'dom_nodes', growthLimit: limits.domNodes, windowMillis }),
    noListenerLeak: noResourceLeak({
      metric: 'js_event_listeners',
      growthLimit: limits.listeners,
      windowMillis,
    }),
  };
}
