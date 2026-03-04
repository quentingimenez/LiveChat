import fs from 'node:fs';
import path from 'node:path';

const rendererEntries = [
  {
    label: 'pairing',
    filePath: path.resolve('dist/renderer/pairing/index.js'),
    expectedSnippets: ["from './pairing-controller.js'", "from './pairing-dom.js'"]
  },
  {
    label: 'overlay',
    filePath: path.resolve('dist/renderer/overlay/index.js'),
    expectedSnippets: ["from '../legacy-loader.js'"]
  },
  {
    label: 'board',
    filePath: path.resolve('dist/renderer/board/index.js'),
    expectedSnippets: ["from '../legacy-loader.js'"]
  }
];

const forbiddenPatterns = [/\brequire\s*\(/, /\bexports\./, /\bmodule\.exports\b/];
const failures = [];

for (const entry of rendererEntries) {
  if (!fs.existsSync(entry.filePath)) {
    failures.push(`[${entry.label}] Missing compiled entry: ${entry.filePath}`);
    continue;
  }

  const source = fs.readFileSync(entry.filePath, 'utf8');

  for (const pattern of forbiddenPatterns) {
    if (pattern.test(source)) {
      failures.push(`[${entry.label}] Found CommonJS marker ${pattern} in ${entry.filePath}`);
    }
  }

  for (const snippet of entry.expectedSnippets) {
    if (!source.includes(snippet)) {
      failures.push(`[${entry.label}] Missing expected ESM import snippet: ${snippet}`);
    }
  }
}

if (failures.length > 0) {
  console.error('Renderer ESM check failed:');
  for (const failure of failures) {
    console.error(`  - ${failure}`);
  }
  process.exit(1);
}

console.log('Renderer ESM check passed');
for (const entry of rendererEntries) {
  console.log(`  - ${entry.label}: ${path.relative(process.cwd(), entry.filePath)}`);
}
