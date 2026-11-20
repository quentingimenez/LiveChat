// Unit tests of the queue row formatting (Node's built-in test runner).

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  jobMeta,
  jobOriginLabel,
  jobOriginTitle,
  jobTitle,
  readableText,
  uploadName,
} from '../../web/app/core/job-format.js';

const media = (overrides = {}) => ({
  id: 'a1',
  kind: 'video',
  mime: 'video/mp4',
  durationSec: 30,
  width: 1280,
  height: 720,
  isVertical: false,
  sizeBytes: 2 * 1024 * 1024,
  sourceUrl: 'https://www.youtube.com/watch?v=x',
  ...overrides,
});

test('jobTitle prefers the board title, then the source title', () => {
  assert.equal(jobTitle({ title: 'Chat qui tombe', sourceTitle: 'Source', kind: 'video' }), 'Chat qui tombe');
  assert.equal(jobTitle({ title: null, sourceTitle: 'Vidéo envoyée par un client', kind: 'video' }), 'Vidéo envoyée par un client');
});

test('jobTitle falls back to the uploaded file name', () => {
  assert.equal(jobTitle({ title: null, media: media({ sourceUrl: 'upload:vine-boom.mp3' }) }), 'vine-boom.mp3');
  assert.equal(jobTitle({ title: null, media: media({ sourceUrl: 'upload:mon%20clip.mp4' }) }), 'mon clip.mp4');
});

test('jobTitle never shows the undecoded rich payload', () => {
  const rich = '__LC_OVERLAY_RICH__::eyJjYXB0aW9uIjoiaGVsbG8ifQ==';
  assert.equal(jobTitle({ title: null, text: rich, kind: 'video' }), 'Vidéo');
  assert.equal(readableText(rich), null);
});

test('jobTitle falls back to the readable text then the kind label', () => {
  assert.equal(jobTitle({ title: null, text: 'gg les gars', kind: null }), 'gg les gars');
  assert.equal(jobTitle({ title: null, kind: 'audio' }), 'Audio');
  assert.equal(jobTitle({ title: null, kind: 'image' }), 'Image');
  assert.equal(jobTitle(null), 'Média');
});

test('jobOriginLabel maps the server origins', () => {
  assert.equal(jobOriginLabel({ origin: 'board' }), 'Board');
  assert.equal(jobOriginLabel({ origin: 'once' }), 'Une fois');
  assert.equal(jobOriginLabel({ origin: 'direct' }), 'Direct');
  assert.equal(jobOriginLabel({ origin: 'inconnu' }), null);
  assert.equal(jobOriginLabel({}), null);
  assert.equal(jobOriginTitle({ origin: 'once' }), 'Média joué une fois, hors board');
});

test('jobMeta joins the media details of a one-off play', () => {
  const job = { title: null, sourceTitle: 'Vidéo vue une fois', durationSec: 30, kind: 'video', media: media() };
  assert.equal(jobMeta(job), 'Vidéo · 0:30 · 1280×720 · 2 Mo · youtube.com');
});

test('jobMeta uses the media duration and hides the upload pseudo-URL', () => {
  const job = { durationSec: 8, kind: 'image', media: media({ kind: 'image', durationSec: null, width: 1080, height: 1920, sizeBytes: 512 * 1024, sourceUrl: 'upload:shot.png' }) };
  assert.equal(jobMeta(job), 'Image · 0:08 · 1080×1920 · 512 Ko');
});

test('jobMeta is null without a media (text job) or without details', () => {
  assert.equal(jobMeta({ kind: null, text: 'gg', media: null }), null);
  assert.equal(jobMeta({ media: null }), null);
});

test('uploadName only accepts upload sources', () => {
  assert.equal(uploadName('upload:clip.mp4'), 'clip.mp4');
  assert.equal(uploadName('https://example.test/clip.mp4'), null);
  assert.equal(uploadName('upload:sha256:abc'), null);
  assert.equal(uploadName(null), null);
});
