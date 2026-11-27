// Taille des vignettes du board : un curseur local (localStorage) règle la
// largeur de la piste de la grille. Ce fichier protège les bornes et le pas du
// contrôle, la valeur par défaut (232 px, la largeur d'avant le curseur), le
// repli sur un stockage vide, corrompu ou indisponible, et l'invariant de la
// grille : la largeur vient de la préférence, jamais de la fenêtre, donc
// redimensionner ne change que le nombre de colonnes par ligne.
//
// Le curseur est toujours replié derrière son icône : la rangée ne lui réserve
// jamais 176 px, et le panneau s'ouvre au survol ou au focus, horizontalement
// quand la place le permet puis verticalement sous 740 px de barre. Le mode
// compact ne recopie jamais le curseur, il réutilise le même contrôle.
//
// Les vues ont besoin d'un DOM et le harnais n'en a pas : le câblage du
// curseur et la règle CSS se lisent dans les sources.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  applyBoardCardWidth,
  BOARD_CARD_WIDTH_STEP,
  BOARD_CARD_WIDTH_STORAGE_KEY,
  currentBoardCardWidth,
  DEFAULT_BOARD_CARD_WIDTH,
  initBoardCardSize,
  MAX_BOARD_CARD_WIDTH,
  MIN_BOARD_CARD_WIDTH,
  normalizeBoardCardWidth,
  readBoardCardWidth,
  rememberBoardCardWidth,
} from '../../web/app/core/board-card-size.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

// Storage minimal, comme `localStorage` : un appel qui lève vérifie le repli.
function memoryStorage({ initial = {}, failRead = false, failWrite = false, map = new Map() } = {}) {
  for (const [key, value] of Object.entries(initial)) {
    map.set(key, value);
  }
  return {
    map,
    getItem(key) {
      if (failRead) {
        throw new Error('storage unavailable');
      }
      return map.has(key) ? map.get(key) : null;
    },
    setItem(key, value) {
      if (failWrite) {
        throw new Error('quota exceeded');
      }
      map.set(key, String(value));
    },
  };
}

// Racine DOM minimale : seule `style.setProperty` est utilisée.
function fakeRoot() {
  const properties = new Map();
  return {
    properties,
    style: {
      setProperty(name, value) {
        properties.set(name, value);
      },
    },
  };
}

// Contenu de l'appel qui commence à `marker`, guillemets et gabarits ignorés.
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
    if (char === "'" || char === '"' || char === '`') {
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

// Déclarations de la règle dont le sélecteur est exactement `selector`.
function ruleOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    if (match[1].replace(/\/\*[\s\S]*?\*\//g, '').trim() === selector) {
      return match[2];
    }
  }
  return null;
}

