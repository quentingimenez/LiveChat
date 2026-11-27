// Agencement de la barre du board : une seule rangée pour la recherche, les
// types, les filtres et les actions ; l'occupation du board devient l'en-tête
// du panneau de filtres au lieu d'une seconde ligne toujours visible.
//
// La rangée ne se replie jamais : le champ de recherche prend la place
// disponible (son plancher est la largeur exacte de son placeholder, sans
// palier de fenêtre), les types ne s'empilent ni ne se tassent, puis les
// libellés des actions s'effacent en largeur au profit de leurs icônes —
// l'action elle-même ne quitte jamais la ligne. Sous le minimum de la fenêtre
// board, les types passent en groupe horizontal sur une 2e rangée avec les
// filtres et l'icône du curseur (qui s'ouvre alors à la verticale), sans
// jamais se mettre à la verticale.
//
// Ces invariants protègent le retour utilisateur : plus d'actions rejetées sur
// une deuxième ligne, plus de types empilés en 2 × 2, plus de saut de la
// recherche quand un seuil bascule, plus de légende orpheline quand le panneau
// est fermé.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

// Contenu de l'appel ou du bloc qui commence à `marker`, guillemets et gabarits
// ignorés : les sélecteurs testés peuvent contenir des parenthèses ou des
// accolades dans leurs chaînes.
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

test('the toolbar keeps its actions on the tools row', () => {
  const board = read('views/board.js');
  const toolbar = balancedBlock(board, 'const toolbar = h', '(', ')');
  const toolsAt = toolbar.indexOf("class: 'board-tools'");

  assert.notEqual(toolsAt, -1, 'the toolbar must keep its tools group');
  const toolsCall = toolbar.lastIndexOf('h(', toolsAt);
  const tools = balancedBlock(toolbar.slice(toolsCall), 'h', '(', ')');
  assert.match(tools, /class: 'board-actions'/, 'the actions must live inside the tools row');
  assert.match(tools, /class: 'board-tools-secondary'/, 'les types, les filtres et le curseur voyagent dans leur propre groupe');
  assert.ok(
    tools.indexOf("class: 'board-tools-secondary'") < tools.indexOf("class: 'board-actions'"),
    'le groupe secondaire précède les actions : il passera sous elles quand la rangée manquera de place',
  );
  assert.doesNotMatch(toolbar, /board-usage/, 'the usage line must leave the toolbar');
  assert.match(board, /state\.filtersOpen \? renderFilters\(boardUsage\) : null/, 'the panel must receive the usage text');
});

