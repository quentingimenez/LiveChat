// Éditeur de média — zoom Ctrl + molette et déplacement à la molette maintenue.
//
// Ce que ce fichier protège :
//  - la frise garde une fenêtre visible `{ start, end }` : le zoom est ancré
//    sous le pointeur, ne sort jamais du média, et se remet entière d'un
//    double-clic ;
//  - l'aperçu (image, vidéo et aperçu sur l'overlay) se zoome autour du
//    pointeur sans perdre l'endroit regardé : le défilement est ajusté du
//    déplacement du contenu ;
//  - une molette sans Ctrl ne zoome rien (le test « cas sans changement ») ;
//  - la molette maintenue déplace la scène et la frise dans le sens opposé au
//    pointeur, main fermée, sans le défilement automatique de Chromium.
//
// Les calculs sont testés par leurs fonctions pures ; les branchements DOM
// sont vérifiés dans la source des éditeurs, comme le fait déjà
// editor-timeline.test.mjs (pas de DOM sous node:test).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  MAX_STAGE_ZOOM,
  MAX_TIMELINE_ZOOM,
  MIN_STAGE_ZOOM,
  anchorScroll,
  clampTimelineView,
  defaultTimelineView,
  minTimelineSpan,
  panTimelineByDrag,
  panTimelineView,
  revealTimelineTime,
  timelineZoomFactor,
  wheelZoomFactor,
  zoomTimelineView,
  zoomedByWheel,
} from '../../web/app/core/editor-helpers.js';
import { createMiddlePan } from '../../web/app/views/editor/middle-pan.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

const av = read('views/editor/av.js');
const image = read('views/editor/image.js');
const preview = read('views/editor/display-preview.js');
const stageZoom = read('views/editor/stage-zoom.js');
const middlePan = read('views/editor/middle-pan.js');
const tools = read('views/editor/tools.js');
const editor = read('styles/editor.css');
const annotationsSource = read('views/editor/video-annotations.js');

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

function span(view) {
  return Math.round((view.end - view.start) * 1000) / 1000;
}

// Élément minimal pour piloter le geste sans DOM : listeners, curseur, classe.
function fakeElement() {
  const listeners = new Map();
  return {
    style: { cursor: '' },
    classList: {
      values: new Set(),
      add(name) {
        this.values.add(name);
      },
      remove(name) {
        this.values.delete(name);
      },
      contains(name) {
        return this.values.has(name);
      },
    },
    addEventListener(type, handler) {
      const list = listeners.get(type) ?? [];
      list.push(handler);
      listeners.set(type, list);
    },
    removeEventListener(type, handler) {
      const list = listeners.get(type) ?? [];
      const index = list.indexOf(handler);
      if (index >= 0) {
        list.splice(index, 1);
      }
    },
    fire(type, event) {
      if (typeof event.preventDefault !== 'function') {
        event.defaultPrevented = false;
        event.preventDefault = () => {
          event.defaultPrevented = true;
        };
      }
      for (const handler of listeners.get(type) ?? []) {
        handler(event);
      }
    },
    setPointerCapture() {},
  };
}

// --- frise ---------------------------------------------------------------

test('la frise démarre entière sur le média', () => {
  assert.deepEqual(defaultTimelineView(6), { start: 0, end: 6 });
  assert.deepEqual(defaultTimelineView(0), { start: 0, end: 0 });
});

test('un cran de molette zoome autour du pointeur', () => {
  const view = zoomTimelineView({ start: 0, end: 6 }, 6, 3, 2);
  assert.deepEqual(view, { start: 1.5, end: 4.5 });
  assert.equal(span(view), 3);
});

test('le zoom de la frise ne sort jamais du média', () => {
  assert.deepEqual(zoomTimelineView({ start: 0, end: 6 }, 6, 0, 2), { start: 0, end: 3 });
  assert.deepEqual(zoomTimelineView({ start: 0, end: 6 }, 6, 6, 2), { start: 3, end: 6 });
  assert.deepEqual(zoomTimelineView({ start: 0, end: 6 }, 6, 3, 0.5), { start: 0, end: 6 });
});

