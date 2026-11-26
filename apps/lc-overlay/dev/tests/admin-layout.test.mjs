// Layout invariants of the administration lists at the board window minimum
// (960×640) and in the default device table (≥1121px). The overlay has no DOM
// at test time, so these assertions read the stylesheet: the rules themselves
// are the contract, like admin-rows.test.mjs does for the row tracks.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const css = readFileSync(new URL('../../web/app/styles/admin.css', import.meta.url), 'utf8');
const baseCss = readFileSync(new URL('../../web/app/styles/base.css', import.meta.url), 'utf8');
const adminJs = readFileSync(new URL('../../web/app/views/admin.js', import.meta.url), 'utf8');
const desktop = css.slice(0, css.indexOf('@media'));

// Body of one at-rule, braces balanced, so a rule can be looked up in the card
// breakpoint without catching the 720px override.
function atRuleBody(source, header) {
  const start = source.indexOf(header);
  assert.ok(start >= 0, `missing ${header}`);
  const open = source.indexOf('{', start);
  let depth = 0;
  for (let index = open; index < source.length; index += 1) {
    if (source[index] === '{') {
      depth += 1;
    } else if (source[index] === '}') {
      depth -= 1;
      if (depth === 0) {
        return source.slice(open + 1, index);
      }
    }
  }
  throw new Error(`unclosed ${header}`);
}

// Every rule of a stylesheet fragment, with its selector list.
function rules(source) {
  const found = [];
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(source)) !== null) {
    const selectors = match[1]
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split(',')
      .map((part) => part.trim());
    found.push({ selectors, body: match[2] });
  }
  return found;
}

function rulesOf(source, selector) {
  return rules(source).filter((rule) => rule.selectors.includes(selector));
}

// Every declaration of the rules whose selector list contains `selector`.
function declarations(source, selector) {
  return rulesOf(source, selector).flatMap((rule) =>
    rule.body
      .split(';')
      .map((line) => line.trim())
      .filter(Boolean),
  );
}

function declarationValue(source, selector, property) {
  const values = [];
  for (const rule of rulesOf(source, selector)) {
    const match = new RegExp(`(?:^|;)\\s*${property}\\s*:\\s*([^;]+)`, 'i').exec(`;${rule.body}`);
    if (match) {
      values.push(match[1].trim());
    }
  }
  return values;
}

function value(source, selector, property) {
  const values = declarationValue(source, selector, property);
  assert.ok(values.length > 0, `${selector} declares no ${property}`);
  return values[0];
}

// Top-level tracks of a grid-template-columns value: `minmax(0, 1fr) 200px`
// is three tracks, not one per `minmax(` occurrence.
function tracks(value) {
  const found = [];
  let depth = 0;
  let current = '';
  for (const char of value) {
    if (char === '(') {
      depth += 1;
    } else if (char === ')') {
      depth -= 1;
    }
    if (/\s/.test(char) && depth === 0) {
      if (current) {
        found.push(current);
        current = '';
      }
      continue;
    }
    current += char;
  }
  if (current) {
    found.push(current);
  }
  return found;
}

const media1120 = atRuleBody(css, '@media (max-width: 1120px)');
const media720 = atRuleBody(css, '@media (max-width: 720px)');

// ---------------------------------------------------------- code creation

test('codes: the two buttons stay on the fields line, bottom-aligned', () => {
  assert.equal(value(desktop, '.code-create', 'flex-wrap'), 'nowrap');
  assert.equal(value(desktop, '.code-create', 'align-items'), 'flex-end');
  assert.equal(value(desktop, '.code-fields', 'flex'), '1 1 0');
  assert.equal(value(desktop, '.code-fields', 'flex-wrap'), 'nowrap');
  assert.equal(value(desktop, '.code-fields', 'min-width'), '0');
  assert.equal(value(desktop, '.code-field', 'min-width'), '0');
  assert.equal(value(desktop, '.code-field .field', 'min-width'), '0');
  assert.equal(value(desktop, '.code-actions', 'flex'), 'none');
});

// ------------------------------------------------------- code card columns

test('codes: before measuring, the three fields spread with equal gaps', () => {
  const meta = '.line.is-code .line-meta';
  assert.equal(value(media1120, meta, 'display'), 'flex');
  assert.equal(value(media1120, meta, 'flex-wrap'), 'nowrap');
  assert.equal(value(media1120, meta, 'justify-content'), 'space-between');
  assert.equal(declarationValue(media1120, meta, 'grid-template-columns').length, 0);
  // The cells size themselves to their text instead of sharing fixed tracks:
  // a short author no longer leaves a wide empty column on its right.
  const cell = `${meta} .list-cell`;
  assert.equal(value(media1120, cell, 'flex'), '0 1 auto');
  assert.equal(value(media1120, `${cell}:not(:first-child)`, 'flex-shrink'), '0');
  assert.equal(value(desktop, '.code-author', 'min-width'), '0');
});

