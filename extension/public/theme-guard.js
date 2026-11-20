/* Thème de l'extension, appliqué avant le premier rendu.
 *
 * `lib/theme.ts` enregistre la préférence dans `chrome.storage.local` et recopie
 * sa valeur dans le localStorage de la page : ce script classique (chargé en
 * tête du popup et des options) la relit et pose `data-lce-theme` sur <html>
 * sans attendre les modules. Sans lui, un utilisateur en thème clair verrait un
 * flash sombre le temps du chargement.
 */
(() => {
  try {
    const preference = window.localStorage.getItem('lce.theme.v1');
    const prefersLight =
      typeof window.matchMedia === 'function' &&
      window.matchMedia('(prefers-color-scheme: light)').matches;
    const light = preference === 'light' || (preference === 'system' && prefersLight);

    if (light) {
      document.documentElement.setAttribute('data-lce-theme', 'light');
    }
  } catch {
    // localStorage indisponible (mode privé, origine opaque) : sombre par défaut.
  }
})();
