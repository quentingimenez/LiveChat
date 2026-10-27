const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');

const { resolveRuntimePaths } = require('../../dist/main/app/runtime-paths.js');

function loadPreloadWithElectronMock(mockElectron) {
  const preloadModulePath = require.resolve('../../dist/preload/index.js');
  const originalLoad = Module._load;

  delete require.cache[preloadModulePath];

  Module._load = function patchedLoad(request, parent, isMain) {
    if (request === 'electron') {
      return mockElectron;
    }

    return originalLoad.call(this, request, parent, isMain);
  };

  try {
    require(preloadModulePath);
  } finally {
    Module._load = originalLoad;
  }
}

test('preload runtime exposes livechatOverlay API and forwards IPC channels', async () => {
  const invokeCalls = [];
  const sendCalls = [];
  const listenersByChannel = new Map();
  let exposedApi = null;
  let exposedName = '';

  const mockElectron = {
    contextBridge: {
      exposeInMainWorld(name, api) {
        exposedName = name;
        exposedApi = api;
      }
    },
    ipcRenderer: {
      invoke(channel, payload) {
        invokeCalls.push({ channel, payload });
        return Promise.resolve({ ok: true, channel });
      },
      send(channel, payload) {
        sendCalls.push({ channel, payload });
      },
      on(channel, listener) {
        const listeners = listenersByChannel.get(channel) || [];
        listeners.push(listener);
        listenersByChannel.set(channel, listeners);
      },
      removeListener(channel, listener) {
        const listeners = listenersByChannel.get(channel) || [];
        listenersByChannel.set(
          channel,
          listeners.filter((current) => current !== listener)
        );
      }
    }
  };

  loadPreloadWithElectronMock(mockElectron);

  assert.equal(exposedName, 'livechatOverlay');
  assert.ok(exposedApi);

  await exposedApi.getConfig();
  await exposedApi.consumePairing({ serverUrl: 'https://api.livechat.test', code: 'ABCD' });
  await exposedApi.getMemeBindings();
  await exposedApi.setMemeBindings({ bindings: { 'Ctrl+1': 'item-1' } });
  await exposedApi.triggerMeme({ itemId: 'item-1', trigger: 'ui' });
  await exposedApi.stopMemePlayback();

  assert.deepEqual(
    invokeCalls.map((call) => call.channel),
    [
      'overlay:get-config',
      'pairing:consume',
      'meme-board:get-bindings',
      'meme-board:set-bindings',
      'meme-board:trigger',
      'meme-board:stop'
    ]
  );
  assert.deepEqual(invokeCalls[1], {
    channel: 'pairing:consume',
    payload: {
      serverUrl: 'https://api.livechat.test',
      code: 'ABCD'
    }
  });

  exposedApi.rendererReady();
  exposedApi.reportError({ jobId: 'job-1', code: 'ERR', message: 'error' });
  exposedApi.reportPlaybackState({ jobId: 'job-1', state: 'playing', remainingMs: 2000 });
  exposedApi.reportPlaybackStop({ jobId: 'job-1' });

  assert.deepEqual(
    sendCalls.map((call) => call.channel),
    ['overlay:renderer-ready', 'overlay:error', 'overlay:playback-state', 'overlay:playback-stop']
  );

  let playPayload = null;
  const unsubscribePlay = exposedApi.onPlay((payload) => {
    playPayload = payload;
  });
  assert.equal((listenersByChannel.get('overlay:play') || []).length, 1);

  listenersByChannel.get('overlay:play')[0](null, { jobId: 'job-55' });
  assert.deepEqual(playPayload, { jobId: 'job-55' });

  unsubscribePlay();
  assert.equal((listenersByChannel.get('overlay:play') || []).length, 0);
});

test('runtime-paths resolves a preload file that exists in local dist build', () => {
  const baseDir = path.resolve('dist/main/app');
  const runtimePaths = resolveRuntimePaths({
    userDataPath: '/tmp/livechat-user',
    baseDir
  });

  assert.equal(runtimePaths.preloadScriptPath, path.join('/tmp/livechat-user', 'runtime-assets', 'dist', 'preload', 'index.js'));
  assert.equal(fs.existsSync(runtimePaths.preloadScriptPath), true);
});
