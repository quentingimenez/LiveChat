// The media info window shows the origin link of the media, not only its
// host: a right click on a card could already copy « le lien d’origine », but
// the window opened by « Infos… » (and by the drawer's « Informations ») kept
// the bare hostname. The window now renders the whole http(s) address, chosen
// by the same `sourceLink` helper as the context menu, with a copy button.
//
// These assertions read the view source on purpose: board.js needs a DOM and
// cannot be imported by Node. The link rule itself (`sourceLink`) is unit
// tested in selection.test.mjs.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

const board = read('views/board.js');
const boardCss = read('styles/board.css');

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

test('the info window shows the whole origin link, not just its host', () => {
  const facts = functionBody('infoFacts');
  assert.match(facts, /sourceLink\(item\)/, 'the window uses the shared link rule');
  assert.match(facts, /h\('span', \{ class: 'source-url' \}, link\)/, 'the link text is the address itself');
  assert.doesNotMatch(facts, /hostname/, 'the bare hostname must not come back');
});

test('the info window copies the link it shows', () => {
  const facts = functionBody('infoFacts');
  assert.match(facts, /'aria-label': 'Copier le lien d’origine'/);
  assert.match(facts, /title: 'Copier le lien d’origine'/);
  assert.match(facts, /onClick: \(\) => copyText\(link, 'Lien copié\.'\)/);
  // The button lives in the source row, which only exists with a link.
  assert.match(facts, /link\s*\n\s*\? \[[\s\S]*?'Source'[\s\S]*?source-cell[\s\S]*?\]\s*\n\s*:\s*null/);
});

test('the context menu copies the same link through the same helper', () => {
  const menu = functionBody('openCardMenu');
  assert.match(menu, /const link = sourceLink\(item\)/);
  assert.match(menu, /copyText\(link, 'Lien copié\.'\)/);
  assert.ok(!menu.includes('sourceUrl'), 'the menu must not parse the source URL itself');
});

test('the copy button sits on the link first line, and a long link wraps in its column', () => {
  const cell = ruleOf(boardCss, '.facts .source-cell');
  assert.ok(cell, 'the source row has its own rule');
  assert.match(cell, /display:\s*flex/);
  assert.match(cell, /align-items:\s*flex-start/);
  // One text line tall, so the 16px icon centers on the first line of the
  // address; a 32px button sat ~6px lower than the text.
  const button = ruleOf(boardCss, '.facts .source-cell .icon-btn');
  assert.ok(button, 'the copy button has its own size');
  assert.match(button, /width:\s*20px/);
  assert.match(button, /height:\s*20px/);
  const url = ruleOf(boardCss, '.facts .source-url');
  assert.ok(url, 'the address has its own rule');
  assert.match(url, /min-width:\s*0/);
  assert.match(url, /overflow-wrap:\s*anywhere/);
});
