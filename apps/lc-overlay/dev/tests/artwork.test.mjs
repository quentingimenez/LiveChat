// Unit tests of the picture of an audio media: files, dropped links and the
// download of an image link.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  ARTWORK_MESSAGES,
  artworkExtension,
  artworkFileName,
  droppedArtworkLink,
  fetchArtwork,
  httpErrorMessage,
  isImageType,
  MAX_ARTWORK_BYTES,
  pickArtworkFile,
} from '../../web/app/core/artwork.js';

const png = (size = 4, name = 'cover.png') => ({ name, type: 'image/png', size });

test('isImageType ignores parameters and case', () => {
  assert.equal(isImageType('image/png'), true);
  assert.equal(isImageType('Image/JPEG; charset=binary'), true);
  assert.equal(isImageType('text/html; charset=utf-8'), false);
  assert.equal(isImageType(''), false);
  assert.equal(isImageType(null), false);
});

test('artworkExtension: from the name, else from the type', () => {
  assert.equal(artworkExtension({ name: 'Cover.PNG', type: 'image/png' }), '.png');
  assert.equal(artworkExtension({ name: 'cover', type: 'image/jpeg' }), '.jpg');
  assert.equal(artworkExtension({ name: '', type: '' }), '');
});

test('pickArtworkFile: no file, an image, something else, too large', () => {
  assert.equal(pickArtworkFile([]), null);
  assert.equal(pickArtworkFile(null), null);
  const image = png();
  assert.deepEqual(pickArtworkFile([{ name: 'a.mp3', type: 'audio/mpeg', size: 4 }, image]), { file: image });
  assert.deepEqual(pickArtworkFile([{ name: 'a.mp3', type: 'audio/mpeg', size: 4 }]), { error: ARTWORK_MESSAGES.notImageFile });
  assert.deepEqual(pickArtworkFile([png(MAX_ARTWORK_BYTES + 1)]), { error: ARTWORK_MESSAGES.tooLarge });
  assert.deepEqual(pickArtworkFile([png(MAX_ARTWORK_BYTES)]).error, undefined);
});

test('droppedArtworkLink: the image of the HTML wins over the page it links to', () => {
  assert.equal(
    droppedArtworkLink({
      html: '<a href="https://site.test/page"><img alt="x" src="https://cdn.test/a.png?w=1&amp;h=2"></a>',
      uriList: 'https://site.test/page',
    }),
    'https://cdn.test/a.png?w=1&h=2',
  );
  assert.equal(droppedArtworkLink({ html: "<img src='https://cdn.test/b.jpg'>" }), 'https://cdn.test/b.jpg');
  assert.equal(droppedArtworkLink({ html: '<p>no image</p>', uriList: '# comment\r\nhttps://cdn.test/c.gif' }), 'https://cdn.test/c.gif');
  assert.equal(droppedArtworkLink({ text: '  https://cdn.test/d.webp  ' }), 'https://cdn.test/d.webp');
  // A data: or relative source is not a link that can be downloaded.
  assert.equal(droppedArtworkLink({ html: '<img src="data:image/png;base64,AAAA">', text: 'hello' }), null);
  assert.equal(droppedArtworkLink({}), null);
  assert.equal(droppedArtworkLink(), null);
});

test('artworkFileName: the name of the link, else one from the type', () => {
  assert.equal(artworkFileName('https://cdn.test/img/My%20Cover.JPG?x=1', 'image/jpeg'), 'My Cover.JPG');
  assert.equal(artworkFileName('https://cdn.test/render?id=4', 'image/png'), 'render.png');
  assert.equal(artworkFileName('https://cdn.test/', 'image/webp'), 'image.webp');
  assert.equal(artworkFileName('https://cdn.test/a%2F..%2Fb', 'image/png'), 'a..b');
});

// --- download of a link ---
function respond(body, { status = 200, type = 'image/png', length } = {}) {
  const headers = {};
  if (type) {
    headers['content-type'] = type;
  }
  if (length !== undefined) {
    headers['content-length'] = String(length);
  }
  return async () => new Response(body, { status, headers });
}

