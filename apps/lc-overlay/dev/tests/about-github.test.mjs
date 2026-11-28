// « À propos » → GitHub : the board cannot open a remote page in a new window
// (Tauri has no `on_new_window` handler, so a `target="_blank"` link silently
// does nothing) and navigating the WebView would replace the board. The icon
// therefore asks the shell to hand the fixed repository URL to the default
// browser. This file protects that contract, the link markup and the URL.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');

const settings = read('web/app/views/settings.js');
const styles = read('web/app/styles/settings.css');
const lib = read('src/lib.rs');
const commands = read('src/commands.rs');
const repository = read('src/repository.rs');

function renderAbout() {
  const start = settings.indexOf('function renderAbout(settings)');
  const end = settings.indexOf('// « Retour » is a sibling', start);
  assert.ok(start >= 0, 'renderAbout is missing from settings.js');
  assert.ok(end > start, 'renderAbout has no end marker');
  return settings.slice(start, end);
}

test('the About section links the public repository with a GitHub icon', () => {
  assert.match(
    settings,
    /const REPOSITORY_URL = 'https:\/\/github\.com\/quentingimenez\/LiveChat';/,
    'the repository URL is not the shared constant',
  );
  assert.match(settings, /registerIcons\(\s*\{[\s\S]*?github:/, 'the GitHub icon is not registered');
  const about = renderAbout();
  assert.match(about, /class: 'settings-github'/, 'About has no GitHub link');
  assert.match(about, /href: REPOSITORY_URL/, 'the link does not carry the repository URL');
  assert.match(about, /icon\('github'\)/, 'the link has no GitHub icon');
  assert.match(about, /'aria-label': 'Ouvrir le dépôt GitHub'/, 'the icon-only link has no label');
});

test('clicking the icon opens the browser instead of navigating the board', () => {
  const about = renderAbout();
  assert.doesNotMatch(about, /target:\s*'_blank'/, 'a new-window link is silently ignored by wry');
  assert.match(about, /event\.preventDefault\(\)/, 'the link lets the WebView navigate away');
  assert.match(about, /openRepository\(\)/, 'the link does not hand the URL to the shell');
  assert.match(settings, /invoke\('open_repository'\)/, 'the shell command is not invoked');
  assert.match(settings, /notify\(errorMessage\(error\), true\)/, 'a failure is silent');
});

test('the GitHub icon and link keep their settings styles', () => {
  assert.match(styles, /\.settings-github\s*\{/, 'no GitHub link style');
  assert.match(styles, /\.settings-github \.icon\s*\{/, 'the icon has no size');
  assert.match(styles, /\.settings-about-name\s*\{[\s\S]*?gap:/, 'the name and icon are not aligned');
});

test('the shell registers a board-only command with a fixed URL', () => {
  assert.match(lib, /pub mod repository;/, 'repository module is not declared');
  assert.match(lib, /repository::open_repository,/, 'command is not registered');
  assert.match(commands, /"open_repository",/, 'command is missing from the IPC matrix');
  assert.match(
    repository,
    /pub const REPOSITORY_URL: &str = "https:\/\/github\.com\/quentingimenez\/LiveChat";/,
    'the shell does not pin the repository URL',
  );
  assert.match(
    repository,
    /command_allowed\(window\.label\(\), "open_repository"\)/,
    'the command is not restricted to the board',
  );
  assert.match(
    repository,
    /pub fn open_repository\(window: WebviewWindow<Wry>\)/,
    'the command accepts an arbitrary URL from the renderer',
  );
  assert.match(repository, /Command::new\("cmd"\)/, 'Windows opens the default browser through cmd');
  assert.match(repository, /\.args\(\["\/C", "start", "", url\]\)/, 'cmd does not receive the fixed URL');
});
