// L’option « Ignorer le cache serveur » couvre les deux sources du dialogue
// Ajouter : un lien déjà connu du serveur, ou un fichier dont le SHA-256 est
// déjà dans le cache de la guilde. Un média déjà sur la board bloque l’envoi
// tant que le contournement n’est pas coché. Ces tests gardent le câblage que
// les helpers purs ne peuvent pas prouver : présence du contrôle, debounce et
// séquence des lookups, condition d’affichage et corps des requêtes.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const addMedia = read('views/add-media.js');
const api = read('core/api.js');
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
const syncCache = balancedBlock(dialog, 'function syncCacheOption', '{', '}');
const syncBoard = balancedBlock(dialog, 'function syncBoardItem', '{', '}');
const updateSubmit = balancedBlock(dialog, 'function updateSubmit', '{', '}');
const applyCache = balancedBlock(dialog, 'function applyCacheVisibility', '{', '}');
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
const startFile = balancedBlock(uploads, 'export function startFileUpload', '(', ')');
const fileEntry = balancedBlock(
  uploads.slice(uploads.indexOf('export function startFileUpload')),
  'const entry = {',
  '{',
  '}',
);
const sendFile = balancedBlock(uploads, 'async function sendFile', '{', '}');
const preflight = balancedBlock(dialog, 'async function runFilePreflight', '{', '}');
const schedulePreflight = balancedBlock(dialog, 'function scheduleFilePreflight', '{', '}');
const fileCacheUi = balancedBlock(dialog, 'function updateFileCacheUi', '{', '}');

test('le dialogue offre un seul contournement, commun aux deux sources', () => {
  assert.match(dialog, /class: 'switch cache-switch'/);
  assert.match(dialog, /'Ignorer le cache serveur'/);
  assert.doesNotMatch(linkPanel, /forceRefreshOption/);
  assert.doesNotMatch(filePanel, /forceRefreshOption/);
  assert.match(dialog, /filePanel,\s*forceRefreshOption,\s*h\(/);
});

test('le contournement suit la source et le lookup global du serveur', () => {
  assert.match(syncCache, /cacheLookupUrl\(mode, linkUrl\.value\)/);
  assert.match(syncCache, /lookupMediaCache\(state\.activeGuildId, url\)/);
  assert.match(syncCache, /cacheBypassVisible\(mode, cached\)/);
  assert.match(syncCache, /token !== cacheToken/);
  assert.match(syncCache, /setTimeout\(/);
  assert.match(syncCache, /300/);
  assert.match(applyCache, /forceRefreshOption\.hidden = !visible/);
  assert.match(applyCache, /forceRefreshInput\.disabled = !visible/);
  assert.match(applyCache, /forceRefreshInput\.checked = false/);
  assert.match(syncFields, /syncCacheOption\(\)/);
});

test('un média déjà sur la board bloque l’envoi et affiche le message', () => {
  assert.match(dialog, /class: 'add-media-board-hint'/);
  assert.match(dialog, /Le média est déjà présent sur la board\./);
  assert.match(dialog, /onChange: \(\) => updateSubmit\(\)/);
  assert.match(syncBoard, /boardItemForUrl\(state\.items, linkUrl\.value\)/);
  assert.match(updateSubmit, /boardDuplicateNeedsRefresh\(action, mode, boardItem, forceRefreshInput\.checked\)/);
  assert.match(updateSubmit, /!boardItemIsEdited\(boardItem\)/);
  assert.match(updateSubmit, /boardHint\.hidden/);
  assert.match(syncCache, /boardItem !== null/);
  assert.match(syncCache, /applyCacheVisibility\(true\)/);
});

test('le lookup passe par la route dédiée avec un délai court', () => {
  assert.match(api, /\/overlay\/media-board\/cache/);
  assert.match(api, /timeoutMs: 5000/);
  assert.match(api, /body\.cached === true/);
});

test('le formulaire transmet la case au lancement de l’envoi', () => {
  assert.match(payload, /forceRefresh: forceRefreshFor\(mode, forceRefreshInput\.checked\)/);
});

test('l’entrée d’envoi garde le choix et le corps l’envoie au board', () => {
  assert.match(startLink, /forceRefresh = false/);
  assert.match(linkEntry, /forceRefresh: Boolean\(forceRefresh\)/);
  assert.match(sendLink, /const body = linkUploadBody\(entry\)/);
});

test('le préflight fichier hache puis interroge le cache avant de laisser envoyer', () => {
  assert.match(dialog, /filePreflightState\(chosenFiles, fileChecks\)/);
  assert.match(preflight, /hashFile\(file\)/);
  assert.match(preflight, /lookupUploadCache\(state\.activeGuildId, sha256\)/);
  assert.match(preflight, /token !== fileToken/);
  assert.match(schedulePreflight, /setTimeout\(/);
  assert.match(schedulePreflight, /150/);
  assert.match(fileCacheUi, /applyCacheVisibility\(!checking && anyCached\)/);
  assert.match(fileCacheUi, /Vérification des fichiers…/);
  assert.match(updateSubmit, /fileChecking/);
  assert.match(updateSubmit, /allOnBoard/);
  assert.match(updateSubmit, /!forceRefreshInput\.checked/);
});

test('un ajout saute les fichiers de la board et envoie le contournement', () => {
  assert.match(dialog, /filesToUpload\(action, forceRefresh, chosenFiles, fileChecks\)/);
  assert.match(dialog, /sha256: check\?\.sha256 \|\| null/);
  assert.match(dialog, /forceRefresh,\s*\}/);
});

test('un fichier en cache démarre une ingestion sans téléverser ses octets', () => {
  assert.match(startFile, /sha256 = null/);
  assert.match(startFile, /forceRefresh = false/);
  assert.match(fileEntry, /sha256: sha256 \|\| null/);
  assert.match(fileEntry, /forceRefresh: Boolean\(forceRefresh\)/);
  assert.match(sendFile, /sha256: entry\.sha256 \|\| null/);
  assert.match(sendFile, /forceRefresh: Boolean\(entry\.forceRefresh\)/);
  assert.match(sendFile, /started\.ingestionId/);
  assert.match(sendFile, /markConverting\(entry, started\.ingestionId\)/);
});

test('le préflight fichier passe par la route dédiée du cache upload', () => {
  assert.match(api, /\/overlay\/media-board\/uploads\/lookup/);
  assert.match(api, /timeoutMs: 5000/);
  assert.match(api, /body\.cached === true/);
  assert.match(api, /body\.onBoard === true/);
});
