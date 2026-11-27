// L'en-tête de l'app et l'en-tête collant du board restent devant l'aperçu au
// survol. Une carte défilée presque entièrement sous eux garde un rectangle
// au-dessus du chrome : l'aperçu, position fixed, démarre sa transition à ce
// rectangle et repeignait sa première image par-dessus l'en-tête de l'app et
// la barre d'outils. Le chrome est opaque et doit masquer cette image ; la
// bande de glissement du volume, elle aussi fixe, reste sous l'aperçu et sous
// le chrome au lieu de leur prendre des clics.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const headerCss = read('styles/header.css');
const boardCss = read('styles/board.css');
const detailCss = read('styles/media-detail.css');

// Declarations of the rule whose selector list contains `selector`.
function ruleOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    const selectors = match[1]
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split(',')
      .map((part) => part.trim());
    if (selectors.includes(selector)) {
      return match[2];
    }
  }
  return '';
}

function zIndexOf(css, selector) {
  const declarations = ruleOf(css, selector);
  const value = /(?:^|;)\s*z-index:\s*(-?\d+)/.exec(declarations);
  assert.ok(value, `${selector} n’a pas de z-index`);
  return Number(value[1]);
}

test('the app header and the board head mask the hover preview', () => {
  const preview = zIndexOf(detailCss, '.hp-overlay');
  const head = zIndexOf(boardCss, '.board-head');
  const header = ruleOf(headerCss, '.header');
  const headerZ = Number(/(?:^|;)\s*z-index:\s*(-?\d+)/.exec(header)?.[1]);

  assert.ok(preview < head, `l’aperçu (${preview}) reste sous l’en-tête du board (${head})`);
  assert.ok(
    Number.isInteger(headerZ),
    'l’en-tête de l’app a besoin de son propre niveau pour passer devant l’aperçu',
  );
  assert.ok(preview < headerZ, `l’aperçu (${preview}) reste sous l’en-tête de l’app (${headerZ})`);
  assert.match(header, /position:\s*relative/, 'l’en-tête doit créer son contexte d’empilement');
  assert.match(header, /background:\s*var\(--slate-800\)/, 'l’en-tête opaque masque la première image de l’aperçu');
});

test('the volume drag band stays under the preview and the fixed chrome', () => {
  const band = zIndexOf(detailCss, '.hp-volume-drag-margin');
  const preview = zIndexOf(detailCss, '.hp-overlay');
  const head = zIndexOf(boardCss, '.board-head');

  assert.ok(band < preview, `la bande (${band}) reste sous l’aperçu (${preview})`);
  assert.ok(band < head, `la bande (${band}) ne prend pas les clics de l’en-tête du board (${head})`);
  const header = zIndexOf(headerCss, '.header');
  assert.ok(band < header, `la bande (${band}) ne prend pas les clics de l’en-tête de l’app (${header})`);
});
