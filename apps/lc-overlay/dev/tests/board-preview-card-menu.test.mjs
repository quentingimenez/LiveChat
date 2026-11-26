// Le survol d'une carte ouvre un aperçu qui la recouvre entièrement : la carte
// garde ses évènements mais n'est plus peinte (board.css, .card.is-previewing),
// si bien que son propre gestionnaire de clic droit ne reçoit plus rien. Un
// clic droit sur l'aperçu doit donc ouvrir le menu de la carte dont il a grandi
// — le menu du board, pas un second menu qui divergerait — et refermer l'aperçu
// sur sa carte, transition comprise, plutôt que de le faire disparaître.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { previewAllowed } from '../../web/app/core/detail-actions.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const board = read('views/board.js');
const contextMenu = read('core/contextmenu.js');
const mediaDetail = read('views/media-detail.js');

// The source from `start` to the end of the `end` match after it, so an
// assertion reads one handler and not the whole file.
function between(source, start, end) {
  const from = source.indexOf(start);
  const to = source.indexOf(end, from);
  assert.ok(from >= 0 && to > from, `${start} … ${end} introuvable`);
  return source.slice(from, to + end.length);
}

test('l’aperçu transmet son clic droit au menu de la carte', () => {
  const listener = between(mediaDetail, "el.addEventListener('contextmenu'", 'opts.onCardMenu(event, card);');
  assert.match(listener, /event\.preventDefault\(\)/, 'le menu natif est écarté');
  assert.match(listener, /event\.stopPropagation\(\)/, 'le clic ne remonte pas comme un second menu');
  // L'aperçu ne s'efface pas d'un coup : il joue sa fermeture sur la carte
  // dont il a grandi, et le menu s'ouvre par-dessus (il est plus haut dans
  // l'empilement), donc la transition reste lisible sous lui.
  assert.doesNotMatch(listener, /close\(true\)/, 'l’aperçu ne disparaît pas d’un coup');
  const closing = listener.indexOf('close();');
  const opened = listener.indexOf('opts.onCardMenu(event, card);');
  assert.ok(closing >= 0 && closing < opened, 'l’aperçu commence à se refermer avant que le menu s’ouvre');
});

test('le board ouvre le menu de la carte que l’aperçu recouvre', () => {
  assert.match(
    board,
    /onCardMenu: \(event, card\) => \{[\s\S]{0,400}openCardMenu\(event, entry\.item\)/,
    'l’aperçu ouvre le menu du board pour le média de sa carte',
  );
  assert.match(board, /const entry = cards\.get\(card\.dataset\.itemId\)/, 'la carte transmet son média');
});

test('un menu ouvert n’écarte l’aperçu que sur sa propre carte', () => {
  // Dès que l'aperçu s'efface, la carte redevient l'élément survolé sous le
  // pointeur : sans cette règle, l'aperçu se rouvrirait 60 ms plus tard
  // par-dessus le menu qu'il vient d'ouvrir.
  assert.equal(previewAllowed({ hoverPreview: true, menuOnCard: true }), false);
  // Le pointeur peut quitter cette carte sans refermer le menu : l'aperçu de
  // la carte survolée doit alors s'ouvrir, sinon le survol actif reste celui
  // d'avant et la carte sous le pointeur n'a plus d'aperçu du tout.
  assert.equal(previewAllowed({ hoverPreview: true, menuOnCard: false }), true);
  assert.equal(previewAllowed({ hoverPreview: true }), true, 'sans menu, rien ne change');
  assert.match(
    board,
    /menuOnCard: contextMenuOpen\(\) && itemId === menuItemId/,
    'le board compare la carte survolée à celle du menu',
  );
  assert.match(
    between(board, 'function openCardMenu(event, item) {', 'openContextMenu(event, ['),
    /menuItemId = item\.id;/,
    'ouvrir un menu retient sa carte',
  );
  assert.match(contextMenu, /export function contextMenuOpen\(\)/, 'le menu sait s’il est ouvert');
});

test('l’aperçu demande la carte survolée', () => {
  assert.match(mediaDetail, /opts\.hoverEnabled\(card\?\.dataset\?\.itemId\)/, 'la carte est transmise au board');
  const calls = [...mediaDetail.matchAll(/hoverEnabled\(card\)/g)];
  assert.ok(calls.length >= 3, `chaque décision d’aperçu passe la carte (${calls.length})`);
});
