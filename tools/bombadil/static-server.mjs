// Serveur statique des runs Bombadil du board.
//
// Bombadil recharge la page des dizaines de fois par minute et l'arbre servi
// peut être réécrit pendant l'exploration (autre checkout, éditeur, fichier
// généré). Un fichier qui disparaît entre le `stat` et l'ouverture ne doit
// jamais tuer le processus qui pilote Bombadil : le 2026-11-20, l'ancien
// `createReadStream(file).pipe(response)` a fait mourir node sur un `ENOENT`
// non capturé, laissant un Chrome orphelin qui a figé le lanceur 35 minutes.
// Le serveur répond désormais 404 dans ce cas.

import { createReadStream, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { extname, join, normalize, sep } from 'node:path';

const types = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
  '.ico': 'image/x-icon',
  '.mp4': 'video/mp4',
  '.webm': 'video/webm',
  '.mp3': 'audio/mpeg',
  '.ogg': 'audio/ogg',
  '.wav': 'audio/wav',
  '.woff2': 'font/woff2',
};

/**
 * Crée le serveur HTTP qui expose `root` à Bombadil.
 *
 * `openStream` et `statFile` sont injectables pour que le test puisse
 * reproduire un fichier qui disparaît entre le `stat` et l'ouverture, sans
 * dépendre d'une course réelle.
 */
export function createStaticServer({ root, openStream = createReadStream, statFile = statSync }) {
  return createServer((request, response) => {
    const path = decodeURIComponent(new URL(request.url ?? '/', 'http://localhost').pathname);
    const file = normalize(join(root, path));
    let size = 0;
    try {
      if (!file.startsWith(root + sep)) {
        throw new Error('outside root');
      }
      const stat = statFile(file);
      if (!stat.isFile()) {
        throw new Error('not a file');
      }
      size = stat.size;
    } catch {
      response.writeHead(404).end();
      return;
    }

    // L'en-tête 200 part seulement une fois le flux ouvert : une ouverture qui
    // échoue après le `stat` devient un 404, pas une exception non capturée.
    let stream;
    try {
      stream = openStream(file);
    } catch {
      response.writeHead(404).end();
      return;
    }
    stream.once('error', () => {
      if (response.headersSent) {
        response.destroy();
      } else {
        response.writeHead(404).end();
      }
    });
    stream.once('open', () => {
      response.writeHead(200, {
        'Content-Type': types[extname(file).toLowerCase()] ?? 'application/octet-stream',
        'Content-Length': size,
        'Cache-Control': 'no-store',
      });
      stream.pipe(response);
    });
  });
}
