import fs from 'node:fs';
import path from 'node:path';

const distDir = path.resolve('dist');
const maxExeSizeMb = Number.parseInt(process.env.MAX_WIN_EXE_SIZE_MB || '400', 10);

if (!fs.existsSync(distDir)) {
  throw new Error('dist/ folder not found');
}

const entries = fs.readdirSync(distDir, { withFileTypes: true });
const exeFiles = entries
  .filter((entry) => entry.isFile() && entry.name.toLowerCase().endsWith('.exe'))
  .map((entry) => entry.name);

const setupCandidates = exeFiles.filter((name) => /setup/i.test(name));
const portableCandidates = exeFiles.filter((name) => !/setup/i.test(name));

if (setupCandidates.length === 0) {
  throw new Error('No Windows setup .exe found in dist/');
}

if (portableCandidates.length === 0) {
  throw new Error('No Windows portable .exe found in dist/');
}

const latestYmlPath = path.join(distDir, 'latest.yml');
if (!fs.existsSync(latestYmlPath)) {
  throw new Error('latest.yml not found in dist/');
}

const setupPath = path.join(distDir, setupCandidates[0]);
const portablePath = path.join(distDir, portableCandidates[0]);
const setupSizeMb = fs.statSync(setupPath).size / (1024 * 1024);
const portableSizeMb = fs.statSync(portablePath).size / (1024 * 1024);

if (setupSizeMb > maxExeSizeMb || portableSizeMb > maxExeSizeMb) {
  throw new Error(
    `Windows exe size anomaly: setup=${setupSizeMb.toFixed(1)}MB portable=${portableSizeMb.toFixed(1)}MB (max ${maxExeSizeMb}MB)`
  );
}

console.log('Windows artifacts check passed');
console.log(`  setup: ${setupCandidates[0]} (${setupSizeMb.toFixed(1)}MB)`);
console.log(`  portable: ${portableCandidates[0]} (${portableSizeMb.toFixed(1)}MB)`);
console.log('  metadata: latest.yml');
