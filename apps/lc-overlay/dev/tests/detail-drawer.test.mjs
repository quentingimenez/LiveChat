// Unit tests of the board's detail drawer: what a click, a hover and a leave
// show.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  cardClicked,
  cardHovered,
  cardPreviewAt,
  CLOSED_VIEW,
  drawerUsed,
  artworkOf,
  cardPicture,
  isPeeking,
  mediaPath,
  pictureKey,
  pointerLeft,
  previewKey,
  pruneView,
  realignPreviewAt,
  sameView,
  shownId,
  thumbnailOf,
  withVersion,
} from '../../web/app/core/detail-drawer.js';

const pinnedOn = (id) => ({ selectedId: id, pinned: true, peekId: null });

test('nothing pinned: the drawer follows the pointer and closes when it leaves', () => {
  const hovered = cardHovered(CLOSED_VIEW, 'a');
  assert.deepEqual(hovered, { selectedId: 'a', pinned: false, peekId: null });
  assert.equal(shownId(hovered), 'a');
  assert.equal(isPeeking(hovered), true);
  assert.deepEqual(cardHovered(hovered, 'b'), { selectedId: 'b', pinned: false, peekId: null });
  assert.deepEqual(pointerLeft(hovered), CLOSED_VIEW);
});

test('a click pins a media, a click on the pinned one closes the drawer', () => {
  const pinned = cardClicked(CLOSED_VIEW, 'a');
  assert.deepEqual(pinned, pinnedOn('a'));
  assert.equal(isPeeking(pinned), false);
  assert.deepEqual(cardClicked(pinned, 'a'), CLOSED_VIEW);
  assert.deepEqual(cardClicked(pinned, 'b'), pinnedOn('b'));
});

test('regression: hovering another card still shows its details once a media is pinned', () => {
  const peek = cardHovered(pinnedOn('a'), 'b');
  assert.equal(shownId(peek), 'b');
  assert.equal(peek.selectedId, 'a');
  assert.equal(peek.pinned, true);
  assert.equal(isPeeking(peek), true);
  assert.equal(shownId(cardHovered(peek, 'c')), 'c');
});

test('details on hover off: a hover changes nothing, a pinned drawer keeps its media', () => {
  assert.deepEqual(cardHovered(CLOSED_VIEW, 'a', false), CLOSED_VIEW);
  assert.deepEqual(cardHovered(pinnedOn('a'), 'b', false), pinnedOn('a'));
  assert.deepEqual(cardHovered(pinnedOn('a'), 'a', false), pinnedOn('a'));
  // Clicking and leaving behave as before: the setting only removes the hover.
  assert.deepEqual(cardClicked(CLOSED_VIEW, 'a'), pinnedOn('a'));
  assert.deepEqual(pointerLeft(pinnedOn('a')), pinnedOn('a'));
});

test('the drawer returns to the pinned media when the pointer leaves or comes back on its card', () => {
  const peek = cardHovered(pinnedOn('a'), 'b');
  assert.deepEqual(pointerLeft(peek), pinnedOn('a'));
  assert.deepEqual(cardHovered(peek, 'a'), pinnedOn('a'));
  // Leaving without a peek keeps the pinned media.
  assert.deepEqual(pointerLeft(pinnedOn('a')), pinnedOn('a'));
});

test('clicking while peeking: the peeked card gets pinned, the pinned one closes', () => {
  const peek = cardHovered(pinnedOn('a'), 'b');
  assert.deepEqual(cardClicked(peek, 'b'), pinnedOn('b'));
  assert.deepEqual(cardClicked(peek, 'a'), CLOSED_VIEW);
});

test('the later clicks of a double click leave the drawer as it is', () => {
  // The board holds back the first click (detail 1) of a double click.
  for (const view of [CLOSED_VIEW, pinnedOn('a'), pinnedOn('b'), cardHovered(pinnedOn('b'), 'a'), cardHovered(CLOSED_VIEW, 'a')]) {
    assert.deepEqual(cardClicked(view, 'a', 2), view);
    assert.deepEqual(cardClicked(view, 'a', 3), view);
  }
  // A keyboard activation (detail 0) or a call without a count toggles.
  assert.deepEqual(cardClicked(pinnedOn('a'), 'a', 0), CLOSED_VIEW);
  assert.deepEqual(cardClicked(pinnedOn('a'), 'a'), CLOSED_VIEW);
});

