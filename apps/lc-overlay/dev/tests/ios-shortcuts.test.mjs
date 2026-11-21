// « Mon profil » → « Ajouter un appareil » → « Raccourci iOS » : the three
// shortcut buttons used to be `target="_blank"` links. The Tauri board has no
// `on_new_window` handler, so wry cancels the new-window request and the click
// silently did nothing. The buttons now copy the signed shortcut's URL, which
// the member opens from the iPhone; this file keeps that contract, and checks
// that the URLs point at files actually shipped in ios/.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  IOS_SHORTCUTS,
  iosShortcutCopyMessage,
  iosShortcutUrl,
} from '../../web/app/core/ios-shortcuts.js';

const source = readFileSync(new URL('../../web/app/views/profile.js', import.meta.url), 'utf8');

function iosShortcutSection() {
  const start = source.indexOf('// --- iOS shortcuts ---');
  const end = source.indexOf('const makeIosToken');
  assert.ok(start >= 0, 'no iOS shortcut section in profile.js');
  assert.ok(end > start, 'makeIosToken does not follow the iOS shortcut section');
  return source.slice(start, end);
}

test('the iOS shortcut buttons copy their link instead of opening a new window', () => {
  const section = iosShortcutSection();
  assert.doesNotMatch(
    section,
    /target:\s*'_blank'/,
    'the board handles no new window: this link does nothing when clicked',
  );
  assert.doesNotMatch(section, /h\(\s*'a'/, 'the shortcuts are still anchors, not buttons');
  assert.match(section, /IOS_SHORTCUTS/, 'the panel does not use the shared shortcut list');
  assert.match(
    section,
    /onClick:[\s\S]{0,240}?copyText\(iosShortcutUrl\(shortcut\)/,
    'a shortcut button does not copy its URL',
  );
});

test('the panel says that a button copies the link to open on the iPhone', () => {
  const section = iosShortcutSection();
  assert.match(section, /copie son adresse/i, 'the panel does not explain what the buttons do');
  assert.match(section, /iPhone/, 'the panel does not say where to open the link');
});

test('each button targets the matching signed shortcut in ios/', () => {
  assert.deepEqual(
    IOS_SHORTCUTS.map((shortcut) => shortcut.label),
    ['Photo', 'Vidéo', 'Lien'],
  );
  for (const shortcut of IOS_SHORTCUTS) {
    assert.equal(
      iosShortcutUrl(shortcut),
      `https://raw.githubusercontent.com/quentingimenez/LiveChat/main/ios/${shortcut.file}`,
    );
    assert.doesNotThrow(
      () => readFileSync(new URL(`../../../../ios/${shortcut.file}`, import.meta.url)),
      `${shortcut.file} is not shipped in ios/`,
    );
  }
});

test('the copy toast names the shortcut and the iPhone', () => {
  for (const shortcut of IOS_SHORTCUTS) {
    const message = iosShortcutCopyMessage(shortcut);
    assert.match(message, new RegExp(shortcut.label));
    assert.match(message, /copié/i);
    assert.match(message, /iPhone/);
  }
});
