// One motion table for the board: durations and curves used by the hover
// preview, the detail window and the card hover. These source-level checks
// read the stylesheets and the DOM module as text and pin the single table
// (styles/motion.css), the tokens every rule reads, and the computed-style
// reader: a literal duration must not creep back beside the table.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const stripComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, '');
const normalize = (selector) => selector.replace(/\s+/g, ' ').trim();

// Every `selector { declarations }` rule at any depth, with the at-rules that
// enclose it. Comments are dropped first so a commented example never counts.
function rules(css) {
  const source = stripComments(css);
  const found = [];
  const stack = [];
  let buffer = '';
  for (const char of source) {
    if (char === '{') {
      stack.push(buffer.trim());
      buffer = '';
    } else if (char === '}') {
      const selector = stack.pop();
      if (selector && !selector.startsWith('@')) {
        found.push({ selector: normalize(selector), body: buffer, at: [...stack] });
      }
      buffer = '';
    } else {
      buffer += char;
    }
  }
  return found;
}

const selectorsOf = (rule) => rule.selector.split(',').map(normalize);
const isReduced = (rule) => rule.at.some((at) => at.startsWith('@media (prefers-reduced-motion: reduce)'));

// First rule whose selector list contains `selector`, optionally inside (or
// outside) the reduced-motion media query.
function ruleOf(css, selector, { reduced } = {}) {
  const rule = rules(css).find(
    (entry) => selectorsOf(entry).includes(selector)
      && (reduced === undefined ? true : isReduced(entry) === reduced),
  );
  return rule ? rule.body : null;
}

const escape = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const declare = (name, value) => new RegExp(`${escape(name)}:\\s*${escape(value)};`);

// The table as written in the design: every token and its base value.
const TOKENS = {
  '--ease-enter': 'cubic-bezier(0.22, 0.61, 0.36, 1)',
  '--ease-move': 'cubic-bezier(0.4, 0, 0.2, 1)',
  '--motion-reveal': '100ms',
  '--motion-card-lift': '120ms',
  '--motion-card-zoom': '220ms',
  '--motion-preview-open': '240ms',
  '--motion-preview-close': '150ms',
  '--motion-detail-open': '320ms',
  '--motion-detail-close': '210ms',
  '--motion-backdrop-in': '320ms',
  '--motion-backdrop-lead': '90ms',
};

// Only the durations a viewer asking for reduced motion still sees change:
// every movement that explains where something comes from is shortened, never
// removed. The cards' own transitions are cut by base.css, so their tokens
// keep the full values.
const REDUCED = {
  '--motion-preview-open': '200ms',
  '--motion-preview-close': '120ms',
  '--motion-detail-open': '220ms',
  '--motion-detail-close': '150ms',
  '--motion-backdrop-in': '200ms',
  '--motion-backdrop-lead': '100ms',
};

test('motion.css is imported right after base.css, the sheet that declares the palette', () => {
  const imports = Array.from(
    read('app.css').matchAll(/@import url\('\.\/styles\/([^']+)'\);/g),
    (match) => match[1],
  );
  const baseAt = imports.indexOf('base.css');
  assert.ok(baseAt >= 0, 'base.css is imported');
  assert.equal(imports[baseAt + 1], 'motion.css', 'the motion table comes right after base.css');
});

test('the :root table declares every token with its designed value', () => {
  const root = ruleOf(read('styles/motion.css'), ':root', { reduced: false });
  assert.ok(root, 'motion.css declares its tokens on :root');
  for (const [name, value] of Object.entries(TOKENS)) {
    assert.match(root, declare(name, value), `${name} is ${value}`);
  }
});

test('the reduce block is :root:not(.force-motion) and keeps only the shortened durations', () => {
  const reduced = ruleOf(read('styles/motion.css'), ':root:not(.force-motion)');
  assert.ok(reduced, 'the reduce block targets :root:not(.force-motion)');
  for (const [name, value] of Object.entries(REDUCED)) {
    assert.match(reduced, declare(name, value), `reduced ${name} is ${value}`);
  }
  for (const name of ['--motion-reveal', '--motion-card-lift', '--motion-card-zoom']) {
    assert.doesNotMatch(reduced, new RegExp(`${escape(name)}\\s*:`), `${name} keeps its full value`);
  }
});