test('la frise a un zoom maximal et un zoom minimal', () => {
  let view = { start: 0, end: 6 };
  for (let index = 0; index < 50; index += 1) {
    view = zoomTimelineView(view, 6, 3, 1.5);
  }
  assert.equal(span(view), minTimelineSpan(6));
  assert.equal(minTimelineSpan(6), 0.2);
  assert.equal(minTimelineSpan(600), 6);
  assert.equal(MAX_TIMELINE_ZOOM, 100);
  assert.ok(span(view) >= 0.2);
});

test('déplacer la frise reste borné', () => {
  assert.deepEqual(panTimelineView({ start: 1, end: 3 }, 6, 10), { start: 4, end: 6 });
  assert.deepEqual(panTimelineView({ start: 1, end: 3 }, 6, -10), { start: 0, end: 2 });
  assert.deepEqual(panTimelineView({ start: 0, end: 6 }, 6, 4), { start: 0, end: 6 });
});

test('tirer la frise vers la droite remonte le temps', () => {
  // 100 px sur 1000 : 0,2 s de fenêtre en arrière, le contenu suit la souris.
  assert.deepEqual(panTimelineByDrag({ start: 1, end: 3 }, 6, 100, 1000), { start: 0.8, end: 2.8 });
  assert.deepEqual(panTimelineByDrag({ start: 3, end: 5 }, 6, -100, 1000), { start: 3.2, end: 5.2 });
  // Bords, largeur nulle et geste absent ne produisent ni saut ni NaN.
  assert.deepEqual(panTimelineByDrag({ start: 0, end: 2 }, 6, 100, 1000), { start: 0, end: 2 });
  assert.deepEqual(panTimelineByDrag({ start: 3, end: 5 }, 6, -1000, 0), { start: 4, end: 6 });
  assert.deepEqual(panTimelineByDrag({ start: 1, end: 3 }, 6, 0, 1000), { start: 1, end: 3 });
  assert.deepEqual(panTimelineByDrag({ start: 0, end: 6 }, 6, 100, 1000), { start: 0, end: 6 });
});

test('la frise suit la tête de lecture sans sauter quand elle est visible', () => {
  const inside = { start: 2, end: 4 };
  assert.deepEqual(revealTimelineTime(inside, 6, 3), inside);
  const after = revealTimelineTime({ start: 0, end: 2 }, 6, 4);
  assert.equal(span(after), 2);
  assert.ok(after.start < 4 && after.end >= 4, 'la tête doit rester dans la fenêtre');
  const before = revealTimelineTime({ start: 4, end: 6 }, 6, 1);
  assert.equal(span(before), 2);
  assert.ok(before.start <= 1 && before.end > 1, 'la tête doit rester dans la fenêtre');
});

test('le facteur de zoom de la frise se lit sur la fenêtre', () => {
  assert.equal(timelineZoomFactor({ start: 0, end: 6 }, 6), 1);
  assert.equal(timelineZoomFactor({ start: 1, end: 3 }, 6), 3);
  assert.equal(timelineZoomFactor({ start: 3, end: 3 }, 6), 1);
});

test('une durée inconnue ne casse pas la fenêtre', () => {
  assert.deepEqual(defaultTimelineView(NaN), { start: 0, end: 0 });
  assert.deepEqual(clampTimelineView({ start: 1, end: 2 }, 0), { start: 0, end: 0 });
  assert.deepEqual(zoomTimelineView({ start: 0, end: 0 }, 0, 0, 2), { start: 0, end: 0 });
  assert.deepEqual(panTimelineView({ start: 0, end: 0 }, 0, 5), { start: 0, end: 0 });
});

// --- molette commune ------------------------------------------------------

test('une molette sans Ctrl ne zoome pas', () => {
  assert.equal(zoomedByWheel(2, { ctrlKey: false, metaKey: false, deltaY: -120 }), null);
  assert.equal(zoomedByWheel(2, { deltaY: -120 }), null);
});

test('Ctrl + molette zoome, Ctrl + Meta aussi', () => {
  assert.ok(zoomedByWheel(2, { ctrlKey: true, deltaY: -120 }) > 2);
  assert.ok(zoomedByWheel(2, { metaKey: true, deltaY: -120 }) > 2);
  assert.ok(zoomedByWheel(2, { ctrlKey: true, deltaY: 120 }) < 2);
});

