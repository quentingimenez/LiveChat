// L'en-tête du board (recherche, types, filtres, actions) et le panneau de
// filtres ouvert restent collés en haut de la vue pendant que la grille
// défile. La fenêtre de détail est une modale au-dessus de cet en-tête.
//
// Ces invariants protègent le retour utilisateur : les contrôles ne doivent
// plus sortir de l'écran dès que l'on fait défiler les cartes.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

// Contenu de l'appel ou du bloc qui commence à `marker`, guillemets et gabarits
// ignorés : les sélecteurs testés peuvent contenir des parenthèses ou des
// accolades dans leurs chaînes.
function balancedBlock(source, marker, openChar, closeChar) {
  const start = source.indexOf(marker);
  assert.notEqual(start, -1, `missing block: ${marker}`);
  const open = source.indexOf(openChar, start);
  let depth = 0;
  let quote = '';
  for (let index = open; index < source.length; index += 1) {
    const char = source[index];
    if (quote) {
      if (char === '\\') {
        index += 1;
      } else if (char === quote) {
        quote = '';
      }
      continue;
    }
    if (char === "'" || char === '"' || char === '`') {
      quote = char;
    } else if (char === openChar) {
      depth += 1;
    } else if (char === closeChar) {
      depth -= 1;
      if (depth === 0) {
        return source.slice(start, index + 1);
      }
    }
  }
  assert.fail(`unterminated block after ${marker}`);
}

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

test('the toolbar and the open filters panel share one sticky head', () => {
  const board = read('views/board.js');
  const headAt = board.indexOf('const head = h');
  assert.notEqual(headAt, -1, 'the board must group its header in a head');
  const head = balancedBlock(board.slice(headAt), 'const head = h', '(', ')');

  assert.match(head, /class: 'board-head'/, 'the head must carry the sticky class');
  assert.match(head, /\btoolbar\b/, 'the head must contain the toolbar');
  assert.match(head, /state\.filtersOpen \? renderFilters\(boardUsage\) : null/, 'the head must contain the open filters panel');
  assert.ok(head.indexOf('toolbar') < head.indexOf('state.filtersOpen'), 'the tools come before the panel');

  const returnedAt = board.indexOf('return h(', headAt);
  assert.notEqual(returnedAt, -1, 'the view must return its content');
  const returned = balancedBlock(board.slice(returnedAt), 'return h(', '(', ')');

  assert.match(returned, /^\s*head,$/m, 'the head replaces the toolbar and the panel in the view');
  assert.doesNotMatch(returned, /state\.filtersOpen/, 'the panel must not stay a sibling of the head');
  assert.ok(returned.indexOf('head,') < returned.indexOf("class: 'board'"), 'the head comes before the grid');
});

test('the head sticks, masks the cards and stays under the selection bar and the detail window', () => {
  const css = read('styles/board.css');
  const head = ruleOf(css, '.board-head');
  const shell = ruleOf(read('styles/media-detail.css'), '.hp-page-shell');
  const bar = ruleOf(css, '.selection-bar');

  assert.ok(head, 'the head rule must exist');
  assert.match(head, /position:\s*sticky/);
  assert.match(head, /top:\s*0/);
  assert.match(head, /display:\s*flex/);
  assert.match(head, /flex-direction:\s*column/);
  assert.match(head, /background:\s*var\(--slate-800\)/, 'the head must mask the cards behind it');

  assert.ok(shell, 'the detail window shell rule must exist');
  assert.ok(bar, 'the selection bar rule must exist');
  const z = Number(/(?:^|\s)z-index:\s*(-?\d+)/.exec(head)?.[1]);
  const shellZ = Number(/(?:^|\s)z-index:\s*(\d+)/.exec(shell)?.[1]);
  const barZ = Number(/(?:^|\s)z-index:\s*(\d+)/.exec(bar)?.[1]);
  assert.ok(Number.isInteger(z), 'the head needs its own stacking level');
  assert.ok(z < barZ, 'the selection bar keeps its place above the head');
  assert.ok(z < shellZ, 'the detail window stacks above the head it covers');
});

test('the strip above the stuck head matches the view top padding', () => {
  const view = ruleOf(read('styles/base.css'), '.view');
  const mask = ruleOf(read('styles/board.css'), '.board-head::before');

  const padTop = /padding:\s*(\d+)px/.exec(view)?.[1];
  assert.ok(padTop, 'the view must keep its top padding');
  assert.ok(mask, 'the stuck head needs the strip that hides the cards above it');
  assert.equal(/top:\s*-(\d+)px/.exec(mask)?.[1], padTop, 'the strip starts at the top of the view');
  assert.equal(/height:\s*(\d+)px/.exec(mask)?.[1], padTop, 'the strip covers the view padding');
  assert.match(mask, /background:\s*var\(--slate-800\)/, 'the strip uses the view colour');
});
