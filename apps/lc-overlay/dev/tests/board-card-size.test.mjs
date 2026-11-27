// Taille des vignettes du board : un curseur local (localStorage) règle la
// largeur de la piste de la grille. Ce fichier protège les bornes et le pas du
// contrôle, la valeur par défaut (232 px, la largeur d'avant le curseur), le
// repli sur un stockage vide, corrompu ou indisponible, et l'invariant de la
// grille : la largeur vient de la préférence, jamais de la fenêtre, donc
// redimensionner ne change que le nombre de colonnes par ligne.
//
// Quand la barre devient trop étroite, la rangée se replie en icône et ouvre un
// panneau vertical au survol ou au focus : le mode compact ne recopie jamais le
// curseur, il réutilise le même contrôle.
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

test('la barre replie le curseur en panneau vertical quand la place manque', () => {
  const boardCss = read('styles/board.css');
  const toolbar = ruleOf(boardCss, '.board-toolbar');
  assert.ok(toolbar, '.board-toolbar doit exister');
  assert.match(toolbar, /container-type:\s*inline-size/, 'la barre se mesure elle-même, pas la fenêtre');
  assert.match(toolbar, /container-name:\s*board-toolbar/, 'un nom évite les requêtes d’un autre conteneur');

  const at = boardCss.indexOf('@container board-toolbar');
  assert.notEqual(at, -1, 'il faut une requête de conteneur sur la largeur des outils');
  const compact = balancedBlock(boardCss.slice(at), '@container board-toolbar', '{', '}');
  // 840 px : la largeur des outils quand la fenêtre du board est à son minimum
  // (960 px). En dessous, la rangée ne tient déjà plus ses actions : le curseur
  // rend sa place plutôt que de passer seul sur une deuxième ligne.
  assert.match(compact, /^@container board-toolbar \(max-width: 840px\)/);

  const label = ruleOf(compact, '.board-size');
  assert.ok(label, 'le mode compact redéfinit le contrôle');
  assert.match(label, /position:\s*relative/, 'le panneau se pose sous l’icône');
  assert.match(label, /width:\s*32px/, 'l’icône seule remplace la rangée');

  const panel = ruleOf(compact, '.board-size-panel');
  assert.ok(panel, 'le panneau compact a sa propre règle');
  assert.match(panel, /position:\s*absolute/, 'le panneau flotte au-dessus du board');
  assert.match(panel, /opacity:\s*0/, 'il est fermé au repos');
  assert.match(panel, /pointer-events:\s*none/, 'fermé, il ne capte pas la souris');
  assert.doesNotMatch(panel, /visibility:\s*hidden/, 'le curseur reste dans l’ordre de tabulation');

  const open = balancedBlock(compact, '.board-size:hover .board-size-panel', '{', '}');
  assert.match(open, /opacity:\s*1/, 'le survol ouvre le panneau');
  assert.match(open, /pointer-events:\s*auto/, 'ouvert, il redevient manipulable');
  assert.match(compact, /\.board-size:focus-within\s+\.board-size-panel/, 'le clavier ouvre aussi le panneau');
  assert.match(compact, /\.board-size-panel::before/, 'un pont de survol comble l’écart vers l’icône');

  const range = ruleOf(compact, '.board-size-range');
  assert.ok(range, 'le curseur passe en vertical');
  assert.match(range, /writing-mode:\s*vertical-lr/);
  assert.match(range, /direction:\s*rtl/, 'le maximum reste en haut, comme un curseur vertical natif');

  const slider = balancedBlock(read('views/board.js'), 'function cardSizeSlider', '{', '}');
  const returned = slider.slice(slider.indexOf('return h('));
  assert.match(
    returned,
    /h\('span', \{ class: 'board-size-panel' \}, input, output\)/,
    'le panneau enveloppe le curseur puis sa valeur, sans les dupliquer',
  );
});

test('la préférence est peinte avant le premier rendu de l’app', () => {
  const main = read('main.js');
  assert.match(main, /from '\.\/core\/board-card-size\.js'/, 'main.js importe le module');
  assert.match(main, /\ninitBoardCardSize\(\);/, 'main.js applique la préférence au démarrage');
});
