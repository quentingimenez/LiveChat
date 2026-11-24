// Rafraîchir une carte depuis sa source existe à deux endroits : le menu de la
// carte (C) et le dialogue Ajouter quand l’URL est déjà sur la board (E). Ces
// tests gardent le câblage : le menu n’apparaît que pour un lien avec le droit
// d’ajouter, l’ingestion repart en `forceRefresh: true`, et la carte existante
// est rechargée au lieu d’afficher « déjà sur le board ».

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
  assert.match(menu, /link && can\('addMedia'\)/);
  assert.match(menu, /refreshSource\(item\)/);
  const helper = balancedBlock(board, 'function refreshSource(item)', '{', '}');
  assert.match(helper, /startLinkRefresh\(\{ guildId: state\.activeGuildId, item \}\)/);
});

test('le dialogue repère l’URL déjà présente et propose le rafraîchissement', () => {
  const dialog = balancedBlock(addMedia, 'export function openAddDialog(options)', '{', '}');
  const linkPanel = balancedBlock(dialog, 'const linkPanel = h(', '(', ')');
  const syncFields = balancedBlock(dialog, 'function syncFields()', '{', '}');
  const syncDuplicate = balancedBlock(dialog, 'function syncDuplicateLink()', '{', '}');
  assert.match(dialog, /class: 'add-media-duplicate'/);
  assert.match(dialog, /'Déjà sur la board\.'/);
  assert.match(dialog, /'Rafraîchir depuis la source'/);
  assert.match(dialog, /boardItemForUrl\(state\.items, linkUrl\.value\)/);
  assert.match(linkPanel, /duplicateLink/);
  assert.match(syncFields, /syncDuplicateLink\(\)/);
  assert.match(syncDuplicate, /canAdd && action === 'add' && mode === 'link'/);
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
