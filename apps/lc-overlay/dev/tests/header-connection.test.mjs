// Le compteur d'overlays connectés reste visible quand la fenêtre se réduit :
// c'est le seul signe, dans l'app, que des overlays écoutent. La rangée se
// replie (les actions passent sous le titre) plutôt que de masquer le
// compteur. Ce fichier protège le libellé, sa place avant les boutons et
// l'absence de toute règle qui le cache à une largeur donnée.
//
// Le harnais Node n'a pas de DOM : le rendu et la feuille se lisent dans les
// sources.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

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

test('le compteur d’overlays connectés est rendu avant les boutons', () => {
  const header = read('views/header.js');
  assert.match(header, /class: 'meta header-connection'/, 'le compteur a sa classe');
  // Le nombre et l'accord (« 1 overlay connecté », « 6 overlays connectés »)
  // viennent de l'état, pas d'un libellé figé.
  assert.match(header, /\$\{connected\} overlay\$\{connected > 1 \? 's' : ''\} connecté\$\{connected > 1 \? 's' : ''\}/);
  assert.match(header, /connected !== null/, 'un compteur inconnu ne s’affiche pas');
  // Il ouvre la rangée des actions : même quand celles-ci s'enroulent sous le
  // titre, le compteur reste sur la première ligne avec lui.
  assert.ok(
    header.indexOf("class: 'meta header-connection'") < header.indexOf("class: 'btn is-quiet'"),
    'le compteur précède les boutons d’action',
  );
});

test('aucune règle ne masque le compteur quand la fenêtre se réduit', () => {
  const css = read('styles/header.css');
  assert.doesNotMatch(css, /\.header-connection\s*\{[^}]*display:\s*none/s, 'le compteur ne disparaît plus');
  assert.doesNotMatch(css, /@media[^{}]*\{[\s\S]*?header-connection/, 'aucune requête de largeur ne le cible');
  // La rangée se replie au lieu de perdre le compteur.
  const top = ruleOf(css, '.header-top');
  assert.match(top, /flex-wrap:\s*wrap/, 'la rangée peut passer à la ligne');
  const actions = ruleOf(css, '.header-actions');
  assert.match(actions, /flex-wrap:\s*wrap/, 'les actions peuvent s’enrouler');
});
