#!/usr/bin/env node
// Agrégateur de CI pour les pull requests.
//
// Pourquoi ce script : l'auto-merge GitHub n'attend que les « required status
// checks » de la protection de branche. Les workflows rust/overlay/extension
// sont filtrés par chemins ; les exiger directement bloquerait les PR qui ne
// touchent pas ces chemins. Ce gate tourne sur toutes les PR, attend les check
// runs réellement créés pour le commit de tête, puis publie un unique check
// « ci-gate » que la protection peut exiger.
//
// Règles :
// - le check du gate lui-même est exclu (sinon interblocage) ;
// - la sentinelle `versions & release tooling` doit être présente : c'est le
//   seul workflow qui tourne sur toutes les PR, elle prouve que les checks de
//   CI ont bien été créés ;
// - un check `failure`, `cancelled`, `timed_out`, `action_required` ou `stale`
//   fait échouer le gate ; `success`, `neutral` et `skipped` passent ;
// - le gate attend une période de stabilité sans nouveau check avant de
//   conclure, pour ne pas rater un job créé un peu après les autres (les jobs
//   qui dépendent d'un autre via `needs` n'apparaissent qu'à la fin de celui-ci).

import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const API = 'https://api.github.com';
const OK_CONCLUSIONS = new Set(['success', 'neutral', 'skipped']);
const DEFAULT_SENTINEL = 'versions & release tooling';
const DEFAULT_CHECK_NAME = 'ci-gate';
const DEFAULT_CHECK_APP = 'github-actions';

export class GateError extends Error {
  constructor(message, details = {}) {
    super(message);
    this.name = 'GateError';
    this.details = details;
  }
}

function positiveInt(value, fallback, name) {
  if (value === undefined || value === '') return fallback;
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new GateError(`${name} doit être un entier positif, reçu ${JSON.stringify(value)}`);
  }
  return parsed;
}

export function readConfig(env = process.env) {
  const repo = env.REPO || env.GITHUB_REPOSITORY;
  const headSha = env.PR_HEAD_SHA;
  const token = env.GITHUB_TOKEN;
  const missing = [];
  if (!repo) missing.push('REPO');
  if (!headSha) missing.push('PR_HEAD_SHA');
  if (!token) missing.push('GITHUB_TOKEN');
  if (missing.length > 0) {
    throw new GateError(`variables d'environnement manquantes : ${missing.join(', ')}`);
  }
  if (!/^[^/]+\/[^/]+$/.test(repo)) {
    throw new GateError(`REPO doit être owner/name, reçu ${JSON.stringify(repo)}`);
  }
  return {
    repo,
    headSha,
    token,
    sentinel: env.GATE_SENTINEL || DEFAULT_SENTINEL,
    checkName: env.GATE_CHECK_NAME || DEFAULT_CHECK_NAME,
    checkApp: env.GATE_CHECK_APP || DEFAULT_CHECK_APP,
    timeoutMs: positiveInt(env.GATE_TIMEOUT_SECONDS, 3300, 'GATE_TIMEOUT_SECONDS') * 1000,
    pollMs: positiveInt(env.GATE_POLL_SECONDS, 15, 'GATE_POLL_SECONDS') * 1000,
    stabilityMs: positiveInt(env.GATE_STABILITY_SECONDS, 60, 'GATE_STABILITY_SECONDS') * 1000,
  };
}

async function requestJson(ctx, path) {
  let lastError;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    let response;
    try {
      response = await ctx.fetchImpl(`${API}${path}`, {
        headers: {
          accept: 'application/vnd.github+json',
          authorization: `Bearer ${ctx.config.token}`,
          'x-github-api-version': '2022-11-28',
          'user-agent': 'livechat-ci-gate',
        },
      });
    } catch (error) {
      lastError = error;
    }
    if (response) {
      if (response.ok) return await response.json();
      const body = await response.text().catch(() => '');
      lastError = new GateError(
        `API GitHub ${response.status} ${response.statusText} pour ${path}${body ? ` : ${body.slice(0, 300)}` : ''}`,
      );
      if (response.status !== 429 && response.status < 500) throw lastError;
    }
    if (attempt < 3) await ctx.sleep(attempt * 1000);
  }
  throw lastError;
}

