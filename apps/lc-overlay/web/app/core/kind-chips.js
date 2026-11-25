// One chip of the board's type row. The stylesheet paints both `.is-active`
// and `[aria-pressed="true"]`, so a click must flip both: toggling only the
// class left the previous type highlighted next to the new one until the
// toolbar was rendered again.
export function syncKindChips(chips, kind) {
  for (const chip of chips) {
    const active = chip.dataset.kind === kind;
    chip.classList.toggle('is-active', active);
    chip.setAttribute('aria-pressed', String(active));
  }
}
