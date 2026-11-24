import { createHash } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Garde-fou anti-retour : le cœur partagé ne doit pas être recopié dans les
// sites, et deux modules de production ne doivent pas rester byte-identiques.
// Sur 0296afa ce test est ROUGE : les trois modales ont le même SHA-256.
const contentDir = resolve(dirname(fileURLToPath(import.meta.url)));
const srcDir = resolve(contentDir, '..');
const sites = ['twitter.ts', 'youtube.ts', 'tiktok.ts'] as const;

const isProductionModule = (name: string): boolean =>
  name.endsWith('.ts') && !name.endsWith('.test.ts') && name !== 'testSupport.ts';

const sha256 = (buffer: Buffer): string => createHash('sha256').update(buffer).digest('hex').toUpperCase();

// Identifiants déplacés dans shared-core : toute déclaration locale dans un
// site est une copie interdite (les appels, eux, sont autorisés).
const movedDeclarations: ReadonlyArray<readonly [string, RegExp]> = [
  ['attachLongPressCompose', /\b(?:const|let|var|function)\s+attachLongPressCompose\b/],
  ['LongPressComposeOptions', /\b(?:interface|type)\s+LongPressComposeOptions\b/],
  ['LongPressComposeBinding', /\b(?:interface|type)\s+LongPressComposeBinding\b/],
  ['buttonResetTimers', /\b(?:const|let|var)\s+buttonResetTimers\b/],
  ['buttonClickTimers', /\b(?:const|let|var)\s+buttonClickTimers\b/],
  ['SINGLE_CLICK_DELAY_MS', /\b(?:const|let|var)\s+SINGLE_CLICK_DELAY_MS\b/],
  ['ToastLevel', /\btype\s+ToastLevel\s*=|\binterface\s+ToastLevel\b/],
  ['TOAST_STYLE_ID', /\b(?:const|let|var)\s+TOAST_STYLE_ID\b/],
  ['TOAST_CONTAINER_ID', /\b(?:const|let|var)\s+TOAST_CONTAINER_ID\b/],
  ['ensureToastStyles', /\b(?:const|let|var|function)\s+ensureToastStyles\b/],
  ['getToastContainer', /\b(?:const|let|var|function)\s+getToastContainer\b/],
  ['showToast', /\b(?:const|let|var|function)\s+showToast\b/],
  ['isShowToastPayload', /\b(?:const|let|var|function)\s+isShowToastPayload\b/],
  ['isAuthStateResponse', /\b(?:const|let|var|function)\s+isAuthStateResponse\b/],
  ['clearButtonClickTimer', /\b(?:const|let|var|function)\s+clearButtonClickTimer\b/],
  ['scheduleSingleClickAction', /\b(?:const|let|var|function)\s+scheduleSingleClickAction\b/],
  ['clearButtonResetTimer', /\b(?:const|let|var|function)\s+clearButtonResetTimer\b/],
];

const stripComments = (source: string): string =>
  source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:])\/\/.*$/gm, '$1');

describe('noyau partagé des content scripts', () => {
  it('aucun module de production n’est byte-identique à un autre', async () => {
    const entries = await readdir(contentDir, { withFileTypes: true });
    const modules = entries.filter((entry) => entry.isFile() && isProductionModule(entry.name));
    const byHash = new Map<string, string[]>();

    for (const module of modules) {
      const buffer = await readFile(resolve(contentDir, module.name));
      const hash = sha256(buffer);
      const group = byHash.get(hash) ?? [];
      group.push(relative(srcDir, resolve(contentDir, module.name)));
      byHash.set(hash, group);
    }

    const duplicates = [...byHash.entries()].filter(([, names]) => names.length > 1);
    expect(
      duplicates.map(([hash, names]) => `${hash.slice(0, 16)} : ${names.join(', ')}`),
      'deux modules de production ont le même SHA-256',
    ).toEqual([]);
  });

  it('les sites n’ont aucune copie locale des identifiants déplacés', async () => {
    const violations: string[] = [];

    for (const name of sites) {
      const source = stripComments(await readFile(resolve(contentDir, name), 'utf8'));

      for (const [identifier, pattern] of movedDeclarations) {
        if (pattern.test(source)) {
          violations.push(`${name} redéclare ${identifier}`);
        }
      }

      if (!/from\s*'\.\/shared-core'/.test(source)) {
        violations.push(`${name} n'importe pas ./shared-core`);
      }
    }

    expect(violations, 'copies locales du noyau partagé').toEqual([]);
  });
});
