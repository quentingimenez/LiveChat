// Mise en page de la barre « en cours » de la file (node:test).
//
// Ce que ce fichier protège : le nom du média garde sa propre ligne et peut
// passer sur deux lignes au maximum ; l'origine, l'auteur et l'horloge restent
// sur une seule ligne, sans être repoussés par un titre long. Les vues
// demandent un DOM : les assertions lisent la source, comme les autres tests
// de la file.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const queue = read('views/queue.js');
const css = read('styles/queue.css');

function nowBar() {
  const start = queue.indexOf("class: 'now'");
  assert.notEqual(start, -1, 'la barre en cours existe');
  const end = queue.indexOf("class: 'progress'", start);
  assert.notEqual(end, -1, 'la barre en cours porte sa progression');
  return queue.slice(start, end);
}

test('le nom du média a sa propre ligne, limitée à deux lignes', () => {
  const bar = nowBar();
  const title = bar.match(/h\('strong', \{ class: 'now-title'[\s\S]*?\}, title\)/);
  assert.ok(title, 'le titre utilise la classe now-title');
  assert.match(queue, /const title = jobTitle\(playing\);/);
  assert.doesNotMatch(title[0], /class: 'ellipsis'/);
  assert.ok(
    bar.indexOf("class: 'now-title'") < bar.indexOf("class: 'row now-meta'"),
    'le titre précède la rangée origine/auteur/horloge',
  );

  const rule = css.match(/\.now-title \{[\s\S]*?\r?\n\}/);
  assert.ok(rule, 'la règle .now-title existe');
  assert.match(rule[0], /display: -webkit-box;/);
  assert.match(rule[0], /-webkit-box-orient: vertical;/);
  assert.match(rule[0], /-webkit-line-clamp: 2;/);
  assert.match(rule[0], /line-clamp: 2;/);
  assert.match(rule[0], /overflow: hidden;/);
});

test("l'origine, l'auteur et l'horloge tiennent sur une seule ligne", () => {
  const bar = nowBar();
  const meta = bar.match(/class: 'row now-meta'[\s\S]*?'data-queue-clock'/);
  assert.ok(meta, 'la ligne now-meta porte l’horloge');
  assert.match(meta[0], /originBadge\(playing\)/);
  assert.match(meta[0], /par \$\{playing\.authorName\}/);
  assert.doesNotMatch(meta[0], /now-title/, 'le titre reste hors de la rangée méta');

  const rule = css.match(/\.now-meta \{[\s\S]*?\r?\n\}/);
  assert.ok(rule, 'la règle .now-meta existe');
  assert.match(rule[0], /flex-wrap: nowrap;/);
  assert.match(rule[0], /min-width: 0;/);
});

test("l'auteur trop long s'abrège au lieu de casser la ligne", () => {
  const rule = css.match(/\.now-author \{[\s\S]*?\r?\n\}/);
  assert.ok(rule, 'la règle .now-author existe');
  assert.match(rule[0], /text-overflow: ellipsis;/);
  assert.match(rule[0], /white-space: nowrap;/);
});
