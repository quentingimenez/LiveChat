// Ordre de la file après un glisser-déposer : le média déposé prend la place
// de la cible, les lignes entre les deux glissent autour. Un dépôt sur soi-même
// ou sur un identifiant inconnu renvoie une copie inchangée.
export function moveTo<T extends { id: string }>(items: T[], fromId: string, toId: string): T[] {
  const from = items.findIndex((item) => item.id === fromId);
  const to = items.findIndex((item) => item.id === toId);
  if (from < 0 || to < 0 || from === to) {
    return items.slice();
  }
  const next = items.slice();
  const [moved] = next.splice(from, 1);
  next.splice(Math.min(to, next.length), 0, moved);
  return next;
}
