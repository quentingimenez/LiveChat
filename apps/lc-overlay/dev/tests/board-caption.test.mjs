// What this file protects: the media caption is a local, transient draft of the
// board window. It is never written to the item, never PATCHed, is emptied when
// the window opens, and is consumed by the trigger or dropped when the window
// closes. The trigger payload carries it for the server.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const board = readFileSync(new URL('../../web/app/views/board.js', import.meta.url), 'utf8').replace(
  /\r\n/g,
  '\n',
);
const mediaDetail = readFileSync(
  new URL('../../web/app/views/media-detail.js', import.meta.url),
  'utf8',
).replace(/\r\n/g, '\n');

function bodyOf(name) {
  const match = new RegExp(
    `\\n(?:export )?(?:async )?function ${name}\\(.*?\\) \\{([\\s\\S]*?)\\n\\}`,
  ).exec(board);
  assert.ok(match, `no body for ${name}`);
  return match[1];
}

test('the caption is a draft in memory, not an item field written to the server', () => {
  assert.match(board, /const legendDrafts = new Map\(\);/, 'one module-level draft map');
  assert.match(bodyOf('legendDraft'), /legendDrafts\.get\(itemId\)/, 'the field reads the map');
  assert.match(bodyOf('legendDraft'), /\.trim\(\)/, 'the draft is trimmed');

  const detail = bodyOf('detailBody');
  assert.match(detail, /inlineField\(item, 'message', canPlay\)/, 'playable media can type a caption');
  assert.doesNotMatch(detail, /item\.message/, 'the legacy stored message never seeds the window');

  const commit = bodyOf('commitInline');
  const branchStart = commit.indexOf("if (field === 'message')");
  const branchEnd = commit.indexOf('const before =');
  assert.ok(branchStart >= 0 && branchEnd > branchStart, 'the caption has its own commit branch');
  const branch = commit.slice(branchStart, branchEnd);
  assert.match(branch, /value\.trim\(\)/, 'an empty caption is trimmed away');
  assert.match(branch, /legendDrafts\.delete\(item\.id\)/);
  assert.match(branch, /legendDrafts\.set\(item\.id, caption\)/);
  assert.doesNotMatch(branch, /\bapi\(|\bPATCH\b|loadItems/, 'no request and no board reload for a caption');
  assert.match(branch, /syncDetail\(\)/, 'the window shows the committed draft');
  assert.match(commit, /api\('PATCH', `\/overlay\/media-board\/items\/\$\{encodeURIComponent\(item\.id\)\}`, \{ \[field\]: value \}\)/, 'the title still saves to the server');
});

test('the caption draft is emptied when the window opens and when it closes', () => {
  assert.match(
    board,
    /onBeforeOpen: \(id\) => \{\s*(?:\/\/[^\n]*\n\s*)+legendDrafts\.delete\(id\);/,
    'opening the window always starts from an empty caption',
  );

  const closeStart = board.indexOf('onClose: (id) => {');
  assert.ok(closeStart >= 0, 'the board closes the window');
  const close = board.slice(closeStart, board.indexOf('\n  },', closeStart));
  assert.match(close, /legendDrafts\.delete\(id\)/, 'closing drops the draft');
  assert.match(close, /mediaDetail\.openId\(\) !== id/, 'a window closed behind a newer one still drops its draft');
  assert.match(close, /if \(mediaDetail\.openId\(\)\) \{\s*return;/, 'a newer window keeps its own draft');

  // The window controller owns the opening: the hook must run before the
  // information column (and its caption field) is built.
  const before = mediaDetail.indexOf('opts.onBeforeOpen(id);');
  const column = mediaDetail.indexOf('const info = detailColumn(desc);');
  assert.ok(before >= 0 && column > before, 'the board clears the draft before the column renders');
});

test('a caption blur keeps the pressed control in place until the click lands', () => {
  const input = bodyOf('inlineInput');
  assert.match(input, /const finishCaptionBlur = \(\) => \{/, 'the rebuild is deferred');
  assert.match(input, /afterNextClick\(document, finishCaptionBlur/, 'it waits for the click');
  assert.match(input, /input\.contains\(event\.target\)/, 'clicking back into the field keeps it open');
  assert.match(input, /inlineEdit\.field !== 'message'/, 'a new inline edit keeps its own render');
  assert.match(input, /if \(mediaDetail\.openId\(\) === item\.id\) \{\s*syncDetail\(\);/, 'a closed window is not rebuilt');
});

test('Escape restores the caption the edit started from', () => {
  const input = bodyOf('inlineInput');
  assert.match(input, /const captionBefore = isTitle \? '' : legendDraft\(item\.id\);/, 'the edit snapshots the caption');
  assert.match(input, /legendDrafts\.set\(item\.id, captionBefore\)/, 'cancelling restores it');
  assert.match(input, /legendDrafts\.delete\(item\.id\)/, 'cancelling an empty caption drops the typed draft');
});

test('a sent caption is consumed by the play, and the newest text wins', () => {
  const play = bodyOf('playItem');
  assert.match(play, /const caption = legendDraft\(item\.id\)/, 'the play snapshots the draft');
  assert.match(play, /sendTrigger\(item, caption\)/, 'the snapshot is what the trigger sends');
  assert.match(play, /if \(sent && legendDraft\(item\.id\) === caption\)/, 'only an accepted, unchanged caption is consumed');
  assert.match(play, /legendDrafts\.delete\(item\.id\)/);
  assert.match(play, /syncDetail\(\)/, 'the emptied field is shown again');

  assert.match(bodyOf('playSelection'), /legendDrafts\.delete\(item\.id\)/, 'a batch play consumes each caption too');

  const trigger = bodyOf('sendTrigger');
  assert.match(
    trigger,
    /invoke\('media_board_trigger', \{ payload: \{ itemId: item\.id, guildId: state\.activeGuildId, trigger: 'ui', message \} \}\)/,
    'the local caption travels in the trigger payload',
  );
});
