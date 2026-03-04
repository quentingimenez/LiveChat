const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');

const { resolveRuntimePaths } = require('../../dist/main/app/runtime-paths.js');

test('runtime-paths resolves runtime assets from baseDir and userDataPath', () => {
  const userDataPath = '/tmp/livechat-user';
  const baseDir = '/workspace/app/dist/main/app';

  const runtimePaths = resolveRuntimePaths({ userDataPath, baseDir });

  assert.deepEqual(runtimePaths, {
    configPath: path.join(userDataPath, 'config.json'),
    appIconPath: path.join(baseDir, '../../../icon.png'),
    preloadScriptPath: path.join(baseDir, '../../../dist/preload/index.js'),
    overlayHtmlPath: path.join(baseDir, '../../../renderer/overlay.html'),
    pairingHtmlPath: path.join(baseDir, '../../../renderer/pairing.html'),
    boardHtmlPath: path.join(baseDir, '../../../renderer/board.html')
  });
});

test('runtime-paths copies runtime window assets to a stable userData cache when source files exist', () => {
  const fixtureRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'livechat-runtime-paths-'));
  const userDataPath = path.join(fixtureRoot, 'user-data');
  const appRoot = path.join(fixtureRoot, 'app');
  const baseDir = path.join(appRoot, 'dist', 'main', 'app');

  fs.mkdirSync(path.join(appRoot, 'renderer'), { recursive: true });
  fs.mkdirSync(path.join(appRoot, 'dist', 'renderer', 'overlay'), { recursive: true });
  fs.mkdirSync(path.join(appRoot, 'dist', 'renderer', 'pairing'), { recursive: true });
  fs.mkdirSync(path.join(appRoot, 'dist', 'renderer', 'board'), { recursive: true });
  fs.mkdirSync(path.join(appRoot, 'dist', 'preload'), { recursive: true });
  fs.mkdirSync(baseDir, { recursive: true });

  fs.writeFileSync(path.join(appRoot, 'icon.png'), 'icon');
  fs.writeFileSync(path.join(appRoot, 'renderer', 'overlay.html'), '<html></html>');
  fs.writeFileSync(path.join(appRoot, 'renderer', 'pairing.html'), '<html></html>');
  fs.writeFileSync(path.join(appRoot, 'renderer', 'board.html'), '<html></html>');
  fs.writeFileSync(path.join(appRoot, 'dist', 'preload', 'index.js'), '"use strict";');
  fs.writeFileSync(path.join(appRoot, 'dist', 'renderer', 'overlay', 'index.js'), 'export {};');
  fs.writeFileSync(path.join(appRoot, 'dist', 'renderer', 'pairing', 'index.js'), 'export {};');
  fs.writeFileSync(path.join(appRoot, 'dist', 'renderer', 'board', 'index.js'), 'export {};');

  const runtimePaths = resolveRuntimePaths({ userDataPath, baseDir });

  assert.equal(runtimePaths.configPath, path.join(userDataPath, 'config.json'));
  assert.equal(runtimePaths.appIconPath, path.join(userDataPath, 'runtime-assets', 'icon.png'));
  assert.equal(runtimePaths.preloadScriptPath, path.join(userDataPath, 'runtime-assets', 'dist', 'preload', 'index.js'));
  assert.equal(runtimePaths.overlayHtmlPath, path.join(userDataPath, 'runtime-assets', 'renderer', 'overlay.html'));
  assert.equal(runtimePaths.pairingHtmlPath, path.join(userDataPath, 'runtime-assets', 'renderer', 'pairing.html'));
  assert.equal(runtimePaths.boardHtmlPath, path.join(userDataPath, 'runtime-assets', 'renderer', 'board.html'));

  assert.equal(fs.existsSync(runtimePaths.appIconPath), true);
  assert.equal(fs.existsSync(runtimePaths.preloadScriptPath), true);
  assert.equal(fs.existsSync(path.join(userDataPath, 'runtime-assets', 'dist', 'renderer', 'overlay', 'index.js')), true);
  assert.equal(fs.existsSync(path.join(userDataPath, 'runtime-assets', 'dist', 'renderer', 'board', 'index.js')), true);

  fs.rmSync(fixtureRoot, { recursive: true, force: true });
});
