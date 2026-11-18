// Nightly admin console specification: admin-console.ts plus the
// runaway-leak properties.

import { leakProperties } from './leaks.ts';

export * from './admin-console.ts';

export const { noDomNodeLeak, noListenerLeak } = leakProperties({ domNodes: 85_000, listeners: 2_000 });
