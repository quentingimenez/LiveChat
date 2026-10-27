const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { syncAutoStartOnStartup } = require('../../dist/main/services/startup-auto-start-service.js');

function createTempConfig(content) {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'livechat-autostart-'));
  const configPath = path.join(tempDir, 'config.json');
  fs.writeFileSync(configPath, content, 'utf-8');

  return {
    configPath,
    cleanup() {
      fs.rmSync(tempDir, { recursive: true, force: true });
    },
  };
}

test('startup-auto-start applies persisted preference when available and supported', () => {
  const tempConfig = createTempConfig(JSON.stringify({ autoStart: false }));
  const applyCalls = [];
  const saveCalls = [];

  try {
    syncAutoStartOnStartup({
      configPath: tempConfig.configPath,
      persistedAutoStartEnabled: false,
      supportsAutoStart: true,
      getSystemAutoStartEnabled: () => true,
      applyAutoStartSetting: (enabled) => {
        applyCalls.push(enabled);
      },
      saveAutoStartConfig: (enabled) => {
        saveCalls.push(enabled);
      },
    });

    assert.deepEqual(applyCalls, [false]);
    assert.deepEqual(saveCalls, []);
  } finally {
    tempConfig.cleanup();
  }
});

test('startup-auto-start saves system value when no persisted autoStart field exists', () => {
  const tempConfig = createTempConfig(JSON.stringify({ enabled: true }));
  const applyCalls = [];
  const saveCalls = [];

  try {
    syncAutoStartOnStartup({
      configPath: tempConfig.configPath,
      persistedAutoStartEnabled: false,
      supportsAutoStart: true,
      getSystemAutoStartEnabled: () => true,
      applyAutoStartSetting: (enabled) => {
        applyCalls.push(enabled);
      },
      saveAutoStartConfig: (enabled) => {
        saveCalls.push(enabled);
      },
    });

    assert.deepEqual(applyCalls, []);
    assert.deepEqual(saveCalls, [true]);
  } finally {
    tempConfig.cleanup();
  }
});

test('startup-auto-start disables persisted preference when platform does not support auto-start', () => {
  const tempConfig = createTempConfig(JSON.stringify({ autoStart: true }));
  const saveCalls = [];

  try {
    syncAutoStartOnStartup({
      configPath: tempConfig.configPath,
      persistedAutoStartEnabled: true,
      supportsAutoStart: false,
      getSystemAutoStartEnabled: () => true,
      applyAutoStartSetting: () => {},
      saveAutoStartConfig: (enabled) => {
        saveCalls.push(enabled);
      },
    });

    assert.deepEqual(saveCalls, [false]);
  } finally {
    tempConfig.cleanup();
  }
});

test('startup-auto-start reports inspect errors and falls back to system value', () => {
  const tempConfig = createTempConfig('{invalid-json');
  const errors = [];
  const saveCalls = [];

  try {
    syncAutoStartOnStartup({
      configPath: tempConfig.configPath,
      persistedAutoStartEnabled: true,
      supportsAutoStart: true,
      getSystemAutoStartEnabled: () => false,
      applyAutoStartSetting: () => {},
      saveAutoStartConfig: (enabled) => {
        saveCalls.push(enabled);
      },
      onInspectError: (error) => {
        errors.push(error);
      },
    });

    assert.equal(errors.length, 1);
    assert.deepEqual(saveCalls, [false]);
  } finally {
    tempConfig.cleanup();
  }
});
