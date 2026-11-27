// Keeps the scroller where the viewer left it when only the content of the same
// view is repainted (a settings update check, the board's selection mode, a
// refresh, filters). Chromium's scroll anchoring moves the scroller while its
// children are replaced, so the position is saved around the swap and put back
// once the new content is in place. A real view change (page, guild, tab,
// admin section) starts at the top: a position carried over would land in the
// middle (or at the end) of another page.

export function viewScrollToKeep({ previousView, nextView, scrollTop }) {
  if (previousView !== nextView) {
    return 0;
  }
  const top = Number(scrollTop);
  return Number.isFinite(top) && top > 0 ? top : 0;
}