test('using the drawer pins what it shows', () => {
  assert.deepEqual(drawerUsed(cardHovered(pinnedOn('a'), 'b')), pinnedOn('b'));
  assert.deepEqual(drawerUsed(cardHovered(CLOSED_VIEW, 'a')), pinnedOn('a'));
  assert.deepEqual(drawerUsed(pinnedOn('a')), pinnedOn('a'));
  assert.deepEqual(drawerUsed(CLOSED_VIEW), CLOSED_VIEW);
});

test('pruneView drops a peek on a deleted media', () => {
  const peek = cardHovered(pinnedOn('a'), 'b');
  assert.deepEqual(pruneView(peek, (id) => id === 'a'), pinnedOn('a'));
  assert.equal(pruneView(peek, () => true), peek);
  // A peek left behind by a drawer that is no longer pinned is dropped.
  assert.deepEqual(pruneView({ selectedId: 'a', pinned: false, peekId: 'b' }, () => true), { selectedId: 'a', pinned: false, peekId: null });
});

test('sameView compares the three fields', () => {
  assert.equal(sameView(pinnedOn('a'), pinnedOn('a')), true);
  assert.equal(sameView(pinnedOn('a'), cardHovered(pinnedOn('a'), 'b')), false);
});

// --- preview of the shown media ---
const audio = (artwork) => ({
  id: 'a',
  mediaUrl: 'https://srv.test/media/a1?exp=100&sig=x',
  media: { kind: 'audio' },
  artwork,
});

test('artworkOf: only a picture with a URL counts', () => {
  assert.equal(artworkOf(audio(null)), null);
  assert.equal(artworkOf(audio(undefined)), null);
  assert.equal(artworkOf(audio({ url: '' })), null);
  assert.equal(artworkOf(audio({ url: null, width: 1, height: 1 })), null);
  assert.equal(artworkOf(audio({ url: 'https://srv.test/media/p1?sig=y' })).url, 'https://srv.test/media/p1?sig=y');
});

test('mediaPath drops the signature of a URL', () => {
  assert.equal(mediaPath('https://srv.test/media/a1?exp=100&sig=x'), 'https://srv.test/media/a1');
  assert.equal(mediaPath(''), null);
  assert.equal(mediaPath(null), null);
});

test('previewKey: a new signature keeps the preview', () => {
  const before = audio({ url: 'https://srv.test/media/p1?exp=100&sig=x' });
  const after = { ...before, mediaUrl: 'https://srv.test/media/a1?exp=900&sig=z', artwork: { url: 'https://srv.test/media/p1?exp=900&sig=z' } };
  assert.equal(previewKey(before), previewKey(after));
});

test('regression: removing, adding or replacing the picture of an audio rebuilds its preview', () => {
  const withPicture = audio({ url: 'https://srv.test/media/p1?sig=x' });
  const without = audio(null);
  const replaced = audio({ url: 'https://srv.test/media/p2?sig=x' });
  assert.notEqual(previewKey(withPicture), previewKey(without));
  assert.notEqual(previewKey(withPicture), previewKey(replaced));
});

test('previewKey: the picture only matters for an audio', () => {
  const video = { id: 'v', mediaUrl: 'https://srv.test/media/v1?sig=x', media: { kind: 'video' }, artwork: null };
  assert.equal(previewKey(video), previewKey({ ...video, artwork: { url: 'https://srv.test/media/p1' } }));
});

test('regression: a picture replaced at the same URL is fetched again', () => {
  // The editor's frame rewrites the same cover, so its signed URL does not
  // change: without the version the card would keep showing the old image.
  const before = { media: { kind: 'video' }, thumbnail: { url: 'https://h/m/t?sig=1', version: 13301 } };
  const after = { ...before, thumbnail: { ...before.thumbnail, version: 20480 } };
  assert.equal(before.thumbnail.url, after.thumbnail.url);
  assert.notEqual(cardPicture(before).url, cardPicture(after).url);
  assert.match(cardPicture(after).url, /[?&]v=20480$/);
});

test('regression: a card rebuilds when its picture is replaced, not when its URL is re-signed', () => {
  // The signed URL changes every hour; the picture, only when it is rewritten.
  const before = { media: { kind: 'video' }, thumbnail: { url: 'https://h/m/t?exp=100&sig=x', version: 13301 } };
  const resigned = { ...before, thumbnail: { ...before.thumbnail, url: 'https://h/m/t?exp=900&sig=z' } };
  const replaced = { ...before, thumbnail: { ...before.thumbnail, version: 20480 } };
  assert.equal(pictureKey(before), pictureKey(resigned));
  assert.notEqual(pictureKey(before), pictureKey(replaced));
});