test('le zoom de l’aperçu est borné et ne bouge pas aux bornes', () => {
  assert.equal(zoomedByWheel(MAX_STAGE_ZOOM, { ctrlKey: true, deltaY: -5000 }), null);
  assert.equal(zoomedByWheel(MIN_STAGE_ZOOM, { ctrlKey: true, deltaY: 5000 }), null);
  assert.equal(zoomedByWheel(MIN_STAGE_ZOOM, { ctrlKey: true, deltaY: -1e6, deltaMode: 2 }), MAX_STAGE_ZOOM);
});

test('un deltaY en lignes ou en pages pèse plus qu’en pixels', () => {
  const pixels = wheelZoomFactor(-3, 0);
  assert.ok(wheelZoomFactor(-3, 1) > pixels, 'deltaMode ligne doit zoomer plus');
  assert.ok(wheelZoomFactor(-3, 2) > wheelZoomFactor(-3, 1), 'deltaMode page doit zoomer plus');
  assert.ok(wheelZoomFactor(-120) > 1);
  assert.ok(wheelZoomFactor(120) < 1);
});

// --- aperçu --------------------------------------------------------------

test('le zoom de l’aperçu garde le point sous le pointeur', () => {
  const before = { left: 100, top: 50, width: 400, height: 300 };
  const pointer = { x: 200, y: 125 };
  // Contenu agrandi autour du même coin : le point a glissé de 100 px.
  const after = { left: 100, top: 50, width: 800, height: 600 };
  const next = anchorScroll({ scrollLeft: 0, scrollTop: 0, pointer, before, after });
  assert.equal(next.left, 100);
  // 25 % de 600 px = 150 px, soit 75 px de défilement pour le ramener sous
  // un pointeur situé à 75 px du haut du contenu.
  assert.equal(next.top, 75);
});

test('chaque axe de l’aperçu garde son ancre', () => {
  const before = { left: 0, top: 0, width: 200, height: 100 };
  const pointer = { x: 50, y: 75 };
  const after = { left: 0, top: 0, width: 400, height: 200 };
  // 50/200 = 25 % → 100 px ; 75/100 = 75 % → 150 px.
  assert.deepEqual(
    anchorScroll({ scrollLeft: 10, scrollTop: 20, pointer, before, after }),
    { left: 10 + 100 - 50, top: 20 + 150 - 75 },
  );
});

test('un contenu dégénéré ne donne pas de NaN', () => {
  const before = { left: 0, top: 0, width: 0, height: 0 };
  const after = { left: 0, top: 0, width: 10, height: 10 };
  const next = anchorScroll({ scrollLeft: 0, scrollTop: 0, pointer: { x: 5, y: 5 }, before, after });
  assert.deepEqual(next, { left: 0, top: 0 });
});

// --- molette maintenue ----------------------------------------------------

test('le bouton du milieu déplace et affiche une main fermée', () => {
  const element = fakeElement();
  const moves = [];
  const pan = createMiddlePan(element, (dx, dy) => moves.push([dx, dy]));
  const down = { button: 1, clientX: 100, clientY: 80, pointerId: 3, preventDefault() {} };
  element.fire('pointerdown', down);
  assert.equal(element.style.cursor, 'grabbing');
  assert.equal(element.classList.contains('is-panning'), true);
  element.fire('pointermove', { buttons: 4, clientX: 145, clientY: 50 });
  assert.deepEqual(moves, [[45, -30]]);
  element.fire('pointerup', { button: 1 });
  assert.equal(element.style.cursor, '');
  assert.equal(element.classList.contains('is-panning'), false);
  pan.destroy();
});

test('un clique gauche ne démarre pas le déplacement', () => {
  const element = fakeElement();
  const moves = [];
  const pan = createMiddlePan(element, (dx, dy) => moves.push([dx, dy]));
  element.fire('pointerdown', { button: 0, clientX: 10, clientY: 10, pointerId: 4 });
  element.fire('pointermove', { buttons: 1, clientX: 30, clientY: 30 });
  assert.equal(element.style.cursor, '');
  assert.deepEqual(moves, []);
  pan.destroy();
});

