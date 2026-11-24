// Keeps the board where the viewer left it when only its content is repainted
// (selection mode, refresh, filters). A real view change starts at the top: a
// position carried over would land in the middle of another page, so it is left
// to the browser there (null).

export function boardScrollToKeep({ hadBoard, hasBoard, scrollTop }) {
  if (!hadBoard || !hasBoard) {
    return null;
  }
  const top = Number(scrollTop);
  return Number.isFinite(top) && top > 0 ? top : 0;
}
