// `browser.storage.local` contient l'ingestToken en clair (paramètres de
// l'extension). Par défaut, Chrome MV3 l'expose aux contextes non fiables —
// donc aux content scripts injectés sur TikTok/X/YouTube. On le restreint aux
// contextes de confiance dès le chargement du service worker.
//
// Limite technique documentée : ce n'est pas du chiffrement. Le stockage reste
// en clair dans le profil du navigateur ; setAccessLevel réduit seulement qui
// peut lire, pas comment il est stocké. Chiffrer avec une clé conservée dans la
// même extension ne protégerait de rien (MV3 n'offre ni keystore OS ni native
// messaging ici). Voir `README.md` § « Limites connues ».

interface LocalStorageAreaWithAccessLevel {
  setAccessLevel?: (options: { accessLevel: 'TRUSTED_CONTEXTS' }) => Promise<void>;
}

export const restrictLocalStorageToTrustedContexts = async (): Promise<void> => {
  try {
    const scope = globalThis as unknown as {
      browser?: { storage?: { local?: LocalStorageAreaWithAccessLevel } };
    };
    const localArea = scope.browser?.storage?.local;

    // Chrome < 102 et Firefox n'exposent pas setAccessLevel : no-op documenté,
    // jamais d'exception — le démarrage du service worker doit continuer.
    if (typeof localArea?.setAccessLevel !== 'function') {
      return;
    }

    await localArea.setAccessLevel({ accessLevel: 'TRUSTED_CONTEXTS' });
  } catch {
    // API refusée par le navigateur : on continue avec le niveau par défaut.
  }
};
