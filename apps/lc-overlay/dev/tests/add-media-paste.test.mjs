// Pasting a media copied in the Windows file explorer must open the add-media
// dialog with the files already chosen, like dropping a link opens it with the
// link. Text pasted in a field, a paste while another dialog is open and a
// paste outside the board keep their native behavior.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { isPastedMedia, pastedMediaFiles } from '../../web/app/core/media-files.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const board = read('views/board.js');
const addMedia = read('views/add-media.js');

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

function file(name, type, size) {
  return { name, type, size };
}

function clipboard(...files) {
  return { files };
}

test('un média copié depuis l’explorateur est reconnu', () => {
  assert.equal(isPastedMedia(file('photo.jpg', 'image/jpeg', 11748)), true);
  assert.equal(isPastedMedia(file('clip.mp4', 'video/mp4', 2048)), true);
  assert.equal(isPastedMedia(file('son.mp3', 'audio/mpeg', 512)), true);
});

test('un type média inconnu reste un média quand le fichier a un contenu', () => {
  assert.equal(isPastedMedia(file('clip.mkv', '', 4096)), true);
});

test('un texte, un document et un dossier ne sont pas des médias', () => {
  assert.equal(isPastedMedia(file('note.txt', 'text/plain', 10)), false);
  assert.equal(isPastedMedia(file('doc.pdf', 'application/pdf', 100)), false);
  assert.equal(isPastedMedia(file('un-dossier', '', 0)), false);
  assert.equal(isPastedMedia(file('vide.png', 'image/png', 0)), false);
});

test('un presse-papiers sans fichier ne contient aucun média', () => {
  assert.deepEqual(pastedMediaFiles(clipboard()), []);
  assert.deepEqual(pastedMediaFiles(undefined), []);
});

test('un collage mixte ne garde que les médias', () => {
  const first = file('photo.jpg', 'image/jpeg', 11748);
  const second = file('clip.mkv', '', 4096);
  const files = pastedMediaFiles(clipboard(first, file('note.txt', 'text/plain', 10), second, file('un-dossier', '', 0)));

  assert.deepEqual(files, [first, second]);
});

test('la board ouvre le dialogue prérempli sur un collage de médias', () => {
  const listener = balancedBlock(board, "document.addEventListener('paste'", '(', ')');

  assert.match(listener, /event\.defaultPrevented/, 'a handled paste must keep its handler');
  assert.match(listener, /isEditable\(event\.target\)/, 'a field keeps its native paste');
  assert.match(listener, /getElementById\('dialog-root'\)/, 'an open dialog keeps its own paste');
  assert.match(listener, /pastedMediaFiles\(event\.clipboardData\)/);
  assert.match(listener, /event\.preventDefault\(\)/, 'the media paste must not also paste as text');
  assert.match(listener, /openAddDialog\(\{ files \}\)/, 'the dialog opens with the pasted files');
});

test('le collage ne s’active que sur la board et si l’action est permise', () => {
  const active = balancedBlock(board, 'function boardIsActive', '{', '}');
  assert.match(active, /state\.activeGuildId/);
  assert.match(active, /state\.page === 'guild'/);
  assert.match(active, /state\.tab === 'board'/);

  const allowed = balancedBlock(board, 'function canPasteFiles', '{', '}');
  assert.match(allowed, /can\('addMedia'\)/);
  assert.match(allowed, /can\('play'\)/, 'a play-only member may play a pasted media once');
});

test('le dialogue ouvre la source Fichier et garde le lien intact', () => {
  const dialog = balancedBlock(addMedia, 'export function openAddDialog', '{', '}');

  assert.match(dialog, /Array\.isArray\(options\.files\)/, 'the dialog accepts pasted files');
  assert.match(dialog, /chosenFiles = prefillFiles\.slice\(\)/);
  assert.match(dialog, /setMode\(initialSource\(chosenFiles\)\)/, 'pasted files open on the file source');
  assert.match(dialog, /renderFileList\(\)/, 'the pasted files must be listed before the first paint');
  assert.match(dialog, /fileTitle\.focus\(\)/, 'one file focuses its optional title');
  assert.match(dialog, /submit\.focus\(\)/, 'several files focus the submit button');
  assert.match(dialog, /prefill \? linkTitle : linkUrl/, 'a link keeps its own focus');
});
