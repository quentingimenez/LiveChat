import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

const readPage = async (name: 'options' | 'popup'): Promise<string> =>
  readFile(resolve(root, `${name}.html`), 'utf8');

const readScript = async (name: 'options' | 'popup'): Promise<string> =>
  readFile(resolve(root, `src/${name}/${name}.ts`), 'utf8');

/** Identifiants que le script cherche dans la page. */
const elementIds = (source: string): string[] =>
  [...source.matchAll(/getElementById\('([^']+)'\)/g)].map((match) => match[1]);

describe('pages d’entrée options et popup', () => {
  it('expose chaque élément manipulé par le script de la page', async () => {
    for (const name of ['options', 'popup'] as const) {
      const [html, script] = await Promise.all([readPage(name), readScript(name)]);
      const ids = elementIds(script);

      expect(ids.length).toBeGreaterThan(0);
      for (const id of ids) {
        expect(html, `${name}.html doit porter id="${id}"`).toContain(`id="${id}"`);
      }
    }
  });

  it('garde la configuration manuelle repliée et sans champ auteur', async () => {
    const html = await readPage('options');

    expect(html).toMatch(/<details[^>]*>\s*<summary>Configuration manuelle<\/summary>/);
    expect(html).not.toContain('authorName');
    expect(html).not.toContain('authorImage');
    // Une seule adresse de serveur : plus de champ miroir `api-url-manual`.
    expect(html).not.toContain('api-url-manual');
    expect(html.match(/id="api-url"/g)).toHaveLength(1);
  });

  it('garde les options d’envoi repliées dans le popup', async () => {
    const html = await readPage('popup');

    expect(html).toMatch(/<details[^>]*>\s*<summary>Options<\/summary>/);
    expect(html).not.toContain('authorName');
    expect(html).not.toContain('authorImage');
  });
});
