import fs from 'node:fs';
import path from 'node:path';

const distDir = path.resolve('dist');

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

console.log('Windows artifacts check passed');
console.log(`  setup: ${setupCandidates[0]}`);
console.log(`  portable: ${portableCandidates[0]}`);
console.log('  metadata: latest.yml');
