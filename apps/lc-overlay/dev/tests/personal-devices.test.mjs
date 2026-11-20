// Protects the wording and affordances of the personal device list in
// "Mon profil": a member must recognise each device and never be offered to
// revoke the session displaying the dialog.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  canRevokePersonalDevice,
  personalDeviceCountLabel,
  personalDeviceKindLabel,
  personalDeviceSummary,
} from '../../web/app/core/personal-devices.js';

const NOW = Date.parse('2026-11-20T12:00:00.000Z');
const PROFILE = readFileSync(
  new URL('../../web/app/views/profile.js', import.meta.url),
  'utf8',
);

test('kind labels distinguish PC, extension and iOS shortcut', () => {
  assert.equal(personalDeviceKindLabel('overlay'), 'PC');
  assert.equal(personalDeviceKindLabel('extension'), 'Extension');
  assert.equal(personalDeviceKindLabel('ios'), 'Raccourci iOS');
  assert.equal(personalDeviceKindLabel('inconnu'), 'Appareil');
  assert.equal(personalDeviceKindLabel(undefined), 'Appareil');
});

test('an overlay summary shows platform, version and presence', () => {
  assert.equal(
    personalDeviceSummary(
      {
        kind: 'overlay',
        platform: 'windows',
        overlayVersion: '3.7.0',
        connected: true,
        lastSeenAt: '2026-11-20T11:59:00.000Z',
      },
      NOW,
    ),
    'Windows · v3.7.0 · En ligne',
  );
});

test('an overlay summary falls back to last seen and tolerates missing fields', () => {
  assert.equal(
    personalDeviceSummary(
      {
        kind: 'overlay',
        connected: false,
        lastSeenAt: '2026-11-20T11:55:00.000Z',
      },
      NOW,
    ),
    'Vu il y a 5 min',
  );
  assert.equal(personalDeviceSummary({ kind: 'overlay' }, NOW), 'Jamais vu');
});

test('an ingest client summary only reports its last use', () => {
  assert.equal(
    personalDeviceSummary(
      { kind: 'extension', lastSeenAt: '2026-11-20T09:00:00.000Z' },
      NOW,
    ),
    'Vu il y a 3 h',
  );
  assert.equal(personalDeviceSummary({ kind: 'ios' }, NOW), 'Jamais vu');
});

test('the count is pluralised and never negative', () => {
  assert.equal(personalDeviceCountLabel(0), 'Aucun appareil');
  assert.equal(personalDeviceCountLabel(1), '1 appareil');
  assert.equal(personalDeviceCountLabel(2), '2 appareils');
  assert.equal(personalDeviceCountLabel(-3), 'Aucun appareil');
});

test('the calling device and unknown rows are never revocable', () => {
  assert.equal(canRevokePersonalDevice({ kind: 'overlay', isSelf: true }), false);
  assert.equal(canRevokePersonalDevice({ kind: 'overlay', isSelf: false }), true);
  assert.equal(canRevokePersonalDevice({ kind: 'ingest' }), true);
  assert.equal(canRevokePersonalDevice(null), false);
});

test('the profile dialog wires the list and both revoke routes', () => {
  assert.match(PROFILE, /api\('GET', '\/overlay\/devices'\)/);
  assert.match(
    PROFILE,
    /\/overlay\/devices\/\$\{device\.kind === 'overlay' \? 'overlay' : 'ingest'\}\/\$\{encodeURIComponent\(device\.id\)\}/,
  );
  assert.match(PROFILE, /canRevokePersonalDevice\(device\)/);
  assert.match(PROFILE, /loadPersonalDevices\(\)/);
});

test('revocation confirms inside the row instead of replacing the profile dialog', () => {
  const start = PROFILE.indexOf('const personalDeviceRow');
  const end = PROFILE.indexOf('const loadPersonalDevices');
  assert.ok(start >= 0 && end > start, 'personalDeviceRow must stay before loadPersonalDevices');
  const row = PROFILE.slice(start, end);
  assert.match(row, /profile-personal-confirm/);
  assert.match(row, /'Révoquer \?'/);
  assert.match(row, /'Annuler'/);
  // confirmDialog would replace the whole dialog root, closing "Mon profil".
  assert.doesNotMatch(row, /confirmDialog/);
});
