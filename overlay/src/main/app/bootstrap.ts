export function bootstrapLegacyMainRuntime(): void {
  // Compatibility adapter: preserve current behavior while moving the entrypoint to TypeScript.
  // The runtime implementation is migrated incrementally from main.runtime.js into typed modules.
  require('../../../main.runtime.js');
}
