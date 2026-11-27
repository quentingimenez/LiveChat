// Board card size: the width of the grid track the cards are laid on.
//
// A display preference of this machine, like the theme: it lives in the
// WebView's localStorage and is painted as the `--board-card-width` custom
// property on <html>, which `styles/board.css` reads for the grid track. The
// width is remembered across restarts but never leaves the machine — the shell
// config and the stream know nothing about it.
//
// One width for every card, whatever the media's own shape: the slider only
// moves that width, the rest of the card (16 / 10 thumbnail, two-line title,
// meta line) follows the CSS. Resizing the window still only changes how many
// columns fit, never the size of a card.
//
// No import: this module also loads under Node for its unit tests.

export const BOARD_CARD_WIDTH_PROPERTY = '--board-card-width';
export const BOARD_CARD_WIDTH_STORAGE_KEY = 'livechat.board.card-width';

// The width of the V1 grid: a board without a stored preference keeps exactly
// the size it had before the slider.
export const DEFAULT_BOARD_CARD_WIDTH = 232;

// Bounds of the slider. 160 px still holds the kind, author and date meta line;
// 320 px is the widest card that keeps two readable columns on a 960 px board.
export const MIN_BOARD_CARD_WIDTH = 160;
export const MAX_BOARD_CARD_WIDTH = 320;

// One step every 8 px: the 16 / 10 thumbnail grows by 5 px in height, small
// enough to feel continuous and large enough to land on whole pixels.
export const BOARD_CARD_WIDTH_STEP = 8;

// Last width applied on screen, so a toolbar rebuilt by a refresh (search,
// filters, `board.changed`) shows its slider where the user left it.
let current = DEFAULT_BOARD_CARD_WIDTH;

// A finite number, or `null` for an absent or unreadable value. `Number('')`
// is 0 and `Number('232px')` is NaN: only a real numeric value gets through.
function finiteNumber(value) {
  if (typeof value === 'number') {
    return Number.isFinite(value) ? value : null;
  }
  if (typeof value === 'string' && value.trim() !== '') {
    const number = Number(value);
    return Number.isFinite(number) ? number : null;
  }
  return null;
}

// Whole pixels, one step apart, inside the slider's bounds. An absent or
// unreadable value is the default, never a bound: a broken localStorage entry
// must not resize the board.
export function normalizeBoardCardWidth(value) {
  const number = finiteNumber(value);
  if (number === null) {
    return DEFAULT_BOARD_CARD_WIDTH;
  }
  const stepped = Math.round(number / BOARD_CARD_WIDTH_STEP) * BOARD_CARD_WIDTH_STEP;
  // The bounds are step multiples, so clamping a stepped value keeps it on a
  // step: the slider never starts between two notches.
  return Math.min(MAX_BOARD_CARD_WIDTH, Math.max(MIN_BOARD_CARD_WIDTH, stepped));
}

// `window.localStorage` when the page has one: private mode throws on access.
function defaultStorage() {
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
  } catch {
    return null;
  }
}

function defaultRoot() {
  return typeof document === 'undefined' || !document.documentElement ? null : document.documentElement;
}

// Stored preference; the default when storage is unavailable (private mode) or
// holds a value another version wrote.
export function readBoardCardWidth(storage = defaultStorage()) {
  if (!storage) {
    return DEFAULT_BOARD_CARD_WIDTH;
  }
  try {
    const raw = storage.getItem(BOARD_CARD_WIDTH_STORAGE_KEY);
    return raw === null || raw === undefined ? DEFAULT_BOARD_CARD_WIDTH : normalizeBoardCardWidth(raw);
  } catch {
    return DEFAULT_BOARD_CARD_WIDTH;
  }
}

// Stores the width, normalized so a hand-edited entry never comes back. Quota
// or private mode only disables the memory across restarts: returns the width
// that was asked for either way.
export function rememberBoardCardWidth(width, storage = defaultStorage()) {
  const normalized = normalizeBoardCardWidth(width);
  if (storage) {
    try {
      storage.setItem(BOARD_CARD_WIDTH_STORAGE_KEY, String(normalized));
    } catch {
      // Full or blocked storage: the choice still applies to this session.
    }
  }
  return normalized;
}

// Paints the width for the grid and keeps it as the current one. `root` is
// <html> in the app; a missing document (unit tests, degraded WebView) leaves
// only the current value behind.
export function applyBoardCardWidth(width, root = defaultRoot()) {
  current = normalizeBoardCardWidth(width);
  if (root && root.style) {
    root.style.setProperty(BOARD_CARD_WIDTH_PROPERTY, `${current}px`);
  }
  return current;
}

export function currentBoardCardWidth() {
  return current;
}

// Boots the preference: the stored width is painted before the first render,
// so the skeleton grid already has the remembered size.
export function initBoardCardSize(storage = defaultStorage(), root = defaultRoot()) {
  return applyBoardCardWidth(readBoardCardWidth(storage), root);
}