test('codes: the measured list aligns the three fields and splits the gutters', () => {
  const measured = '.list.has-code-columns .line.is-code .line-meta';
  assert.equal(value(media1120, measured, 'display'), 'grid');
  assert.equal(value(media1120, measured, 'justify-content'), 'space-between');
  assert.equal(value(media1120, measured, 'column-gap'), '0');
  assert.deepEqual(
    tracks(declarationValue(media1120, measured, 'grid-template-columns').at(-1)),
    ['minmax(0, var(--code-author-width))', 'var(--code-expiry-width)', 'var(--code-uses-width)'],
  );
  // The widths come from a probe that measures the widest label + value of
  // each field in the card layout, at any window width.
  assert.equal(value(desktop, '.list-cell.code-cell-probe', 'display'), 'flex');
  assert.equal(value(desktop, '.list-cell.code-cell-probe', 'position'), 'absolute');
  assert.equal(value(desktop, '.list-cell.code-cell-probe', 'width'), 'max-content');
  assert.match(adminJs, /scheduleCodeColumnMeasure\(list\)/);
  assert.match(adminJs, /classList\.add\('has-code-columns'\)/);
  assert.match(adminJs, /--code-author-width/);
  assert.match(adminJs, /code-cell-probe/);
});

test('the code card keeps its copy and revoke controls on one line', () => {
  // The shared card grid also names .line.is-code; the later, code-specific
  // rule is the one the cascade applies. 152px is the measured width of the
  // 42px copy control, the 8px gap and the 95px « Révoquer… » button.
  assert.equal(
    declarationValue(media1120, '.line.is-code', 'grid-template-columns').at(-1),
    'minmax(0, 1fr) 152px',
  );
});

test('the code author truncates on its label line, not under it', () => {
  const meta = '.line.is-code .line-meta';
  assert.equal(value(media1120, `${meta} .list-cell`, 'flex-wrap'), 'nowrap');
  assert.equal(value(media1120, `${meta} .list-cell:first-child .code-author`, 'flex'), '1 1 auto');
});

// ----------------------------------------------------- member/device cards

test('member and device cards centre their actions across both lines', () => {
  const cardAreas = rules(media1120).find(
    (rule) => rule.selectors.includes('.line.is-device') && /grid-template-areas/.test(rule.body),
  );
  assert.ok(cardAreas, 'no card areas for the device row');
  for (const selector of ['.line.is-member', '.line.is-device', '.line.is-code', '.line.is-ban', '.line.is-invite']) {
    assert.ok(cardAreas.selectors.includes(selector), `${selector} is not in the two-line card grid`);
  }
  assert.match(cardAreas.body, /'main actions'[\s\S]*'meta actions'/, cardAreas.body);
  assert.equal(value(media1120, '.line-actions', 'grid-area'), 'actions');
  assert.equal(value(media1120, '.line-actions', 'align-self'), 'center');
});

test('the 720px fallback keeps the actions on their own line', () => {
  const smallAreas = rules(media720).find(
    (rule) => rule.selectors.includes('.line.is-device') && /grid-template-areas/.test(rule.body),
  );
  assert.ok(smallAreas, 'the 720px card grid must stay');
  assert.match(smallAreas.body, /'main'[\s\S]*'meta'[\s\S]*'actions'/, smallAreas.body);
});

// ----------------------------------------------------- device version cell

test('the default device table puts the platform on a second line under the version', () => {
  assert.equal(value(desktop, '.device-version', 'display'), 'grid');
  // Two explicit columns keep the version and its optional badge on the first
  // line; the platform then spans both and lands on the second line.
  assert.equal(value(desktop, '.device-version', 'grid-template-columns'), 'auto auto');
  assert.equal(declarationValue(desktop, '.device-version', 'flex-wrap').length, 0);
  assert.equal(value(desktop, '.device-platform', 'grid-column'), '1 / -1');
});

test('device cards keep the platform beside the version', () => {
  // At the board minimum the row is a card, not the default table: the OS
  // label belongs on the version's value line, not under it.
  assert.equal(value(media1120, '.line.is-device .device-version', 'display'), 'flex');
  assert.equal(value(media1120, '.line.is-device .device-version', 'flex-wrap'), 'wrap');
  assert.equal(value(media1120, '.line.is-device .device-platform', 'grid-column'), 'auto');
});

// ---------------------------------------------------------- device actions

test('the member and device « Action » heads start at the action column', () => {
  for (const selector of ['.list-head.is-member > :last-child', '.list-head.is-device > :last-child']) {
    assert.deepEqual(declarationValue(desktop, selector, 'justify-self'), ['start'], selector);
  }
  assert.equal(declarationValue(desktop, '.line-actions', 'justify-self').length, 0);
});

