// La fenêtre de détails, son aperçu au survol, les dialogues, l'éditeur, le
// menu contextuel et les notifications vivent tous dans le contexte
// d'empilement de la racine (#dialog-root n'en ouvre aucun) : seul leur
// z-index décide ce qui se voit. Sans ordre de couches, le dialogue de
// suppression s'ouvre derrière la fenêtre qui le demande, le menu d'une carte
// derrière l'aperçu qui la recouvre, et le message de confirmation derrière le
// dialogue qui vient de se fermer.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const boardCss = read('styles/board.css');
const contextCss = read('styles/contextmenu.css');
const detailCss = read('styles/media-detail.css');
const dialogCss = read('styles/dialog.css');
const editorCss = read('styles/editor.css');
const toastCss = read('styles/toast.css');

// Déclarations de la règle dont la liste de sélecteurs contient `selector`.
function declarationsOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    const selectors = match[1]
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split(',')
      .map((part) => part.trim());
    if (selectors.includes(selector)) {
      return match[2];
    }
  }
  return '';
}

function zIndexOf(css, selector) {
  const declarations = declarationsOf(css, selector);
  const value = /(?:^|;)\s*z-index:\s*(-?\d+)/.exec(declarations);
  assert.ok(value, `${selector} n’a pas de z-index`);
  return Number(value[1]);
}

// Chaque surface, avec la couche dont elle dépend quand elle s'ouvre.
const couches = {
  apercu: zIndexOf(detailCss, '.hp-overlay'),
  fenetre: zIndexOf(detailCss, '.hp-page-shell'),
  vol: zIndexOf(detailCss, '.hp-flight'),
  dialogue: zIndexOf(dialogCss, '.backdrop'),
  voileModal: zIndexOf(boardCss, '.drop-veil.is-modal'),
  editeur: zIndexOf(editorCss, '.editor'),
  menu: zIndexOf(contextCss, '.ctx-menu'),
  notification: zIndexOf(toastCss, '.toasts'),
};

test('un dialogue ouvert depuis la fenêtre se peint au-dessus d’elle', () => {
  // Les actions « Supprimer » (et « Ajouter à une guilde ») ouvrent un dialogue
  // par-dessus la fenêtre ouverte : derrière elle, il est invisible et son
  // bouton semble ne rien faire.
  assert.ok(
    couches.dialogue > couches.fenetre,
    `le dialogue (${couches.dialogue}) passe devant la fenêtre (${couches.fenetre})`,
  );
});

test('l’éditeur reste au-dessus du dialogue ouvert dessous', () => {
  // Le dialogue « Ajouter un média » peut rester ouvert quand l'éditeur
  // s'ouvre : l'éditeur est plein écran et doit le recouvrir.
  assert.ok(
    couches.editeur > couches.dialogue,
    `l’éditeur (${couches.editeur}) passe devant le dialogue (${couches.dialogue})`,
  );
});

test('le voile de dépôt reste au-dessus du dialogue qu’il recouvre', () => {
  // Pendant un dépôt sur le dialogue, le voile montre la destination et capte
  // le dépôt : sous le dialogue, la surbrillance et la cible disparaissent.
  assert.ok(
    couches.voileModal > couches.dialogue,
    `le voile (${couches.voileModal}) passe devant le dialogue (${couches.dialogue})`,
  );
  assert.ok(
    couches.voileModal < couches.editeur,
    `le voile (${couches.voileModal}) reste sous l’éditeur (${couches.editeur})`,
  );
});

test('le menu d’une carte se peint au-dessus de l’aperçu et de la fenêtre', () => {
  // Le clic droit sur une carte (ou sur l'aperçu qui la recouvre) ouvre le menu
  // à l'endroit du pointeur : sous l'aperçu, il n'apparaît jamais.
  assert.ok(
    couches.menu > couches.apercu,
    `le menu (${couches.menu}) passe devant l’aperçu (${couches.apercu})`,
  );
  assert.ok(
    couches.menu > couches.fenetre,
    `le menu (${couches.menu}) passe devant la fenêtre (${couches.fenetre})`,
  );
});

test('une notification reste lisible par-dessus les surfaces', () => {
  // « Média supprimé. » s'affiche pendant que la fenêtre se referme, et après
  // une action confirmée dans un dialogue.
  for (const [nom, couche] of Object.entries({ dialogue: couches.dialogue, fenetre: couches.fenetre })) {
    assert.ok(
      couches.notification > couche,
      `la notification (${couches.notification}) passe devant ${nom} (${couche})`,
    );
  }
});

test('l’aperçu au survol reste sous la fenêtre et sous ses surfaces', () => {
  // L'aperçu est une prévisualisation de la carte : ce que la fenêtre ouvre
  // par-dessus lui doit le couvrir, pas l'inverse.
  for (const [nom, couche] of Object.entries({
    fenetre: couches.fenetre,
    dialogue: couches.dialogue,
    editeur: couches.editeur,
  })) {
    assert.ok(couches.apercu < couche, `l’aperçu (${couches.apercu}) reste sous ${nom} (${couche})`);
  }
});