test('sans rien à déplacer, le geste ne démarre pas mais Chromium est bloqué', () => {
  const element = fakeElement();
  const pan = createMiddlePan(element, () => assert.fail('aucun déplacement attendu'), { canStart: () => false });
  const native = { button: 1, cancelable: true, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } };
  element.fire('mousedown', native);
  assert.equal(native.defaultPrevented, true, 'le défilement natif doit être annulé');
  const down = { button: 1, clientX: 100, clientY: 80, pointerId: 5, preventDefault() {} };
  element.fire('pointerdown', down);
  assert.equal(element.style.cursor, '');
  assert.equal(element.classList.contains('is-panning'), false);
  pan.destroy();
});

test('relâcher le bouton en dehors termine le geste', () => {
  const element = fakeElement();
  const moves = [];
  const pan = createMiddlePan(element, (dx, dy) => moves.push([dx, dy]));
  element.fire('pointerdown', { button: 1, clientX: 0, clientY: 0, pointerId: 6 });
  element.fire('pointermove', { buttons: 0, clientX: 50, clientY: 20 });
  assert.equal(element.classList.contains('is-panning'), false);
  assert.equal(element.style.cursor, '');
  assert.deepEqual(moves, []);
  pan.destroy();
});

// --- branchements DOM ----------------------------------------------------

