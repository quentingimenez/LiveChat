const test = require('node:test');
const assert = require('node:assert/strict');

const { createAutoStartRuntimeService } = require('../../dist/main/services/auto-start-runtime-service.js');

test('auto-start-runtime-service disables auto-start when login item API is unavailable', () => {
  const saved = [];
  const service = createAutoStartRuntimeService({
    autoStartService: {
      supportsAutoStart: () => true,
      getSystemAutoStartEnabled: () => true,
      applyAutoStartSetting: () => true
    },
    canUseLoginItemApi: false,
    saveAutoStartConfig: (enabled) => {
      saved.push(enabled);
    }
  });

  assert.equal(service.supportsAutoStart(), false);
  assert.equal(service.getSystemAutoStartEnabled(), false);
  assert.equal(service.applyAutoStartSetting(true), false);
  assert.deepEqual(saved, [false]);
});

test('auto-start-runtime-service delegates to auto-start service when supported', () => {
  const calls = [];
  const service = createAutoStartRuntimeService({
    autoStartService: {
      supportsAutoStart: () => true,
      getSystemAutoStartEnabled: () => {
        calls.push('inspect');
        return true;
      },
      applyAutoStartSetting: (enabled) => {
        calls.push(`apply:${enabled}`);
        return enabled === true;
      }
    },
    canUseLoginItemApi: true,
    saveAutoStartConfig: () => {
      calls.push('save');
    }
  });

  assert.equal(service.supportsAutoStart(), true);
  assert.equal(service.getSystemAutoStartEnabled(), true);
  assert.equal(service.applyAutoStartSetting(true), true);
  assert.deepEqual(calls, ['inspect', 'apply:true']);
});