test('the usage legend only exists at the head of the filter panel', () => {
  const filters = balancedBlock(read('views/board.js'), 'function renderFilters', '{', '}');

  assert.match(filters, /^function renderFilters\(boardUsage\)/, 'renderFilters must receive the usage text');
  assert.match(filters, /boardUsage\s*\?\s*h\(/, 'the usage header must be conditional');
  assert.match(filters, /has-usage/, 'the panel must expose when it carries a usage header');
  assert.match(filters, /shelf-usage/, 'the usage line must be the shelf header');
  assert.doesNotMatch(filters, /filters-title/, 'the panel no longer needs a redundant title');
  assert.ok(filters.indexOf('shelf-usage') < filters.indexOf('shelf-controls'), 'the usage header comes before the controls');
});

test('le champ de recherche rétrécit jusqu’à la largeur de son texte', () => {
  const css = read('styles/board.css');
  const search = ruleOf(css, '.board-search');

  assert.ok(search, 'le champ garde sa règle dans la barre');
  const min = /min-width:\s*(\d+)px/.exec(search);
  const basis = /flex:\s*1 1 (\d+)px/.exec(search);
  assert.ok(min, 'le champ fixe une largeur minimale explicite');
  assert.ok(basis, 'sa base flex est cette même largeur : il cède avant les autres contrôles');
  assert.equal(min[1], basis[1], 'base et plancher sont la même valeur');

  // 168 px : « Rechercher un média » à 14 px mesure 130 px, plus 2 × 10 px de
  // padding et 2 × 1 px de bordure, plus les ~14 px que Chrome réserve au
  // bouton d’effacement natif de `type="search"` (à 153 px le placeholder est
  // tronqué). C’est la largeur exacte du texte, pas l’ancien plancher
  // intrinsèque de l’input (≈ 228 px). Un test Node ne mesure pas la police :
  // la vérification dans le navigateur prouve que le placeholder tient.
  assert.equal(min[1], '168', 'largeur du placeholder, paddings et bordures comprises');
  assert.match(search, /max-width:\s*320px/, 'il retrouve sa largeur de lecture quand la place revient');
});

test('la rangée ne se replie plus : les actions y gardent leur place', () => {
  const css = read('styles/board.css');
  const tools = ruleOf(css, '.board-tools');
  const chips = ruleOf(css, '.chips');

  assert.ok(tools, 'la rangée des outils garde sa règle');
  assert.match(tools, /flex-wrap:\s*nowrap/, 'la rangée ne se replie jamais : les actions restent dessus');
  assert.match(ruleOf(css, '.board-tools-secondary'), /display:\s*contents/, 'le groupe secondaire reste transparent sur une rangée');
  assert.ok(chips, 'les types gardent leur conteneur');
  assert.match(chips, /flex-wrap:\s*nowrap/, 'les types ne s’empilent jamais, même quand la place manque');
  assert.match(
    css,
    /\.board-tools \.filters-toggle,\s*\.board-size\s*\{[^}]*flex:\s*0 0 auto/,
    'Filtres et curseur ne se compriment pas : la recherche cède avant eux',
  );
});

test('la recherche n’a plus de palier de fenêtre : elle ne saute pas à 1025 px', () => {
  const css = read('styles/board.css');

  assert.doesNotMatch(
    css,
    /@media \(max-width: 1024px\)\s*\{\s*\.board-search/,
    'la largeur de la recherche se décide sur la barre (container query), pas sur la fenêtre',
  );
  assert.match(ruleOf(css, '.board-search'), /max-width:\s*320px/, 'sa largeur de lecture reste 320 px');
  const compact = balancedBlock(css, '@container board-toolbar (max-width: 1024px)', '{', '}');
  assert.match(
    ruleOf(compact, '.board-search'),
    /max-width:\s*260px/,
    'le mode icônes garde la même largeur qu’au bord libellé : aucun saut au passage de 1153 à 1152 px',
  );
});


test('sous le minimum de la fenêtre, les types passent en groupe sur une 2e rangée', () => {
  const css = read('styles/board.css');
  const narrow = balancedBlock(css, '@container board-toolbar (max-width: 740px)', '{', '}');

  assert.match(ruleOf(narrow, '.board-tools'), /flex-wrap:\s*wrap/, 'la barre serrée passe à deux rangées');
  const group = ruleOf(narrow, '.board-tools-secondary');
  assert.ok(group, 'les types, les filtres et le curseur forment la 2e rangée');
  assert.match(group, /display:\s*flex/, 'le groupe secondaire devient une vraie rangée flex');
  assert.match(group, /flex:\s*1 1 100%/, 'il occupe toute la largeur de sa rangée');
  assert.match(group, /order:\s*3/, 'il passe après la recherche et les actions');
  const search = ruleOf(narrow, '.board-tools > .board-search');
  assert.match(search, /order:\s*1/, 'la recherche garde la première place');
  assert.match(search, /max-width:\s*none/, 'la recherche remplit sa rangée : plus de vide à droite');
  assert.match(ruleOf(narrow, '.board-actions'), /order:\s*2/, 'les actions restent sur la première rangée');
  assert.match(
    ruleOf(narrow, '.board-tools-secondary > .filters-toggle'),
    /margin-left:\s*auto/,
    'les filtres et le curseur collent au bord droit de la 2e rangée',
  );
  const panel = ruleOf(narrow, '.board-size-panel');
  assert.match(panel, /flex-direction:\s*column/, 'le panneau du curseur passe en vertical');
  assert.doesNotMatch(panel, /left:\s*50%/, 'il reste accroché au bord droit de son icône, plus centré sous elle');
  assert.doesNotMatch(narrow, /grid-template-areas/, 'plus de grille : la 2e rangée est une simple ligne flex');
  assert.match(ruleOf(css, '.chips'), /flex-wrap:\s*nowrap/, 'aucune règle ne replie les types en 2 × 2');
});

test('quand la place manque, les actions passent en icônes sans perdre leur nom', () => {
  const css = read('styles/board.css');
  // 1025 px est la largeur minimale d'une rangée libellée depuis que le
  // curseur de taille n'occupe plus que son icône : 356 px d'actions,
  // recherche 260, types 260, filtres 85, icône 32, gouttières 32. À la taille
  // par défaut du board (1400 px), la barre en fait 1272 : les libellés restent
  // donc toujours là, et le repli n'arrive qu'en dessous de 1025.
  const at = css.indexOf('@container board-toolbar (max-width: 1024px)');
  assert.notEqual(at, -1, 'le repli des libellés se mesure sur la barre, pas sur la fenêtre');

  const compact = balancedBlock(css, '@container board-toolbar (max-width: 1024px)', '{', '}');
  const label = ruleOf(compact, '.board-action-label');
  assert.ok(label, 'le mode icônes doit effacer les libellés');
  assert.match(label, /max-width:\s*0/, 'la largeur du libellé tombe à zéro : il quitte le flux au lieu d’être découpé');
  assert.match(label, /min-width:\s*0/, 'un libellé flex ne garde pas sa largeur minimale');
  assert.match(label, /opacity:\s*0/);
  assert.doesNotMatch(label, /display:\s*none/, 'le libellé reste dans le DOM, il porte le nom accessible');
  assert.doesNotMatch(label, /visibility:\s*hidden/, 'le libellé reste dans le DOM, il porte le nom accessible');
  const base = ruleOf(css, '.board-action-label');
  assert.match(base, /max-width:\s*120px/, 'le libellé part de sa largeur naturelle');
  assert.match(base, /transition:[^;]*max-width/, 'sa largeur s’anime : le passage libellé → icône ne casse pas la rangée');
  assert.match(ruleOf(css, '.board-actions .btn'), /transition:[^;]*padding/, 'les boutons accompagnent le mouvement');
  assert.match(ruleOf(compact, '.board-actions .btn'), /padding:\s*0 9px/);
  assert.match(ruleOf(compact, '.board-actions .btn'), /gap:\s*0/, 'le libellé à largeur nulle ne laisse pas de gouttière');

  const board = read('views/board.js');
  const full = balancedBlock(board, 'const toolbar = h', '(', ')');
  assert.match(full, /icon\('check'\)/, 'Sélectionner gagne sa coche');
  assert.match(full, /state\.selecting \? 'Terminer la sélection' : 'Sélectionner des médias'/);
  const labels = full.match(/class: 'board-action-label'/g) ?? [];
  assert.equal(labels.length, 4, 'Sélectionner, Lire, Jouer et Ajouter gardent leur libellé');
  for (const title of [
    'Lire un texte à voix haute sur les overlays',
    'Jouer sur les overlays',
    'Ajouter des médias',
  ]) {
    assert.ok(full.includes(`title: '${title}'`), `infobulle manquante : ${title}`);
  }
});


test('a role without options keeps the panel usable', () => {
  const filters = balancedBlock(read('views/board.js'), 'function renderFilters', '{', '}');

  assert.match(filters, /optionSwitches\.length > 0/, 'an empty options column must not be rendered');
  assert.match(filters, /optionSwitches\.length > 0\s*\?\s*h\('span', \{ class: 'filter-sep'/, 'the separator follows the options block');
});

test('the stylesheet lays the tools on one row and the filters on a shelf', () => {
  const css = read('styles/board.css');
  const toolbar = ruleOf(css, '.board-toolbar');
  const actions = ruleOf(css, '.board-tools > .board-actions');
  const shelf = ruleOf(css, '.filters.shelf');
  const usage = ruleOf(css, '.filters.shelf .shelf-usage');
  const controls = ruleOf(css, '.filters.shelf .shelf-controls');
  const field = ruleOf(css, '.filters.shelf .filter');
  const reset = ruleOf(css, '.filters.shelf .filters-reset');

  assert.ok(toolbar, 'the toolbar rule must exist');
  assert.doesNotMatch(toolbar, /flex-direction:\s*column/, 'the toolbar is no longer two stacked rows');
  assert.ok(actions, 'the actions need their own flex item rule');
  assert.match(actions, /flex:\s*0 0 auto/);
  assert.match(actions, /flex-wrap:\s*nowrap/);
  assert.match(actions, /margin-left:\s*auto/);
  assert.ok(usage, 'the shelf needs a usage header rule');
  assert.ok(controls, 'the shelf controls need their own row');
  assert.match(controls, /display:\s*flex/);
  assert.match(controls, /flex-wrap:\s*wrap/);
  assert.ok(shelf, 'the shelf rule must exist');
  assert.match(shelf, /max-width:\s*none/);
  assert.ok(field, 'the shelf filters must share the line');
  assert.match(field, /flex:\s*1 1 140px/);
  assert.match(field, /max-width:\s*200px/);
  assert.ok(reset, 'the reset button must be pushed to the end of the shelf');
  assert.match(reset, /margin-left:\s*auto/);
});
