// With « Ouvrir l'aperçu au survol » off, a single click opens the details at
// once and a double click plays the media. The first click opens the window; the
// second one either lands on the card (the window ignores the pointer while it
// flies in) or on the window itself, so both paths close that window and play
// instead of leaving it open over the board. These source-level checks pin that
// wiring; the click decision itself is unit-tested in selection.test.mjs.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const board = read('views/board.js');
const mediaDetail = read('views/media-detail.js');
const settings = read('views/settings.js');
const selection = read('core/selection.js');
const detailActions = read('core/detail-actions.js');

// Body of a top-level function declaration, without its closing brace line.
function functionBody(name, source = board) {
  const match = new RegExp(`\\nfunction ${name}\\([^)]*\\) \\{([\\s\\S]*?)\\n\\}`).exec(source);
  assert.ok(match, `no body for ${name}`);
  return match[1];
}

test('a card click hands the hover preview setting to the click decision', () => {
  const handler = functionBody('handleCardClick');
  assert.match(handler, /cardPointerAction\(\{/);
  assert.match(handler, /\bhoverPreview\b/, 'the card click follows the device setting');
  assert.match(handler, /\bclickCount: event\.detail\b/);
  assert.match(handler, /onControl/, 'a control inside the card keeps its own click');

  const calls = board.match(/onClick: \(event\) => handleCardClick\(entry, event/g) || [];
  assert.equal(calls.length, 1, 'the thumbnail shares the handler');
  assert.match(
    board,
    /handleCardClick\(entry, event, Boolean\(event\.target\.closest\('button, input, a, select'\)\)\)/,
    'the card background hands its controls to the same handler',
  );
});

test('a single click opens the details at once, with nothing waiting', () => {
  const apply = functionBody('applyCardPointerAction');
  assert.match(apply, /action === 'details'[\s\S]{0,80}selectItem\(entry\.item\.id\)/);
  assert.doesNotMatch(board, /scheduleCardOpen|pendingCardOpen|cancelCardOpen/);
  assert.doesNotMatch(board, /'defer'/);
  assert.doesNotMatch(selection, /'defer'/);
  assert.doesNotMatch(detailActions, /deferredOpenAllowed/);
});

test('the double click on the card plays instead of leaving the window open', () => {
  const apply = functionBody('applyCardPointerAction');
  assert.match(
    apply,
    /mediaDetail\.openId\(\) === entry\.item\.id[\s\S]{0,40}closeDetail\(\)/,
    'the window opened by the first click goes away with the play',
  );
  assert.match(apply, /playItem\(entry\.item, control\)/);
});

test('the second click of a double click is caught on the window it opened', () => {
  const catcher = functionBody('catchCardDoubleClick');
  assert.match(catcher, /event\.detail < 2/, 'only the second click of a double click is caught');
  assert.match(catcher, /DOUBLE_CLICK_WINDOW/, 'a slow click belongs to the window again');
  assert.match(catcher, /Math\.hypot\(event\.clientX - armed\.x, event\.clientY - armed\.y\)/);
  assert.match(catcher, /hp-page-shell/, 'the click landed on the window the first click opened');
  assert.match(catcher, /mediaDetail\.openId\(\) !== armed\.entry\.item\.id/, 'only that card\'s window');
  assert.match(catcher, /closeDetail\(\)/);
  assert.match(catcher, /playItem\(armed\.entry\.item, armed\.control\)/);
  assert.match(catcher, /event\.stopPropagation\(\)/, 'the window must not act on that click');
  assert.match(board, /document\.addEventListener\('click', catchCardDoubleClick, true\)/);
  // The native double click that follows would reopen the window over the play.
  assert.match(board, /document\.addEventListener\('dblclick', swallowCardDoubleClickEvent, true\)/);
});

test('a double click only arms for the mouse click that opened the details', () => {
  const handler = functionBody('handleCardClick');
  assert.match(handler, /action === 'details' && event\.detail === 1 && !hoverPreview/);
  assert.match(handler, /armCardDoubleClick\(entry, event\)/);

  const arm = functionBody('armCardDoubleClick');
  assert.match(arm, /can\('play'\)/, 'without the Play command the second click changes nothing');
  assert.match(arm, /state\.selecting/);
  assert.match(arm, /state\.capturing/);
  assert.match(arm, /at: performance\.now\(\)/);
});

test('a double click plays through the card Play button', () => {
  const control = functionBody('cardPlayButton');
  assert.match(control, /closest\('\.card'\)\?\.querySelector\('\.play'\) \|\| null/);
  assert.match(functionBody('armCardDoubleClick'), /cardPlayButton\(event\.currentTarget\)/);
});

test('the catch and the preview share one double-click window', () => {
  assert.match(detailActions, /export const DOUBLE_CLICK_WINDOW = /);
  assert.match(mediaDetail, /import \{ DOUBLE_CLICK_WINDOW \} from '\.\.\/core\/detail-actions\.js'/);
  assert.doesNotMatch(mediaDetail, /const (BODY_)?DOUBLE_CLICK_WINDOW =/, 'the window is not redeclared');
  assert.match(board, /DOUBLE_CLICK_WINDOW/, 'the board uses the preview\'s window');
});

test('the settings hint explains the single and the double click', () => {
  assert.match(settings, /Désactivé : un clic ouvre les détails, un double-clic joue le média sur les overlays\./);
  assert.doesNotMatch(settings, /double-clique une carte/);
});
