// La barre d'outils du board et le panneau de filtres forment un en-tête collant
// qui doit masquer tout ce qui passe derrière lui — en particulier l'aperçu au
// survol, une surcouche fixe qui grandit depuis la carte et peut mordre sur le
// padding horizontal de la vue.
//
// Retour utilisateur protégé ici : l'en-tête s'arrêtait à la colonne de contenu
// et l'aperçu apparaissait dans les 24 px de padding, à côté de lui. Ces tests
// tiennent le contrat : l'en-tête couvre la boîte de padding de la vue, sans
// déplacer la colonne de contenu (marges négatives compensées par le padding).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

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

// Padding horizontal de la vue : deuxième valeur du raccourci (ou la seule).
function inlinePaddingOf(view) {
  const shorthand = /padding:\s*([^;]+)/.exec(view)?.[1];
  assert.ok(shorthand, 'the view must keep its padding');
  const values = shorthand.trim().split(/\s+/).map((value) => /(-?\d+)px/.exec(value)?.[1]);
  assert.ok(values.every(Boolean), 'the view padding must stay in pixels');
  const inline = values.length === 1 ? values[0] : values[1];
  return Number(inline);
}

test('the head covers the view padding, not just the content column', () => {
  const view = ruleOf(read('styles/base.css'), '.view');
  const head = ruleOf(read('styles/board.css'), '.board-head');

  assert.ok(view, 'the view rule must exist');
  assert.ok(head, 'the head rule must exist');
  const inline = inlinePaddingOf(view);
  assert.ok(inline > 0, 'the view keeps an inline padding');

  // The hover preview is fixed and may reach into the view padding: a head
  // boxed to the content column leaves those strips unmasked.
  assert.match(head, /margin-inline:\s*-(\d+)px/, 'the head must pull out to the view padding box');
  assert.match(head, /padding-inline:\s*(\d+)px/, 'the head must pad its content back onto the column');
  assert.equal(Number(/margin-inline:\s*-(\d+)px/.exec(head)?.[1]), inline, 'the pull-out matches the view inline padding');
  assert.equal(Number(/padding-inline:\s*(\d+)px/.exec(head)?.[1]), inline, 'the padding matches the view inline padding');
});

test('the width stays automatic so the head follows the view at every size', () => {
  const head = ruleOf(read('styles/board.css'), '.board-head');
  const mask = ruleOf(read('styles/board.css'), '.board-head::before');

  assert.ok(head, 'the head rule must exist');
  assert.ok(mask, 'the mask strip must exist');
  // A fixed width would stop following the view once the rail or the window
  // changes; the negative margins must stretch the auto width instead.
  assert.doesNotMatch(head, /(?:^|[;\s])width:/, 'the head must keep an automatic width');
  // The strip positions itself against the head's padding box: left/right 0
  // therefore follow the widened box, covering the padding strips too.
  assert.match(mask, /left:\s*0/, 'the strip starts at the head padding edge');
  assert.match(mask, /right:\s*0/, 'the strip ends at the head padding edge');
  // top: -20px sits outside the head: clipping the head would cut the strip.
  assert.doesNotMatch(head, /overflow:\s*hidden/, 'the head must not clip its mask strip');
});
