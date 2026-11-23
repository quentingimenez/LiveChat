// Protège scripts/ci/gate.mjs : le gate agrégé ne doit passer que lorsque
// tous les check runs de CI du commit de tête sont terminés et verts, sans
// attendre sa propre exécution.
import test from 'node:test';
import assert from 'node:assert/strict';

import { GateError, runGate } from '../gate.mjs';

const SENTINEL = 'versions & release tooling';
const ENV = {
  REPO: 'o/r',
  PR_HEAD_SHA: 'sha',
  GITHUB_TOKEN: 'token',
  GATE_TIMEOUT_SECONDS: '30',
  GATE_POLL_SECONDS: '1',
  GATE_STABILITY_SECONDS: '2',
};

let nextId = 1;

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function run(name, status = 'completed', conclusion = 'success', app = 'github-actions') {
  return { id: nextId, name, status, conclusion, app: { slug: app } };
}

function makeFetch(pages) {
  let pageIndex = 0;
  return async (input) => {
    const url = new URL(input);
    if (url.pathname === '/repos/o/r/commits/sha/check-runs') {
      const current = pages[Math.min(pageIndex, pages.length - 1)];
      pageIndex += 1;
      return json(current);
    }
    throw new Error(`requête inattendue : ${url.pathname}`);
  };
}

function clock() {
  let value = 0;
  return {
    now: () => value,
    sleep: async (ms) => {
      value += ms;
    },
  };
}

test('attend la sentinelle puis la stabilité avant de conclure', async () => {
  const pending = run(SENTINEL, 'in_progress', null);
  const ok = run(SENTINEL);
  const fmt = run('fmt');
  const clippy = run('clippy');
  const pages = [
    { total_count: 1, check_runs: [pending] },
    { total_count: 3, check_runs: [ok, fmt, clippy] },
  ];
  const result = await runGate({
    env: ENV,
    fetchImpl: makeFetch(pages),
    ...clock(),
    log: () => {},
  });

  assert.deepEqual(
    result.runs.map((item) => item.name).sort(),
    ['clippy', 'fmt', SENTINEL],
  );
});

test('échoue en nommant le check fautif', async () => {
  const ok = run(SENTINEL);
  const clippy = run('clippy', 'completed', 'failure');
  const fmt = run('fmt');
  const pages = [{ total_count: 3, check_runs: [ok, clippy, fmt] }];

  await assert.rejects(
    runGate({ env: ENV, fetchImpl: makeFetch(pages), ...clock(), log: () => {} }),
    (error) => {
      assert.ok(error instanceof GateError);
      assert.match(error.message, /1 check\(s\) en échec/);
      assert.match(error.message, /clippy \(failure\)/);
      assert.doesNotMatch(error.message, /fmt/);
      return true;
    },
  );
});

test('ignore son propre check, qui reste en cours pendant tout le gate', async () => {
  const own = run('ci-gate', 'in_progress', null);
  const ok = run(SENTINEL);
  const fmt = run('fmt');
  const pages = [{ total_count: 3, check_runs: [own, ok, fmt] }];

  const result = await runGate({
    env: ENV,
    fetchImpl: makeFetch(pages),
    ...clock(),
    log: () => {},
  });

  assert.deepEqual(result.runs.map((item) => item.name).sort(), ['fmt', SENTINEL]);
});

test('attend un check run qui apparaît après le premier passage', async () => {
  const ok = run(SENTINEL);
  const fmt = run('fmt');
  const testPending = run('test (ubuntu-latest)', 'in_progress', null);
  const testOk = run('test (ubuntu-latest)');
  const pages = [
    { total_count: 2, check_runs: [ok, fmt] },
    { total_count: 3, check_runs: [ok, fmt, testPending] },
    { total_count: 3, check_runs: [ok, fmt, testOk] },
  ];

  const result = await runGate({
    env: ENV,
    fetchImpl: makeFetch(pages),
    ...clock(),
    log: () => {},
  });

  assert.deepEqual(result.runs.map((item) => item.name).sort(), ['fmt', 'test (ubuntu-latest)', SENTINEL]);
});

test('timeout quand la sentinelle n’apparaît jamais', async () => {
  const env = { ...ENV, GATE_TIMEOUT_SECONDS: '3' };
  const pages = [{ total_count: 0, check_runs: [] }];

  await assert.rejects(
    runGate({ env, fetchImpl: makeFetch(pages), ...clock(), log: () => {} }),
    (error) => {
      assert.ok(error instanceof GateError);
      assert.match(error.message, /timeout/);
      assert.match(error.message, /sentinelle/);
      return true;
    },
  );
});

test('timeout quand un check run reste en cours', async () => {
  const env = { ...ENV, GATE_TIMEOUT_SECONDS: '3' };
  const ok = run(SENTINEL);
  const pending = run('test (ubuntu-latest)', 'in_progress', null);
  const pages = [{ total_count: 2, check_runs: [ok, pending] }];

  await assert.rejects(
    runGate({ env, fetchImpl: makeFetch(pages), ...clock(), log: () => {} }),
    (error) => {
      assert.ok(error instanceof GateError);
      assert.match(error.message, /timeout/);
      assert.match(error.message, /test \(ubuntu-latest\) \(in_progress\)/);
      return true;
    },
  );
});

test('accepte les checks skippés et neutres', async () => {
  const ok = run(SENTINEL);
  const draft = run('brouillon de release', 'completed', 'skipped');
  const audit = run('audit (bloquant)', 'completed', 'neutral');
  const pages = [{ total_count: 3, check_runs: [ok, draft, audit] }];

  const result = await runGate({
    env: ENV,
    fetchImpl: makeFetch(pages),
    ...clock(),
    log: () => {},
  });

  assert.deepEqual(
    result.runs.map((item) => item.name).sort(),
    ['audit (bloquant)', 'brouillon de release', SENTINEL],
  );
});
