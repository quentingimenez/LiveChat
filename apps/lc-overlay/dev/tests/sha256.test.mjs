// Protects the incremental SHA-256 used before an upload preflight: the digest
// must stay identical when a file is fed in arbitrary slices.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { createSha256, hashFile } from '../../web/app/core/sha256.js';

const encoder = new TextEncoder();

function digest(text) {
  const sha = createSha256();
  sha.update(encoder.encode(text));
  return sha.hex();
}

test('SHA-256 matches the reference vectors', () => {
  assert.equal(
    digest(''),
    'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
  );
  assert.equal(
    digest('abc'),
    'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
  );
  assert.equal(
    digest('abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq'),
    '248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1',
  );
  assert.equal(
    digest('a'.repeat(1_000_000)),
    'cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0',
  );
});

test('a digest is independent from the slice boundaries', () => {
  const bytes = encoder.encode('abc');
  const whole = createSha256();
  whole.update(bytes);
  const split = createSha256();
  split.update(bytes.subarray(0, 1));
  split.update(bytes.subarray(1, 2));
  split.update(bytes.subarray(2));
  assert.equal(split.hex(), whole.hex());
});

test('hashFile slices a blob and reports its progress', async () => {
  const bytes = encoder.encode('abc');
  const progress = [];
  const hex = await hashFile(new Blob([bytes]), (fraction) => progress.push(fraction));
  assert.equal(
    hex,
    'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
  );
  assert.equal(progress.at(-1), 1);
});

test('hashFile handles an empty file', async () => {
  const progress = [];
  const hex = await hashFile(new Blob([]), (fraction) => progress.push(fraction));
  assert.equal(
    hex,
    'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
  );
  assert.deepEqual(progress, [1]);
});
