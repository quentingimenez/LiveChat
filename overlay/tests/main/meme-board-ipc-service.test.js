const test = require('node:test');
const assert = require('node:assert/strict');

const { createMemeBoardIpcService } = require('../../dist/main/services/meme-board-ipc-service.js');

test('meme-board-ipc-service returns normalized bindings from config', () => {
  const service = createMemeBoardIpcService({
    loadConfig: () => ({
      memeBindings: {
        'Ctrl+1': 'item-a',
      },
    }),
    normalizeMemeBindings: (candidate) => {
      const entries = candidate && typeof candidate === 'object' ? Object.entries(candidate) : [];
      return Object.fromEntries(entries.filter(([key, value]) => key && value));
    },
    applyMemeBindings() {
      return { ok: true, appliedBindings: {}, failedAccelerators: [] };
    },
    emitMemeTriggerSignal() {
      return { ok: true };
    },
    emitManualStopSignal() {
      return { ok: true };
    },
  });

  assert.deepEqual(service.getBindings(), {
    bindings: {
      'Ctrl+1': 'item-a',
    },
  });
});

test('meme-board-ipc-service applies bindings in strict persistent mode', () => {
  const applyCalls = [];

  const service = createMemeBoardIpcService({
    loadConfig: () => ({ memeBindings: {} }),
    normalizeMemeBindings: (candidate) => {
      if (!candidate || typeof candidate !== 'object') {
        return {};
      }

      return Object.fromEntries(Object.entries(candidate));
    },
    applyMemeBindings: (nextBindings, options) => {
      applyCalls.push({ nextBindings, options });

      return {
        ok: true,
        appliedBindings: nextBindings,
        failedAccelerators: ['Ctrl+9'],
      };
    },
    emitMemeTriggerSignal() {
      return { ok: true };
    },
    emitManualStopSignal() {
      return { ok: true };
    },
  });

  const result = service.setBindings({
    bindings: {
      'Ctrl+2': 'item-b',
    },
  });

  assert.deepEqual(applyCalls[0], {
    nextBindings: {
      'Ctrl+2': 'item-b',
    },
    options: {
      strict: true,
      persist: true,
    },
  });

  assert.deepEqual(result, {
    ok: true,
    bindings: {
      'Ctrl+2': 'item-b',
    },
    failedAccelerators: ['Ctrl+9'],
  });
});

test('meme-board-ipc-service forwards trigger and stop requests', () => {
  const triggerCalls = [];
  let stopCalls = 0;

  const service = createMemeBoardIpcService({
    loadConfig: () => ({ memeBindings: {} }),
    normalizeMemeBindings: () => ({}),
    applyMemeBindings() {
      return { ok: true, appliedBindings: {}, failedAccelerators: [] };
    },
    emitMemeTriggerSignal: (itemId, trigger) => {
      triggerCalls.push({ itemId, trigger });
      return { ok: true };
    },
    emitManualStopSignal: () => {
      stopCalls += 1;
      return { ok: true, reason: 'manual_stop' };
    },
  });

  const triggerResult = service.trigger({
    itemId: 'item-42',
    trigger: 'button',
  });
  const stopResult = service.stop();

  assert.deepEqual(triggerCalls, [
    {
      itemId: 'item-42',
      trigger: 'button',
    },
  ]);
  assert.equal(stopCalls, 1);
  assert.deepEqual(triggerResult, { ok: true });
  assert.deepEqual(stopResult, { ok: true, reason: 'manual_stop' });
});
