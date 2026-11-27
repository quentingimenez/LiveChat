// Keeps the board where the viewer left it when only its content is repainted
// (selection mode, refresh, filters). A real view change starts at the top, even
// when the next page is a board too (another guild): the browser keeps the old
// offset as long as the new content is tall enough, so it must be reset here.
// `null` leaves a repaint of a non-board page to the browser, which keeps that
// page's own position.

export function boardScrollToKeep({ hadBoard, hasBoard, scrollTop, viewChanged = false }) {
  if (viewChanged) {
    return 0;
  }
  if (!hadBoard || !hasBoard) {
    return null;
  }
  const top = Number(scrollTop);
  return Number.isFinite(top) && top > 0 ? top : 0;
}
