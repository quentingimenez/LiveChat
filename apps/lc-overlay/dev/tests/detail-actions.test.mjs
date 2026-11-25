// Decisions of the board's media detail window, kept pure so they can be
// pinned without a DOM: when the hover preview is allowed, the one meta line
// it shows, and which actions the window offers (and in which order).

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  detailActionIds,
  previewAllowed,
  previewMeta,
} from '../../web/app/core/detail-actions.js';

const allowed = (overrides = {}) => ({
  hoverPreview: true,
  selecting: false,
  capturing: false,
  dialogOpen: false,
  ...overrides,
});

test('previewAllowed only opens the preview with the hover preview turned on', () => {
  assert.equal(previewAllowed(allowed()), true);
  // The setting is off, or was never read: no hover preview at all.
  assert.equal(previewAllowed(allowed({ hoverPreview: false })), false);
  assert.equal(previewAllowed(allowed({ hoverPreview: undefined })), false);
  assert.equal(previewAllowed(allowed({ hoverPreview: 'true' })), false);
});

test('previewAllowed is refused by the selection mode, the shortcut capture or a dialog', () => {
  assert.equal(previewAllowed(allowed({ selecting: true })), false);
  assert.equal(previewAllowed(allowed({ capturing: true })), false);
  assert.equal(previewAllowed(allowed({ dialogOpen: true })), false);
  // Any truthy value counts, not only `true`.
  assert.equal(previewAllowed(allowed({ selecting: 1 })), false);
  assert.equal(previewAllowed(allowed({ capturing: 'oui' })), false);
  assert.equal(previewAllowed(allowed({ dialogOpen: {} })), false);
});

test('previewAllowed treats missing flags as false', () => {
  assert.equal(previewAllowed({ hoverPreview: true }), true);
  assert.equal(previewAllowed({ hoverPreview: true, selecting: undefined, capturing: undefined, dialogOpen: undefined }), true);
  assert.equal(previewAllowed({}), false);
  assert.equal(previewAllowed(), false);
});

test('previewMeta joins the kind, the duration and the author with a middle dot', () => {
  assert.equal(
    previewMeta({ kindLabel: 'Vidéo', duration: '12:34', author: 'Quentin' }),
    'Vidéo · 12:34 · par Quentin',
  );
});

test('previewMeta omits an unknown duration and an unknown author', () => {
  assert.equal(previewMeta({ kindLabel: 'Image', duration: '', author: '' }), 'Image');
  assert.equal(previewMeta({ kindLabel: 'Image', duration: null, author: null }), 'Image');
  assert.equal(previewMeta({ kindLabel: 'Audio', duration: '3:05', author: '' }), 'Audio · 3:05');
  assert.equal(previewMeta({ kindLabel: 'Vidéo', duration: '', author: 'Sarah' }), 'Vidéo · par Sarah');
});

test('previewMeta drops an empty kind instead of leading with a separator', () => {
  assert.equal(previewMeta({ kindLabel: '', duration: '2:00', author: 'Léa' }), '2:00 · par Léa');
  assert.equal(previewMeta({}), '');
  assert.equal(previewMeta(), '');
});

test('detailActionIds offers nothing when no action applies', () => {
  assert.deepEqual(detailActionIds({}), []);
  assert.deepEqual(detailActionIds({
    canPlay: false,
    canOpenEditor: false,
    canRefresh: false,
    canImport: false,
    canDelete: false,
  }), []);
  // The link and the information are shown in the column itself now: they are
  // not actions any more.
  assert.deepEqual(detailActionIds({ hasLink: true }), []);
});

test('detailActionIds returns every remaining action in the decided order', () => {
  assert.deepEqual(detailActionIds({
    canPlay: true,
    canOpenEditor: true,
    canRefresh: true,
    canImport: true,
    canDelete: true,
  }), ['play', 'edit', 'refresh', 'import', 'delete']);
});

test('detailActionIds adds each flag on its own', () => {
  assert.deepEqual(detailActionIds({ canPlay: true }), ['play']);
  assert.deepEqual(detailActionIds({ canOpenEditor: true }), ['edit']);
  assert.deepEqual(detailActionIds({ canRefresh: true }), ['refresh']);
  assert.deepEqual(detailActionIds({ canImport: true }), ['import']);
  assert.deepEqual(detailActionIds({ canDelete: true }), ['delete']);
});

test('detailActionIds never brings the information dialog back', () => {
  const ids = detailActionIds({
    canPlay: true,
    canOpenEditor: true,
    canRefresh: true,
    canImport: true,
    canDelete: true,
  });
  assert.ok(!ids.includes('info'), 'the information dialog is gone');
  assert.ok(!ids.includes('copyLink'), 'the source row owns the copy button');
});

test('detailActionIds only reads the flags as booleans', () => {
  assert.deepEqual(detailActionIds({ canDelete: 'yes' }), ['delete']);
  assert.deepEqual(detailActionIds({ canImport: 0, canRefresh: null }), []);
});
