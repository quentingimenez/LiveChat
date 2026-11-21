/**
 * Fusionne le manifeste commun avec la surcouche d'une cible de navigateur.
 *
 * Les objets sont fusionnés récursivement ; les tableaux et les scalaires de la
 * surcouche remplacent ceux de la base (pas de concaténation implicite). La
 * version n'est jamais dupliquée dans les fichiers de manifeste : elle vient de
 * `package.json`, source unique pour le tag de release et les stores.
 */
const isPlainObject = (value) =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

// `background` est exclusif : Chromium attend un service worker, Firefox des
// scripts. Le fusionner champ à champ laisserait `service_worker` dans le
// manifeste Firefox si la base en contenait un un jour.
const REPLACED_KEYS = new Set(['background']);

export const mergeManifest = (base, override, version) => {
  const merge = (target, source) => {
    for (const [key, value] of Object.entries(source ?? {})) {
      if (REPLACED_KEYS.has(key)) {
        target[key] = structuredClone(value);
      } else if (isPlainObject(value) && isPlainObject(target[key])) {
        merge(target[key], value);
      } else if (isPlainObject(value)) {
        target[key] = merge({}, value);
      } else {
        target[key] = Array.isArray(value) ? [...value] : value;
      }
    }

    return target;
  };

  const merged = merge(structuredClone(base), override);
  merged.version = version;

  return merged;
};
