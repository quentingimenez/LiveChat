// Les messages d'erreur réseau peuvent contenir l'URL et les en-têtes de la
// requête, donc le jeton Bearer. On le masque avant tout affichage/retour.
export const redactSecrets = (message: string, secrets: readonly string[]): string => {
  const normalized = secrets.filter(
    (secret): secret is string => typeof secret === 'string' && secret.length > 0,
  );

  if (normalized.length === 0) {
    return message;
  }

  // Une seule passe sur la chaîne d'origine : le texte de remplacement n'est
  // jamais retraité (un secret d'une lettre ne peut pas casser `[redacted]`).
  // Les variantes les plus longues d'abord pour masquer « Bearer <jeton> »
  // avant le jeton seul.
  const escaped = normalized
    .flatMap((secret) => [`Bearer ${secret}`, secret])
    .sort((a, b) => b.length - a.length)
    .map((secret) => secret.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));

  return message.replace(new RegExp(escaped.join('|'), 'g'), '[redacted]');
};
