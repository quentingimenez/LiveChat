// Bouton flottant « revenir en haut » de la board : la vue défile seule et la
// barre d'outils (recherche, filtres, ajout) disparaît avec les premières
// rangées, donc un bouton flottant ramène la grille en haut. Ce fichier protège
// la règle d'apparition (jamais tout en haut, dès que la barre d'outils est
// passée), le retour sans animation quand l'utilisateur refuse le mouvement, et
// le câblage : le bouton n'existe que dans la board, suit le défilement de
// `#view`, et passe sous la fenêtre de détail, la barre de sélection ou les
// toasts.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { BACK_TO_TOP_AFTER_PX, backToTopBehavior, showsBackToTop } from '../../web/app/core/back-to-top.js';
import { prefersReducedMotion } from '../../web/app/core/dom.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const board = read('views/board.js');
const css = read('styles/board.css');
const detailCss = read('styles/media-detail.css');
const toastCss = read('styles/toast.css');

// Content of the block starting at `marker`, strings, templates and comments
// ignored: selectors and calls under test may contain braces or parentheses, and
// a comment may hold an apostrophe (which would open a fake string).
function balancedBlock(source, marker, openChar, closeChar) {
  const start = source.indexOf(marker);
  assert.notEqual(start, -1, `missing block: ${marker}`);
  const open = source.indexOf(openChar, start);
  let depth = 0;
  let quote = '';
  let lineComment = false;
  let blockComment = false;
  for (let index = open; index < source.length; index += 1) {
    const char = source[index];
    const next = source[index + 1];
    if (lineComment) {
      if (char === '\n') {
        lineComment = false;
      }
      continue;
    }
    if (blockComment) {
      if (char === '*' && next === '/') {
        blockComment = false;
        index += 1;
      }
      continue;
    }
    if (quote) {
      if (char === '\\') {
        index += 1;
      } else if (char === quote) {
        quote = '';
      }
      continue;
    }
    if (char === '/' && next === '/') {
      lineComment = true;
      index += 1;
    } else if (char === '/' && next === '*') {
      blockComment = true;
      index += 1;
    } else if (char === "'" || char === '"' || char === '`') {
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

function ruleOf(selector, source = css) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(source)) !== null) {
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

// --- règle d'apparition -----------------------------------------------------

test('le bouton reste caché en haut de la board', () => {
  assert.equal(BACK_TO_TOP_AFTER_PX > 0, true, 'un seuil nul le montrerait dès le premier pixel');
  assert.equal(showsBackToTop(0), false);
  assert.equal(showsBackToTop(BACK_TO_TOP_AFTER_PX), false, 'au seuil, la barre d’outils est encore là');
});

test('le bouton apparaît une fois la barre d’outils passée', () => {
  assert.equal(showsBackToTop(BACK_TO_TOP_AFTER_PX + 1), true);
  assert.equal(showsBackToTop(2400), true);
});

test('une position inconnue laisse le bouton caché', () => {
  for (const value of [undefined, null, Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, '400']) {
    assert.equal(showsBackToTop(value), false, `${String(value)} ne doit pas révéler le bouton`);
  }
});

test('le retour est animé sauf si l’utilisateur refuse le mouvement', () => {
  assert.equal(backToTopBehavior(false), 'smooth');
  assert.equal(backToTopBehavior(true), 'auto');
});

test('le réglage lu est bien celui du mouvement réduit', () => {
  // `prefersReducedMotion` (core/dom.js) lit `window.matchMedia` : en Node, on
  // fournit un window minimal le temps du test, comme le ferait la vue.
  const previous = globalThis.window;
  const asked = [];
  try {
    globalThis.window = {
      matchMedia: (query) => {
        asked.push(query);
        return { matches: query === '(prefers-reduced-motion: reduce)' };
      },
    };
    assert.equal(prefersReducedMotion(), true);
    assert.deepEqual(asked, ['(prefers-reduced-motion: reduce)'], 'la requête média doit être celle du CSS');
    // Une vue sans matchMedia (shell ancien) garde l’animation : seul un refus
    // explicite doit la couper.
    globalThis.window = {};
    assert.equal(prefersReducedMotion(), false);
  } finally {
    globalThis.window = previous;
  }
});

// --- câblage ----------------------------------------------------------------

test('la board seule porte le bouton', () => {
  const render = balancedBlock(board, 'export function renderBoard()', '{', '}');
  assert.match(render, /backToTopButton\(\)/);
  for (const view of ['views/queue.js', 'views/settings.js', 'views/admin.js', 'views/view.js']) {
    assert.doesNotMatch(
      read(view),
      /class: 'back-to-top'/,
      `${view} ne doit pas ajouter le bouton`,
    );
  }
});

test('le bouton reste après la barre de sélection', () => {
  // `.board:has(+ .selection-bar)` (board.css) s'appuie sur deux frères
  // adjacents : insérer le bouton entre eux casserait le retrait bas de la barre.
  const render = balancedBlock(board, 'export function renderBoard()', '{', '}');
  const selectionBar = render.indexOf('renderSelectionBar()');
  const button = render.indexOf('backToTopButton()');
  assert.notEqual(selectionBar, -1);
  assert.notEqual(button, -1);
  assert.ok(button > selectionBar, 'le bouton doit venir après la barre de sélection');
});

test('le bouton est accessible et démarre caché', () => {
  const builder = balancedBlock(board, 'function backToTopButton()', '{', '}');
  assert.match(builder, /class: 'back-to-top'/);
  assert.match(builder, /type: 'button'/);
  assert.match(builder, /'aria-label': 'Revenir en haut'/);
  assert.match(builder, /icon\('up'\)/);
  assert.match(builder, /hidden: !backToTop \|\| !view \|\| !showsBackToTop\(view\.scrollTop\)/);
});

test('le défilement de la vue met le bouton à jour', () => {
  // C'est `#view` qui défile (base.css) : un écouteur sur `window` ne verrait
  // jamais le scroll de la board.
  const viewNode = balancedBlock(board, 'const viewNode = document.getElementById', '{', '}');
  assert.match(viewNode, /viewNode\.addEventListener\('scroll', syncBackToTop, \{ passive: true \}\)/);
  const sync = balancedBlock(board, 'function syncBackToTop()', '{', '}');
  assert.match(sync, /button\.hidden = !backToTop \|\| !showsBackToTop\(view\.scrollTop\)/);
});

test('le réglage « Bouton Revenir en haut » pilote le bouton', () => {
  const settings = read('views/settings.js');
  assert.match(settings, /id: 'settings-back-to-top'/);
  assert.match(settings, /checked: settings\.backToTop !== false/);
  assert.match(settings, /applyPatch\(\{ backToTop: event\.target\.checked \}\)/);
  assert.match(settings, /Bouton « Revenir en haut »/);

  const apply = balancedBlock(board, 'function applyBackToTop(view)', '{', '}');
  assert.match(apply, /backToTop = view\.backToTop !== false/);
  assert.match(apply, /syncBackToTop\(\)/);

  const device = balancedBlock(board, 'function applyDeviceSettings(view)', '{', '}');
  assert.match(device, /applyHoverPreview\(view\)/);
  assert.match(device, /applyBackToTop\(view\)/);
  assert.match(board, /invoke\('settings_get'\)\.then\(applyDeviceSettings\)/);
  assert.match(board, /listen\('overlay:\/\/device-settings', applyDeviceSettings\)/);
});

test('le clic ramène la vue en haut', () => {
  const builder = balancedBlock(board, 'function backToTopButton()', '{', '}');
  assert.match(builder, /scrollTo\(\{ top: 0, behavior: backToTopBehavior\(prefersReducedMotion\(\)\) \}\)/);
});

// --- placement --------------------------------------------------------------

test('le bouton flotte au-dessus de la grille', () => {
  const fab = ruleOf('.back-to-top');
  assert.match(fab, /position:\s*fixed/);
  assert.match(fab, /right:\s*24px/);
  assert.match(fab, /bottom:\s*20px/);
  const width = /width:\s*(\d+)px/.exec(fab);
  const height = /height:\s*(\d+)px/.exec(fab);
  assert.ok(width && height, 'le bouton doit fixer sa taille');
  assert.equal(width[1], height[1], 'un bouton carré');
  assert.ok(Number(width[1]) >= 32, 'cible d’au moins 32 px, comme les boutons icône');
});

test('le bouton passe sous les calques existants', () => {
  const zIndex = (selector, source = css) => {
    const rule = ruleOf(selector, source);
    assert.ok(rule, `règle absente : ${selector}`);
    return Number(/z-index:\s*(\d+)/.exec(rule)[1]);
  };
  const fab = zIndex('.back-to-top');
  assert.ok(fab < zIndex('.hp-page-shell', detailCss), 'la fenêtre de détail reste devant le bouton');
  assert.ok(fab < zIndex('.selection-bar'), 'la barre de sélection reste devant le bouton');
  assert.ok(fab < zIndex('.toasts', toastCss), 'les toasts restent devant le bouton');
});

test('le bouton remonte au-dessus de la barre « en direct »', () => {
  const rule = ruleOf('body:has(.onair:not([hidden])) .back-to-top');
  assert.match(rule, /bottom:\s*72px/);
});

test('la fenêtre de détail recouvre le bouton', () => {
  // Le détail n'est plus un tiroir posé sur la colonne de droite : c'est une
  // page fixe sur toute la vue, avec son voile. Le bouton n'a donc plus à se
  // décaler à sa gauche, il passe dessous — et la règle de décalage du tiroir
  // ne doit pas traîner dans la feuille de style.
  const shell = ruleOf('.hp-page-shell', detailCss);
  assert.match(shell, /position:\s*fixed/, 'la page de détail doit sortir du flux');
  assert.match(ruleOf('.hp-backdrop', detailCss), /inset:\s*0/, 'le voile doit couvrir la vue');
  assert.doesNotMatch(css, /body:has\(#view \.detail\)/, 'règle de décalage du tiroir devenue morte');
});

test('le bouton revérifie son état une fois monté', () => {
  // La vue restaure son défilement après le montage : la valeur lue dans le
  // constructeur peut encore être celle de l'onglet précédent. Un contrôle en
  // fin de tâche évite de faire clignoter le bouton sur une board revenue en
  // haut.
  const builder = balancedBlock(board, 'function backToTopButton()', '{', '}');
  assert.match(builder, /queueMicrotask\(syncBackToTop\)/);
});

test('le bouton disparaît en mode sélection', () => {
  assert.match(ruleOf('body:has(.selection-bar) .back-to-top'), /display:\s*none/);
});
