// The board now opens the media detail window from views/media-detail.js: the
// floating side drawer is gone. These source-level checks pin the wiring, so a
// later edit cannot silently bring the drawer back or forget the stylesheet.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const board = read('views/board.js');
const baseCss = read('styles/base.css');
const boardCss = read('styles/board.css');
const detailCss = read('styles/media-detail.css');
// The tests below match boundaries with plain newlines; a Windows checkout
// (core.autocrlf=true) must not turn every body regex into a failure.
const mediaDetail = read('views/media-detail.js').replace(/\r\n/g, '\n');

// Body of a top-level function declaration, without its closing brace line.
function functionBody(name, source = board) {
  const match = new RegExp(`\\nfunction ${name}\\([^)]*\\) \\{([\\s\\S]*?)\\n\\}`).exec(source);
  assert.ok(match, `no body for ${name}`);
  return match[1];
}

// Body of a function nested in detailBody (declared two spaces in).
function nestedBody(name, source = board) {
  const match = new RegExp(`\\n  function ${name}\\([^)]*\\) \\{([\\s\\S]*?)\\n  \\}`).exec(source);
  assert.ok(match, `no body for ${name}`);
  return match[1];
}

// Declarations of the rule whose selector list contains `selector`.
function ruleOf(css, selector) {
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

test('the board imports and creates the media detail controller once', () => {
  assert.match(board, /import \{ createMediaDetail \} from '\.\/media-detail\.js'/);
  assert.equal((board.match(/createMediaDetail\(/g) || []).length, 1, 'one controller for the board');
  assert.match(board, /grid: \(\) => document\.getElementById\('grid'\)/, 'the controller watches the real grid');
  assert.match(board, /view: \(\) => document\.getElementById\('view'\)/);
  assert.match(board, /onOpen: \(id\) => \{\s*state\.detailId = id;/, 'the open media feeds state.detailId');
  assert.match(board, /onClose: \(id\) => \{[\s\S]{0,700}state\.detailId = null;/, 'a close clears state.detailId');
});

test('a card opens the window and refreshes only its information column', () => {
  assert.match(board, /mediaDetail\.openDetail\(cards\.get\(id\)\.node\)/, 'a click opens the window');
  assert.match(board, /mediaDetail\.hoverCard\(event\.currentTarget\)/, 'the cards feed the hover preview');
  assert.match(board, /mediaDetail\.leaveCard\(event\.currentTarget\)/);
  assert.match(board, /mediaDetail\.prefetchCard\(event\.currentTarget\)/, 'the cards warm their media first');
  assert.match(board, /mediaDetail\.cancelPrefetch\(event\.currentTarget\)/);
  assert.match(board, /mediaDetail\.refreshDetail\(\)/, 'a board refresh rebuilds the information column');
  assert.match(board, /mediaDetail\.closeDetail\(\)/, 'a media that left the board closes the window');
  assert.match(board, /mediaDetail\.setVolume\(mediaLevel\(state\.activeGuildId\)\)/, 'the sound setting reaches the window player');
});

test('a card warms its media before the hover preview delay', () => {
  const enter = board.indexOf('onMouseenter: (event) => {');
  const leave = board.indexOf('onMouseleave: (event) => {');
  const prefetch = board.indexOf('mediaDetail.prefetchCard(event.currentTarget)', enter);
  const hover = board.indexOf('mediaDetail.hoverCard(event.currentTarget)', enter);
  const cancel = board.indexOf('mediaDetail.cancelPrefetch(event.currentTarget)', leave);
  const hide = board.indexOf('mediaDetail.leaveCard(event.currentTarget)', leave);

  assert.ok(enter >= 0 && leave > enter, 'both handlers are objects');
  assert.ok(prefetch > enter, 'entering a card starts the prefetch');
  assert.ok(hover > prefetch, 'the prefetch starts before the preview is scheduled');
  assert.ok(cancel > leave && hide > cancel, 'leaving drops a pending prefetch before hiding');

  assert.match(mediaDetail, /import \{ createMediaPrefetcher \} from '\.\.\/core\/media-prefetch\.js'/);
  assert.match(mediaDetail, /prefetcher\.schedule\(id, desc\)/);
  assert.match(mediaDetail, /prefetcher\.cancel\(id\)/);
  assert.match(mediaDetail, /prefetcher\.take\(\{ kind, url, expiresAt: desc\.expiresAt \}\)/);
  assert.match(mediaDetail, /buildStage\(desc, carriedVisual, carriedPlayer, prefetched\)/);
  assert.match(functionBody('describeItem'), /const url = previewUrl\(item\);/);
  assert.match(functionBody('describeItem'), /expiresAt: urlExpiresAt\(url\),/);

  // Warming the bytes is independent of the hover preview setting: the window
  // must be instant even when the setting is off.
  const prefetchBody = /\n  function prefetchCard\(card\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(prefetchBody, 'prefetchCard is found');
  assert.doesNotMatch(prefetchBody[1], /hoverEnabled/);
  assert.match(prefetchBody[1], /if \(!card \|\| detailPage\) \{/);
});

test('the preview and the window reuse the prefetched element', () => {
  const build = /\n  function build\(card, desc\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(build, 'build is found');
  assert.match(build[1], /const prefetched = prefetcher\.take\(\{ kind, url, expiresAt: desc\.expiresAt \}\);/);
  assert.match(build[1], /prefetched && prefetched\.kind === 'video'/);
  assert.match(build[1], /prefetched && prefetched\.kind === 'audio'/);
  assert.match(build[1], /prefetched && prefetched\.kind === 'image'/);

  const open = mediaDetail.indexOf('function openDetail(card)');
  const take = mediaDetail.indexOf('const prefetched = fromPreview ? null : prefetcher.take(', open);
  const stage = mediaDetail.indexOf('buildStage(desc, carriedVisual, carriedPlayer, prefetched)', open);
  assert.ok(open >= 0 && take > open, 'the window takes the prefetched element');
  assert.ok(stage > take, 'the stage receives it');

  // A warm hover opens the preview before the debounce: the pending prefetch
  // must be dropped, or the same bytes would be requested twice.
  const previewOpen = /\n  function open\(card\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(previewOpen, 'open is found');
  const cancelAt = previewOpen[1].indexOf('prefetcher.cancel(id);');
  const buildAt = previewOpen[1].indexOf('const el = build(card, desc);');
  assert.ok(cancelAt >= 0 && buildAt > cancelAt, 'the preview cancels the pending prefetch before building');

  const stageBody = /\n  function buildStage\(desc, carriedVisual, carriedPlayer, prefetched\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(stageBody, 'buildStage is found');
  assert.match(stageBody[1], /const prefetchedVideo = prefetched && prefetched\.kind === 'video'/);
  assert.match(stageBody[1], /else if \(prefetched && prefetched\.kind === 'audio'\)/);
  assert.match(stageBody[1], /else if \(prefetched && prefetched\.kind === 'image'\)/);

  // A detached prefetched element has no rect to fly from: only the preview's
  // own media rides with the page.
  const fly = mediaDetail.indexOf('const flyVisual = fromPreview', open);
  assert.ok(fly > open, 'the flight media is chosen after the prefetch handoff');
  assert.match(mediaDetail.slice(fly, fly + 160), /: null;/);
});

test('no trace of the removed drawer remains in the board', () => {
  for (const word of ['selectedId', 'detailPinned', 'peekId', 'placeDrawer', 'replaceDrawer', 'cardStill', 'prunePreviews', 'buildPreview', 'detailPreview', 'renderDetail', 'scheduleHoverClose', 'STILL_MAX_WIDTH']) {
    assert.ok(!board.includes(word), `board.js must not keep ${word}`);
  }
  assert.doesNotMatch(board, /\bhoverDetail\(/, 'the drawer hover timer is gone');
  assert.doesNotMatch(board, /class: `detail/);
  assert.doesNotMatch(board, /\baside\b/);
  assert.doesNotMatch(board, /drawerOpen/);
});

test('the drawer rules are gone from board.css, its shared controls stayed', () => {
  assert.doesNotMatch(boardCss, /^\.detail\b/m);
  assert.doesNotMatch(boardCss, /^\.preview\b/m);
  assert.doesNotMatch(boardCss, /\.is-selected\b/);
  for (const kept of ['.shortcut-cell', '.shortcut-key', '.inline-edit', '.inline-input']) {
    assert.ok(boardCss.includes(kept), `${kept} is still used by the window`);
  }
});

test('the information dialog is gone, its controls live in the detail column', () => {
  assert.doesNotMatch(board, /openInfoDialog/, 'the dialog is removed');
  assert.doesNotMatch(board, /infoFacts/, 'its fact builder is removed');
  assert.ok(!board.includes('Infos…'), 'the context menu entry is removed');
  const detail = functionBody('detailBody');
  for (const needle of ['parseDisplayDuration', 'uploadArtwork', 'artworkOnOverlay', 'hp-page-artwork']) {
    assert.ok(detail.includes(needle), `detailBody must own ${needle}`);
  }
});

test('the favorite toggle lives in the window top bar, filled by describe(id).tools()', () => {
  // The validated layout puts the star on the right of the top bar, next to
  // « Détails du média », not under the byline.
  const describe = functionBody('describeItem');
  assert.match(describe, /tools:\s*\(\)\s*=>\s*\(?canFavorite\(\)\s*\?\s*favoriteTool\(item\)\s*:\s*null\)?/);
  assert.match(functionBody('favoriteTool'), /toggleFavorite\(item, event\.currentTarget\)/);
  assert.doesNotMatch(functionBody('detailBody'), /fav-toggle/, 'the byline does not own the favorite');
  assert.doesNotMatch(functionBody('detailBody'), /favorite/, 'detailBody no longer builds the favorite');

  // The module renders that node in `.hp-page-tools` and rebuilds it with the
  // information column, so a board refresh paints the new favorite state.
  assert.match(mediaDetail, /describe\(id\)\.tools\(\)/, 'the contract documents the tools() option');
  assert.match(mediaDetail, /class: 'hp-page-tools'/);
  assert.match(mediaDetail, /function detailTools\(desc\) \{[\s\S]*?desc\.tools\(\)/);
  const refreshAt = mediaDetail.indexOf('function refreshDetail()');
  assert.ok(refreshAt >= 0, 'refreshDetail exists');
  assert.ok(
    mediaDetail.indexOf('detailTools(desc)', refreshAt) > refreshAt,
    'refreshDetail rebuilds the tools too',
  );
  assert.match(ruleOf(detailCss, '.hp-page-tools'), /margin-left:\s*auto/);
});

test('the head is a one-line kicker with author and date, then the title', () => {
  const detail = functionBody('detailBody');
  const kicker = detail.indexOf("class: 'hp-page-kicker'");
  const title = detail.indexOf("inlineField(item, 'title')");
  assert.ok(kicker >= 0 && title > kicker, 'head order: kicker, title');
  assert.doesNotMatch(detail, /hp-page-byline/, 'the separate byline is gone');

  // Format, author and date share the kicker line, which is cut with an
  // ellipsis instead of wrapping; the whole text is still available as a title.
  assert.match(detail, /const author = authorName\(item\);/);
  assert.match(detail, /' · Ajouté par '/);
  assert.match(detail, /h\('strong', null, author\)/);
  assert.match(detail, /added \? ` le \$\{added\}` : null/);
  const kickerRule = ruleOf(detailCss, '.hp-page-kicker');
  assert.match(kickerRule, /white-space:\s*nowrap/);
  assert.match(kickerRule, /overflow:\s*hidden/);
  assert.match(kickerRule, /text-overflow:\s*ellipsis/);
  assert.doesNotMatch(detailCss, /\.hp-page-byline/, 'no rule left for the removed byline');

  // The title control outdents its frame by the column's clip room (8px), like
  // the caption, so the hover frame's left border stays visible; 8px of
  // padding keeps the heading text and the pencil off the frame's edges.
  const heading = ruleOf(detailCss, '.hp-page-head .inline-edit.is-title');
  assert.match(heading, /width:\s*calc\(100% \+ 8px\)/);
  assert.match(heading, /margin-left:\s*-8px/);
  assert.match(heading, /padding-left:\s*8px/);
  assert.match(heading, /padding-right:\s*8px/);
  const titleRule = ruleOf(detailCss, ".hp-page[data-layout='atelier'] .hp-page-title");
  assert.match(titleRule, /font-size:\s*28px/);
});

test('a long title and a long caption are cut with an ellipsis', () => {
  const clamp = (declarations) => {
    assert.match(declarations, /display:\s*-webkit-box/);
    assert.match(declarations, /overflow:\s*hidden/);
    assert.match(declarations, /-webkit-box-orient:\s*vertical/);
  };

  const title = ruleOf(detailCss, 'h2.hp-page-title');
  clamp(title);
  assert.match(title, /-webkit-line-clamp:\s*2/);
  const editableTitle = ruleOf(detailCss, '.hp-page-head .inline-edit.is-title .inline-text');
  clamp(editableTitle);
  assert.match(editableTitle, /-webkit-line-clamp:\s*2/);

  for (const selector of [
    'p.hp-page-legend',
    '.hp-page-legend-block .inline-edit.is-message .inline-text',
  ]) {
    const caption = ruleOf(detailCss, selector);
    clamp(caption);
    assert.match(caption, /-webkit-line-clamp:\s*5/);
  }

  // The fields keep their own inner scroll while editing: the box clamp only
  // targets the read-only heading and caption.
  const titleClamp = /h2\.hp-page-title,[^{]*\{[^}]*\}/.exec(detailCss)?.[0] ?? '';
  assert.doesNotMatch(titleClamp, /\.inline-input/);
  const captionClamp = /p\.hp-page-legend,[^{]*\{[^}]*\}/.exec(detailCss)?.[0] ?? '';
  assert.doesNotMatch(captionClamp, /textarea/);
});

test('the facts follow the validated order', () => {
  const detail = functionBody('detailBody');
  const labels = [
    "'Format'",
    "'Taille'",
    "'Dimensions'",
    "'Échelle'",
    "'Source'",
    "'Raccourci'",
    "'Durée d’affichage'",
  ];
  const positions = labels.map((label) => detail.indexOf(label));
  positions.forEach((position, index) => {
    assert.ok(position >= 0, `${labels[index]} is a fact row`);
  });
  for (let index = 1; index < positions.length; index += 1) {
    assert.ok(positions[index] > positions[index - 1], `${labels[index]} comes after ${labels[index - 1]}`);
  }
  // Format is the kind and the duration, not the old combined line.
  assert.match(detail, /fact\('Format', \[kindLabel, duration\]\.filter\(Boolean\)\.join\(' · '\)\)/);
  // The scale is « N % » and only gets a row when one was chosen.
  assert.match(detail, /const scale = item\.displayScalePercent \? `\$\{item\.displayScalePercent\} %` : null/);
  assert.match(detail, /if \(scale\) \{\s*fact\('Échelle', scale\);\s*\}/);
});

test('the legend block sits above the pinned facts and edits the local draft', () => {
  const detail = functionBody('detailBody');
  const legend = detail.indexOf("class: 'hp-page-legend-block'");
  const artwork = detail.indexOf('isAudio && item.canEdit ? artworkSection() : null');
  const facts = detail.indexOf("class: 'hp-page-facts'");
  assert.ok(legend >= 0 && artwork > legend, 'the audio picture follows the legend block');
  assert.ok(facts > artwork, 'the pinned facts come after the scrolling content');
  assert.match(detail, /'Légende'/);
  assert.match(detail, /inlineField\(item, 'message', canPlay\)/, 'the caption control is available to playable media');
  assert.doesNotMatch(detail, /item\.message/, 'the stored legacy caption never reaches the window');
  assert.match(board, /empty: 'Ajouter une légende'/, 'an empty local draft invites to write one');
});

test('the actions put the play button above a two-column grid of secondary actions', () => {
  const detail = functionBody('detailBody');
  assert.match(detail, /class: 'hp-page-actionzone'/);
  assert.match(detail, /class: 'hp-page-actions'/);
  assert.match(detail, /class: 'hp-page-secondary'/);

  const secondary = ruleOf(detailCss, '.hp-page-secondary');
  assert.match(secondary, /display:\s*grid/);
  assert.match(secondary, /grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/);
  assert.match(secondary, /border-top:\s*1px\s+solid\s+var\(--line\)/);
  const button = ruleOf(detailCss, '.hp-page-secondary .btn');
  assert.match(button, /justify-content:\s*flex-start/);
  assert.match(button, /text-overflow:\s*ellipsis/);
});

test('the facts and the actions are pinned in a footer at the bottom of the column', () => {
  // The footer is the last block of the column and holds the facts, then the
  // actions: a page too tall for the window can never hide an action.
  const detail = functionBody('detailBody');
  const footer = detail.indexOf("class: 'hp-page-footer'");
  const facts = detail.indexOf("class: 'hp-page-facts'");
  const actions = detail.indexOf("class: 'hp-page-actionzone'");
  assert.ok(footer >= 0 && facts > footer, 'the footer wraps the facts');
  assert.ok(actions > facts, 'the actions come after the facts in the footer');

  const rule = ruleOf(detailCss, '.hp-page-footer');
  assert.match(rule, /position:\s*sticky/);
  assert.match(rule, /bottom:\s*0/);
  assert.match(rule, /flex:\s*none/, 'the pinned facts and actions never shrink');
  assert.match(rule, /margin-top:\s*auto/, 'short columns push the footer down');
  assert.match(rule, /background:\s*var\(--slate-800\)/, 'scrolled content stays hidden behind it');

  // At the minimum window height the footer is taller than the room left
  // under the media: sticking it would cover the legend. It becomes a normal
  // block there, so it can never overlap the caption.
  const short = detailCss.slice(
    detailCss.indexOf('@media (max-height: 780px)'),
    detailCss.indexOf('@media (max-width: 900px)'),
  );
  assert.notEqual(short, '', 'a short-height layout exists');
  assert.match(ruleOf(short, '.hp-page-footer'), /position:\s*static/, 'the footer unpins when the height is short');
  const stacked = detailCss.slice(
    detailCss.indexOf('@media (max-width: 900px)'),
    detailCss.indexOf('@media (max-width: 620px)'),
  );
  assert.match(ruleOf(stacked, '.hp-page-footer'), /position:\s*static/, 'the stacked layout never pins over the caption');
});

test('the display time draft survives a refresh of the information column', () => {
  // A refresh rebuilds the whole column through detailBody: what the user typed
  // in « Durée d’affichage » (with the caret) must come back, like the inline
  // title and caption.
  assert.match(board, /let displayDurationDraft = null;/, 'one module-level draft');
  const detail = functionBody('detailBody');
  assert.match(
    detail,
    /if \(displayDurationDraft && displayDurationDraft\.itemId !== item\.id\) \{\s*displayDurationDraft = null;\s*\}/,
    'a draft typed for another media is dropped',
  );

  const field = nestedBody('displayDurationField');
  // The rebuilt field reads the draft and takes the focus back without
  // scrolling the column.
  assert.match(field, /const draft = displayDurationDraft && displayDurationDraft\.itemId === item\.id \? displayDurationDraft : null;/);
  assert.match(field, /value: draft \? draft\.value : item\.displayDurationSec \?\? ''/);
  assert.match(field, /input\.focus\(\{ preventScroll: true \}\)/);
  assert.match(field, /input\.setSelectionRange\(draft\.selectionStart, draft\.selectionEnd\)/);

  // Typing and focus both record the value and the caret in the module draft.
  assert.match(
    field,
    /displayDurationDraft = \{\s*itemId: item\.id,\s*value: input\.value,\s*selectionStart: input\.selectionStart,\s*selectionEnd: input\.selectionEnd,\s*\}/,
  );
  assert.match(field, /addEventListener\('input', remember\)/);
  assert.match(field, /addEventListener\('focus', remember\)/);

  // A re-render removes the field without the user leaving it: the same
  // isConnected guard as inlineInput means only a real blur saves and clears.
  assert.match(
    field,
    /input\.addEventListener\('blur', \(\) => \{\s*setTimeout\(\(\) => \{\s*if \(input\.isConnected\) \{/,
  );
  assert.match(field, /const forget = \(\) => \{[\s\S]{0,200}displayDurationDraft = null;/);
  assert.match(field, /event\.key === 'Escape'[\s\S]{0,200}forget\(\)/);
});

test('the information column scrolls under its footer without a scrollbar', () => {
  const page = ruleOf(detailCss, '.hp-page');
  assert.match(page, /overflow:\s*hidden/, 'the window itself does not scroll');
  const info = ruleOf(detailCss, '.hp-page-info');
  assert.match(info, /overflow-y:\s*auto/, 'the column still reaches its own content');
  assert.match(info, /scrollbar-width:\s*none/, 'its scrollbar is never drawn');
  assert.doesNotMatch(info, /scrollbar-gutter/, 'no gutter is reserved for a hidden bar');
  assert.match(ruleOf(detailCss, '.hp-page-info::-webkit-scrollbar'), /display:\s*none/);

  // The column is capped by the stage height, so a long title or caption
  // scrolls inside it instead of pushing the window down.
  assert.match(page, /--hp-page-stage-height:\s*clamp\(380px,\s*58vh,\s*620px\)/);
  assert.match(ruleOf(detailCss, '.hp-page-stage'), /height:\s*var\(--hp-page-stage-height\)/);
  assert.match(info, /max-height:\s*var\(--hp-page-stage-height\)/);
  const atelier = ruleOf(detailCss, ".hp-page[data-layout='atelier']");
  assert.match(atelier, /--hp-page-stage-height:\s*clamp\(430px,\s*calc\(100vh - 300px\),\s*680px\)/);

  // Below the window's minimum height, the whole page is the scroll container
  // (still without a bar): no nested column scroll can hide the actions, and
  // the caption at the top of the column is never under the footer.
  const short = detailCss.slice(
    detailCss.indexOf('@media (max-height: 780px)'),
    detailCss.indexOf('@media (max-width: 1100px)'),
  );
  assert.match(ruleOf(short, '.hp-page'), /overflow-y:\s*auto/);
  assert.match(ruleOf(short, '.hp-page::-webkit-scrollbar'), /display:\s*none/);
  assert.match(ruleOf(short, '.hp-page-info'), /max-height:\s*none/);
  assert.match(ruleOf(short, '.hp-page-footer'), /position:\s*static/);

  // The stacked fallback scrolls the window, still without drawing a bar. It
  // starts below the overlay window's 960px minimum, so that minimum keeps
  // the caption beside the stage rather than below it.
  const stacked = detailCss.slice(
    detailCss.indexOf('@media (max-width: 900px)'),
    detailCss.indexOf('@media (max-width: 620px)'),
  );
  assert.match(stacked, /scrollbar-width:\s*none/);
  assert.match(ruleOf(stacked, '.hp-page::-webkit-scrollbar'), /display:\s*none/);
  assert.ok(detailCss.indexOf('@media (max-width: 900px)') >= 0 && 900 < 960);
});

// The column reserves the left outdent of its inline fields inside its
// scrollport, and pulls itself left by the same amount so the text stays put.
function clipRoom(source) {
  const info = ruleOf(source, '.hp-page-info');
  return {
    padding: Number(/padding-left:\s*(\d+)px/.exec(info)?.[1]),
    margin: Number(/margin-left:\s*(-?\d+)px/.exec(info)?.[1]),
  };
}

function atelierLeftPadding(source) {
  const rule = ruleOf(source, ".hp-page[data-layout='atelier'] .hp-page-info");
  const parts = /padding:\s*([^;]+);/.exec(rule)?.[1].trim().split(/\s+/);
  return parts && parts.length === 4 ? Number.parseFloat(parts[3]) : Number.NaN;
}

test('the information column keeps the inline fields outdents inside its clip', () => {
  // The title and caption frames hang 8px left of the column so the frame and
  // its left border stay visible while the text keeps the column's gutter. The
  // column is the scroll container and clips at its padding edge: with no room
  // reserved inside it, the caption lost its whole left border and padding,
  // and the title its border. The negative margin compensates the padding so
  // the column's text does not move right by 8px.
  const title = ruleOf(detailCss, '.hp-page-head .inline-edit.is-title');
  const caption = ruleOf(detailCss, '.hp-page-legend-block .inline-edit');
  const titleOutdent = Math.abs(Number(/margin-left:\s*(-?\d+)px/.exec(title)?.[1]));
  const captionOutdent = Math.abs(Number(/margin:\s*0 0 0 (-?\d+)px/.exec(caption)?.[1]));
  const outdent = Math.max(titleOutdent, captionOutdent);
  assert.ok(outdent >= 8, 'the widest inline frame hangs 8px outside the column');

  const room = clipRoom(detailCss);
  assert.ok(room.padding >= outdent, `the scrollport reserves ${room.padding}px for a ${outdent}px outdent`);
  assert.equal(room.margin, -room.padding, 'the clip room is compensated, so the text does not move');

  // Atelier and the stacked layout keep the room; a padding shorthand that
  // zeroes its left side would shift the whole column.
  assert.ok(atelierLeftPadding(detailCss) >= outdent, 'the atelier layout keeps the clip room');
  const stacked = detailCss.slice(
    detailCss.indexOf('@media (max-width: 900px)'),
    detailCss.indexOf('@media (max-width: 620px)'),
  );
  assert.ok(atelierLeftPadding(stacked) >= outdent, 'the stacked layout keeps the clip room');
});

test('the title pencil centres on the heading line, not on the box top', () => {
  // The base 2px icon offset is tuned for the caption's 14px text. The title is
  // a 26–28px heading with a 1.22 line box, so its 14px pencil used to sit
  // about 8px above the first line's centre.
  const title = ruleOf(detailCss, '.hp-page-title');
  const lineHeight = Number(/line-height:\s*([\d.]+)/.exec(title)?.[1]);
  const icon = ruleOf(boardCss, '.inline-edit .icon');
  const size = Number(/width:\s*(\d+)px/.exec(icon)?.[1]);
  const pencil = ruleOf(detailCss, '.hp-page-head .inline-edit.is-title .icon');
  const offset = /margin-top:\s*calc\(\(([\d.]+)em\s*-\s*(\d+)px\)\s*\/\s*2\)/.exec(pencil);

  assert.ok(offset, 'the title pencil gets a top offset from the heading line');
  assert.equal(Number(offset[1]), lineHeight, 'the line box is the title line-height');
  assert.equal(Number(offset[2]), size, 'the offset subtracts the icon height');
  assert.match(ruleOf(boardCss, '.inline-edit'), /align-items:\s*flex-start/, 'a wrapped title keeps the pencil on line one');
});

test('the inline fields at the column edge keep their focus ring visible', () => {
  // The title and caption frames start on the scrollport's left clip edge. The
  // default focus ring (base.css) is 2px wide and sits 2px outside the frame,
  // so it disappears on that side. Either the column reserves those 4px, or
  // the ring is moved inside by its outline-offset.
  const ring = ruleOf(baseCss, ':focus-visible');
  const width = Number(/outline:\s*(\d+)px/.exec(ring)?.[1]);
  const defaultOffset = Number(/outline-offset:\s*(-?\d+)px/.exec(ring)?.[1]);
  const room = clipRoom(detailCss);
  const title = ruleOf(detailCss, '.hp-page-head .inline-edit.is-title');
  const caption = ruleOf(detailCss, '.hp-page-legend-block .inline-edit');
  const titleOutdent = Math.abs(Number(/margin-left:\s*(-?\d+)px/.exec(title)?.[1]));
  const captionOutdent = Math.abs(Number(/margin:\s*0 0 0 (-?\d+)px/.exec(caption)?.[1]));
  const spare = room.padding - Math.max(titleOutdent, captionOutdent);
  const selectors = [
    '.hp-page-head .inline-edit.is-title:focus-visible',
    '.hp-page-head .inline-input.is-title:focus-visible',
    '.hp-page-legend-block .inline-edit:focus-visible',
    '.hp-page-legend-block .inline-input:focus-visible',
  ];

  for (const selector of selectors) {
    const focus = ruleOf(detailCss, selector);
    const ownOffset = /outline-offset:\s*(-?\d+)px/.exec(focus)?.[1];
    const offset = ownOffset === undefined ? defaultOffset : Number(ownOffset);
    const outward = Math.max(0, width + offset);
    assert.ok(spare >= outward, `${selector} keeps its ${outward}px ring inside (${spare}px spare, ${offset}px offset)`);
  }
});

// The ring base.css draws around a focused control: 2px wide, 2px outside the
// box, so it reaches 4px out unless the rule moves it inside.
function ringOutward(baseSource = baseCss) {
  const ring = ruleOf(baseSource, ':focus-visible');
  const width = Number(/outline:\s*(\d+)px/.exec(ring)?.[1]);
  const offset = Number(/outline-offset:\s*(-?\d+)px/.exec(ring)?.[1]);
  return Math.max(0, width + offset);
}

// Right padding of a rule, read from the longhand or from the shorthand.
function paddingRightOf(rule) {
  const longhand = /padding-right:\s*(-?\d+)px/.exec(rule);
  if (longhand) {
    return Number(longhand[1]);
  }
  const parts = /padding:\s*([^;]+);/.exec(rule)?.[1].trim().split(/\s+/);
  if (!parts) {
    return Number.NaN;
  }
  // One value, or the second of a two-, three- and four-value shorthand.
  return Number.parseFloat(parts.length === 1 ? parts[0] : parts[1]);
}

test('the action buttons keep their focus ring clear of the column clip edge', () => {
  // The column is a scrollport: it clips at its padding box. The play button
  // and the secondary actions span the column, so their ring lost its right
  // edge, and the last row lost its bottom edge under the sticky footer. The
  // room comes from the column's own padding, compensated by a negative margin
  // so neither the buttons nor the separators above them move.
  const outward = ringOutward();
  const info = ruleOf(detailCss, '.hp-page-info');
  const right = Number(/padding-right:\s*(\d+)px/.exec(info)?.[1]);
  const marginRight = Number(/margin-right:\s*(-?\d+)px/.exec(info)?.[1]);
  assert.ok(right >= outward, `the scrollport reserves ${right}px for a ${outward}px ring`);
  assert.equal(marginRight, -right, 'the right room is compensated, so the buttons keep their width');

  // The sticky footer owns the bottom edge of the column: its own padding is
  // inside the clip, so the last row's ring stays whole.
  const footer = ruleOf(detailCss, '.hp-page-footer');
  const bottom = Number(/padding-bottom:\s*(\d+)px/.exec(footer)?.[1]);
  assert.ok(bottom >= outward, `the footer reserves ${bottom}px under its last row`);

  // Atelier and the stacked layout keep the room: the negative margin stays in
  // every layout, so a zeroed right padding would push the column out of it.
  const atelier = ruleOf(detailCss, ".hp-page[data-layout='atelier'] .hp-page-info");
  assert.ok(paddingRightOf(atelier) >= right, 'the atelier layout keeps the right room');
  const stacked = detailCss.slice(
    detailCss.indexOf('@media (max-width: 900px)'),
    detailCss.indexOf('@media (max-width: 620px)'),
  );
  assert.ok(
    paddingRightOf(ruleOf(stacked, ".hp-page[data-layout='atelier'] .hp-page-info")) >= right,
    'the stacked layout keeps the right room',
  );
});

test('the preview tooltip names the action: « Lecture » or « Pause »', () => {
  const attach = /\n  function attachPlayer\(el, media, player, card, desc\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(attach, 'attachPlayer is found');
  assert.match(attach[1], /media\.title = 'Lecture';/, 'a paused preview offers « Lecture »');
  assert.match(attach[1], /player\.addEventListener\('play'[\s\S]*?media\.title = 'Pause';/);
  assert.match(attach[1], /player\.addEventListener\('pause'[\s\S]*?media\.title = 'Lecture';/);
  assert.doesNotMatch(mediaDetail, /Cliquer pour mettre en pause|Cliquer pour reprendre la lecture/);
  // The accessible name stays descriptive even though the tooltip is terse.
  assert.match(mediaDetail, /Mettre l’aperçu en pause/);
  assert.match(mediaDetail, /Lire l’aperçu/);
});

test('the window covers the view, not the whole window', () => {
  const detailCss = read('styles/media-detail.css');
  const shellRule = /\.hp-page-shell \{[^}]*\}/.exec(detailCss)?.[0] || '';
  assert.match(shellRule, /position: fixed/, 'the shell stays fixed');
  assert.doesNotMatch(shellRule, /inset: 0/, 'the shell no longer pins itself to the window');
  assert.match(mediaDetail, /viewEl\.getBoundingClientRect\(\)/, 'the shell measures the view');
  assert.match(mediaDetail, /window\.addEventListener\('resize', onResize\)/);
  assert.match(mediaDetail, /new ResizeObserver\(onResize\)/);
  assert.match(mediaDetail, /setVolume\(volume\)/);
});

test('a single click toggles the preview at once; a double click restores then opens', () => {
  // The play/pause of a single click used to wait 300 ms so a double click
  // could open the window. It now toggles straight away: a double click puts
  // playback back as it was before its first click, then opens the window.
  assert.ok(!mediaDetail.includes('mediaClickTimer'), 'no delayed toggle timer');
  assert.ok(!mediaDetail.includes('_cancelPendingClick'), 'no pending-click cancellation');
  assert.ok(!mediaDetail.includes('}, 300);'), 'no 300 ms toggle');
  assert.doesNotMatch(mediaDetail, /300\s*ms/, 'no comment promises a 300 ms wait');

  const click = /\n    media\.addEventListener\('click', \(event\) => \{([\s\S]*?)\n    \}\);/.exec(mediaDetail);
  assert.ok(click, 'the media click listener exists');
  assert.match(
    click[1],
    /lastMediaClickAt = now;\s*clickStartPaused = player\.paused;\s*togglePlayback\(\);/,
    'a single click saves the state and toggles at once',
  );
  assert.doesNotMatch(click[1], /setTimeout/, 'a single click waits for nothing');
  assert.match(click[1], /restoreBeforeOpen\(\);\s*openDetail\(card\);/, 'the second click restores before opening');

  const pointerdown = /\n    media\.addEventListener\('pointerdown', \(event\) => \{([\s\S]*?)\n    \}\);/.exec(mediaDetail);
  assert.ok(pointerdown, 'the media pointerdown listener exists');
  assert.match(pointerdown[1], /restoreBeforeOpen\(\);\s*openDetail\(card\);/, 'the second press restores before opening');

  const dblclick = /\n    media\.addEventListener\('dblclick', \(event\) => \{([\s\S]*?)\n    \}\);/.exec(mediaDetail);
  assert.ok(dblclick, 'the media dblclick listener exists');
  assert.match(dblclick[1], /restoreBeforeOpen\(\);\s*openDetail\(card\);/, 'the dblclick restores before opening');

  // performance.now() starts near zero: a 0 sentinel makes the first press of
  // a freshly loaded page look like the second press of a double click.
  assert.match(mediaDetail, /let lastMediaClickAt = Number\.NEGATIVE_INFINITY;/);
  assert.match(mediaDetail, /let lastMediaPointerAt = Number\.NEGATIVE_INFINITY;/);
  assert.match(mediaDetail, /let lastPointerAt = Number\.NEGATIVE_INFINITY;/);
  assert.match(mediaDetail, /let lastBodyClickAt = Number\.NEGATIVE_INFINITY;/);
  assert.match(mediaDetail, /let lastBodyPointerAt = Number\.NEGATIVE_INFINITY;/);
});

test('the hover delay waits once, then follows the pointer', () => {
  // The first preview waits long enough to show intent; while one is open, or
  // just after one closed, the next opens fast. The decision is the pure
  // showDelay helper; the controller only owns its two facts.
  const imported = /import \{([\s\S]*?)\} from '\.\.\/core\/detail-motion\.js'/.exec(mediaDetail);
  assert.ok(imported, 'media-detail imports its motion helpers');
  assert.match(imported[1], /\bHOVER_TIMING\b/);
  assert.match(imported[1], /\bshowDelay\b/);

  assert.doesNotMatch(mediaDetail, /\bSHOW_DELAY\b/, 'the fixed show delay is gone');
  assert.doesNotMatch(mediaDetail, /\bHIDE_DELAY\b/, 'the fixed hide delay is gone');
  assert.match(mediaDetail, /let lastPreviewClosedAt = Number\.NEGATIVE_INFINITY;/);

  const scheduleShow = /\n  function scheduleShow\(card\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(scheduleShow, 'no scheduleShow body');
  assert.match(
    scheduleShow[1],
    /showDelay\(\{\s*previewOpen: Boolean\(previewEl\),\s*now: performance\.now\(\),\s*lastClosedAt: lastPreviewClosedAt,\s*\}\)/,
    'scheduleShow asks showDelay with the controller state',
  );

  const close = /\n  function close\(immediate\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(close, 'no close body');
  assert.match(close[1], /lastPreviewClosedAt = performance\.now\(\);/, 'close records when a preview closed');

  const scheduleHide = /\n  function scheduleHide\(\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(scheduleHide, 'no scheduleHide body');
  assert.match(scheduleHide[1], /HOVER_TIMING\.hide/);
});

test('closing the window cancels an in-flight shortcut capture', () => {
  // The capture listens on the window's own control: closing the window by
  // any route must stop it, or the next key press would save a shortcut with
  // no visible control.
  assert.match(board, /onClose: \(id\) => \{[\s\S]{0,700}state\.capturing = false;/);
  assert.match(board, /export function closeDetail\(\) \{[\s\S]{0,400}state\.capturing = false;/);
});

test('the app stylesheet imports the window styles after the board styles', () => {
  const app = read('app.css');
  const boardAt = app.indexOf("@import url('./styles/board.css');");
  const detailAt = app.indexOf("@import url('./styles/media-detail.css');");
  assert.notEqual(boardAt, -1, 'board.css is imported');
  assert.notEqual(detailAt, -1, 'media-detail.css is imported');
  assert.ok(boardAt < detailAt, 'the window styles come after the board styles');
});

test('regression: leaving the card for the preview that just covered it keeps the preview', () => {
  // When the preview grows under a still pointer, Chromium fires the
  // preview's pointerenter (which cancels a pending hide) before the card's
  // mouseleave: a hide scheduled by that late mouseleave closed the preview
  // 60 ms after it opened. Leaving the card while the pointer is over the
  // preview must schedule nothing.
  const leave = /\n    leaveCard\(\) \{([\s\S]*?)\n    \},/.exec(mediaDetail);
  assert.ok(leave, 'no leaveCard in the controller');
  assert.match(leave[1], /previewEl && previewEl\.matches\(':hover'\)\) \{\s*return;/);
  assert.ok(leave[1].indexOf(":hover')") < leave[1].indexOf('scheduleHide()'), 'checked before scheduling the hide');
});

test('the grid says whether the hover preview is on, at render and when the setting changes', () => {
  assert.match(board, /id: 'grid'/);
  assert.match(board, /hoverPreview \? ' is-hover-preview' : ''/, 'the rendered grid carries the setting');
  const apply = functionBody('applyHoverPreview');
  assert.match(apply, /getElementById\('grid'\)\?\.classList\.toggle\('is-hover-preview', hoverPreview\)/);
});

test('the card hover lives in one section and only plays without the hover preview', () => {
  // Two distinct hover behaviours: the card's own lift, zoom and revealed
  // buttons, or the preview. Never both at once, and the card's one is
  // readable in a single place.
  const start = boardCss.indexOf('/* --- Card hover ---');
  const end = boardCss.indexOf('/* --- End of card hover ---');
  assert.ok(start >= 0 && end > start, 'board.css has a « Card hover » section');
  const outside = boardCss.slice(0, start) + boardCss.slice(end);
  assert.doesNotMatch(outside, /\.card[\w.-]*:hover/, 'no card hover rule outside the section');

  const own = '.grid:not(.is-hover-preview:not(.is-selecting)) .card:hover';
  const selectors = [];
  const rule = /([^{}]+)\{[^{}]*\}/g;
  let match;
  while ((match = rule.exec(boardCss.slice(start, end))) !== null) {
    selectors.push(match[1].replace(/\/\*[\s\S]*?\*\//g, '').trim());
  }
  assert.deepEqual(selectors, [own, `${own} .thumb img`, `${own} .fav`, `${own} .play`]);
});

test('regression: under reduced motion the preview still grows out of its card, only faster', () => {
  // The viewer's Windows asks for reduced motion. A plain fade made the
  // preview look like a second card popping over the first one; like the
  // detail window, the preview keeps its movement and shortens it. Both
  // values now come from the single motion table (styles/motion.css).
  const start = detailCss.indexOf('@media (prefers-reduced-motion: reduce)');
  assert.ok(start >= 0, 'media-detail.css handles reduced motion');
  const block = detailCss.slice(start, detailCss.indexOf('\n}\n', start));
  assert.doesNotMatch(block, /transform:\s*none/, 'the preview is never pinned at its final size');
  const open = /\n  \.hp-overlay \{([^}]*)\}/.exec(block);
  assert.ok(open, 'the opening transition is shortened, not removed');
  assert.match(open[1], /transition:[^;]*transform var\(--motion-preview-open\)[^;]*!important/);
  assert.doesNotMatch(open[1], /\d+ms/, 'the shortened duration is a token, not a literal');
  const close = /\n  \.hp-overlay\.is-out \{([^}]*)\}/.exec(block);
  assert.ok(close, 'the closing transition is shortened, not removed');
  assert.match(close[1], /transition:[^;]*transform var\(--motion-preview-close\)[^;]*!important/);
  assert.doesNotMatch(close[1], /\d+ms/);

  // The growing scene is the overlay plus the media band it crops and the
  // progress bar and body it pulls up: base.css cuts every transition with
  // !important, so each of those children repeats its shortened transition.
  for (const selector of ['.hp-overlay .hp-media', '.hp-overlay .hp-progress', '.hp-overlay .hp-body']) {
    const rule = ruleOf(block, selector);
    assert.match(rule, /transition:/, `${selector} keeps its movement`);
    assert.match(rule, /!important/, `${selector} beats base.css`);
    assert.doesNotMatch(rule, /\d+ms/, `${selector} reads the token`);
  }
  assert.match(
    ruleOf(block, '.hp-overlay .hp-media'),
    /clip-path var\(--motion-preview-open\) var\(--ease-enter\)/,
    'the media band opens with the same token',
  );
  for (const selector of ['.hp-overlay.is-out .hp-media', '.hp-overlay.is-out .hp-progress', '.hp-overlay.is-out .hp-body']) {
    const rule = ruleOf(block, selector);
    assert.match(rule, /transition:/, `${selector} keeps its return`);
    assert.match(rule, /var\(--motion-preview-close\) var\(--ease-move\)/, `${selector} reads the close token`);
    assert.match(rule, /!important/);
  }

  // The table is where the shortened 200 ms opening actually lives.
  const motion = read('styles/motion.css');
  const reduceStart = motion.indexOf('@media (prefers-reduced-motion: reduce)');
  assert.ok(reduceStart >= 0, 'motion.css carries the reduced values');
  const reduced = motion.slice(reduceStart);
  assert.match(reduced, /--motion-preview-open:\s*200ms/);
  assert.match(reduced, /:root:not\(\.force-motion\)/);
});

test('regression: the preview starts exactly at the card, not at a fallback scale', () => {
  // The preview is measured before its geometry custom properties exist: a
  // first style without them would animate from the fallbacks (scale 1, no
  // clip) and the preview would pop up larger than its card. Placement runs
  // under `is-placing`, which suspends transitions, and the class comes off
  // only once the card-sized start frame has been committed.
  const open = /\n  function open\(card\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(open, 'no open body');
  const body = open[1];
  const at = (text, from = 0) => {
    const index = body.indexOf(text, from);
    assert.ok(index >= 0, `missing: ${text}`);
    return index;
  };
  const placing = at("el.classList.add('is-placing');");
  const append = at('document.body.appendChild(el);', placing);
  assert.ok(placing < append, 'transitions are suspended before the preview is in the document');
  let last = append;
  for (const name of ['--hp-x', '--hp-y', '--hp-s', '--hp-crop', '--hp-shift', '--hp-clip', '--hp-radius']) {
    last = at(`'${name}'`, last);
  }
  const grow = at('growFrame({');
  assert.ok(grow > append && grow < last, 'growFrame measures the preview before the properties are set');
  const commit = at('el.getBoundingClientRect();', last);
  assert.ok(last < commit, 'every custom property is set before the start frame is committed');
  const unplace = at("el.classList.remove('is-placing');", commit);
  const enter = at("window.requestAnimationFrame(() => el.classList.add('is-in'));", unplace);
  assert.ok(commit < unplace && unplace < enter, 'transitions come back after the commit, before the opening');
  assert.doesNotMatch(mediaDetail, /setProperty\('transition'/, 'the placement class replaces inline transitions');
});

test('the start frame paints a ghost of the card, not the preview, and crossfades it', () => {
  // The start frame had the card's rect and thumbnail band but the preview's
  // own title, meta line, play button and mute control: it still read as
  // another card. open() clones the card into the preview, sized to the card
  // and hidden from assistive tech, so the first frame is the card's own
  // pixels; media-detail.css crossfades that ghost away as the preview grows.
  const open = /\n  function open\(card\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(open, 'no open body');
  const body = open[1];

  const cloneAt = body.indexOf('const ghost = card.cloneNode(true);');
  assert.ok(cloneAt >= 0, 'open() clones the card');
  const appendAt = body.indexOf('el.appendChild(ghost);', cloneAt);
  assert.ok(appendAt > cloneAt, 'the ghost is appended to the preview');
  const ghost = body.slice(cloneAt, appendAt);

  // The card keeps its card styles; the live state of the card does not come
  // along.
  for (const name of ['is-previewing', 'is-picked', 'is-selected']) {
    assert.match(
      ghost,
      new RegExp(`ghost\\.classList\\.remove\\([^)]*'${name}'`),
      `the ghost drops ${name}`,
    );
  }
  assert.match(ghost, /ghost\.classList\.add\('hp-card-ghost'\)/, 'the ghost has its own class');
  assert.match(ghost, /ghost\.setAttribute\('aria-hidden', 'true'\)/, 'the ghost is silent for assistive tech');
  assert.match(ghost, /ghost\.setAttribute\('inert', ''\)/, 'the ghost cannot take focus or clicks');
  assert.match(ghost, /ghost\.removeAttribute\('id'\)/, 'the clone drops its own id');
  assert.match(ghost, /ghost\.querySelectorAll\('\[id\]'\)/, 'the clone drops the ids inside it');
  const mediaSelector = /ghost\.querySelectorAll\('([^']*video[^']*)'\)/.exec(ghost);
  assert.ok(mediaSelector, 'the clone removes its playing media');
  assert.doesNotMatch(mediaSelector[1], /\bimg\b/, 'the poster img stays: it is the card pixels');

  // Sized to the card rect, measured before the clone so open() reuses it for
  // placePreview and growFrame.
  assert.ok(
    body.indexOf('const cardRect = card.getBoundingClientRect();') < cloneAt,
    'the card rect is measured before the ghost is sized',
  );
  assert.match(ghost, /ghost\.style\.width = `\$\{cardRect\.width\}px`/);
  assert.match(ghost, /ghost\.style\.height = `\$\{cardRect\.height\}px`/);

  // The ghost belongs to the card-sized start frame: it is appended before the
  // preview is measured (absolutely positioned, it does not change that size)
  // and before the start frame is committed.
  const measured = body.indexOf('const size = { width: el.offsetWidth, height: el.offsetHeight };');
  assert.ok(measured > appendAt, 'the ghost is appended before the preview is measured');
  assert.ok(
    appendAt < body.indexOf("el.classList.remove('is-placing');"),
    'the ghost is in place before the start frame is committed',
  );
});

test('the card stays hidden until the preview has finished closing onto it', () => {
  // The preview stands for the card while it shrinks back: showing the real
  // card at the first frame of the close would double it. Only the timeout
  // that removes the preview brings the card back, and not if a new preview
  // already took that same card.
  const close = /\n  function close\(immediate\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(close, 'no close body');
  const body = close[1];

  const timed = /window\.setTimeout\(\(\) => \{([\s\S]*?)\}, previewClose \+ 40\);/.exec(body);
  assert.ok(timed, 'a non-immediate close removes the preview after its animation');
  const removal = timed[1];
  assert.match(removal, /card\.classList\.remove\('is-previewing'\);/, 'the card comes back with the removal');
  assert.match(removal, /card !== previewCard/, 'unless a new preview took that card meanwhile');
  assert.ok(
    removal.indexOf("card.classList.remove('is-previewing')") < removal.indexOf('el.remove()'),
    'the card is shown as the preview goes away',
  );

  const immediate = /if \(immediate\) \{([\s\S]*?)\} else \{/.exec(body);
  assert.ok(immediate, 'the immediate branch exists');
  assert.match(immediate[1], /card\.classList\.remove\('is-previewing'\)/, 'an immediate close shows the card at once');
  assert.ok(
    !body.slice(0, body.indexOf('if (immediate)')).includes('is-previewing'),
    'nothing unhides the card before the close animation',
  );
});

test('regression: the open preview keeps its shadow outside the clip', () => {
  // clip-path also cuts what is painted outside the box: an open preview
  // clipped at its own edges lost its whole drop shadow. Once open, the clip
  // reaches past every edge (negative insets) so the shadow shows; the
  // corners stay rounded by border-radius and overflow: hidden.
  const open = ruleOf(detailCss, '.hp-overlay.is-in');
  const clip = /clip-path:\s*inset\(([^)]*)\)/.exec(open);
  assert.ok(clip, 'the open preview declares its clip');
  const insets = clip[1].split(/\s+round\s+/)[0].trim().split(/\s+/);
  assert.equal(insets.length, 4, 'four explicit insets');
  for (const inset of insets) {
    assert.match(inset, /^-\d+px$/, `${inset} lets the shadow out`);
  }
  assert.match(open, /box-shadow:\s*\n?\s*0 38px 96px/);
});

test('the dark backdrop darkens progressively, starting just before the window lands', () => {
  const opening = /\n  function transitionIn\(page, shell, backdrop, carry\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(opening, 'transitionIn is found');
  const body = opening[1];
  assert.match(body, /const \{ detailOpen: duration, easeEnter, backdropIn, backdropLead \} = motion\(\);/);
  // One fade, kept apart from the opening animations (finish() cancels those):
  // it waits until backdropLead before the end of the opening, holds the
  // backdrop hidden meanwhile (fill backwards), then runs past the landing.
  assert.match(
    body,
    /backdrop\._fadeIn = backdrop\.animate\(\[\{ opacity: HIDDEN \}, \{ opacity: 1 \}\], \{\s*delay: Math\.max\(0, duration - backdropLead\),\s*duration: backdropIn,\s*easing: 'ease-in-out',\s*fill: 'backwards',/,
  );
  assert.doesNotMatch(body, /animations\.push\(backdrop\.animate/, 'the opening list never holds the backdrop fade');
  const finish = /const finish = \(\) => \{([\s\S]*?)\n    \};/.exec(body);
  assert.ok(finish, 'finish() is found');
  assert.doesNotMatch(finish[1], /backdrop/, 'landing does not restart the fade');
});

test('closing during the backdrop fade starts from its real opacity', () => {
  const closing = /\n  function transitionOut\(page, card, thumb\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(closing, 'transitionOut is found');
  const body = closing[1];
  const read = body.indexOf('const backdropFrom = Number(getComputedStyle(backdrop).opacity)');
  const stop = body.indexOf('backdrop._fadeIn.cancel()');
  assert.ok(read >= 0 && stop > read, 'the opacity is read before the fade is stopped');
  assert.match(body, /backdrop\.animate\(\[\{ opacity: backdropFrom \}, \{ opacity: 0 \}\]/);
});

test('opening the window carries its content with the growing frame', () => {
  // The page used to stay at its final place while a clip-path mask uncovered
  // it. Now the content itself starts on the card and travels with the
  // surface: one transform + clip animation, same timing as the surface, with
  // its transform origin at the top left.
  const opening = /\n  function transitionIn\(page, shell, backdrop, carry\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(opening, 'transitionIn is found');
  const body = opening[1];

  assert.match(
    body,
    /page\.animate\(pageFlightFrames\(\{\s*origin: originRect,\s*page: pageRect,\s*originRadius,\s*pageRadius,\s*\}\), \{ duration, easing: easeEnter \}\)/,
    'the page flies from the origin rect at the surface timing',
  );
  const originSet = body.indexOf("page.style.transformOrigin = '0 0';");
  const pageAnimation = body.indexOf('page.animate(pageFlightFrames(');
  assert.ok(originSet >= 0, 'the page gets the top-left transform origin');
  assert.ok(pageAnimation > originSet, 'the origin is set before the page animates');

  // Landing restores the CSS origin, shared with the fallback branch.
  const finish = /const finish = \(\) => \{([\s\S]*?)\n    \};/.exec(body);
  assert.ok(finish, 'finish() is found');
  assert.match(finish[1], /page\.style\.transformOrigin = '';/);

  // The content still fades in on its own curve while it travels.
  assert.match(body, /\{ opacity: HIDDEN, offset: 0 \}/);
  assert.match(body, /\{ opacity: 1, offset: 0\.7 \}/);
});

test('closing the window reverses the content flight onto the card', () => {
  const closing = /\n  function transitionOut\(page, card, thumb\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(closing, 'transitionOut is found');
  const body = closing[1];

  assert.match(
    body,
    /pageFlightFrames\(\{\s*origin: cardRect,\s*page: pageRect,\s*originRadius: cardRadius,\s*pageRadius,\s*\}\)/,
    'the page flies back to the card rect',
  );
  assert.match(body, /\.reverse\(\)/, 'the opening path is played backwards');
  assert.match(body, /offset: 1 - frame\.offset/, 'the reversed offsets are recomputed');
  assert.match(body, /page\.style\.transformOrigin = '0 0';/, 'the page gets the top-left transform origin');
  assert.match(
    body,
    /page\.animate\([\s\S]*?\{ duration, easing: easeMove, fill: 'forwards' \}/,
    'the reversed page flight lands with the surface',
  );
  // The content still leaves early.
  assert.match(body, /\{ opacity: 0, offset: 0\.35 \}/);
});

test('neither transition masks the page with insetFor any more', () => {
  for (const [name, signature] of [
    ['transitionIn', 'page, shell, backdrop, carry'],
    ['transitionOut', 'page, card, thumb'],
  ]) {
    const body = new RegExp(`\\n  function ${name}\\(${signature}\\) \\{([\\s\\S]*?)\\n  \\}\\n`).exec(mediaDetail);
    assert.ok(body, `${name} is found`);
    assert.doesNotMatch(body[1], /insetFor\(pageRect/, `${name} no longer clips the page in place`);
  }

  const imported = /import \{([\s\S]*?)\} from '\.\.\/core\/detail-motion\.js'/.exec(mediaDetail);
  assert.ok(imported, 'media-detail imports its motion helpers');
  assert.match(imported[1], /\bpageFlightFrames\b/, 'the container transform helper is imported');
  assert.doesNotMatch(imported[1], /\binsetFor\b/, 'the in-place clip helper is no longer imported');
  // Nothing uses the in-place clip any more: it is gone, not kept as dead code.
  assert.doesNotMatch(read('core/detail-motion.js'), /insetFor/);
});

test('the carried media rides with the travelling page, both ways', () => {
  const opening = /\n  function transitionIn\(page, shell, backdrop, carry\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  const closing = /\n  function transitionOut\(page, card, thumb\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(opening && closing, 'both transitions are found');

  // Opening: the page's own flight is recorded and the media aims at the
  // stage of that moving page, from the preview's media.
  assert.match(opening[1], /pageTrack = \{ origin: originRect, page: pageRect \};/);
  assert.match(
    opening[1],
    /mediaTrackFrames\(\{ \.\.\.pageTrack, media: to, outside: from \}\)/,
    'the media follows the page from the preview',
  );
  assert.match(
    opening[1],
    /flight\.animate\(keyframes, \{ duration, easing: easeEnter \}\)/,
    'on the page easing',
  );

  // Closing: it stays with the shrinking page, then lands on the thumbnail.
  assert.match(
    closing[1],
    /mediaTrackFrames\(\{\s*origin: cardRect,\s*page: pageRect,\s*media: from,\s*outside: thumbRect,\s*leaving: true,\s*\}\)/,
    'the media follows the page back to the card',
  );
  assert.match(
    closing[1],
    /track\.map\([\s\S]*?\{ duration, easing: easeMove, fill: 'forwards' \}/,
    'on the page easing',
  );
});

test('the stage is measured before the page starts travelling', () => {
  // Once the page's flight exists its box is the scaled-down one: the media
  // would aim at the stage shrunk onto the preview.
  const opening = /\n  function transitionIn\(page, shell, backdrop, carry\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(opening, 'transitionIn is found');
  const body = opening[1];
  const measured = body.indexOf('const slotBox = visual && carry.slot ? contentBoxOf(carry.slot) : null;');
  const travels = body.indexOf('page.animate(pageFlightFrames(');
  assert.ok(measured >= 0 && travels >= 0, 'both are found');
  assert.ok(measured < travels, 'the stage is measured first');
  assert.match(body, /const to = containRect\(slotBox, carry\.ratio\);/);
  assert.doesNotMatch(body, /contentBoxOf\(carry\.slot\), carry\.ratio/);
});

test('the preview is never narrower than its card, so its first frame is the card', () => {
  // growFrame never scales up: a preview narrower than the card would start
  // smaller than the card it replaces (portrait media in a wide column).
  const build = /\n  function build\(card, desc\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(build, 'build is found');
  assert.match(
    build[1],
    /el\.style\.width = `\$\{Math\.ceil\(Math\.max\(260, size\.width, card\.getBoundingClientRect\(\)\.width\)\)\}px`;/,
  );
});

test('a late-loading picture reopens the preview only under the pointer', () => {
  const reopen = /_poster\.addEventListener\('load', \(\) => \{([\s\S]*?)\}, \{ once: true \}\);/.exec(mediaDetail);
  assert.ok(reopen, 'the load handler is found');
  assert.match(
    reopen[1],
    /if \(previewEl === el && !hideTimer && \(card\.matches\(':hover'\) \|\| el\.matches\(':hover'\)\)\) \{/,
    'no reopening once the pointer left or a hide is pending',
  );
});

test('a closing preview lets the pointer through', () => {
  const css = read('styles/media-detail.css');
  const rule = /\n\.hp-overlay\.is-out \{([^}]*)\}/.exec(css);
  assert.ok(rule, 'the closing rule is found');
  assert.match(rule[1], /pointer-events: none;/);
});

test('closing measures the stage before the page starts shrinking', () => {
  const closing = /\n  function transitionOut\(page, card, thumb\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(closing, 'transitionOut is found');
  const body = closing[1];
  const measured = body.indexOf('const stageBox = stage ? contentBoxOf(stage) : null;');
  const shrinks = body.indexOf('page.animate(');
  assert.ok(measured >= 0 && shrinks >= 0, 'both are found');
  assert.ok(measured < shrinks, 'the stage is measured first');
  assert.match(body, /containRect\(stageBox, mediaRatioOf\(visual, page\._ratio\)\)/);
});
