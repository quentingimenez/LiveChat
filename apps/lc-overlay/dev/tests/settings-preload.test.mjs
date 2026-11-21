// Device settings are local (a Tauri IPC, not HTTP): reading them once at
// startup makes the guild « Réglages » tab render immediately instead of
// showing its loading line on first open. These assertions pin the preload
// wiring without a DOM: `initDeviceSettings` runs at boot, and the read stays
// safe when the tab is already open (`loading` guard, render only when shown).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

const settings = read('views/settings.js');
const main = read('main.js');

const init = settings.slice(
  settings.indexOf('export function initDeviceSettings'),
  settings.indexOf('export async function loadDeviceSettings'),
);
const load = settings.slice(
  settings.indexOf('export async function loadDeviceSettings'),
  settings.indexOf('export function showSettingsRemoteUpdate'),
);

test('device settings are read once at boot, after the listeners', () => {
  assert.ok(init.includes('queueMicrotask(loadDeviceSettings)'));
  assert.ok(init.indexOf('queueMicrotask(loadDeviceSettings)') > init.indexOf("listen('overlay://navigate'"));
  assert.ok(main.includes('initDeviceSettings()'));
});

test('the boot read cannot stack a second request', () => {
  assert.match(load, /if \(loading\) \{\s*return;\s*\}/);
});

test('a preload does not paint the board when settings are not shown', () => {
  assert.match(load, /if \(settingsShown\(\)\) \{\s*renderView\(\);\s*\}/);
});
