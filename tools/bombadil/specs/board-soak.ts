// Nightly board specification: board.ts plus the runaway-leak properties.

import { leakProperties } from './leaks.ts';

export * from './board.ts';

export const { noDomNodeLeak, noListenerLeak } = leakProperties({ domNodes: 70_000, listeners: 9_000 });
