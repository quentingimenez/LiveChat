import { readFile, readdir } from 'node:fs/promises';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Garde de sécurité : avec storage.local en TRUSTED_CONTEXTS, aucun content
// script ne doit tenter de lire/écrire un stockage de l'extension. Le thème
// passe désormais par des messages runtime (voir inpage-theme.test.ts qui
// vérifie aussi le chemin transitif via lib/theme.ts).

const contentDir = resolve(dirname(fileURLToPath(import.meta.url)));
const srcDir = resolve(contentDir, '..');
const STORAGE_ACCESS = /\b(?:browser|chrome)\s*\.\s*storage\s*\.\s*(?:local|session|onChanged)\b|\b(?:window\s*\.\s*)?(?:localStorage|sessionStorage)\b/;

const stripComments = (source: string): string =>
  source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:])\/\/.*$/gm, '$1');

describe('isolation des content scripts', () => {
  it('aucun module de extension/src/content n’accède à un stockage', async () => {
    const entries = await readdir(contentDir, { withFileTypes: true });
    const modules = entries.filter((entry) => entry.isFile() && entry.name.endsWith('.ts') && !entry.name.endsWith('.test.ts'));
    const violations: string[] = [];

    for (const module of modules) {
      const file = resolve(contentDir, module.name);
      const source = stripComments(await readFile(file, 'utf8'));
      const match = source.match(STORAGE_ACCESS);

      if (match) {
        violations.push(`${relative(srcDir, file)}: ${match[0]}`);
      }
    }

    expect(violations, `${modules.length} modules content analysés`).toEqual([]);
  });
});
