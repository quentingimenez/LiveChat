// Protège le comportement « application » de la fenêtre : le balayage de la
// souris et Ctrl+A ne doivent plus surligner l'interface, un glisser interne ne
// doit plus produire de copie fantôme, seuls les champs de saisie gardent la
// sélection de texte, et les vignettes ne sont ni sélectionnables ni
// draggables.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

// Règles CSS à plat, commentaires retirés, sélecteurs normalisés sur une ligne.
function rulesOf(css) {
  const source = css.replace(/\/\*[\s\S]*?\*\//g, '');
  return [...source.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((match) => ({
    selector: match[1].trim().replace(/\s+/g, ' '),
    declarations: match[2],
  }));
}

function ruleOf(css, selector) {
  return rulesOf(css).find((rule) => rule.selector === selector) || null;
}

test('la fenêtre ne sélectionne pas son interface au balayage ni avec Ctrl+A', () => {
  const windowRule = ruleOf(read('styles/base.css'), 'html, body');

  assert.ok(windowRule, 'html/body doivent porter la règle de sélection');
  assert.match(windowRule.declarations, /(?:^|;)\s*user-select:\s*none/);
  assert.match(windowRule.declarations, /-webkit-user-select:\s*none/);
});

test('les champs de saisie gardent la sélection de texte', () => {
  const fieldRule = rulesOf(read('styles/base.css')).find(
    (rule) =>
      /(?:^|;)\s*user-select:\s*text/.test(rule.declarations)
      && rule.selector.includes('input')
      && rule.selector.includes('textarea'),
  );

  assert.ok(fieldRule, 'input et textarea doivent réactiver la sélection');
});

test('les vignettes ne sont ni sélectionnables ni draggables', () => {
  const board = read('styles/board.css');
  const thumb = ruleOf(board, '.thumb');
  const media = ruleOf(board, '.thumb img, .thumb video');

  assert.ok(thumb, 'la règle .thumb doit exister');
  assert.match(thumb.declarations, /(?:^|;)\s*user-select:\s*none/);
  assert.ok(media, 'les médias de vignette doivent avoir leur règle');
  assert.match(media.declarations, /(?:^|;)\s*user-select:\s*none/);
  assert.match(media.declarations, /-webkit-user-drag:\s*none/);
});

test('un glisser interne ne produit plus de copie fantôme, sauf la file', () => {
  const board = read('views/board.js');

  assert.match(
    board,
    /addEventListener\('dragstart',\s*\(event\)\s*=>\s*\{[\s\S]{0,200}?closest\?\.\('\.line\.is-queue'\)[\s\S]{0,100}?event\.preventDefault\(\);/,
    'dragstart doit être annulé sauf pour une ligne de file',
  );
  assert.doesNotMatch(board, /internalDrag/, 'le drapeau de drag interne ne doit plus exister');
});
