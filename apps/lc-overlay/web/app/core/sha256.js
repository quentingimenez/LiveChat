// Incremental SHA-256 used by the file import preflight.
//
// Web Crypto only digests a whole buffer, which would load a 2 GiB video in
// memory. The upload path already reads the file in slices, so this module
// keeps the same shape: feed chunks, then read the hexadecimal digest.

const K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
  0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
  0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
  0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
  0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
  0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

const INITIAL = new Uint32Array([
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
  0x1f83d9ab, 0x5be0cd19,
]);

/** Slice size used when hashing a `File` or `Blob`. */
export const HASH_CHUNK_SIZE = 4 * 1024 * 1024;

function rotr(value, bits) {
  return (value >>> bits) | (value << (32 - bits));
}

function compress(state, block) {
  const w = new Uint32Array(64);
  const view = new DataView(block.buffer, block.byteOffset, 64);
  for (let index = 0; index < 16; index += 1) {
    w[index] = view.getUint32(index * 4);
  }
  for (let index = 16; index < 64; index += 1) {
    const previous = w[index - 15];
    const previous2 = w[index - 2];
    const s0 = rotr(previous, 7) ^ rotr(previous, 18) ^ (previous >>> 3);
    const s1 = rotr(previous2, 17) ^ rotr(previous2, 19) ^ (previous2 >>> 10);
    w[index] = (w[index - 16] + s0 + w[index - 7] + s1) >>> 0;
  }

  let [a, b, c, d, e, f, g, h] = state;
  for (let index = 0; index < 64; index += 1) {
    const s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
    const choice = (e & f) ^ (~e & g);
    const temp1 = (h + s1 + choice + K[index] + w[index]) >>> 0;
    const s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
    const majority = (a & b) ^ (a & c) ^ (b & c);
    const temp2 = (s0 + majority) >>> 0;
    h = g;
    g = f;
    f = e;
    e = (d + temp1) >>> 0;
    d = c;
    c = b;
    b = a;
    a = (temp1 + temp2) >>> 0;
  }

  state[0] = (state[0] + a) >>> 0;
  state[1] = (state[1] + b) >>> 0;
  state[2] = (state[2] + c) >>> 0;
  state[3] = (state[3] + d) >>> 0;
  state[4] = (state[4] + e) >>> 0;
  state[5] = (state[5] + f) >>> 0;
  state[6] = (state[6] + g) >>> 0;
  state[7] = (state[7] + h) >>> 0;
}

/** Creates an incremental SHA-256 state. */
export function createSha256() {
  const state = new Uint32Array(INITIAL);
  const buffer = new Uint8Array(64);
  let buffered = 0;
  let length = 0;
  let finalized = false;

  function update(bytes) {
    if (finalized) {
      throw new Error('SHA-256 already finalized');
    }
    if (!(bytes instanceof Uint8Array)) {
      throw new TypeError('SHA-256 update expects a Uint8Array');
    }
    length += bytes.length;
    let offset = 0;
    if (buffered > 0) {
      const take = Math.min(64 - buffered, bytes.length);
      buffer.set(bytes.subarray(0, take), buffered);
      buffered += take;
      offset = take;
      if (buffered === 64) {
        compress(state, buffer);
        buffered = 0;
      }
    }
    while (offset + 64 <= bytes.length) {
      compress(state, bytes.subarray(offset, offset + 64));
      offset += 64;
    }
    if (offset < bytes.length) {
      buffer.set(bytes.subarray(offset), 0);
      buffered = bytes.length - offset;
    }
  }

  function hex() {
    if (!finalized) {
      finalized = true;
      const paddingLength = buffered < 56 ? 64 : 128;
      const padding = new Uint8Array(paddingLength);
      padding.set(buffer.subarray(0, buffered));
      padding[buffered] = 0x80;
      const bits = length * 8;
      const view = new DataView(padding.buffer);
      view.setUint32(paddingLength - 8, Math.floor(bits / 0x1_0000_0000));
      view.setUint32(paddingLength - 4, bits >>> 0);
      for (let offset = 0; offset < paddingLength; offset += 64) {
        compress(state, padding.subarray(offset, offset + 64));
      }
    }
    let result = '';
    for (const word of state) {
      result += word.toString(16).padStart(8, '0');
    }
    return result;
  }

  return { update, hex };
}

/**
 * Hashes a `File`/`Blob` by slices, calling `onProgress(fraction)` after each
 * slice. The digest is the lowercase hexadecimal SHA-256.
 */
export async function hashFile(file, onProgress) {
  const digest = createSha256();
  if (file.size === 0) {
    onProgress?.(1);
    return digest.hex();
  }
  for (let start = 0; start < file.size; start += HASH_CHUNK_SIZE) {
    const end = Math.min(start + HASH_CHUNK_SIZE, file.size);
    const bytes = new Uint8Array(await file.slice(start, end).arrayBuffer());
    digest.update(bytes);
    onProgress?.(end / file.size);
  }
  return digest.hex();
}