async function listCheckRuns(ctx) {
  const runs = [];
  for (let page = 1; page <= 10; page += 1) {
    const data = await requestJson(
      ctx,
      `/repos/${ctx.config.repo}/commits/${ctx.config.headSha}/check-runs?per_page=100&filter=latest&page=${page}`,
    );
    const batch = data.check_runs || [];
    runs.push(...batch);
    if (batch.length === 0 || runs.length >= (data.total_count ?? runs.length)) break;
  }
  return runs;
}

function isOwnCheck(run, config) {
  return run.name === config.checkName && (run.app?.slug || '') === config.checkApp;
}

function checkStateKey(runs) {
  return runs
    .map((run) => `${run.id}:${run.name}:${run.status}:${run.conclusion}`)
    .sort()
    .join('|');
}

function describeFailures(runs) {
  return runs
    .map((run) => `- ${run.name} (${run.conclusion || 'conclusion inconnue'})`)
    .join('\n');
}

function defaultSleep(ms) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}

export async function runGate({
  env = process.env,
  fetchImpl = globalThis.fetch,
  sleep = defaultSleep,
  now = () => Date.now(),
  log = console.log,
} = {}) {
  const config = readConfig(env);
  if (typeof fetchImpl !== 'function') {
    throw new GateError('fetch est indisponible : Node.js 20 ou plus est requis');
  }
  const ctx = { config, fetchImpl, sleep };
  const startedAt = now();
  let lastKey = null;
  let stableSince = null;
  let lastProgressAt = Number.NEGATIVE_INFINITY;

  const progress = (message) => {
    if (now() - lastProgressAt >= 30_000) {
      lastProgressAt = now();
      log(message);
    }
  };

  for (;;) {
    if (now() - startedAt >= config.timeoutMs) {
      const runs = await listCheckRuns(ctx).catch(() => []);
      const included = runs.filter((run) => !isOwnCheck(run, config));
      const names = included.map((run) => run.name);
      const pending = included.filter((run) => run.status !== 'completed');
      const why = names.includes(config.sentinel)
        ? `checks encore en cours : ${pending.map((run) => `${run.name} (${run.status})`).join(', ') || 'aucun'}`
        : `sentinelle ${JSON.stringify(config.sentinel)} absente`;
      throw new GateError(`ci-gate timeout après ${Math.round((now() - startedAt) / 1000)} s — ${why}`);
    }

    const runs = await listCheckRuns(ctx);
    const included = runs.filter((run) => !isOwnCheck(run, config));
    const names = included.map((run) => run.name);

    if (!names.includes(config.sentinel)) {
      progress(`ci-gate : en attente de la sentinelle ${JSON.stringify(config.sentinel)} (${included.length} check(s))`);
      await sleep(config.pollMs);
      continue;
    }

    const failed = included.filter((run) => run.status === 'completed' && !OK_CONCLUSIONS.has(run.conclusion));
    if (failed.length > 0) {
      throw new GateError(`ci-gate : ${failed.length} check(s) en échec :\n${describeFailures(failed)}`, {
        runs: included,
      });
    }

    const pending = included.filter((run) => run.status !== 'completed');
    if (pending.length > 0) {
      lastKey = null;
      stableSince = null;
      progress(`ci-gate : ${pending.length} check(s) en cours (${pending.map((run) => `${run.name}:${run.status}`).join(', ')})`);
      await sleep(config.pollMs);
      continue;
    }

    const key = checkStateKey(included);
    if (key !== lastKey) {
      lastKey = key;
      stableSince = now();
      progress(`ci-gate : ${included.length} check(s) complets, vérification de stabilité…`);
      await sleep(config.pollMs);
      continue;
    }
    if (now() - stableSince >= config.stabilityMs) {
      log(`ci-gate : OK — ${included.length} check(s) de PR complets et stables`);
      return { runs: included, elapsedMs: now() - startedAt };
    }
    await sleep(config.pollMs);
  }
}

const isMain = process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href;
if (isMain) {
  runGate({ env: process.env })
    .then((result) => {
      console.log(`ci-gate : ${result.runs.length} check(s) vérifiés en ${Math.round(result.elapsedMs / 1000)} s`);
    })
    .catch((error) => {
      console.error(error instanceof GateError ? error.message : error?.stack || String(error));
      process.exitCode = 1;
    });
}
