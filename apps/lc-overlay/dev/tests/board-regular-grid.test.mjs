// The board lays its cards out as the regular V1 grid: one card shape whatever
// the media's own shape. This file used to pin the masonry helpers
// (core/masonry.js, the per-card row span); it now pins the regular grid in the
// same spirit: the CSS and the card builders must keep one fixed width, one
// height, a 16 / 10 thumbnail and the title + meta body. The width is fixed
// (232px) so that resizing the window only changes the number of columns per
// row, never the size of the cards.
//
// The views need a DOM and the harness has none: these checks read the source.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const board = read('views/board.js');
const uploads = read('views/uploads.js');
const boardCss = read('styles/board.css');

// Declarations of the rule whose selector is exactly `selector`.
function ruleOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    if (match[1].replace(/\/\*[\s\S]*?\*\//g, '').trim() === selector) {
      return match[2];
    }
  }
  return null;
}

// Body of a top-level function declaration, without its closing brace line.
function functionBody(source, name) {
  const match = new RegExp(`\\n(?:export )?function ${name}\\([^)]*\\) \\{([\\s\\S]*?)\\n\\}`).exec(source);
  assert.ok(match, `no body for ${name}`);
  return match[1];
}

test('the board keeps one card width and only the column count follows the window', () => {
  const grid = ruleOf(boardCss, '.grid');
  assert.ok(grid, '.grid must exist in board.css');
  // A fixed 232px track: resizing the window only adds or removes a column
  // (three at 960, four at 1120, five at 1400), the cards themselves keep
  // their width. The tracks stay anchored to the content's left edge: a
  // centred row would move every card by a sub-pixel step on each width and
  // shimmer while the window is dragged.
  assert.match(
    grid,
    /grid-template-columns:\s*repeat\(auto-fill,\s*232px\)/,
    'the track is fixed at 232px, never a share of the available width',
  );
  assert.match(grid, /justify-content:\s*start/, 'the fixed tracks stay anchored, not centred');
  assert.doesNotMatch(grid, /\b1fr\b/, 'no fractional track: a card never stretches with the window');
  assert.match(grid, /gap:\s*22px 18px/, 'the V1 gap');
  assert.match(grid, /align-items:\s*stretch/, 'every card of a row keeps the same height');
  // The masonry is gone: tiny rows, per-card spans and a fixed card width.
  assert.doesNotMatch(grid, /grid-auto-rows/);
  assert.doesNotMatch(boardCss, /--span|--card\b/);

  // No card size comes from the viewport: neither the grid nor the card may
  // declare a width, a min / max width or a basis in vw or %.
  const card = ruleOf(boardCss, '.card');
  assert.ok(card, '.card must exist in board.css');
  const viewportWidth = /(?:^|[\s;])(?:width|min-width|max-width|flex-basis|inline-size):\s*[^;]*(?:vw|[0-9.]+%)/m;
  assert.doesNotMatch(grid, viewportWidth, 'the grid does not size its cards from the viewport');
  assert.doesNotMatch(card, viewportWidth, 'the card does not size itself from the viewport');
});

test('the card is one framed surface with a 16 / 10 thumbnail', () => {
  const card = ruleOf(boardCss, '.card');
  assert.ok(card, '.card must exist in board.css');
  assert.match(card, /display:\s*flex/);
  assert.match(card, /flex-direction:\s*column/);
  assert.match(card, /border:\s*1px solid var\(--line\)/);
  assert.match(card, /border-radius:\s*var\(--radius-m\)/);
  assert.match(card, /background:\s*var\(--slate-800\)/);
  assert.match(card, /overflow:\s*hidden/, 'the thumbnail borrows the card corners');

  // The card's own hover, when the hover preview is off (see the « Card
  // hover » section of board.css).
  const hover = ruleOf(boardCss, '.grid:not(.is-hover-preview:not(.is-selecting)) .card:hover');
  assert.ok(hover, 'the card hover must exist');
  assert.match(hover, /border-color:\s*var\(--slate-600\)/);
  assert.match(hover, /box-shadow:\s*var\(--shadow-menu\)/);
  assert.match(hover, /translateY\(-1px\)/);

  const thumb = ruleOf(boardCss, '.thumb');
  assert.ok(thumb, '.thumb must exist');
  assert.match(thumb, /aspect-ratio:\s*16 \/ 10/, 'portrait media are cropped, not stretched');
  assert.match(thumb, /border:\s*0/, 'the card owns the frame');
  assert.match(thumb, /border-radius:\s*0/);

  const zoom = ruleOf(boardCss, '.grid:not(.is-hover-preview:not(.is-selecting)) .card:hover .thumb img');
  assert.ok(zoom, 'the picture zooms slightly on hover');
  assert.match(zoom, /transform:\s*scale\(1\.02\)/);
});

