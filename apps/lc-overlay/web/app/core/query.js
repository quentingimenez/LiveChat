// Request identity helpers: only the latest read of a resource may paint, and
// two reads of the same resource in flight share one promise.

const latest = new Map();
const inflight = new Map();
let sequence = 0;

// Marks `key` as the newest read and returns a token to check later.
export function requestToken(key) {
  sequence += 1;
  const token = { key, id: sequence };
  latest.set(key, token.id);
  return token;
}

export function isLatest(token) {
  return Boolean(token) && latest.get(token.key) === token.id;
}

// Runs `work` once per key while it is in flight. The promise is removed from
// the map on settle, whether it resolves or rejects.
export function dedupe(key, work) {
  const running = inflight.get(key);
  if (running) {
    return running;
  }
  let promise;
  try {
    promise = Promise.resolve(work());
  } catch (error) {
    promise = Promise.reject(error);
  }
  // The cleanup runs inside `finally`, before the returned promise settles:
  // awaiting the read is enough to know the key is free again.
  const tracked = promise.finally(() => {
    if (inflight.get(key) === tracked) {
      inflight.delete(key);
    }
  });
  inflight.set(key, tracked);
  return tracked;
}

// Rejects with an Error whose `code` is `timeout` when `promise` is slower
// than `ms`. The original promise is left to settle; its result is ignored.
export function withDeadline(promise, ms, code = 'timeout') {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      const error = new Error('timeout');
      error.code = code;
      reject(error);
    }, ms);
    Promise.resolve(promise).then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

export function pendingRequestCount() {
  return inflight.size;
}
