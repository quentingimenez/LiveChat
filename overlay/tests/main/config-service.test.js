const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { createConfigService } = require('../../dist/main/services/config-service.js');

function createTempConfigPath() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'livechat-config-'));
  return path.join(dir, 'config.json');
}

test('config-service loadConfig returns defaults when file does not exist', () => {
  const svc = createConfigService({ configPath: '/tmp/nonexistent-config-test.json' });
  const config = svc.loadConfig();

  assert.equal(config.volume, 1);
  assert.equal(config.enabled, true);
  assert.equal(config.autoStart, false);
  assert.equal(config.guestMode, false);
  assert.equal(config.showText, true);
  assert.equal(config.serverUrl, null);
  assert.equal(config.clientToken, null);
  assert.equal(config.guildId, null);
  assert.deepEqual(config.memeBindings, {});
});

test('config-service loadConfig reads and normalizes a saved config', () => {
  const configPath = createTempConfigPath();

  fs.writeFileSync(configPath, JSON.stringify({
    volume: 0.5,
    autoStart: true,
    guestMode: true,
    serverUrl: 'https://example.com',
    clientToken: 'tok123',
    guildId: 'g1',
    memeBindings: { 'Ctrl+1': 'item-abc' }
  }));

  const svc = createConfigService({ configPath });
  const config = svc.loadConfig();

  assert.equal(config.volume, 0.5);
  assert.equal(config.autoStart, true);
  assert.equal(config.guestMode, true);
  assert.equal(config.serverUrl, 'https://example.com');
  assert.equal(config.clientToken, 'tok123');
  assert.equal(config.guildId, 'g1');
  assert.deepEqual(config.memeBindings, { 'Ctrl+1': 'item-abc' });

  fs.rmSync(path.dirname(configPath), { recursive: true, force: true });
});

test('config-service loadConfig returns defaults for malformed JSON', () => {
  const configPath = createTempConfigPath();
  fs.writeFileSync(configPath, 'not json');

  let readError = null;
  const svc = createConfigService({
    configPath,
    onReadError: (err) => { readError = err; }
  });

  const config = svc.loadConfig();
  assert.equal(config.volume, 1);
  assert.notEqual(readError, null);

  fs.rmSync(path.dirname(configPath), { recursive: true, force: true });
});

test('config-service saveConfig writes and returns normalized config', () => {
  const configPath = createTempConfigPath();

  const svc = createConfigService({ configPath });
  const saved = svc.saveConfig({ volume: 0.7, autoStart: true, guildId: 'g2' });

  assert.equal(saved.volume, 0.7);
  assert.equal(saved.autoStart, true);
  assert.equal(saved.guildId, 'g2');

  const raw = JSON.parse(fs.readFileSync(configPath, 'utf-8'));
  assert.equal(raw.volume, 0.7);
  assert.equal(raw.autoStart, true);

  fs.rmSync(path.dirname(configPath), { recursive: true, force: true });
});

test('config-service normalizeVolume clamps between 0 and 1', () => {
  const svc = createConfigService({ configPath: '/tmp/unused.json' });

  assert.equal(svc.normalizeVolume(0.5), 0.5);
  assert.equal(svc.normalizeVolume(0), 0);
  assert.equal(svc.normalizeVolume(1), 1);
  assert.equal(svc.normalizeVolume(-0.5), 0);
  assert.equal(svc.normalizeVolume(1.5), 1);
  assert.equal(svc.normalizeVolume(NaN), 1);
  assert.equal(svc.normalizeVolume('invalid'), 1);
  assert.equal(svc.normalizeVolume(null), 1);
  assert.equal(svc.normalizeVolume(Infinity), 1);
});

test('config-service normalizeAutoStart only accepts true', () => {
  const svc = createConfigService({ configPath: '/tmp/unused.json' });

  assert.equal(svc.normalizeAutoStart(true), true);
  assert.equal(svc.normalizeAutoStart(false), false);
  assert.equal(svc.normalizeAutoStart(1), false);
  assert.equal(svc.normalizeAutoStart('true'), false);
  assert.equal(svc.normalizeAutoStart(null), false);
});

test('config-service normalizeGuestMode only accepts true', () => {
  const svc = createConfigService({ configPath: '/tmp/unused.json' });

  assert.equal(svc.normalizeGuestMode(true), true);
  assert.equal(svc.normalizeGuestMode(false), false);
  assert.equal(svc.normalizeGuestMode(1), false);
  assert.equal(svc.normalizeGuestMode(null), false);
});

test('config-service normalizeMemeBindings filters invalid entries', () => {
  const svc = createConfigService({ configPath: '/tmp/unused.json' });

  assert.deepEqual(svc.normalizeMemeBindings({ 'Ctrl+1': 'item-1', 'Ctrl+2': 'item-2' }), {
    'Ctrl+1': 'item-1',
    'Ctrl+2': 'item-2'
  });
  assert.deepEqual(svc.normalizeMemeBindings({ '': 'item', 'Ctrl+1': '' }), {});
  assert.deepEqual(svc.normalizeMemeBindings(null), {});
  assert.deepEqual(svc.normalizeMemeBindings('string'), {});
  assert.deepEqual(svc.normalizeMemeBindings([]), {});
});

test('config-service normalizeServerUrl trims and removes trailing slashes', () => {
  const svc = createConfigService({ configPath: '/tmp/unused.json' });

  assert.equal(svc.normalizeServerUrl('https://example.com/'), 'https://example.com');
  assert.equal(svc.normalizeServerUrl('https://example.com///'), 'https://example.com');
  assert.equal(svc.normalizeServerUrl('  https://example.com  '), 'https://example.com');
  assert.equal(svc.normalizeServerUrl(''), '');
});

test('config-service hasPairingConfig checks required fields', () => {
  const svc = createConfigService({ configPath: '/tmp/unused.json' });

  assert.equal(svc.hasPairingConfig({
    ...svc.defaultConfig,
    serverUrl: 'https://example.com',
    clientToken: 'tok',
    guildId: 'g1'
  }), true);

  assert.equal(svc.hasPairingConfig({
    ...svc.defaultConfig,
    serverUrl: 'https://example.com',
    clientToken: null,
    guildId: 'g1'
  }), false);

  assert.equal(svc.hasPairingConfig(svc.defaultConfig), false);
});

test('config-service isGuestModeEnabled checks guestMode flag', () => {
  const svc = createConfigService({ configPath: '/tmp/unused.json' });

  assert.equal(svc.isGuestModeEnabled({ ...svc.defaultConfig, guestMode: true }), true);
  assert.equal(svc.isGuestModeEnabled({ ...svc.defaultConfig, guestMode: false }), false);
  assert.equal(svc.isGuestModeEnabled(svc.defaultConfig), false);
});
