// Placement of the settings page's « Retour » button.
//
// The settings column is at most 880 px wide and centred in the view. On a
// wide board the left margin is large enough to pin the button there while the
// page scrolls; at the minimum window size it must stay in the flow, as before.

export const SETTINGS_COLUMN_WIDTH = 880;
export const SETTINGS_BACK_GAP = 12;

// True when `buttonWidth` plus a small gap fits in the left margin left by the
// centred settings column. The view's padding is already part of the margin:
// `(scrollerWidth - 880) / 2` is the distance from the view's left edge to the
// column.
export function settingsBackInMargin({
  scrollerWidth,
  buttonWidth,
  columnWidth = SETTINGS_COLUMN_WIDTH,
  gap = SETTINGS_BACK_GAP,
} = {}) {
  const scroller = Number(scrollerWidth);
  const button = Number(buttonWidth);
  const column = Number(columnWidth);
  const margin = Number(gap);
  if (!Number.isFinite(scroller) || !Number.isFinite(button) || !Number.isFinite(column) || !Number.isFinite(margin)) {
    return false;
  }
  if (button <= 0 || column <= 0 || margin < 0) {
    return false;
  }
  const available = (scroller - column) / 2;
  return available >= button + margin;
}
