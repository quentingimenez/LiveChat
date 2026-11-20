// Copy and layout of the administration views: the journal names the owner in
// French (« Propriétaire »), shows absolute dates and no raw API path; the
// rename action announces its dialog with an ellipsis; the card layout below
// 1120px labels each value, and the two destructive actions stay apart. These
// assertions read the view source on purpose: admin.js needs a DOM and cannot
// be imported by Node.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const source = read('views/admin.js');
const css = read('styles/admin.css');

test('the journal names the owner instead of « Owner »', () => {
  assert.ok(source.includes("'Propriétaire'"), 'the audit view must show « Propriétaire »');
  assert.match(source, /actor\.method === 'secret'/, 'the shared-secret actor is the owner');
  assert.ok(!source.includes("'Owner'"), 'the audit view must not show the English « Owner »');
});

test('the journal shows an absolute date, not only a relative one', () => {
  assert.match(source, /absoluteDate\(entry\.at\)/);
  assert.doesNotMatch(source, /relativeTime\(entry\.at\)/);
});

test('the journal no longer renders the raw API path', () => {
  assert.ok(!source.includes('auditTarget'), 'the path formatter must be gone');
  assert.ok(!source.includes("'Chemin'"), 'the journal has no path column');
  assert.ok(!source.includes('/admin/api/guilds'), 'no raw API path is rendered');
});

test('« Renommer » announces its dialog with an ellipsis', () => {
  assert.match(source, /'Renommer…'/);
  assert.ok(!source.includes("'Renommer'"), 'the bare label must not survive');
});

test('changing your own role asks for confirmation without disabling the control', () => {
  assert.match(source, /'Changer mon rôle'/);
  assert.match(source, /confirmDialog\([\s\S]{0,400}tone: 'neutral'/);
  assert.doesNotMatch(source, /disabled:\s*member\.isSelf/);
});

test('below 1120 px each cell shows the label of its column', () => {
  const media = css.indexOf('@media (max-width: 1120px)');
  assert.ok(media > 0, 'the card breakpoint exists');
  const desktop = /^\.list-cell\s*\{([^}]*)\}/m.exec(css);
  assert.ok(desktop, 'the cell wrapper has a desktop rule');
  assert.match(desktop[1], /display:\s*contents/, 'the wrapper stays transparent above the breakpoint');
  assert.ok(css.indexOf('.list-cell {') < media, 'the transparent wrapper is the base rule');
  const card = css.slice(media);
  assert.match(card, /\.list-head\s*\{\s*display:\s*none/, 'the column head is hidden');
  assert.match(card, /\.list-cell::before\s*\{[\s\S]*?content:\s*attr\(data-label\)/, 'each value shows its data-label');
  assert.match(card, /\.line-main \.list-cell::before,[\s\S]*?\.line-actions::before\s*\{\s*display:\s*none/, 'identity and action cells skip the redundant label');
});

test('the two destructive actions are separated by at least 16 px', () => {
  const rule = /\.line-danger\s*\{([^}]*)\}/.exec(css);
  assert.ok(rule, 'the destructive group has its own rule');
  assert.match(rule[1], /gap:\s*var\(--space-4\)/);
  assert.match(read('styles/base.css'), /--space-4:\s*16px/);
  assert.match(source, /class: 'line-danger'[\s\S]{0,300}'Exclure…'[\s\S]{0,300}'Bannir…'/);
});