test('la frise écoute Ctrl + molette sans laisser le navigateur zoomer', () => {
  assert.match(av, /track\.addEventListener\('wheel'/);
  assert.match(av, /event\.ctrlKey \|\| event\.metaKey/);
  assert.match(av, /\{ passive: false \}/);
  assert.match(av, /event\.preventDefault\(\)/);
  assert.match(av, /zoomTimelineView\(/);
});

test('la frise peint la fenêtre visible et sait revenir à la vue entière', () => {
  assert.match(av, /timelineView\.start/);
  assert.match(av, /timelineView\.end/);
  assert.match(av, /const timeToX = \(time, width\) =>[\s\S]{0,240}timelineView\.start/);
  assert.match(av, /track\.addEventListener\('dblclick'/);
  assert.match(av, /tl-zoom/);
});

test('l’aperçu de l’image et de la vidéo passent par le même zoom de scène', () => {
  assert.match(image, /createStageZoom\(/);
  assert.match(image, /stageZoom\.available\(\)/);
  // The picture hands its zoomed size over: the stage places it from that.
  assert.match(image, /stageZoom\.fit\(\{ width: cssWidth, height: cssHeight \}\)/);
  assert.match(av, /stageZoom = createStageZoom\(/);
  assert.match(av, /stageZoom\.available\(\)/);
});

test('le zoom de scène écoute Ctrl + molette et garde le contenu centré', () => {
  assert.match(stageZoom, /addEventListener\('wheel'/);
  assert.match(stageZoom, /zoomedByWheel\(/);
  assert.match(stageZoom, /\{ passive: false \}/);
  assert.match(stageZoom, /event\.preventDefault\(\)/);
  assert.match(stageZoom, /anchoredScroll\(/);
  // The frame scrolls virtually (see stage-zoom-layout.test.mjs): no native
  // scroller, whose scrollbar would resize the zoomed content.
  const css = ruleOf(editor, '.stage-zoom');
  assert.ok(css, 'editor.css doit définir .stage-zoom');
  assert.match(css, /overflow:\s*hidden/);
  assert.doesNotMatch(css, /overflow:\s*auto/);
  assert.match(css, /background:\s*var\(--stage\)/);
  assert.match(ruleOf(editor, '.stage-zoom > .stage-zoom-content'), /position:\s*absolute/);
  assert.match(ruleOf(editor, '.stage-zoom-bar.is-x'), /height:\s*10px/);
  assert.match(ruleOf(editor, '.stage-zoom-bar.is-y'), /width:\s*10px/);
  assert.match(stageZoom, /export const STAGE_BAR = 10;/);

  const previewScroll = ruleOf(editor, '.display-preview-scroll');
  assert.ok(previewScroll, 'editor.css doit définir .display-preview-scroll');
  assert.match(previewScroll, /background:\s*var\(--stage\)/);
  assert.match(previewScroll, /overscroll-behavior:\s*none/);

  const video = ruleOf(editor, '.av-inner video');
  assert.ok(video, 'editor.css doit définir .av-inner video');
  // La vidéo reste native : aucun canvas miroir, aucune boucle de dessin.
  assert.doesNotMatch(video, /display:\s*none/);
  assert.equal(ruleOf(editor, '.av-view'), null);
  assert.doesNotMatch(av, /av-view|paintVideoView|drawImage\(media|is-gesture/);
  // Aucun filtre sur la scène : il ferait composer la vidéo par Chromium au
  // lieu de la confier au plan matériel, et sur un écran à large gamut elle
  // s'affiche alors plus pâle que partout ailleurs.
  const stage = ruleOf(editor, '.editor-stage');
  assert.ok(stage, 'editor.css doit définir .editor-stage');
  assert.doesNotMatch(stage, /filter/);
});

test('la vidéo zoomée reste immobile : seul son object-view-box suit la vue', () => {
  // A moving video element loses the hardware overlay for the whole gesture.
  assert.match(av, /windowed: true/);
  assert.match(av, /onPan: \(view\) => placeVideo\(view, false\)/);
  assert.match(av, /videoViewBox\(\{ crop, turns, flipH, flipV, scale, window: view \}\)/);
  assert.match(av, /media\.style\.objectViewBox = `inset\(/);
  // Sized to the shown part, at the top left of `.av-inner`: never offset,
  // never transformed.
  assert.match(av, /media\.style\.width = `\$\{box\.width \* scale\}px`/);
  assert.doesNotMatch(av, /media\.style\.(left|top|transform) =/);
  assert.match(ruleOf(editor, '.av-inner video'), /object-fit:\s*fill/);
  // The crop box and the drawing follow the view's origin.
  assert.match(av, /host: frameLayer/);
  assert.match(av, /annotations\.place\(\)/);
  assert.match(annotationsSource, /function place\(\)/);
  assert.match(annotationsSource, /^    place,$/m);
});

test('l’aperçu sur l’overlay se zoome aussi à la molette', () => {
  assert.match(preview, /addEventListener\('wheel'/);
  assert.match(preview, /zoomedByWheel\(/);
  assert.match(preview, /anchorScroll\(/);
  assert.match(preview, /\{ passive: false \}/);
});

test('la molette maintenue déplace la scène et la frise, sans Chromium', () => {
  // Le geste n'écoute que le bouton du milieu et coupe le défilement natif.
  assert.match(middlePan, /event\.button === 1/);
  assert.match(middlePan, /event\.button !== 1/);
  assert.match(middlePan, /addEventListener\('mousedown', preventNative, \{ capture: true \}\)/);
  assert.match(middlePan, /addEventListener\('auxclick', preventNative, \{ capture: true \}\)/);
  assert.match(middlePan, /event\.preventDefault\(\)/);
  assert.match(middlePan, /setPointerCapture\(/);
  assert.match(middlePan, /PAN_CURSOR = 'grabbing'/);
  assert.match(middlePan, /canStart/);

  // Les trois surfaces défilables partagent le geste ; le contenu suit la
  // souris, le cadre défile donc à l'opposé (scrollLeft -= dx).
  assert.match(stageZoom, /createMiddlePan\(viewport/);
  assert.match(stageZoom, /scrollTo\(scroll\.left - dx, scroll\.top - dy\)/);
  assert.match(preview, /createMiddlePan\(scroll/);
  assert.match(preview, /scrollLeft -= dx/);
  assert.match(av, /createMiddlePan\(track/);
  assert.match(av, /panTimelineByDrag\(timelineView, duration, dx, track\.clientWidth\)/);

  // La frise ne démarre le geste qu'une fois zoomée ; la scène, que si elle
  // déborde. Sans rien à déplacer, le clique molette reste neutralisé.
  assert.match(av, /timelineZoomFactor\(timelineView, duration\) > 1\.0001/);
  assert.match(stageZoom, /canStart: \(\) => axes\.x\.overflow \|\| axes\.y\.overflow/);
  assert.match(preview, /scrollWidth > scroll\.clientWidth/);

  assert.match(editor, /\.tl-track\.is-panning[\s\S]{0,200}cursor:\s*grabbing/);
});

test('la découpe annonce le zoom de la frise', () => {
  assert.match(tools, /Ctrl \+ molette/);
});
