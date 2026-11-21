// Types du module JavaScript qui assemble les manifestes (scripts/lib/manifest.mjs).
// Le résultat est du JSON de manifeste, volontairement non typé champ à champ.
export function mergeManifest(
  base: Record<string, unknown>,
  override: Record<string, unknown>,
  version: string,
): Record<string, any>;
