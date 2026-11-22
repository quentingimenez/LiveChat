// The size fields of the editor (Taille à l’affichage) are number inputs. On
// hover, the browser draws its own spin buttons inside the right edge of the
// field, over the last digit: the panel turns them off, since the slider and
// the presets already move that value.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

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

test('le champ numérique se passe des flèches natives', () => {
  const css = read('styles/editor.css');
  const field = ruleOf(css, ".editor-panel input[type='number']");
  assert.match(field || '', /appearance:\s*textfield/, 'le champ garde l’apparence native des nombres');
  const spin = /\.editor-panel input\[type='number'\]::-webkit-inner-spin-button[^{]*\{([^}]*)\}/.exec(css);
  assert.ok(spin, 'les flèches WebKit ne sont plus retirées');
  assert.match(spin[1], /appearance:\s*none/);
});

test('les deux éditeurs posent bien des champs numériques dans le panneau', () => {
  // Without these fields the rules above would guard nothing: the size is only
  // typed in the two display panels.
  for (const view of ['views/editor/av.js', 'views/editor/image.js']) {
    assert.match(read(view), /class:\s*'field',\s*\n\s*type:\s*'number'/, `${view} n’a plus de champ numérique`);
  }
});
