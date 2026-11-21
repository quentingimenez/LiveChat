// Agencement de la barre du board : une seule rangée pour la recherche, les
// types, les filtres et les actions ; l'occupation du board devient l'en-tête
// du panneau de filtres au lieu d'une seconde ligne toujours visible.
//
// Ces invariants protègent le retour utilisateur : plus d'actions rejetées sur
// une deuxième ligne, plus de légende orpheline quand le panneau est fermé.

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

test('the toolbar keeps its actions on the tools row', () => {
  const board = read('views/board.js');
  const toolbar = balancedBlock(board, 'const toolbar = h', '(', ')');
  const toolsAt = toolbar.indexOf("class: 'board-tools'");

  assert.notEqual(toolsAt, -1, 'the toolbar must keep its tools group');
  const toolsCall = toolbar.lastIndexOf('h(', toolsAt);
  const tools = balancedBlock(toolbar.slice(toolsCall), 'h', '(', ')');
  assert.match(tools, /class: 'board-actions'/, 'the actions must live inside the tools row');
  assert.doesNotMatch(toolbar, /board-usage/, 'the usage line must leave the toolbar');
  assert.match(board, /state\.filtersOpen \? renderFilters\(boardUsage\) : null/, 'the panel must receive the usage text');
});

test('the usage legend only exists at the head of the filter panel', () => {
  const filters = balancedBlock(read('views/board.js'), 'function renderFilters', '{', '}');

  assert.match(filters, /^function renderFilters\(boardUsage\)/, 'renderFilters must receive the usage text');
  assert.match(filters, /boardUsage\s*\?\s*h\(/, 'the usage header must be conditional');
  assert.match(filters, /has-usage/, 'the panel must expose when it carries a usage header');
  assert.match(filters, /shelf-usage/, 'the usage line must be the shelf header');
  assert.doesNotMatch(filters, /filters-title/, 'the panel no longer needs a redundant title');
  assert.ok(filters.indexOf('shelf-usage') < filters.indexOf('shelf-controls'), 'the usage header comes before the controls');
});

test('a role without options keeps the panel usable', () => {
  const filters = balancedBlock(read('views/board.js'), 'function renderFilters', '{', '}');

  assert.match(filters, /optionSwitches\.length > 0/, 'an empty options column must not be rendered');
  assert.match(filters, /optionSwitches\.length > 0\s*\?\s*h\('span', \{ class: 'filter-sep'/, 'the separator follows the options block');
});

test('the stylesheet lays the tools on one row and the filters on a shelf', () => {
  const css = read('styles/board.css');
  const toolbar = ruleOf(css, '.board-toolbar');
  const actions = ruleOf(css, '.board-tools > .board-actions');
  const shelf = ruleOf(css, '.filters.shelf');
  const usage = ruleOf(css, '.filters.shelf .shelf-usage');
  const controls = ruleOf(css, '.filters.shelf .shelf-controls');
  const field = ruleOf(css, '.filters.shelf .filter');
  const reset = ruleOf(css, '.filters.shelf .filters-reset');

  assert.ok(toolbar, 'the toolbar rule must exist');
  assert.doesNotMatch(toolbar, /flex-direction:\s*column/, 'the toolbar is no longer two stacked rows');
  assert.ok(actions, 'the actions need their own flex item rule');
  assert.match(actions, /flex:\s*0 0 auto/);
  assert.match(actions, /flex-wrap:\s*nowrap/);
  assert.match(actions, /margin-left:\s*auto/);
  assert.ok(usage, 'the shelf needs a usage header rule');
  assert.ok(controls, 'the shelf controls need their own row');
  assert.match(controls, /display:\s*flex/);
  assert.match(controls, /flex-wrap:\s*wrap/);
  assert.ok(shelf, 'the shelf rule must exist');
  assert.match(shelf, /max-width:\s*none/);
  assert.ok(field, 'the shelf filters must share the line');
  assert.match(field, /flex:\s*1 1 140px/);
  assert.match(field, /max-width:\s*200px/);
  assert.ok(reset, 'the reset button must be pushed to the end of the shelf');
  assert.match(reset, /margin-left:\s*auto/);
});
