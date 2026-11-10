// AUD-A6-002 : les widgets Twitter tiers sont désactivés en production.
//
// Aucun script distant n'est chargé et aucune URL de domaine tiers n'est
// référencée par le bundle. Le rendu de la carte tweet est statique (texte
// extrait d'un document inerte, cf. `legacy-tweet-card-renderer.js`) et la CSP
// n'autorise plus aucun script tiers (`script-src 'self'`).
export function createOverlayLegacyTwitterWidgetsUtils() {
    async function ensureTwitterWidgets() {
        throw new Error('tweet_widgets_disabled');
    }
    return {
        ensureTwitterWidgets
    };
}