test('fetchArtwork: an image link becomes a File for the upload path', async () => {
  const calls = [];
  const fetchImpl = async (url, options) => {
    calls.push([url, options]);
    return new Response(new Uint8Array([1, 2, 3]), { headers: { 'content-type': 'image/png; charset=binary' } });
  };
  const { file, error } = await fetchArtwork(' https://cdn.test/covers/a.png?sig=1 ', fetchImpl);
  assert.equal(error, undefined);
  assert.ok(file instanceof File);
  assert.equal(file.name, 'a.png');
  assert.equal(file.type, 'image/png');
  assert.equal(file.size, 3);
  assert.deepEqual(new Uint8Array(await file.arrayBuffer()), new Uint8Array([1, 2, 3]));
  assert.equal(calls[0][0], 'https://cdn.test/covers/a.png?sig=1');
  // No cookie and no referrer leave the app.
  assert.deepEqual(calls[0][1], { credentials: 'omit', referrerPolicy: 'no-referrer' });
});

test('fetchArtwork refuses what is not an http(s) link, without any request', async () => {
  let called = false;
  const fetchImpl = async () => {
    called = true;
    return new Response('');
  };
  for (const link of ['', 'pas un lien', 'file:///C:/a.png', 'javascript:alert(1)', null]) {
    assert.deepEqual(await fetchArtwork(link, fetchImpl), { error: ARTWORK_MESSAGES.invalidLink });
  }
  assert.equal(called, false);
});

test('fetchArtwork refuses a response that is not an image', async () => {
  assert.deepEqual(await fetchArtwork('https://site.test/page', respond('<html></html>', { type: 'text/html; charset=utf-8' })), {
    error: ARTWORK_MESSAGES.notImageLink,
  });
  // No declared type: nothing proves it is an image.
  assert.deepEqual(await fetchArtwork('https://site.test/a.png', respond(new Uint8Array([1]), { type: null })), {
    error: ARTWORK_MESSAGES.notImageLink,
  });
});

test('fetchArtwork refuses more than 10 MB, announced or not', async () => {
  assert.deepEqual(await fetchArtwork('https://cdn.test/a.png', respond(new Uint8Array([1]), { length: MAX_ARTWORK_BYTES + 1 })), {
    error: ARTWORK_MESSAGES.tooLarge,
  });
  // No Content-Length: the download stops once over the limit.
  let sent = 0;
  let cancelled = false;
  const chunk = new Uint8Array(1024 * 1024);
  const stream = new ReadableStream({
    pull(controller) {
      sent += 1;
      controller.enqueue(chunk);
    },
    cancel() {
      cancelled = true;
    },
  });
  const result = await fetchArtwork('https://cdn.test/a.png', async () => new Response(stream, { headers: { 'content-type': 'image/png' } }));
  assert.deepEqual(result, { error: ARTWORK_MESSAGES.tooLarge });
  assert.equal(cancelled, true);
  assert.ok(sent <= 13, `read ${sent} MB before giving up`);
  // Exactly at the limit is accepted.
  const exact = await fetchArtwork('https://cdn.test/a.png', respond(new Uint8Array(MAX_ARTWORK_BYTES)));
  assert.equal(exact.file.size, MAX_ARTWORK_BYTES);
});

test('fetchArtwork says plainly when the download fails (CORS, network)', async () => {
  const blocked = async () => {
    throw new TypeError('Failed to fetch');
  };
  assert.deepEqual(await fetchArtwork('https://cdn.test/a.png', blocked), { error: ARTWORK_MESSAGES.unreachable });
  assert.match(ARTWORK_MESSAGES.unreachable, /CORS/);
});

test('fetchArtwork reports an HTTP error and an empty image', async () => {
  assert.deepEqual(await fetchArtwork('https://cdn.test/a.png', respond('gone', { status: 404 })), { error: httpErrorMessage(404) });
  assert.match(httpErrorMessage(404), /404/);
  assert.deepEqual(await fetchArtwork('https://cdn.test/a.png', respond(new Uint8Array(0))), { error: ARTWORK_MESSAGES.empty });
});