test('withVersion: keeps a URL that has no version, and joins with the right separator', () => {
  assert.equal(withVersion('https://h/m/t', null), 'https://h/m/t');
  assert.equal(withVersion('https://h/m/t', ''), 'https://h/m/t');
  assert.equal(withVersion(null, 12), null);
  assert.equal(withVersion('https://h/m/t', 12), 'https://h/m/t?v=12');
  assert.equal(withVersion('https://h/m/t?sig=1', 12), 'https://h/m/t?sig=1&v=12');
});

test('a video card shows its thumbnail, else the video itself', () => {
  const video = { media: { kind: 'video' }, mediaUrl: 'https://h/m/v?sig=1' };
  assert.equal(thumbnailOf(video), null);
  assert.deepEqual(cardPicture(video), { url: 'https://h/m/v?sig=1', kind: 'video' });
  const withThumbnail = { ...video, thumbnail: { url: 'https://h/m/t?sig=1', width: 480, height: 360 } };
  assert.equal(thumbnailOf(withThumbnail).width, 480);
  assert.deepEqual(cardPicture(withThumbnail), { url: 'https://h/m/t?sig=1', kind: 'image' });
});

test('an audio card shows only its picture, never a thumbnail', () => {
  const audio = { media: { kind: 'audio' }, mediaUrl: 'https://h/m/a?sig=1', thumbnail: { url: 'https://h/m/t' } };
  assert.equal(thumbnailOf(audio), null);
  assert.deepEqual(cardPicture(audio), { url: null, kind: 'image' });
  const withArtwork = { ...audio, artwork: { url: 'https://h/m/c?sig=1' } };
  assert.deepEqual(cardPicture(withArtwork), { url: 'https://h/m/c?sig=1', kind: 'image' });
});

test('an image card shows the image', () => {
  const image = { media: { kind: 'image' }, mediaUrl: 'https://h/m/i?sig=1' };
  assert.deepEqual(cardPicture(image), { url: 'https://h/m/i?sig=1', kind: 'image' });
  assert.deepEqual(cardPicture({ media: { kind: 'image' } }), { url: null, kind: 'image' });
});

test('regression: a card seeks its video past the black first frame', () => {
  // Many uploads open on a black fade-in: seeking to 0.1 s showed a black card.
  const clip = { media: { kind: 'video', durationSec: 30 }, mediaUrl: 'https://h/m/v?sig=1' };
  const at = cardPreviewAt(clip);
  assert.ok(at > 0.1, `une vidéo de 30 s se cherche après 0,1 s : ${at}`);
  assert.ok(at < 30, `et avant sa fin : ${at}`);
});

test('the seek point stays inside a short video', () => {
  const short = cardPreviewAt({ media: { kind: 'video', durationSec: 1 } });
  assert.ok(short > 0 && short < 1, `point de lecture dans la vidéo : ${short}`);
});

test('no seek point for a video of unknown length, an image or an audio', () => {
  assert.equal(cardPreviewAt({ media: { kind: 'video' } }), null);
  assert.equal(cardPreviewAt({ media: { kind: 'video', durationSec: 0 } }), null);
  assert.equal(cardPreviewAt({ media: { kind: 'video', durationSec: Number.NaN } }), null);
  assert.equal(cardPreviewAt({ media: { kind: 'image', durationSec: 30 } }), null);
  assert.equal(cardPreviewAt({ media: { kind: 'audio', durationSec: 30 } }), null);
  assert.equal(cardPreviewAt(null), null);
});

test('regression: a video shorter than announced is seeked inside itself', () => {
  // A stale length sends the seek past the real end, where the browser paints
  // a black frame: the announced 3 s of a 1.5 s video.
  const at = realignPreviewAt(3, 1.5);
  assert.ok(at !== null && at > 0 && at < 1.5, `point ramené dans la vidéo : ${at}`);
});

test('a seek point already inside the video is kept', () => {
  assert.equal(realignPreviewAt(3, 30), null);
  assert.equal(realignPreviewAt(0.3, 0.35), null);
  assert.equal(realignPreviewAt(null, 30), null);
  assert.equal(realignPreviewAt(3, Number.POSITIVE_INFINITY), null);
  assert.equal(realignPreviewAt(3, 0), null);
});
