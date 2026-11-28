// Bandeau de mise à jour en haut de la board : il remplace l'installation
// automatique quand celle-ci est désactivée dans les Paramètres.
//
// Le test protège la décision pure (`updateNotice`), pas le DOM : sans DOM, on
// vérifie que le bandeau n'apparaît que pour une mise à jour réellement
// installable, qu'il respecte « Plus tard », et que le câblage (board.html,
// main.js, feuille de style) et le réglage `autoUpdate` restent en place.
//
// L'installation automatique elle-même est testée en Rust (src/updater.rs).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { updateNotice } from '../../web/app/core/update-status.js';

const read = (path) => readFileSync(new URL(`../../web/${path}`, import.meta.url), 'utf8');
const readDev = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

const status = (state, extra = {}) => ({
  state,
  currentVersion: '3.1.0',
  availableVersion: null,
  message: '',
  ...extra,
});

test('no update or no installable version means no banner', () => {
  for (const state of ['idle', 'checking', 'up_to_date', 'error', 'nonsense']) {
    for (const autoUpdate of [true, false]) {
      assert.equal(updateNotice(status(state), { autoUpdate }), null, `${state} shows nothing`);
    }
  }
  for (const value of [null, undefined, 'ready', 42]) {
    assert.equal(updateNotice(value, { autoUpdate: false }), null);
  }
  for (const version of [null, '', '   ', 42]) {
    assert.equal(
      updateNotice(status('ready', { availableVersion: version }), { autoUpdate: false }),
      null,
      'a ready state without a usable version cannot be installed',
    );
  }
});

test('a ready update is only offered when automatic installs are off', () => {
  const ready = status('ready', { availableVersion: '3.2.0' });

  assert.equal(
    updateNotice(ready, { autoUpdate: true }),
    null,
    'zero-click installs stay silent: the board shows no banner',
  );
  assert.deepEqual(updateNotice(ready, { autoUpdate: false }), {
    kind: 'ready',
    version: '3.2.0',
    message: 'LiveChat 3.2.0 est prête à être installée.',
  });
});

test('« Plus tard » hides one version, not the next one', () => {
  const ready = status('ready', { availableVersion: '3.2.0' });
  assert.equal(
    updateNotice(ready, { autoUpdate: false, dismissedVersion: '3.2.0' }),
    null,
    'the postponed version stays hidden',
  );
  assert.deepEqual(updateNotice(ready, { autoUpdate: false, dismissedVersion: '3.1.0' }), {
    kind: 'ready',
    version: '3.2.0',
    message: 'LiveChat 3.2.0 est prête à être installée.',
  });
});

test('a download is only announced when the user installs by hand', () => {
  const downloading = status('downloading', { availableVersion: '3.2.0' });

  assert.equal(updateNotice(downloading, { autoUpdate: true }), null);
  assert.deepEqual(updateNotice(downloading, { autoUpdate: false }), {
    kind: 'downloading',
    version: '3.2.0',
    message: 'Téléchargement de LiveChat 3.2.0…',
  });
});

test('the banner sits above the header and boots with the board', () => {
  const banner = '<div id="update-notice" class="update-notice" role="status" hidden></div>';
  for (const html of [read('board.html'), readDev('board-mock.html')]) {
    assert.ok(html.includes(banner), 'the board owns the banner element');
    assert.ok(
      html.indexOf(banner) < html.indexOf('<header id="header"'),
      'the banner stays above the header',
    );
  }

  const main = read('app/main.js');
  assert.match(main, /import \{ initUpdateNotice \} from '\.\/views\/update-notice\.js';/);
  const init = main.indexOf('initUpdateNotice();');
  assert.ok(init > 0, 'main.js starts the banner');
  assert.ok(
    init > main.indexOf('initDeviceSettings();'),
    'the device settings store exists before the banner subscribes to it',
  );

  assert.match(read('app/app.css'), /@import url\('\.\/styles\/update-notice\.css'\);/);
  assert.match(read('app/styles/update-notice.css'), /\.update-notice\s*\{/);
});

test('the banner reads the autoUpdate setting and can install or postpone', () => {
  const banner = read('app/views/update-notice.js');
  assert.match(banner, /listen\('overlay:\/\/device-settings'/);
  assert.match(banner, /autoUpdate/);
  assert.match(banner, /invoke\('settings_get'\)/);
  assert.match(banner, /invoke\('overlay_install_update'\)/);
  assert.match(banner, /localStorage/);
  assert.match(banner, /'Plus tard'/);

  const settings = read('app/views/settings.js');
  assert.match(settings, /id: 'settings-auto-update'/);
  assert.match(settings, /applyPatch\(\{ autoUpdate: event\.target\.checked \}\)/);
});
