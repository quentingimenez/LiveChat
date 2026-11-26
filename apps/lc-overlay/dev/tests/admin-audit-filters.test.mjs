// Journal filters of the administration view. The overlay has no DOM at test
// time, so these assertions pin the source contract: a text query over the
// displayed date/origin/author/action, origin and action selects, a reset, and
// in-place filtering (rows hidden from their dataset, never a re-render, so
// the focused search field keeps its focus).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const source = readFileSync(new URL('../../web/app/views/admin.js', import.meta.url), 'utf8');
const css = readFileSync(new URL('../../web/app/styles/admin.css', import.meta.url), 'utf8');

// One function of admin.js, up to the next top-level declaration.
function functionSource(name) {
  const start = source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, `missing ${name}`);
  const next = source.indexOf('\nfunction ', start + 10);
  return source.slice(start, next > 0 ? next : undefined);
}

test('the journal rows expose their searchable fields on the dataset', () => {
  const audit = functionSource('renderAudit');
  assert.match(audit, /row\.dataset\.auditSearch = auditSearchText\(/, audit.slice(0, 500));
  assert.match(audit, /row\.dataset\.auditOrigin = origin/);
  assert.match(audit, /row\.dataset\.auditAction = entry\.action/);
  assert.match(audit, /absoluteDate\(entry\.at\)/);
  assert.match(audit, /auditActionLabel\(entry\.action\)/);
});

test('the text query ignores case and accents', () => {
  const normalize = functionSource('auditFilterText');
  assert.match(normalize, /normalize\('NFD'\)/, normalize);
  assert.match(normalize, /replace\(\/\[\\u0300-\\u036f\]\/g, ''\)/, normalize);
  assert.match(normalize, /toLowerCase\(\)/, normalize);
  const matches = functionSource('matchesAuditFilters');
  assert.match(matches, /entry\.search\.includes\(needle\)/, matches);
});

test('the journal offers origin and action selects and a reset button', () => {
  const controls = functionSource('auditFilterControls');
  assert.match(controls, /'Toutes les origines'/);
  assert.match(controls, /{ value: 'Overlay' }/);
  assert.match(controls, /{ value: 'Panneau serveur' }/);
  assert.match(controls, /'Toutes les actions'/);
  assert.match(controls, /auditActionLabel\(id\)/);
  assert.match(controls, /'Réinitialiser'/);
  assert.match(controls, /type: 'search'/);
  assert.match(controls, /'Réinitialiser'[\s\S]*onClick: onReset|onClick: onReset[\s\S]*'Réinitialiser'/);
});

test('filtering happens in place, from the dataset, without a re-render', () => {
  const filter = functionSource('filterAuditRows');
  assert.match(filter, /row\.dataset\.auditSearch/, filter);
  assert.match(filter, /row\.dataset\.auditOrigin/);
  assert.match(filter, /row\.dataset\.auditAction/);
  assert.match(filter, /row\.hidden = !match/);
  assert.doesNotMatch(filter, /renderView|loadAdmin|renderList/, filter);
});

test('the search field, the selects and the reset are wired without re-render', () => {
  const audit = functionSource('renderAudit');
  assert.match(audit, /query\.addEventListener\('input', apply\)/);
  assert.match(audit, /origin\.addEventListener\('change', apply\)/);
  assert.match(audit, /action\.addEventListener\('change', apply\)/);
  const reset = functionSource('resetAuditFilters');
  assert.match(reset, /query\.value = ''/);
  assert.match(reset, /origin\.value = ''/);
  assert.match(reset, /action\.value = ''/);
  assert.match(reset, /refresh\(\)/);
  assert.match(reset, /query\.focus\(\)/);
});

test('the counter follows the filter and an empty result is readable', () => {
  const filter = functionSource('filterAuditRows');
  assert.match(filter, /empty\.hidden = visible > 0/);
  assert.match(filter, /counter\.textContent/);
  assert.match(filter, /sur \$\{total\}/);
  const controls = functionSource('auditFilterControls');
  assert.match(controls, /'Aucun résultat pour ces filtres\.'/);
  assert.match(controls, /role: 'status'/);
  assert.match(controls, /audit-count/);
});

test('the filter toolbar has a wrapping row style', () => {
  assert.match(css, /\.audit-filters\s*\{[\s\S]*?flex-wrap:\s*wrap/, css.slice(css.indexOf('.audit-filters')));
  assert.match(css, /\.audit-search\s*\{[\s\S]*?flex:\s*1 1 /);
});