test('the member and device action tracks fit their button groups', () => {
  // The member track was 320px: the three controls wrapped, the right-aligned
  // group started 149px into the track and the head sat far from it.
  assert.equal(
    value(desktop, '.line.is-member', 'grid-template-columns'),
    '30px minmax(0, 1fr) 150px 130px 326px',
  );
  assert.equal(
    value(desktop, '.line.is-device', 'grid-template-columns'),
    '30px minmax(0, 1fr) 150px 140px 130px 276px',
  );
});

// ------------------------------------------------------ card meta columns

test('device and member cards align their values on stable columns', () => {
  for (const [selector, expectedTracks] of [
    ['.line.is-device .line-meta', 3],
    ['.line.is-member .line-meta', 1],
  ]) {
    assert.equal(value(media1120, selector, 'display'), 'grid', selector);
    const columns = value(media1120, selector, 'grid-template-columns');
    assert.equal(tracks(columns).length, expectedTracks, `${selector}: ${columns}`);
    assert.doesNotMatch(columns, /\b(auto|min-content|max-content|fit-content)\b/, `${selector}: ${columns}`);
  }
});

test('device cards top-align their meta and keep the session badge on the label line', () => {
  const meta = '.line.is-device .line-meta';
  assert.equal(value(media1120, meta, 'align-items'), 'start');
  assert.equal(value(media1120, `${meta} .list-cell`, 'align-items'), 'flex-start');

  const columns = value(media1120, meta, 'grid-template-columns');
  const minimums = tracks(columns).map((track) => {
    const match = track.match(/^minmax\((\d+)px,/);
    assert.ok(match, `expected a minmax pixel track, got ${track}`);
    return Number(match[1]);
  });
  assert.equal(minimums.length, 3, columns);
  // Version must hold its label, the version and the optional badge; the
  // session badge needs about 170px to stay beside its label instead of
  // wrapping under it. Both gaps are 16px.
  assert.ok(minimums[0] >= 170, `version column too narrow: ${columns}`);
  assert.ok(minimums[2] >= 175, `session column too narrow: ${columns}`);
  assert.ok(minimums.reduce((sum, width) => sum + width, 0) + 32 <= 477, `columns overflow the card meta: ${columns}`);
  // A full-width label line keeps the version/value line at the same x on the
  // rows where the « Obsolète » badge makes the cell too wide for an inline
  // label.
  assert.equal(value(media1120, '.line.is-device .line-meta .list-cell::before', 'flex'), '0 0 100%');
});

test('the device list leaves the « Vu » prefix to its column label', () => {
  assert.match(adminJs, /presenceSinceLabel as devicePresenceLabel/);
  assert.match(adminJs, /devicePresenceLabel\(device\)/);
  assert.doesNotMatch(adminJs, /presenceLabel as devicePresenceLabel/);
});

test('the desktop device table keeps its column tracks', () => {
  assert.equal(
    value(desktop, '.line.is-device', 'grid-template-columns'),
    '30px minmax(0, 1fr) 150px 140px 130px 276px',
  );
});

// ----------------------------------------------------------- member role

test('the member role select sits on the identity line and stays compact', () => {
  const selector = '.line.is-member .member-role-select';
  assert.equal(value(media1120, selector, 'height'), '24px');
  assert.equal(value(media1120, selector, 'font-size'), 'var(--text-sm)');
  assert.equal(value(baseCss, '.field', 'height'), '32px');
  assert.equal(declarationValue(media1120, selector, 'height').length, 1, 'the compact select is card-only');
  // The control is in the identity wrapper, so the card keeps it on the first
  // line; on desktop the select is put back in the Rôle track.
  assert.match(adminJs, /class: 'list-cell member-role'/);
  const wide = atRuleBody(css, '@media (min-width: 1121px)');
  assert.equal(value(wide, selector, 'grid-column'), '4');
  assert.equal(value(wide, '.line.is-member .line-actions', 'grid-column'), '5');
  assert.equal(value(wide, '.line.is-member .line-meta .seen', 'grid-column'), '3');
  // Without an explicit row the role cell (4th in the DOM order but 4th in the
  // grid) pushes the card values onto a second desktop row.
  for (const cell of [
    '.line.is-member .avatar',
    '.line.is-member .member-cell',
    '.line.is-member .line-meta .seen',
    selector,
    '.line.is-member .line-actions',
  ]) {
    assert.equal(value(wide, cell, 'grid-row'), '1', cell);
  }
  assert.equal(value(media1120, '.line.is-member .member-role', 'margin-left'), 'auto');
  assert.equal(value(media1120, '.line.is-member .line-meta', 'grid-template-columns'), 'minmax(0, 1fr)');
});