test('the card body holds a two-line title and one meta line', () => {
  const body = ruleOf(boardCss, '.card-body');
  assert.ok(body, '.card-body must exist');
  assert.match(body, /display:\s*flex/);
  assert.match(body, /flex-direction:\s*column/);
  assert.match(body, /padding:\s*10px 12px 12px/);

  const title = ruleOf(boardCss, '.card-title');
  assert.ok(title, '.card-title must exist');
  assert.match(title, /font-size:\s*14px/);
  assert.match(title, /font-weight:\s*600/);
  assert.match(title, /line-height:\s*1\.3/);
  assert.match(title, /min-height:\s*2\.6em/);
  assert.match(title, /-webkit-line-clamp:\s*2/);

  const meta = ruleOf(boardCss, '.card-meta');
  assert.ok(meta, '.card-meta must exist');
  assert.match(meta, /display:\s*flex/);
  assert.match(meta, /font-size:\s*11\.5px/);
  assert.match(meta, /color:\s*var\(--text-soft\)/);
  assert.match(meta, /white-space:\s*nowrap/);
  assert.match(meta, /min-height:\s*15px/);
});

test('board.js builds the regular card and no longer imports the masonry', () => {
  assert.doesNotMatch(board, /core\/masonry\.js/);
  assert.doesNotMatch(board, /cardRatio|cardSpan|cardStyle/);
  assert.doesNotMatch(board, /SKELETON_RATIOS|state\.boardLayout/);

  const card = functionBody(board, 'buildCard');
  assert.match(card, /class: 'card-body'/, 'the card keeps one body');
  assert.match(card, /class: 'card-title'/, 'the title keeps its class');
  assert.match(card, /cardMeta\(item\)/, 'the body adds the meta line');
  assert.match(card, /class: `thumb\$\{artwork \? ' has-artwork' : ''\}`/, 'the thumbnail keeps its class');
  assert.doesNotMatch(card, /style:\s*cardStyle|--span/, 'no inline masonry span');

  const meta = functionBody(board, 'cardMeta');
  assert.match(meta, /class: 'card-meta'/, 'the card keeps one meta line');
  assert.match(meta, /authorName\(item\)/, 'the meta line uses the author helper');
  assert.match(meta, /relativeDateLabel\(item\.createdAt\)/, 'the meta line uses the relative date helper');
  assert.match(meta, /icon\(kind\)/, 'the meta line starts with the kind icon');
  assert.match(meta, /KIND_LABELS\[kind\]/, 'the meta line names the kind');
});

test('the provisional upload card keeps the regular card anatomy', () => {
  assert.doesNotMatch(uploads, /core\/masonry\.js/);
  assert.doesNotMatch(uploads, /cardStyle|AUDIO_RATIO|DEFAULT_RATIO/);
  const card = functionBody(uploads, 'renderUploadCard');
  assert.match(card, /class: 'card-body'/);
  assert.match(card, /class: 'card-title'/);
  assert.doesNotMatch(card, /style:\s*cardStyle|--span/);
});

test('a picked card is outlined on the card surface, the check stays on the thumbnail', () => {
  const picked = ruleOf(boardCss, '.grid.is-selecting .card.is-picked');
  assert.ok(picked, 'a picked card must be outlined');
  assert.match(picked, /border-color:\s*var\(--amber\)/);
  assert.match(picked, /box-shadow:\s*0 0 0 1px var\(--amber\)/);

  const pick = ruleOf(boardCss, '.grid.is-selecting .pick');
  assert.ok(pick, 'the selection check stays on every card');
  assert.match(pick, /top:\s*6px/);
  assert.match(pick, /left:\s*6px/);
});
