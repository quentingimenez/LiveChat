// L’option « Ignorer le cache serveur » est propre à une ingestion de lien :
// la board l’offre dans le panneau Lien du dialogue Ajouter, jamais pour un
// fichier, et l’envoi transmet `forceRefresh` au serveur. Ces tests gardent le
// câblage que les helpers purs ne peuvent pas prouver : présence du contrôle,
// suivi de la source et corps de la requête.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const addMedia = read('views/add-media.js');
const uploads = read('views/uploads.js');

// Content of the call or block starting at `marker`, strings and templates
// ignored: the selectors under test may contain parentheses or braces.
function balancedBlock(source, marker, openChar, closeChar) {
  const start = source.indexOf(marker);
  assert.notEqual(start, -1, `missing block: ${marker}`);
  const open = source.indexOf(openChar, start);
  let depth = 0;
  let quote = '';
  for (let index = open; index < source.length; index += 1) {
    const char = source[index];
    if (quote) {
      if (char === '\\') {
        index += 1;
      } else if (char === quote) {
        quote = '';
      }
      continue;
    }
    if (char === '"' || char === "'" || char === '`') {
      quote = char;
    } else if (char === openChar) {
      depth += 1;
    } else if (char === closeChar) {
      depth -= 1;
      if (depth === 0) {
        return source.slice(start, index + 1);
      }
    }
  }
  assert.fail(`unterminated block after ${marker}`);
}

const dialog = balancedBlock(addMedia, 'export function openAddDialog', '{', '}');
const syncFields = balancedBlock(dialog, 'function syncFields', '{', '}');
const linkPanel = balancedBlock(dialog, 'const linkPanel = h(', '(', ')');
const filePanel = balancedBlock(dialog, 'const filePanel = h(', '(', ')');
const payload = balancedBlock(dialog, 'const payload = {', '{', '}');
const startLink = balancedBlock(uploads, 'export function startLinkUpload', '(', ')');
const linkEntry = balancedBlock(
  uploads.slice(uploads.indexOf('export function startLinkUpload')),
  'const entry = {',
  '{',
  '}',
);
const sendLink = balancedBlock(uploads, 'async function sendLink', '{', '}');

test('le dialogue offre le contournement dans le panneau du lien', () => {
  assert.match(dialog, /class: 'switch cache-switch'/);
  assert.match(dialog, /'Ignorer le cache serveur'/);
  assert.match(linkPanel, /forceRefreshOption/);
  assert.doesNotMatch(filePanel, /forceRefreshOption/);
});

test('le contournement suit la source : lien seulement', () => {
  assert.match(syncFields, /const bypassCache = canBypassServerCache\(mode\)/);
  assert.match(syncFields, /forceRefreshOption\.hidden = !bypassCache/);
  assert.match(syncFields, /forceRefreshInput\.disabled = !bypassCache/);
});

test('le formulaire transmet la case au lancement de l’envoi', () => {
  assert.match(payload, /forceRefresh: forceRefreshFor\(mode, forceRefreshInput\.checked\)/);
});

test('l’entrée d’envoi garde le choix et le corps l’envoie au board', () => {
  assert.match(startLink, /forceRefresh = false/);
  assert.match(linkEntry, /forceRefresh: Boolean\(forceRefresh\)/);
  assert.match(sendLink, /const body = linkUploadBody\(entry\)/);
});
