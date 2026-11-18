// Shared helpers of the Bombadil runners.

import { spawn } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const here = dirname(fileURLToPath(import.meta.url));

/**
 * The command that runs Bombadil. By default the binary shipped in the npm
 * package (Linux and macOS only). `BOMBADIL_CMD` replaces it with a JSON array,
 * e.g. `["wsl.exe","-d","Ubuntu","--","bombadil"]`.
 */
function bombadilCommand() {
  if (process.env.BOMBADIL_CMD) {
    const command = JSON.parse(process.env.BOMBADIL_CMD);
    if (!Array.isArray(command) || command.length === 0) {
      throw new Error('BOMBADIL_CMD must be a non-empty JSON array');
    }
    return command;
  }
  return [process.execPath, join(here, 'node_modules', '@antithesishq', 'bombadil', 'bin', 'bombadil.js')];
}

/**
 * Runs `bombadil browser test` from this folder (spec and output paths are
 * relative to it) and resolves with its exit code: 0 no violation, 1 error,
 * 2 property violated.
 */
export function runBombadil({ origin, spec, output, allowUrl, cookies = [] }) {
  const timeLimit = process.env.BOMBADIL_TIME_LIMIT ?? '3m';
  const args = [
    'browser',
    'test',
    '--headless',
    '--time-limit',
    timeLimit,
    '--exit-on-violation',
    '--output-path',
    output,
    '--output-path-overwrite',
    '--width',
    '1280',
    '--height',
    '800',
  ];
  if (allowUrl) {
    args.push('--allow-url', allowUrl);
  }
  for (const cookie of cookies) {
    args.push('--cookie', cookie);
  }
  if (process.env.BOMBADIL_NO_SANDBOX === '1') {
    args.push('--no-sandbox');
  }
  args.push(origin, spec);

  const [command, ...prefix] = bombadilCommand();
  const shown = args.map((arg, index) => (args[index - 1] === '--cookie' ? '<cookie>' : arg));
  console.log(`bombadil ${shown.join(' ')}`);
  const child = spawn(command, [...prefix, ...args], { cwd: here, stdio: 'inherit' });
  return new Promise((resolve) => {
    child.on('error', (error) => {
      console.error(`bombadil could not start: ${error.message}`);
      resolve(1);
    });
    child.on('exit', (code, signal) => resolve(code ?? (signal ? 1 : 0)));
  });
}

/** Polls `url` until it answers 2xx, or fails after `timeoutMs`. */
export async function waitForHttp(url, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
    } catch {
      // Not listening yet.
    }
    if (Date.now() > deadline) {
      throw new Error(`${url} did not answer within ${timeoutMs} ms`);
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
}