test('la taille par défaut reste la largeur du board d’avant le curseur', () => {
  assert.equal(DEFAULT_BOARD_CARD_WIDTH, 232);
  // Valeur absente, vide, illisible ou non finie : le board garde sa taille
  // normale au lieu de tomber sur une borne.
  for (const raw of [undefined, null, '', '   ', 'abc', '232px', Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
    assert.equal(normalizeBoardCardWidth(raw), DEFAULT_BOARD_CARD_WIDTH, String(raw));
  }
  assert.ok(MIN_BOARD_CARD_WIDTH < DEFAULT_BOARD_CARD_WIDTH);
  assert.ok(DEFAULT_BOARD_CARD_WIDTH < MAX_BOARD_CARD_WIDTH);
  // Les bornes et le défaut tombent pile sur un cran du curseur : sans cela,
  // le cran le plus proche déplacerait la piste au premier mouvement.
  for (const width of [MIN_BOARD_CARD_WIDTH, DEFAULT_BOARD_CARD_WIDTH, MAX_BOARD_CARD_WIDTH]) {
    assert.equal(width % BOARD_CARD_WIDTH_STEP, 0, String(width));
  }
});

test('le curseur borne et aligne la largeur sur son pas', () => {
  assert.equal(normalizeBoardCardWidth(MIN_BOARD_CARD_WIDTH), MIN_BOARD_CARD_WIDTH);
  assert.equal(normalizeBoardCardWidth(MAX_BOARD_CARD_WIDTH), MAX_BOARD_CARD_WIDTH);
  // Hors bornes : ramené au plus proche cran valide, jamais rejeté.
  assert.equal(normalizeBoardCardWidth(10), MIN_BOARD_CARD_WIDTH);
  assert.equal(normalizeBoardCardWidth(9999), MAX_BOARD_CARD_WIDTH);
  // Entre deux crans : le cran le plus proche.
  assert.equal(normalizeBoardCardWidth(233), DEFAULT_BOARD_CARD_WIDTH);
  assert.equal(normalizeBoardCardWidth(236), DEFAULT_BOARD_CARD_WIDTH + BOARD_CARD_WIDTH_STEP);
  assert.equal(normalizeBoardCardWidth('248'), 248);
  // Une valeur décimale (glisser au clavier) ne pose jamais une demi-piste.
  assert.equal(normalizeBoardCardWidth(250.4), 248);
});

test('la préférence survit au redémarrage, un stockage cassé revient au défaut', () => {
  // La clé est un contrat entre versions : la renommer perdrait la taille
  // choisie au premier redémarrage après une mise à jour.
  assert.equal(BOARD_CARD_WIDTH_STORAGE_KEY, 'livechat.board.card-width');
  const storage = memoryStorage();
  assert.equal(readBoardCardWidth(storage), DEFAULT_BOARD_CARD_WIDTH);
  assert.equal(readBoardCardWidth(null), DEFAULT_BOARD_CARD_WIDTH);

  assert.equal(rememberBoardCardWidth(288, storage), 288);
  assert.equal(storage.map.get(BOARD_CARD_WIDTH_STORAGE_KEY), '288');
  assert.equal(readBoardCardWidth(storage), 288);

  // La valeur écrite est normalisée : une entrée hors pas ne se propage pas.
  assert.equal(rememberBoardCardWidth(9999, storage), MAX_BOARD_CARD_WIDTH);
  assert.equal(storage.map.get(BOARD_CARD_WIDTH_STORAGE_KEY), String(MAX_BOARD_CARD_WIDTH));

  // Entrée corrompue à la main, storage indisponible : le board reste lisible.
  assert.equal(readBoardCardWidth(memoryStorage({ initial: { [BOARD_CARD_WIDTH_STORAGE_KEY]: 'la taille' } })), DEFAULT_BOARD_CARD_WIDTH);
  assert.equal(readBoardCardWidth(memoryStorage({ failRead: true })), DEFAULT_BOARD_CARD_WIDTH);
  assert.equal(rememberBoardCardWidth(200, memoryStorage({ failWrite: true })), 200);
});

test('appliquer la taille pose la variable CSS de la grille sur la racine', () => {
  const root = fakeRoot();
  assert.equal(applyBoardCardWidth(256, root), 256);
  assert.equal(root.properties.get('--board-card-width'), '256px');
  assert.equal(currentBoardCardWidth(), 256);

  // La valeur appliquée est normalisée, comme celle du stockage.
  assert.equal(applyBoardCardWidth(3000, root), MAX_BOARD_CARD_WIDTH);
  assert.equal(root.properties.get('--board-card-width'), `${MAX_BOARD_CARD_WIDTH}px`);

  // Au démarrage, la préférence stockée est lue puis peinte avant le premier rendu.
  const stored = fakeRoot();
  const started = initBoardCardSize(memoryStorage({ initial: { [BOARD_CARD_WIDTH_STORAGE_KEY]: '176' } }), stored);
  assert.equal(started, 176);
  assert.equal(stored.properties.get('--board-card-width'), '176px');

  // Ni stockage ni document (harnais Node, WebView dégradée) : défaut, sans lever.
  const bare = fakeRoot();
  assert.equal(initBoardCardSize(null, bare), DEFAULT_BOARD_CARD_WIDTH);
  assert.equal(bare.properties.get('--board-card-width'), `${DEFAULT_BOARD_CARD_WIDTH}px`);
  assert.doesNotThrow(() => initBoardCardSize(null, null));
});

test('la grille suit la variable, avec le défaut du module en repli', () => {
  const boardCss = read('styles/board.css');
  const grid = ruleOf(boardCss, '.grid');
  assert.ok(grid, '.grid doit exister dans board.css');
  assert.match(
    grid,
    new RegExp(`grid-template-columns:\\s*repeat\\(auto-fill,\\s*var\\(--board-card-width,\\s*${DEFAULT_BOARD_CARD_WIDTH}px\\)\\)`),
    'la piste suit --board-card-width et retombe sur la largeur par défaut du module',
  );
  assert.match(grid, /justify-content:\s*start/, 'les pistes restent ancrées à gauche');
  assert.doesNotMatch(grid, /\b1fr\b/, 'aucune piste fractionnaire : une carte ne s’étire pas avec la fenêtre');
  const viewportWidth = /(?:^|[\s;])(?:width|min-width|max-width|flex-basis|inline-size):\s*[^;]*(?:vw|[0-9.]+%)/m;
  assert.doesNotMatch(grid, viewportWidth, 'la largeur ne vient pas de la fenêtre');
  // Le curseur et sa valeur forment un contrôle de la barre d'outils.
  assert.match(boardCss, /\.board-size\s*\{/);
  assert.match(boardCss, /\.board-size-range\s*\{/);
});

test('la barre d’outils porte le curseur et le câble à la préférence', () => {
  const board = read('views/board.js');
  const toolbar = balancedBlock(board, 'const toolbar = h', '(', ')');
  const toolsAt = toolbar.indexOf("class: 'board-tools'");
  const toolsCall = toolbar.lastIndexOf('h(', toolsAt);
  const tools = balancedBlock(toolbar.slice(toolsCall), 'h', '(', ')');

  assert.match(tools, /cardSizeSlider\(\)/, 'le curseur vit dans la rangée des outils');
  assert.ok(tools.indexOf('cardSizeSlider()') < tools.indexOf("class: 'board-actions'"), 'le curseur précède les actions, jamais à leur droite');

  const slider = balancedBlock(board, 'function cardSizeSlider', '{', '}');
  assert.match(slider, /type: 'range'/, 'un curseur, pas un champ numérique');
  assert.match(slider, /min: String\(MIN_BOARD_CARD_WIDTH\)/);
  assert.match(slider, /max: String\(MAX_BOARD_CARD_WIDTH\)/);
  assert.match(slider, /step: String\(BOARD_CARD_WIDTH_STEP\)/);
  assert.match(slider, /'aria-label': 'Taille des vignettes'/);
  assert.match(slider, /applyBoardCardWidth\(Number\(event\.target\.value\)\)/, 'le glissement s’applique en direct');
  assert.match(slider, /rememberBoardCardWidth\(Number\(event\.target\.value\)\)/, 'le relâchement mémorise la taille');
  assert.match(
    slider,
    new RegExp(`resetOnDoubleClick\\(input,\\s*DEFAULT_BOARD_CARD_WIDTH\\)`),
    'un double clic remet la taille par défaut',
  );
});

test('le curseur est toujours replié derrière son icône, ouvert au survol', () => {
  const boardCss = read('styles/board.css');
  const toolbar = ruleOf(boardCss, '.board-toolbar');
  assert.ok(toolbar, '.board-toolbar doit exister');
  assert.match(toolbar, /container-type:\s*inline-size/, 'la barre se mesure elle-même, pas la fenêtre');
  assert.match(toolbar, /container-name:\s*board-toolbar/, 'un nom évite les requêtes d’un autre conteneur');

  // La rangée ne réserve jamais la piste : le contrôle est l'icône, à toutes
  // les largeurs. C'est la seule façon de ne pas reprendre 176 px aux libellés.
  const label = ruleOf(boardCss, '.board-size');
  assert.ok(label, 'le contrôle garde sa règle de base');
  assert.match(label, /position:\s*relative/, 'le panneau se pose à côté de l’icône');
  assert.match(label, /justify-content:\s*center/, 'l’icône est seule dans le contrôle');
  assert.match(label, /width:\s*32px/, 'le contrôle occupe la largeur d’un bouton-icône');

  const panel = ruleOf(boardCss, '.board-size-panel');
  assert.ok(panel, 'le panneau a sa règle de base');
  assert.match(panel, /position:\s*absolute/, 'le panneau flotte au-dessus du board');
  assert.match(panel, /right:\s*calc\(100% \+ 8px\)/, 'il s’ouvre à gauche de l’icône');
  assert.match(panel, /top:\s*50%/, 'il est centré verticalement sur l’icône');
  assert.match(panel, /transform:\s*translate\(4px, -50%\)/, 'fermé, il est décalé de 4 px vers l’icône');
  assert.doesNotMatch(panel, /left:\s*50%/, 'le panneau horizontal n’est pas centré sous l’icône');
  assert.doesNotMatch(panel, /flex-direction:\s*column/, 'la piste reste horizontale par défaut');
  assert.match(panel, /opacity:\s*0/, 'il est fermé au repos');
  assert.match(panel, /pointer-events:\s*none/, 'fermé, il ne capte pas la souris');
  assert.doesNotMatch(panel, /visibility:\s*hidden/, 'le curseur reste dans l’ordre de tabulation');

  const open = balancedBlock(boardCss, '.board-size:hover .board-size-panel', '{', '}');
  assert.match(open, /opacity:\s*1/, 'le survol ouvre le panneau');
  assert.match(open, /pointer-events:\s*auto/, 'ouvert, il redevient manipulable');
  assert.match(open, /transform:\s*translate\(0, -50%\)/, 'ouvert, il revient à hauteur de l’icône');
  assert.match(boardCss, /\.board-size:focus-within\s+\.board-size-panel/, 'le clavier ouvre aussi le panneau');
  const bridge = balancedBlock(boardCss, '.board-size-panel::before', '{', '}');
  assert.match(bridge, /right:\s*-8px/, 'un pont de survol comble l’écart vers l’icône');
  assert.match(bridge, /width:\s*8px/, 'le pont fait la largeur de l’écart');

  const baseRange = ruleOf(boardCss, '.board-size-range');
  assert.match(baseRange, /width:\s*104px/, 'la piste garde sa largeur horizontale');
  assert.doesNotMatch(baseRange, /writing-mode|direction:\s*rtl|height:\s*96px/, 'aucune trace du curseur vertical par défaut');
  assert.doesNotMatch(boardCss, /@container board-toolbar \(max-width: 900px\)/, 'le seuil 900 a disparu avec la rangée inline');

  const slider = balancedBlock(read('views/board.js'), 'function cardSizeSlider', '{', '}');
  const returned = slider.slice(slider.indexOf('return h('));
  assert.match(
    returned,
    /h\('span', \{ class: 'board-size-panel' \}, input, output\)/,
    'le panneau enveloppe le curseur puis sa valeur, sans les dupliquer',
  );
});

test('sous 740 px de barre, le curseur passe en vertical et les types ne s’empilent pas', () => {
  const boardCss = read('styles/board.css');
  const narrow = balancedBlock(boardCss, '@container board-toolbar (max-width: 740px)', '{', '}');

  assert.match(ruleOf(narrow, '.board-tools'), /flex-wrap:\s*wrap/, 'la barre passe à deux rangées');
  const group = ruleOf(narrow, '.board-tools-secondary');
  assert.ok(group, 'les types, les filtres et l’icône forment la seconde rangée');
  assert.match(group, /display:\s*flex/, 'la seconde rangée est une ligne flex');
  assert.match(group, /flex:\s*1 1 100%/, 'elle occupe toute la largeur');
  assert.match(group, /order:\s*3/);
  assert.match(ruleOf(narrow, '.board-tools > .board-search'), /order:\s*1/);
  assert.match(ruleOf(narrow, '.board-tools > .board-search'), /max-width:\s*none/, 'la recherche remplit sa rangée');
  assert.match(ruleOf(narrow, '.board-actions'), /order:\s*2/);
  assert.match(
    ruleOf(narrow, '.board-tools-secondary > .filters-toggle'),
    /margin-left:\s*auto/,
    'les filtres et l’icône collent au bord droit',
  );

  // Les types ne se replient jamais : ils changent de rangée, pas de forme.
  const chips = ruleOf(boardCss, '.chips');
  assert.match(chips, /flex-wrap:\s*nowrap/, 'les types ne s’empilent ni au large ni au serré');

  const panel = ruleOf(narrow, '.board-size-panel');
  assert.match(panel, /flex-direction:\s*column/, 'le panneau passe en colonne');
  assert.doesNotMatch(panel, /left:\s*50%/, 'il reste à gauche de l’icône, centré sur sa hauteur');
  const range = ruleOf(narrow, '.board-size-range');
  assert.match(range, /writing-mode:\s*vertical-lr/);
  assert.match(range, /direction:\s*rtl/);
  assert.match(range, /height:\s*96px/);
});

test('la préférence est peinte avant le premier rendu de l’app', () => {
  const main = read('main.js');
  assert.match(main, /from '\.\/core\/board-card-size\.js'/, 'main.js importe le module');
  assert.match(main, /\ninitBoardCardSize\(\);/, 'main.js applique la préférence au démarrage');
});
