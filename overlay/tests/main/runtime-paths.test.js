const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');

const { resolveRuntimePaths } = require('../../dist/main/app/runtime-paths.js');

test('runtime-paths resolves runtime assets from baseDir and userDataPath', () => {
  const userDataPath = '/tmp/livechat-user';
  const baseDir = '/workspace/app/dist/main/app';

  const runtimePaths = resolveRuntimePaths({ userDataPath, baseDir });

  assert.deepEqual(runtimePaths, {
    configPath: path.join(userDataPath, 'config.json'),
    appIconPath: path.join(baseDir, '../../../icon.png'),
    preloadScriptPath: path.join(baseDir, '../../../preload.js'),
    overlayHtmlPath: path.join(baseDir, '../../../renderer/overlay.html'),
    pairingHtmlPath: path.join(baseDir, '../../../renderer/pairing.html'),
    boardHtmlPath: path.join(baseDir, '../../../renderer/board.html')
  });
});
