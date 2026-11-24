// The board imports a picture from a link through `fetchArtwork`: a site with
// no CORS header is downloaded by the shell (`media_artwork_link`) instead of
// failing in the WebView. The fallback itself is unit tested against a fake
// downloader in artwork.test.mjs, and the shell download in
// apps/lc-overlay/src/net/artwork.rs; these assertions only guard the wiring in
// the view, which reads the view source on purpose (board.js needs a DOM and
// cannot be imported by Node, same as board-info-link.test.mjs).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const board = readFileSync(new URL('../../web/app/views/board.js', import.meta.url), 'utf8');

test('the info window imports the link with the shell downloader as fallback', () => {
  assert.match(
    board,
    /fetchArtwork\(link, globalThis\.fetch, downloadArtworkLink\)/,
    'the shell fallback must be passed to fetchArtwork',
  );
});

test('the shell downloader calls the board-only command with the link', () => {
  assert.match(
    board,
    /function downloadArtworkLink\(url\) \{\s*return invoke\('media_artwork_link', \{ payload: \{ url \} \}\);\s*\}/,
    'media_artwork_link must receive { payload: { url } }',
  );
});
