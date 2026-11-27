// Lecture des aperçus et des détails : quatre réglages d'appareil décident si
// un média démarre seul et avec son, et le niveau par défaut des deux lecteurs
// est le plus bas des volumes général et de guilde, sur une échelle absolue
// 0–100 % (le 100 % de la barre d'aperçu ne reprend jamais le volume des
// paramètres). Les deux sons démarrent coupés ; un média audio ne démarre
// jamais en sourdine, son autoplay attend donc que le son soit activé. Les
// deux réglages d'aperçu ne s'appliquent que si l'aperçu au survol est actif.
// Ce fichier protège ces invariants.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { playbackPlan, previewPlaybackEnabled } from '../../web/app/core/media-playback.js';

// Minimal DOM stand-ins for the renderer's `instanceof` checks at import time.
class FakeElement {
  constructor() {
    this.style = { setProperty() {} };
  }
}
class FakeMediaElement extends FakeElement {
  constructor() {
    super();
    this.dataset = {};
    this.volume = 1;
    this.muted = false;
  }
}
globalThis.HTMLElement = FakeElement;
globalThis.HTMLMediaElement = FakeMediaElement;
globalThis.window = {};

const { mediaLevelOf, playbackSettings } = await import('../../web/app/core/sound.js');

const read = (path) =>
  readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const mediaDetail = read('views/media-detail.js');
const board = read('views/board.js');
const settingsView = read('views/settings.js');

test('autoplay is on by default, the sound needs an explicit true', () => {
  assert.deepEqual(playbackSettings({}), {
    previewAutoplay: true,
    detailAutoplay: true,
    previewSound: false,
    detailSound: false,
  });
  assert.deepEqual(
    playbackSettings({ previewAutoplay: false, previewSound: true }),
    {
      previewAutoplay: false,
      detailAutoplay: true,
      previewSound: true,
      detailSound: false,
    },
    'a switch left out keeps its default',
  );
  assert.equal(
    playbackSettings({ previewSound: 'true' }).previewSound,
    false,
    'only the boolean true enables a sound',
  );
  assert.equal(playbackSettings({ detailAutoplay: 0 }).detailAutoplay, true);
});

test('the players default to the quieter of the general and guild volumes', () => {
  const view = { volume: 0.8, guilds: { g1: { volume: 0.5 }, g2: { volume: 1 } } };
  assert.equal(mediaLevelOf(view, 'g1'), 0.5, 'the guild volume is lower');
  assert.equal(mediaLevelOf(view, 'g2'), 0.8, 'the general volume is lower');
  assert.equal(mediaLevelOf(view, 'unknown'), 0.8, 'a guild without a volume only has the general one');
  assert.equal(mediaLevelOf(view, null), 0.8);
  assert.equal(mediaLevelOf({}, 'g1'), 1, 'both volumes start at 100 %');
  assert.equal(mediaLevelOf({ volume: 2, guilds: { g1: { volume: -2 } } }, 'g1'), 0);
  assert.equal(mediaLevelOf({ volume: 0.3, guilds: { g1: { volume: 'x' } } }, 'g1'), 0.3);
});

test('a video may autoplay muted, an audio waits for its sound setting', () => {
  assert.deepEqual(playbackPlan({ kind: 'video', autoplay: true, sound: true }), { autoplay: true, muted: false });
  assert.deepEqual(playbackPlan({ kind: 'video', autoplay: true, sound: false }), { autoplay: true, muted: true });
  assert.deepEqual(playbackPlan({ kind: 'audio', autoplay: true, sound: false }), { autoplay: false, muted: true });
  assert.deepEqual(playbackPlan({ kind: 'audio', autoplay: true, sound: true }), { autoplay: true, muted: false });
  assert.deepEqual(playbackPlan({ kind: 'video', autoplay: false, sound: true }), { autoplay: false, muted: false });
  assert.deepEqual(playbackPlan({ kind: 'audio', autoplay: false, sound: true }), { autoplay: false, muted: false });
  assert.deepEqual(playbackPlan({ kind: 'image', autoplay: true, sound: true }), { autoplay: true, muted: false });
});

test('the preview bar sets an absolute level, no longer a trim of the settings gain', () => {
  assert.doesNotMatch(mediaDetail, /trimVolume/, 'the perceptual trim is gone from the preview player');
  assert.match(mediaDetail, /let previewLevel = null;/, 'null follows the card default');
  assert.match(mediaDetail, /const level = clampVolume\(Number\(slider\.value\) \/ 100\)/);
  assert.match(mediaDetail, /player\.volume = level;/);
  assert.match(mediaDetail, /slider\.max = '100'/, 'the bar is a plain 0..100 scale');
});

