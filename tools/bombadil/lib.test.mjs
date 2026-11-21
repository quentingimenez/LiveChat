// Options passées à `bombadil browser test` (tools/bombadil/lib.mjs).
//
// Ce que ce fichier protège : la taille de fenêtre de l'exploration. Elle était
// figée à 1280 × 800 dans le code ; les 22 runs de la matrice du 2026-11-20 ont
// donc tous vu le board à une seule taille. `BOMBADIL_WIDTH` et
// `BOMBADIL_HEIGHT` la rendent réglable (la nightly explore d'autres tailles),
// et une valeur invalide doit échouer avant que Bombadil ne démarre, avec le
// nom de la variable fautive.

import assert from 'node:assert/strict';
import test from 'node:test';

import { bombadilArgs } from './lib.mjs';

const target = {
  origin: 'http://127.0.0.1:39418/dev/board-mock.html?media',
  spec: 'specs/board.ts',
  output: 'out/board',
};

const valueOf = (args, flag) => args[args.indexOf(flag) + 1];

test('passe les valeurs par défaut : 1280 × 800 pendant 3 minutes', () => {
  const args = bombadilArgs(target, {});

  assert.deepEqual(args.slice(0, 2), ['browser', 'test']);
  assert.ok(args.includes('--headless'));
  assert.ok(args.includes('--exit-on-violation'));
  assert.equal(valueOf(args, '--time-limit'), '3m');
  assert.equal(valueOf(args, '--width'), '1280');
  assert.equal(valueOf(args, '--height'), '800');
  assert.equal(args.at(-2), target.origin);
  assert.equal(args.at(-1), target.spec);
  assert.ok(!args.includes('--no-sandbox'));
  assert.ok(!args.includes('--allow-url'));
  assert.ok(!args.includes('--cookie'));
});

test('honore la taille de fenêtre demandée', () => {
  const args = bombadilArgs(target, { BOMBADIL_WIDTH: '1920', BOMBADIL_HEIGHT: '1080' });

  assert.equal(valueOf(args, '--width'), '1920');
  assert.equal(valueOf(args, '--height'), '1080');
  assert.equal(args.filter((arg) => arg === '--width').length, 1);
});

test('refuse une dimension qui n’est pas un entier positif, en nommant la variable', () => {
  assert.throws(() => bombadilArgs(target, { BOMBADIL_WIDTH: 'wide' }), /BOMBADIL_WIDTH must be a positive integer, got "wide"/);
  assert.throws(() => bombadilArgs(target, { BOMBADIL_HEIGHT: '0' }), /BOMBADIL_HEIGHT/);
  assert.throws(() => bombadilArgs(target, { BOMBADIL_HEIGHT: '10.5' }), /BOMBADIL_HEIGHT/);
});

test('reprend la limite de temps, l’allow-url, les cookies et le sandbox', () => {
  const cookie = 'lc_admin_session=abc; Path=/admin; HttpOnly';
  const args = bombadilArgs(
    { ...target, allowUrl: 'http://127.0.0.1:39418/dev/board-mock.html*', cookies: [cookie] },
    { BOMBADIL_TIME_LIMIT: '20m', BOMBADIL_NO_SANDBOX: '1' },
  );

  assert.equal(valueOf(args, '--time-limit'), '20m');
  assert.equal(valueOf(args, '--allow-url'), 'http://127.0.0.1:39418/dev/board-mock.html*');
  assert.deepEqual(args.slice(args.indexOf('--cookie'), args.indexOf('--cookie') + 2), ['--cookie', cookie]);
  assert.ok(args.includes('--no-sandbox'));
  assert.equal(args.at(-2), target.origin);
  assert.equal(args.at(-1), target.spec);
});
