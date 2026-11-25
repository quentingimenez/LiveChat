// Rafraîchir une carte depuis sa source (C) : le menu de la carte propose
// l’action pour un lien avec le droit d’ajouter, et l’ingestion repart en
// `forceRefresh: true` pour que le serveur remplace le média et sa cover. Le
// dialogue Ajouter ne doit plus proposer ce bouton : le contournement du cache
// y est la seule action.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const addMedia = read('views/add-media.js');
const board = read('views/board.js');
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
    // An apostrophe in a comment is not a string opener (“Another guild's
    // board…” would swallow the rest of the function).
    if (char === '/' && source[index + 1] === '/') {
      const end = source.indexOf('\n', index);
      index = end === -1 ? source.length : end;
      continue;
    }
    if (char === '/' && source[index + 1] === '*') {
      const end = source.indexOf('*/', index + 2);
      index = end === -1 ? source.length : end + 1;
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

test('le menu d’une carte de lien propose le rafraîchissement', () => {
  const menu = balancedBlock(board, 'function openCardMenu(event, item)', '{', '}');
  assert.match(menu, /'Rafraîchir depuis la source'/);
  assert.match(menu, /link && can\('addMedia'\) && item\.media\.edited !== true/);
  assert.match(menu, /refreshSource\(item\)/);
  const helper = balancedBlock(board, 'function refreshSource(item)', '{', '}');
  assert.match(helper, /startLinkRefresh\(\{ guildId: state\.activeGuildId, item \}\)/);
});

test('le dialogue Ajouter ne propose plus le bouton de rafraîchissement', () => {
  assert.doesNotMatch(addMedia, /add-media-duplicate/);
  assert.doesNotMatch(addMedia, /'Rafraîchir depuis la source'/);
  assert.doesNotMatch(addMedia, /syncDuplicateLink/);
  assert.doesNotMatch(addMedia, /startLinkRefresh/);
});

test('un rafraîchissement réutilise l’ingestion de lien avec forceRefresh', () => {
  const refreshStart = uploads.indexOf('export function startLinkRefresh');
  assert.notEqual(refreshStart, -1);
  const refreshLines = uploads.slice(refreshStart).split(/\r?\n/);
  const refreshEnd = refreshLines.indexOf('}');
  assert.notEqual(refreshEnd, -1);
  const refresh = refreshLines.slice(0, refreshEnd + 1).join('\n');
  const entry = balancedBlock(
    uploads.slice(uploads.indexOf('export function startLinkUpload')),
    'const entry = {',
    '{',
    '}',
  );
  const finish = balancedBlock(uploads, 'async function finishIngestion(entry, result)', '{', '}');
  assert.match(refresh, /forceRefresh: true/);
  assert.match(refresh, /refreshItemId: item\.id/);
  assert.match(refresh, /startLinkUpload\(\{/);
  assert.match(entry, /refreshItemId: refreshItemId \|\| null/);
  assert.match(finish, /entry\.forceRefresh && result\.created === false/);
  assert.match(finish, /requestItemsReload\(\)\.then\(done\)/);
  assert.match(finish, /rafraîchi depuis sa source/);
  const refreshBranch = finish.indexOf('entry.forceRefresh');
  const genericDuplicate = finish.indexOf('if (result.created === false)');
  assert.ok(refreshBranch >= 0 && refreshBranch < genericDuplicate);
});
