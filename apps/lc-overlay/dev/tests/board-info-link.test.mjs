// The media detail window shows the origin link of the media, not only its
// host: a right click on a card could already copy « le lien d’origine », but
// the window opened by « Infos… » kept the bare hostname. The old dialog is
// gone: the detail window's information column now renders the whole http(s)
// address, chosen by the same `sourceLink` helper as the context menu, with a
// copy button next to it.
//
// These assertions read the view source on purpose: board.js needs a DOM and
// cannot be imported by Node. The link rule itself (`sourceLink`) is unit
// tested in selection.test.mjs.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

const board = read('views/board.js');
const detailCss = read('styles/media-detail.css');

// Body of a top-level function declaration, without its closing brace line.
function functionBody(name) {
  const match = new RegExp(`\\nfunction ${name}\\([^)]*\\) \\{([\\s\\S]*?)\\n\\}`).exec(board);
  assert.ok(match, `no body for ${name}`);
  return match[1];
}

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

test('the detail window shows the whole origin link, not just its host', () => {
  const detail = functionBody('detailBody');
  assert.match(detail, /sourceLink\(item\)/, 'the window uses the shared link rule');
  assert.match(
    detail,
    /h\('span', \{ class: 'source-url', title: link \}, link\)/,
    'the link text is the address itself, its tooltip the whole address',
  );
  assert.doesNotMatch(detail, /hostname/, 'the bare hostname must not come back');
});

test('the detail window copies the link it shows', () => {
  const detail = functionBody('detailBody');
  // The button lives in the source row, which only exists with a link.
  const source = detail.slice(detail.indexOf('if (link)'), detail.indexOf('if (canPlay)'));
  assert.match(source, /'Source'/);
  assert.match(source, /source-cell/);
  assert.match(source, /'aria-label': 'Copier le lien d’origine'/);
  assert.match(source, /title: 'Copier le lien d’origine'/);
  assert.match(source, /onClick: \(\) => copyText\(link, 'Lien copié\.'\)/);
});

test('the context menu copies the same link through the same helper', () => {
  const menu = functionBody('openCardMenu');
  assert.match(menu, /const link = sourceLink\(item\)/);
  assert.match(menu, /copyText\(link, 'Lien copié\.'\)/);
  assert.ok(!menu.includes('sourceUrl'), 'the menu must not parse the source URL itself');
});

test('the copy button sits on the link first line, and a long link ellipsizes in its column', () => {
  const cell = ruleOf(detailCss, '.hp-page-facts .source-cell');
  assert.ok(cell, 'the source row has its own rule in the window stylesheet');
  assert.match(cell, /display:\s*flex/);
  assert.match(cell, /align-items:\s*flex-start/);
  // One text line tall, so the 16px icon centers on the first line of the
  // address; a 32px button sat ~6px lower than the text.
  const button = ruleOf(detailCss, '.hp-page-facts .source-cell .icon-btn');
  assert.ok(button, 'the copy button has its own size');
  assert.match(button, /width:\s*20px/);
  assert.match(button, /height:\s*20px/);
  // The validated layout keeps the address on one line: the whole link is in
  // its tooltip and the text ends with an ellipsis instead of wrapping.
  const url = ruleOf(detailCss, '.hp-page-facts .source-url');
  assert.ok(url, 'the address has its own rule');
  assert.match(url, /min-width:\s*0/);
  assert.match(url, /overflow:\s*hidden/);
  assert.match(url, /text-overflow:\s*ellipsis/);
  assert.match(url, /white-space:\s*nowrap/);
  assert.doesNotMatch(url, /overflow-wrap/);
});