test('the preview follows the device autoplay and sound switches', () => {
  const open = /\n  function open\(card\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(open, 'open is found');
  assert.match(open[1], /el\._playback\.autoplay/, 'the preview only plays when the plan allows it');
  assert.doesNotMatch(open[1], /_player\.muted = true/, 'the preview no longer forces silence');
  assert.match(mediaDetail, /playbackPlan\(\{ kind, autoplay: settings\.previewAutoplay, sound: settings\.previewSound \}\)/);
});

test('the details window applies its own autoplay and sound plan', () => {
  for (const name of ['detailVideo', 'detailAudio']) {
    const body = new RegExp(`\\n  function ${name}\\(([^)]*)\\) \\{([\\s\\S]*?)\\n  \\}`).exec(mediaDetail);
    assert.ok(body, `${name} is found`);
    assert.match(body[2], /playbackPlan\(\{ kind: /, `${name} reads the detail switches`);
    assert.match(body[2], /player\.autoplay = plan\.autoplay/, `${name} honours the autoplay switch`);
    assert.match(body[2], /player\.muted = plan\.muted/, `${name} honours the sound switch`);
  }
  const open = /\n  function openDetail\(card\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(open, 'openDetail is found');
  assert.match(open[1], /playbackPlan\(\{ kind, autoplay: settings\.detailAutoplay, sound: settings\.detailSound \}\)/);
  assert.doesNotMatch(open[1], /A new player starts silent/, 'a new player follows the switches, not a forced mute');
});

test('the preview switches only apply while the hover preview is on', () => {
  assert.equal(previewPlaybackEnabled({ hoverPreview: false }), false);
  assert.equal(previewPlaybackEnabled({ hoverPreview: true }), true);
  assert.equal(previewPlaybackEnabled({}), true, 'the hover preview is on by default');
  assert.equal(previewPlaybackEnabled(undefined), true);
});

test('the settings page disables the preview switches without the hover preview', () => {
  assert.match(settingsView, /const previewEnabled = previewPlaybackEnabled\(settings\);/);
  assert.equal(
    (settingsView.match(/disabled: !previewEnabled/g) || []).length,
    2,
    'both hover-preview switches are disabled together',
  );
  assert.match(settingsView, /previewNote\.hidden = previewEnabled/, 'the explanation starts hidden when previews work');
  assert.match(settingsView, /function setPreviewPlaybackEnabled\(enabled\)/);
  assert.match(
    settingsView,
    /setPreviewPlaybackEnabled\(event\.target\.checked\)/,
    'turning the hover preview off disables the two switches without a re-render',
  );
  assert.match(settingsView, /id: 'settings-preview-disabled'/);
  assert.match(settingsView, /L’aperçu au survol est désactivé/);
});

test('a switch that cannot apply reads as inactive', () => {
  const css = read('styles/base.css');
  assert.match(css, /\.switch input\[type='checkbox'\]:disabled \{[^}]*cursor: not-allowed;[^}]*opacity: 0\.45;/);
  assert.match(css, /\.switch:has\(input\[type='checkbox'\]:disabled\)/);

  const settingsCss = read('styles/settings.css');
  assert.match(settingsCss, /\.settings-row:has\(input\[type='checkbox'\]:disabled\) \.settings-label/);
});

test('the settings page offers the four switches in a Lecture section', () => {
  const rows = [
    ['settings-preview-autoplay', 'previewAutoplay', 'settings.previewAutoplay !== false'],
    ['settings-preview-sound', 'previewSound', 'settings.previewSound === true'],
    ['settings-detail-autoplay', 'detailAutoplay', 'settings.detailAutoplay !== false'],
    ['settings-detail-sound', 'detailSound', 'settings.detailSound === true'],
  ];
  assert.match(settingsView, /function renderPlayback\(settings\)/);
  for (const [id, key, checked] of rows) {
    assert.match(settingsView, new RegExp(`id: '${id}'`));
    assert.ok(settingsView.includes(`checked: ${checked}`), `${key} starts from its own default`);
    assert.match(
      settingsView,
      new RegExp(`applyPatch\\(\\{ ${key}: event\\.target\\.checked \\}\\)`),
    );
  }
  const render = settingsView.slice(settingsView.indexOf('export function renderSettings'));
  const calls = ['renderDisplay(settings)', 'renderPlayback(settings)', 'renderSound(settings)'];
  const positions = calls.map((call) => render.indexOf(call));
  assert.ok(positions.every((position) => position >= 0), 'the three sections are rendered');
  assert.deepEqual(positions, [...positions].sort((a, b) => a - b), 'Lecture sits between Affichage and Son');
});

test('the board gives the players the quieter default level', () => {
  assert.match(board, /import \{ mediaLevel \} from '\.\.\/core\/sound\.js'/);
  assert.doesNotMatch(board, /previewVolume/, 'the players no longer reuse the overlay gain');
  assert.match(board, /volume: mediaLevel\(state\.activeGuildId\)/, 'the card descriptions carry the default level');
  assert.match(
    board,
    /mediaDetail\.setVolume\(mediaLevel\(state\.activeGuildId\)\)/,
    'a volume change reaches the open details player',
  );
});
