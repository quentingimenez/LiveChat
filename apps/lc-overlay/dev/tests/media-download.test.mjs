// The details window could not save a media: it offered playback only. These
// checks pin the way out: a click on the media itself toggles playback, and the
// same media can be saved locally — from an icon of the control bar and from a
// right click on the stage (the only gesture a picture, which has no player,
// can offer). The shell opens a native save dialog and does the file writing;
// cancelling it must leave no file and no toast. The strings the shell reports
// back (`invalid_link`, `http_404`…) must reach the user as sentences, never as
// codes: those codes are turned into text here, so a shell change cannot print
// a stack or a code in a toast.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  MEDIA_DOWNLOAD_MESSAGES,
  mediaDownloadError,
} from '../../web/app/core/media-download.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const mediaDetail = read('views/media-detail.js').replace(/\r\n/g, '\n');
const detailCss = read('styles/media-detail.css');
const dom = read('core/dom.js');

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

function attachBody() {
  const attach = /\n  function attachPlayerControls\(([^)]*)\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(attach, 'attachPlayerControls introuvable');
  return { params: attach[1], body: attach[2] };
}

function buildBody() {
  const build = /\n  function buildStage\(([^)]*)\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(build, 'buildStage introuvable');
  return build[2];
}

test('a shell code becomes one readable sentence', () => {
  assert.equal(mediaDownloadError('invalid_link'), MEDIA_DOWNLOAD_MESSAGES.invalidLink);
  assert.equal(mediaDownloadError(new Error('too_large')), MEDIA_DOWNLOAD_MESSAGES.tooLarge);
  assert.equal(mediaDownloadError('  unreachable  '), MEDIA_DOWNLOAD_MESSAGES.unreachable);
});

test('an HTTP failure keeps the status it answered with', () => {
  assert.match(mediaDownloadError('http_404'), /HTTP 404/);
  assert.match(mediaDownloadError('http_503'), /HTTP 503/);
});

test('an unknown or missing code falls back instead of leaking it', () => {
  for (const code of ['', null, undefined, 'boom', 'http_x', 42]) {
    assert.equal(mediaDownloadError(code), MEDIA_DOWNLOAD_MESSAGES.unknown);
  }
});

test('no two codes share a sentence, and none is terse', () => {
  const messages = Object.values(MEDIA_DOWNLOAD_MESSAGES);
  assert.equal(new Set(messages).size, messages.length);
  for (const message of messages) {
    assert.ok(message.length > 10, `phrase trop courte : ${message}`);
  }
});

test('the control bar carries a download button wired to the media action', () => {
  const { params, body } = attachBody();
  assert.match(params, /download/);
  assert.match(body, /class: 'hp-ctl-button hp-ctl-download'/);
  assert.match(body, /'aria-label': 'Télécharger le média'/);
  assert.match(body, /icon\('download'\)/);
  assert.match(body, /if \(downloadButton\) \{\n      root\.append\(downloadButton\);\n    \}/);
  assert.match(body, /downloadButton\.disabled = true;/);
  assert.match(body, /downloadButton\.disabled = false;/);
});

test('a click on the media toggles playback but leaves the bar alone', () => {
  const { body } = attachBody();
  // The bar's play button swaps its glyph while the click bubbles: by then the
  // click's target is detached, so a `root.contains(target)` guard would let the
  // stage toggle a second time. The dispatch path survives that swap.
  assert.match(body, /const onStageClick = \(event\) => \{\n      if \(event\.composedPath\(\)\.includes\(root\)\) \{\n        return;\n      \}\n      togglePlay\(\);\n    \};/);
  assert.doesNotMatch(body, /root\.contains\(event\.target\)/);
  assert.match(body, /bind\(stage, 'click', onStageClick\);/);
  assert.match(body, /stage\.classList\.add\('is-playable'\);/);
  assert.match(body, /stage\.classList\.remove\('is-playable'\);/);
});

test('the stage offers the same download on right click, pictures included', () => {
  const body = buildBody();
  assert.match(body, /const download = url \? \(\) => downloadMedia\(desc\) : null;/);
  assert.match(body, /stage\.addEventListener\('contextmenu', \(event\) => \{/);
  assert.match(body, /label: 'Télécharger le média', icon: 'download'/);
  assert.match(body, /attachPlayerControls\(stage, player, kind, volume, download\)/);
});

test('the download goes through the shell with the media URL and its name', () => {
  assert.match(mediaDetail, /async function downloadMedia\(desc\) \{/);
  assert.match(mediaDetail, /await invoke\('media_download', \{ payload: \{ url, name \} \}\)/);
  assert.match(mediaDetail, /Média enregistré : \$\{fileName\}/);
  assert.doesNotMatch(mediaDetail, /Téléchargements/);
});

test('cancelling the save dialog writes nothing and stays silent', () => {
  assert.match(mediaDetail, /if \(!saved \|\| saved\.canceled\) \{\n      return;\n    \}/);
  assert.match(
    mediaDetail,
    /toast\(\{ message: mediaDownloadError\(error\), kind: 'error', timeout: 9000 \}\);/,
  );
});

test('the download glyph is a stroke icon shared by the bar and the menu', () => {
  assert.match(dom, /download: 'M12 4v11M7 10l5 5 5-5M5 20h14'/);
});

test('the stage advertises its click and the button its transfer', () => {
  assert.match(ruleOf(detailCss, '.hp-page-stage.is-playable'), /cursor:\s*pointer/);
  assert.match(ruleOf(detailCss, '.hp-ctl-button:disabled'), /cursor:\s*default/);
  assert.match(ruleOf(detailCss, '.hp-ctl-button:disabled'), /opacity:\s*0?\.55/);
});
