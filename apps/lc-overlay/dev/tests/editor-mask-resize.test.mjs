// La zone de masquage, dans les deux éditeurs : c'est un rectangle, donc son
// coin bas-droit doit suivre les deux axes indépendamment, et la poignée de la
// photo doit rester cliquable même quand le masque dépasse le média.
// Sur la vidéo, le cadre est rogné : les coins du masque y restent pour que la
// poignée ne sorte jamais de la surface d'annotation.
// Régression : le redimensionnement appliquait un seul facteur radial (le
// masque gardait son ratio), et la poignée héritait du `pointer-events: none`
// posé sur la boîte de sélection.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { boundsOf, clampedDelta, clampedMask, resized, resizedMask } from '../../web/app/views/editor/annotations.js';

const css = readFileSync(new URL('../../web/app/styles/editor.css', import.meta.url), 'utf8');

function rule(selector) {
  const start = css.indexOf(`${selector} {`);
  assert.notEqual(start, -1, `règle ${selector} absente`);
  return css.slice(start, css.indexOf('}', start));
}

const mask = (x1, y1, x2, y2) => ({ id: 7, type: 'mask', mode: 'pixel', strength: 9, x1, y1, x2, y2 });

test('resizedMask: les deux côtés suivent le pointeur séparément', () => {
  const origin = mask(20, 30, 140, 90);
  const next = resizedMask(origin, boundsOf(origin), { x: 100, y: 200 }, 4);
  assert.deepEqual([next.x1, next.y1, next.x2, next.y2], [20, 30, 100, 200]);
  assert.equal(next.mode, 'pixel');
  assert.equal(next.strength, 9);
  assert.notEqual(next, origin, 'copie, pas mutation');
});

test('resizedMask: un glisser au-delà de l’ancre garde une taille minimale', () => {
  const origin = mask(20, 30, 140, 90);
  const next = resizedMask(origin, boundsOf(origin), { x: 0, y: 0 }, 6);
  assert.deepEqual([next.x2, next.y2], [26, 36]);
});

test('resizedMask: un masque tracé à l’envers repart de son coin haut-gauche', () => {
  const origin = mask(200, 150, 50, 30);
  const next = resizedMask(origin, boundsOf(origin), { x: 120, y: 100 }, 4);
  assert.deepEqual([next.x1, next.y1, next.x2, next.y2], [50, 30, 120, 100]);
});

test('resized: seul un masque est libre, le reste garde son ratio', () => {
  const origin = mask(10, 10, 60, 30);
  const anchor = boundsOf(origin);
  const point = { x: 110, y: 90 };
  const factor = Math.hypot(point.x - anchor.x, point.y - anchor.y) / Math.hypot(anchor.w, anchor.h);
  const shape = resized({ ...origin, type: 'shape', shape: 'rect', width: 4, fill: false, color: '#fff' }, anchor, point, factor, 4);
  const ratio = Math.abs(shape.x2 - shape.x1) / Math.abs(shape.y2 - shape.y1);
  assert.ok(Math.abs(ratio - 2.5) < 1e-9, `ratio ${ratio}`);
  assert.equal(resized(origin, anchor, point, factor, 4).x2, 110);
});

test('clampedMask: les deux coins restent dans le cadre vidéo', () => {
  const next = clampedMask(mask(20, 30, 400, 260), 320, 180);
  assert.deepEqual([next.x1, next.y1, next.x2, next.y2], [20, 30, 320, 180]);
  const inside = mask(20, 30, 140, 90);
  assert.deepEqual(clampedMask(inside, 320, 180), inside);
});

test('clampedMask: un masque tracé à l’envers garde son orientation', () => {
  const next = clampedMask(mask(300, 170, 40, 20), 320, 180);
  assert.deepEqual([next.x1, next.y1, next.x2, next.y2], [300, 170, 40, 20]);
});

test('clampedDelta: un masque déplacé s’arrête au bord du cadre', () => {
  const box = { x: 100, y: 50, w: 100, h: 80 };
  assert.deepEqual(clampedDelta(box, 320, 180, 300, -100), { dx: 120, dy: -50 });
  assert.deepEqual(clampedDelta(box, 320, 180, 20, 10), { dx: 20, dy: 10 });
  const wide = { x: 0, y: 0, w: 400, h: 300 };
  assert.deepEqual(clampedDelta(wide, 320, 180, 10, 10), { dx: 0, dy: 0 });
});

test('la couche vide de la vidéo laisse passer le pointeur', () => {
  assert.match(rule('.av-frame-layer'), /pointer-events:\s*none/);
  assert.match(rule('.crop-layer'), /pointer-events:\s*auto/);
});

test('la poignée de sélection reste cliquable hors du média', () => {
  assert.match(rule('.sel-handle'), /pointer-events:\s*auto/);
});
