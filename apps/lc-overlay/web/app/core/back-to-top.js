// "Back to top" of the media board.
//
// The board is the only view that scrolls on its own (`#view`, styles/base.css)
// and the toolbar leaves with the first rows: search, filters and the add
// button are then out of reach. A floating button takes the grid back to the
// top. The two rules worth testing live here, without a DOM: when the button
// shows, and how the view returns.

// The toolbar is about 70 px tall, one row of cards about 350 px. Revealing
// after 120 px means the toolbar has really left (a nudge of a few pixels does
// not flash the button) while a normal board reaches it on the first wheel
// step.
export const BACK_TO_TOP_AFTER_PX = 120;

export function showsBackToTop(scrollTop) {
  // An unknown position (no view yet, a non-finite measure) keeps the button
  // hidden: a stray floating button on a board that has not moved is worse
  // than waiting for the next scroll.
  return Number.isFinite(scrollTop) && scrollTop > BACK_TO_TOP_AFTER_PX;
}

// Users who asked for less motion get the jump instead of the animation.
export function backToTopBehavior(reduceMotion) {
  return reduceMotion ? 'auto' : 'smooth';
}
