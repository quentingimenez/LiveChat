// Serveur statique des runs Bombadil (board de l'overlay).
//
// Ce que ce fichier protège : un fichier qui disparaît entre le `stat` et
// l'ouverture doit donner un 404, jamais une exception non capturée. Le
// 2026-11-20 vers 19:09, une cible `board-member` est morte sur
// `Error: ENOENT: no such file or directory, open '.../views/editor/image.js'`
// (`Emitted 'error' event on ReadStream instance`) parce que le fichier avait
// été réécrit entre les deux : node est tombé, Chrome est resté orphelin et le
// lanceur a été figé 35 minutes. Le serveur continue de servir les requêtes
// suivantes après le 404.

import assert from 'node:assert/strict';
import { createReadStream, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { PassThrough } from 'node:stream';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { createStaticServer } from './static-server.mjs';

// Le serveur écoute sur un port libre : deux cibles Bombadil peuvent tourner en
// parallèle sans se voler 39418.
const listen = async (server) => {
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return `http://127.0.0.1:${server.address().port}`;
};

const makeRoot = (t) => {
  const root = mkdtempSync(join(tmpdir(), 'bombadil-static-'));
  writeFileSync(join(root, 'page.html'), '<h1>board</h1>');
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
};

test('sert un fichier existant avec son type et sans cache', async (t) => {
  const root = makeRoot(t);
  const server = createStaticServer({ root });
  t.after(() => server.close());
  const base = await listen(server);

  const response = await fetch(`${base}/page.html`);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('content-type'), 'text/html; charset=utf-8');
  assert.equal(response.headers.get('cache-control'), 'no-store');
  assert.equal(await response.text(), '<h1>board</h1>');
});

test('répond 404 pour un fichier absent', async (t) => {
  const root = makeRoot(t);
  const server = createStaticServer({ root });
  t.after(() => server.close());
  const base = await listen(server);

  const response = await fetch(`${base}/absent.js`);
  assert.equal(response.status, 404);
});

test('répond 404 quand le fichier disparaît entre le stat et l’ouverture, et continue de servir', async (t) => {
  const root = makeRoot(t);
  // `vanish` reproduit la course observée : le stat a réussi, l'ouverture
  // échoue (ENOENT), comme un fichier réécrit par une autre session.
  let vanish = true;
  const server = createStaticServer({
    root,
    openStream: (file) => {
      if (!vanish) return createReadStream(file);
      const stream = new PassThrough();
      setImmediate(() => {
        const error = new Error(`ENOENT: no such file or directory, open '${file}'`);
        error.code = 'ENOENT';
        stream.emit('error', error);
      });
      return stream;
    },
  });
  t.after(() => server.close());
  const base = await listen(server);

  const failed = await fetch(`${base}/page.html`);
  assert.equal(failed.status, 404, 'un fichier ouvert en échec doit répondre 404, pas couper la requête');
  assert.equal(await failed.text(), '');

  // Le serveur doit survivre à l'erreur : la cible Bombadil suivante le réutilise.
  vanish = false;
  const served = await fetch(`${base}/page.html`);
  assert.equal(served.status, 200);
  assert.equal(await served.text(), '<h1>board</h1>');
});

test('refuse un chemin hors de la racine', async (t) => {
  const root = makeRoot(t);
  // Un fichier voisin de la racine ne doit jamais être servi (traversée).
  writeFileSync(join(root, '..', 'secret.txt'), 'secret');
  const server = createStaticServer({ root });
  t.after(() => server.close());
  const base = await listen(server);

  const response = await fetch(`${base}/..%2fsecret.txt`);
  assert.equal(response.status, 404);
});

// Le serveur pointé par run-board.mjs est construit par la même fabrique, sur
// un VRAI arbre : on vérifie ici que le montage `apps/lc-overlay` fonctionne
// (board-mock.html existe) sans lancer Bombadil.
test('sert le board mock du dépôt', async (t) => {
  const root = fileURLToPath(new URL('../../apps/lc-overlay', import.meta.url));
  const boardServer = createStaticServer({ root });
  t.after(() => boardServer.close());
  const base = await listen(boardServer);

  const response = await fetch(`${base}/dev/board-mock.html`);
  assert.equal(response.status, 200);
  assert.match(await response.text(), /board-mock/);
});
