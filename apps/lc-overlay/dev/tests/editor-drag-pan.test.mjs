// Éditeur de média — un geste de glissement reste sous le pointeur quand la
// scène défile ou se zoome pendant le maintien.
//
// Ce que ce fichier protège :
//  - un bord de rognage tenu suit le pointeur quand la scène se déplace sous
//    lui (molette maintenue, clique molette, zoom Ctrl + molette) ;
//  - la boîte de rognage entière se déplace de la même façon ;
//  - sans déplacement de scène, rejouer le geste ne change rien ;
//  - les deux éditeurs (image et vidéo) branchent bien ce rejeu sur le
//    défilement de la scène et sur le redimensionnement du zoom ;
//  - les annotations vidéo rejouent aussi leur geste avec le dernier pointeur.
//
// Le calcul du rognage est testé par son comportement, avec un DOM minimal ;
// les branchements des gros éditeurs (qui demandent un canvas et un média)
// sont vérifiés dans leur source, comme le fait editor-wheel-zoom.test.mjs.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

class FakeNode {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.style = {};
    this.attributes = {};
    this.listeners = {};
    this.hidden = false;
    this.textContent = '';
    this.title = '';
    this.className = '';
    this.dataset = {};
    this.clientWidth = 0;
    this.clientHeight = 0;
  }

  append(...nodes) {
    this.children.push(...nodes);
  }

  setAttribute(name, value) {
    this.attributes[name] = value;
  }

  addEventListener(type, listener) {
    (this.listeners[type] ||= []).push(listener);
  }

  removeEventListener(type, listener) {
    const list = this.listeners[type] || [];
    const index = list.indexOf(listener);
    if (index >= 0) {
      list.splice(index, 1);
    }
  }

  setPointerCapture() {}

  dispatch(type, event) {
    for (const listener of this.listeners[type] || []) {
      listener({ preventDefault() {}, stopPropagation() {}, target: this, ...event });
    }
  }

  getBoundingClientRect() {
    return { left: 0, top: 0, width: this.clientWidth, height: this.clientHeight };
  }
}

globalThis.Node = FakeNode;
globalThis.document = {
  createElement: (tag) => new FakeNode(tag),
  createTextNode: (text) => Object.assign(new FakeNode('#text'), { textContent: text }),
};

const { createCropOverlay } = await import('../../web/app/views/editor/crop-overlay.js');

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const image = read('views/editor/image.js');
const av = read('views/editor/av.js');
const annotations = read('views/editor/video-annotations.js');
const crop = read('views/editor/crop-overlay.js');

const SIZE = { width: 400, height: 300 };

// Une image de 400 × 300 affichée à 200 × 150, crop au centre. `shown` est le
// rect écran de la couche, que la scène modifie en défilant.
function mount() {
  const host = new FakeNode('div');
  let shown = { left: 50, top: 25, width: 200, height: 150 };
  const commits = [];
  const overlay = createCropOverlay({
    host,
    bounds: () => SIZE,
    aspect: () => 0,
    onChange: () => {},
    onCommit: (rect) => commits.push(rect),
  });
  const layer = host.children[0];
  layer.getBoundingClientRect = () => shown;
  overlay.set({ x: 100, y: 75, width: 100, height: 75 });
  const box = layer.children[0];
  const handle = box.children.find((child) => child.attributes['data-handle'] === 'e');
  handle.dataset.handle = 'e';
  return {
    overlay,
    box,
    handle,
    commits,
    panBy(dx) {
      shown = { ...shown, left: shown.left + dx };
    },
  };
}

// Le pointeur reste immobile ; seule la scène défile sous lui.
function panWhileHoldingEdge() {
  const view = mount();
  // Le bord est à x = 200 source, soit 150 px à l'écran. Le pointerdown part
  // de la boîte (seule à écouter) mais vise la poignée est.
  view.box.dispatch('pointerdown', { button: 0, clientX: 150, clientY: 100, pointerId: 1, target: view.handle });
  view.box.dispatch('pointermove', { clientX: 160, clientY: 100, pointerId: 1 });
  return view;
}

