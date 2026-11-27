// Statut affiché d'une ligne d'historique de la file.
//
// Ce que ce module protège : un média retiré de la file par un opérateur n'est
// pas un échec de lecture. Le serveur passe un job retiré `FAILED` (seule
// transition `PENDING → terminal` admise) et le marque `removedAt` /
// `removedBy` (colonne `removed_at`, migration 0024). Le rendu doit lire ce
// marqueur avant le statut, sinon un retrait volontaire s'affiche « Échec » et
// accuse une lecture qui n'a jamais eu lieu. Un `FAILED` sans marqueur reste un
// vrai échec.

// Statut dessiné dans l'historique : `kind` dit ce qui s'est passé, `label`
// et `title` sont lisibles par l'utilisateur, `icon` et `className` pilotent
// le glyphe coloré (vert terminé, rouge échec, neutre retiré).
export function historyStatus(job) {
  if (!job) {
    return { kind: 'done', className: 'is-ok', label: 'Terminé', title: 'Terminé', icon: 'check' };
  }
  if (job.removedAt) {
    return {
      kind: 'removed',
      className: 'is-muted',
      label: 'Retiré',
      title: job.removedBy ? `Retiré de la file par ${job.removedBy}` : 'Retiré de la file',
      icon: 'remove',
    };
  }
  if (job.status === 'failed') {
    return { kind: 'failed', className: 'is-err', label: 'Échec', title: 'Échec', icon: 'close' };
  }
  return { kind: 'done', className: 'is-ok', label: 'Terminé', title: 'Terminé', icon: 'check' };
}