test('the fade tokens are gone: the preview is the card, never an opacity fade', () => {
  const motion = read('styles/motion.css');
  const detailCss = read('styles/media-detail.css');
  for (const token of ['--motion-preview-fade-in', '--motion-preview-fade-out']) {
    assert.ok(!motion.includes(token), `motion.css must not keep ${token}`);
    assert.ok(!detailCss.includes(token), `media-detail.css must not read ${token}`);
  }
});

test('no .hp-overlay rule keeps a literal millisecond duration', () => {
  for (const rule of rules(read('styles/media-detail.css'))) {
    if (!rule.selector.includes('.hp-overlay')) {
      continue;
    }
    assert.doesNotMatch(rule.body, /\d+(?:\.\d+)?ms\b/, `literal duration in « ${rule.selector} »`);
  }
});

test('the preview container never fades: only the card ghost and the mute control crossfade', () => {
  // The preview is the card, never a second card fading over the first one:
  // its container, its media band, its progress bar and its body keep their
  // movement. The two content crossfades inside it (the card's ghost and the
  // mute control) are pinned by their own tests below.
  for (const rule of rules(read('styles/media-detail.css'))) {
    if (!rule.selector.includes('.hp-overlay')) {
      continue;
    }
    if (rule.selector.includes('.hp-card-ghost') || rule.selector.includes('.hp-mute')) {
      continue;
    }
    assert.doesNotMatch(rule.body, /opacity\s+var\(/, `opacity transition in « ${rule.selector} »`);
  }
});

test('the preview reads the open and close tokens, curves included', () => {
  const detailCss = read('styles/media-detail.css');
  const open = ruleOf(detailCss, '.hp-overlay', { reduced: false });
  assert.ok(open, '.hp-overlay is styled');
  assert.match(open, /transform var\(--motion-preview-open\) var\(--ease-enter\)/);
  assert.match(open, /clip-path var\(--motion-preview-open\) var\(--ease-enter\)/);
  assert.match(open, /box-shadow var\(--motion-preview-open\) var\(--ease-enter\)/);
  assert.doesNotMatch(open, /opacity var\(/);
  // The card is the start frame: top-left origin, translated and scaled onto
  // it, clipped below its bottom edge and rounded like its corners.
  assert.match(open, /transform-origin:\s*0 0/);
  assert.match(open, /translate\(var\(--hp-x, 0\), var\(--hp-y, 0\)\) scale\(var\(--hp-s, 1\)\)/);
  assert.match(open, /inset\(0 0 var\(--hp-clip, 0\) 0 round var\(--hp-radius, 0\)\)/);
  assert.match(open, /opacity:\s*1/);
  assert.match(open, /box-shadow:\s*none/);

  const close = ruleOf(detailCss, '.hp-overlay.is-out', { reduced: false });
  assert.ok(close, '.hp-overlay.is-out is styled');
  assert.match(close, /transform var\(--motion-preview-close\) var\(--ease-move\)/);
  assert.match(close, /clip-path var\(--motion-preview-close\) var\(--ease-move\)/);
  assert.match(close, /box-shadow var\(--motion-preview-close\) var\(--ease-move\)/);
  assert.doesNotMatch(close, /opacity var\(/);
});

test('the preview media, progress bar and body follow the start frame and its end state', () => {
  const detailCss = read('styles/media-detail.css');

  const media = ruleOf(detailCss, '.hp-overlay .hp-media', { reduced: false });
  assert.ok(media, 'the media band is styled');
  assert.match(media, /clip-path:\s*inset\(var\(--hp-crop, 0\) 0 var\(--hp-crop, 0\) 0\)/);
  assert.match(media, /transform:\s*translateY\(calc\(-1 \* var\(--hp-crop, 0\)\)\)/);
  assert.match(media, /transition:\s*[^;]*transform var\(--motion-preview-open\) var\(--ease-enter\)/);
  assert.match(media, /clip-path var\(--motion-preview-open\) var\(--ease-enter\)/);

  for (const selector of ['.hp-overlay .hp-progress', '.hp-overlay .hp-body']) {
    const rule = ruleOf(detailCss, selector, { reduced: false });
    assert.ok(rule, `${selector} is styled`);
    assert.match(rule, /transform:\s*translateY\(var\(--hp-shift, 0\)\)/);
    assert.match(rule, /transition:\s*transform var\(--motion-preview-open\) var\(--ease-enter\)/);
  }

  const inside = ruleOf(detailCss, '.hp-overlay.is-in', { reduced: false });
  assert.ok(inside, '.hp-overlay.is-in is styled');
  assert.match(inside, /transform:\s*translate\(0, 0\) scale\(1\)/);
  // Open, the clip reaches past the edges so the drop shadow is not cut (see
  // the shadow regression test in board-detail-window.test.mjs).
  assert.match(inside, /clip-path:\s*inset\((-\d+px\s+){3}-\d+px round var\(--radius-m\)\)/);
  const insideMedia = ruleOf(detailCss, '.hp-overlay.is-in .hp-media', { reduced: false });
  assert.match(insideMedia, /clip-path:\s*inset\(0 0 0 0\)/);
  assert.match(insideMedia, /transform:\s*none/);
  for (const selector of ['.hp-overlay.is-in .hp-progress', '.hp-overlay.is-in .hp-body']) {
    assert.match(ruleOf(detailCss, selector, { reduced: false }), /transform:\s*none/);
  }

  const closing = ruleOf(detailCss, '.hp-overlay.is-out .hp-media', { reduced: false });
  assert.ok(closing, 'the close puts the band back');
  assert.match(closing, /transition:\s*[^;]*transform var\(--motion-preview-close\) var\(--ease-move\)/);
  for (const selector of ['.hp-overlay.is-out .hp-progress', '.hp-overlay.is-out .hp-body']) {
    assert.match(
      ruleOf(detailCss, selector, { reduced: false }),
      /transition:\s*transform var\(--motion-preview-close\) var\(--ease-move\)/,
    );
  }
});

test('placement runs with transitions off, after the start frame is committed', () => {
  const detailCss = read('styles/media-detail.css');
  const placing = ruleOf(detailCss, '.hp-overlay.is-placing');
  assert.ok(placing, 'the placement class is styled');
  assert.match(placing, /transition:\s*none\s*!important/);
  const children = rules(detailCss).find(
    (rule) => selectorsOf(rule).includes('.hp-overlay.is-placing *'),
  );
  assert.ok(children, 'the children keep transitions off too');
  assert.match(children.body, /transition:\s*none\s*!important/);
});

test('the card is hidden while the preview stands for it, but keeps its pointer events', () => {
  const boardCss = read('styles/board.css');
  const previewing = ruleOf(boardCss, '.card.is-previewing');
  assert.ok(previewing, '.card.is-previewing is styled');
  assert.match(previewing, /opacity:\s*0/);
  assert.doesNotMatch(previewing, /visibility:\s*hidden/, 'opacity keeps the card clickable');
  assert.doesNotMatch(previewing, /pointer-events:\s*none/);
});

test('the preview grows over a ghost of the card that crossfades away', () => {
  const detailCss = read('styles/media-detail.css');

  const ghost = ruleOf(detailCss, '.hp-overlay .hp-card-ghost', { reduced: false });
  assert.ok(ghost, 'the card ghost is styled');
  assert.match(ghost, /position:\s*absolute/);
  assert.match(ghost, /top:\s*0/);
  assert.match(ghost, /left:\s*0/);
  assert.match(ghost, /z-index:\s*6/);
  assert.match(ghost, /margin:\s*0/);
  assert.match(ghost, /pointer-events:\s*none/);
  assert.match(ghost, /transform-origin:\s*0 0/);
  // The overlay is scaled onto the card: the ghost cancels that scale so the
  // card's pixels keep their own size in the start frame.
  assert.match(ghost, /transform:\s*scale\(calc\(1 \/ var\(--hp-s, 1\)\)\)/);
  assert.match(ghost, /opacity:\s*1/);
  assert.match(ghost, /box-shadow:\s*none/, 'the ghost never shows a lifted shadow');
  assert.match(
    ghost,
    /transition:\s*opacity calc\(var\(--motion-preview-open\) \* 0\.5\) linear/,
    'the ghost fades over the first half of the opening',
  );
  assert.doesNotMatch(ghost, /\d+(?:\.\d+)?ms\b/, 'the ghost reads the tokens');

  const open = ruleOf(detailCss, '.hp-overlay.is-in .hp-card-ghost', { reduced: false });
  assert.ok(open, 'the open preview hides the ghost');
  assert.match(open, /opacity:\s*0/);

  const close = ruleOf(detailCss, '.hp-overlay.is-out .hp-card-ghost', { reduced: false });
  assert.ok(close, 'the closing preview brings the ghost back');
  assert.match(close, /opacity:\s*1/);
  assert.match(
    close,
    /transition:\s*opacity calc\(var\(--motion-preview-close\) \* 0\.5\) linear calc\(var\(--motion-preview-close\) \* 0\.5\)/,
    'the ghost returns during the second half of the close, so the last frame is the card',
  );
});

test('the mute control is hidden at the start frame and crossfades with the preview', () => {
  const detailCss = read('styles/media-detail.css');

  const mute = ruleOf(detailCss, '.hp-overlay .hp-mute', { reduced: false });
  assert.ok(mute, 'the mute control crossfades');
  assert.match(mute, /opacity:\s*0/);
  assert.match(
    mute,
    /transition:\s*opacity calc\(var\(--motion-preview-open\) \* 0\.5\) ease calc\(var\(--motion-preview-open\) \* 0\.5\)/,
    'it fades in during the second half of the opening',
  );
  assert.doesNotMatch(mute, /\d+(?:\.\d+)?ms\b/);

  const open = ruleOf(detailCss, '.hp-overlay.is-in .hp-mute', { reduced: false });
  assert.ok(open, 'the open preview shows the mute control');
  assert.match(open, /opacity:\s*1/);

  const close = ruleOf(detailCss, '.hp-overlay.is-out .hp-mute', { reduced: false });
  assert.ok(close, 'the closing preview hides it again');
  assert.match(close, /opacity:\s*0/);
  assert.match(
    close,
    /transition:\s*opacity calc\(var\(--motion-preview-close\) \* 0\.5\) ease(?! calc)/,
    'it is out during the first half of the close',
  );

  // The existing hover and keyboard-focus rules still work once open: the
  // crossfade only adds opacity, it does not replace them.
  assert.match(ruleOf(detailCss, '.hp-mute:hover', { reduced: false }), /color:\s*#fff/);
  assert.match(ruleOf(detailCss, '.hp-mute:focus-visible', { reduced: false }), /outline:/);
});

test('the preview keeps its movement under reduced motion, with the same tokens and !important', () => {
  const detailCss = read('styles/media-detail.css');
  assert.doesNotMatch(detailCss, /force-motion/, 'the tokens replace the debug escape hatch');

  const open = ruleOf(detailCss, '.hp-overlay', { reduced: true });
  assert.ok(open, 'the reduce block repeats the opening transition');
  assert.match(open, /transform var\(--motion-preview-open\) var\(--ease-enter\)/);
  assert.match(open, /clip-path var\(--motion-preview-open\) var\(--ease-enter\)/);
  assert.match(open, /box-shadow var\(--motion-preview-open\) var\(--ease-enter\)/);
  assert.doesNotMatch(open, /opacity var\(/);
  assert.match(open, /!important/, 'base.css cuts every transition with !important');

  const close = ruleOf(detailCss, '.hp-overlay.is-out', { reduced: true });
  assert.ok(close, 'the reduce block repeats the closing transition');
  assert.match(close, /transform var\(--motion-preview-close\) var\(--ease-move\)/);
  assert.match(close, /clip-path var\(--motion-preview-close\) var\(--ease-move\)/);
  assert.match(close, /box-shadow var\(--motion-preview-close\) var\(--ease-move\)/);
  assert.doesNotMatch(close, /opacity var\(/);
  assert.match(close, /!important/);

  // Every child that moves with the overlay repeats its own token: the media
  // band (transform and clip-path), then the progress bar and the body
  // (transform), opening and closing.
  for (const selector of ['.hp-overlay .hp-media', '.hp-overlay .hp-progress', '.hp-overlay .hp-body']) {
    const rule = ruleOf(detailCss, selector, { reduced: true });
    assert.ok(rule, `${selector} keeps moving under reduced motion`);
    assert.match(rule, /transition:/);
    assert.match(rule, /!important/);
    assert.doesNotMatch(rule, /\d+ms/);
  }
  assert.match(
    ruleOf(detailCss, '.hp-overlay .hp-media', { reduced: true }),
    /clip-path var\(--motion-preview-open\) var\(--ease-enter\)/,
  );
  for (const selector of ['.hp-overlay.is-out .hp-media', '.hp-overlay.is-out .hp-progress', '.hp-overlay.is-out .hp-body']) {
    const rule = ruleOf(detailCss, selector, { reduced: true });
    assert.ok(rule, `${selector} keeps returning under reduced motion`);
    assert.match(rule, /var\(--motion-preview-close\) var\(--ease-move\)/);
    assert.match(rule, /!important/);
  }
});

test('the ghost and mute crossfades survive reduced motion, with !important', () => {
  // base.css cuts every transition with !important: each crossfade repeats its
  // own token here, or the card ghost and the mute control would appear and
  // disappear in one frame while the preview keeps moving.
  const detailCss = read('styles/media-detail.css');
  const crossfades = [
    ['.hp-overlay .hp-card-ghost', '--motion-preview-open'],
    ['.hp-overlay.is-out .hp-card-ghost', '--motion-preview-close'],
    ['.hp-overlay .hp-mute', '--motion-preview-open'],
    ['.hp-overlay.is-out .hp-mute', '--motion-preview-close'],
  ];
  for (const [selector, token] of crossfades) {
    const rule = ruleOf(detailCss, selector, { reduced: true });
    assert.ok(rule, `${selector} keeps its crossfade under reduced motion`);
    assert.match(
      rule,
      new RegExp(`transition:\\s*opacity calc\\(var\\(${token}\\) \\* 0\\.5\\)`),
      `${selector} reads ${token}, not a literal`,
    );
    assert.match(rule, /!important/, `${selector} beats base.css`);
    assert.doesNotMatch(rule, /\d+(?:\.\d+)?ms\b/, `${selector} has no literal duration`);
  }
  // Every one of them fades over half its movement.
  for (const selector of ['.hp-overlay .hp-card-ghost', '.hp-overlay .hp-mute']) {
    assert.match(ruleOf(detailCss, selector, { reduced: true }), /calc\(var\([^)]*\) \* 0\.5\)/);
  }

  // Placement still suspends them: its rule comes last, after the reduce
  // block, so its !important wins there too.
  const reduceAt = detailCss.indexOf('@media (prefers-reduced-motion: reduce)');
  const placementAt = detailCss.lastIndexOf('.hp-overlay.is-placing *');
  assert.ok(reduceAt >= 0, 'the reduce block exists');
  assert.ok(placementAt > reduceAt, 'the placement rule stays after the reduce block');
});

test('the card hover transitions read the table too', () => {
  const boardCss = read('styles/board.css');

  const card = ruleOf(boardCss, '.card');
  assert.ok(card, '.card is styled');
  assert.match(card, /border-color var\(--motion-card-lift\) ease/);
  assert.match(card, /box-shadow var\(--motion-card-lift\) ease/);
  assert.match(card, /transform var\(--motion-card-lift\) ease/);
  assert.doesNotMatch(card, /\d+ms/, 'the card lift is the token, not a literal');

  const zoom = ruleOf(boardCss, '.card .thumb img');
  assert.ok(zoom, '.card .thumb img is styled');
  assert.match(zoom, /transform var\(--motion-card-zoom\) ease/);
  assert.doesNotMatch(zoom, /\d+ms/);

  const fav = ruleOf(boardCss, '.fav');
  assert.ok(fav, '.fav is styled');
  assert.match(fav, /opacity var\(--motion-reveal\) ease/);
  assert.doesNotMatch(fav, /\d+ms/);

  const play = ruleOf(boardCss, '.play');
  assert.ok(play, '.play is styled');
  assert.match(play, /opacity var\(--motion-reveal\) ease/);
  assert.match(play, /transform var\(--motion-reveal\) ease/);
  assert.doesNotMatch(play, /\d+ms/);
});

test('the window reads the computed tokens once, no system query of its own', () => {
  const mediaDetail = read('views/media-detail.js');
  assert.match(
    mediaDetail,
    /readMotion\(window\.getComputedStyle\(document\.documentElement\)\)/,
    'the window reads the table from the document element',
  );
  for (const gone of ['prefers-reduced-motion', 'motionDurations', 'EASE_OUT', 'EASE_IN_OUT']) {
    assert.ok(!mediaDetail.includes(gone), `media-detail.js must not keep ${gone}`);
  }
  assert.match(
    mediaDetail,
    /setTimeout\(\(\) => \{[\s\S]*?el\.remove\(\);\s*\}, previewClose \+ 40\)/,
    'the preview removal follows the table instead of a literal 160',
  );
  assert.doesNotMatch(mediaDetail, /, 160\)/);
});