test('un bord de rognage tenu suit la scène qui défile', () => {
  const view = panWhileHoldingEdge();
  assert.equal(Math.round(view.overlay.get().width), 120);
  // La scène recule de 10 px : le même pointeur est maintenant sur 20 px de
  // source plus loin, donc le bord doit encore avancer de 20 px.
  view.panBy(-10);
  view.overlay.replay();
  assert.equal(Math.round(view.overlay.get().width), 140);
  assert.equal(Math.round(view.overlay.get().x), 100);
});

test('la boîte de rognage entière suit la scène qui défile', () => {
  const view = mount();
  // Un clic dans la boîte la déplace ; la scène recule ensuite sous lui.
  view.box.dispatch('pointerdown', { button: 0, clientX: 120, clientY: 100, pointerId: 1, target: view.box });
  view.panBy(-10);
  view.overlay.replay();
  assert.equal(Math.round(view.overlay.get().x), 120);
  assert.equal(Math.round(view.overlay.get().width), 100);
});

test('sans déplacement de scène, rejouer le geste ne change rien', () => {
  const view = panWhileHoldingEdge();
  view.overlay.replay();
  assert.equal(Math.round(view.overlay.get().width), 120);
  assert.equal(Math.round(view.overlay.get().x), 100);
});

test('le relâchement valide la taille suivie pendant le défilement', () => {
  const view = panWhileHoldingEdge();
  view.panBy(-10);
  view.overlay.replay();
  view.box.dispatch('pointerup', { clientX: 160, clientY: 100, pointerId: 1 });
  assert.equal(Math.round(view.commits.at(-1).width), 140);
});

test('l’éditeur image rejoue le geste quand la scène défile et se redimensionne', () => {
  assert.match(image, /onPan: replayGesture/);
  assert.match(image, /function applyDrag\(/);
  assert.match(image, /function replayGesture\(/);
  assert.match(image, /if \(cropOverlay\) \{[\s\S]{0,120}cropOverlay\.replay\(\)/);
  assert.match(image, /stageZoom\.fit\(\{ width: cssWidth, height: cssHeight \}\);[\s\S]{0,160}replayGesture\(\)/);
});

test('l’éditeur vidéo rejoue le geste après avoir replacé la vue', () => {
  assert.match(av, /if \(!resized\) \{[\s\S]{0,200}annotations\.replay\(\)/);
  assert.match(av, /if \(!resized\) \{[\s\S]{0,200}cropOverlay\.replay\(\)/);
  assert.match(annotations, /function replay\(\)/);
  assert.match(annotations, /^\s{4}replay,$/m);
  assert.match(annotations, /drag\.pointerX/);
});

test('le rognage ne fige plus la mesure de la couche au pointerdown', () => {
  assert.match(crop, /function pointerBase\(/);
  assert.match(crop, /function replay\(\)/);
  assert.doesNotMatch(crop, /const shown = layer\.getBoundingClientRect\(\);\s*\n\s*box\.setPointerCapture/);
});

test('la frise relit sa vue quand elle défile ou zoome pendant un glissement', () => {
  assert.match(av, /let timelineDrag = null;/);
  assert.match(av, /function applyTimelineDrag\(/);
  assert.match(av, /function replayTimelineDrag\(/);
  assert.match(av, /timelineDrag\.clientX = \w+\.clientX/);
  assert.match(av, /timelineDrag\.startClientX/);
  assert.match(av, /function setTimelineView\(next\)[\s\S]{0,400}replayTimelineDrag\(\)/);
  assert.match(av, /function applyTimelineDrag\(\)[\s\S]{0,600}track\.getBoundingClientRect\(\)/);
  assert.doesNotMatch(av, /const x = clamp\(moveEvent\.clientX - box\.left/);
});
