// Layout rule of the administration lists (members, invites, bans). Each row is its
// own grid, so a content-sized track gets a different width in a row that has
// no button in it (the current member has no « Exclure ») and the columns no
// longer line up from row to row.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const css = readFileSync(new URL('../../web/app/styles/admin.css', import.meta.url), 'utf8');

// Every `grid-template-columns` of a rule whose selector is exactly `selector`.
function columnTemplates(selector) {
  const templates = [];
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    const selectors = match[1]
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split(',')
      .map((part) => part.trim().split(/\s+/).pop());
    const declaration = /grid-template-columns:\s*([^;]+);/.exec(match[2]);
    if (declaration && selectors.includes(selector)) {
      templates.push(declaration[1].trim());
    }
  }
  return templates;
}

for (const selector of ['.line.is-member', '.line.is-invite', '.line.is-ban']) {
  test(`${selector}: no track depends on the content of the row`, () => {
    const templates = columnTemplates(selector);
    assert.ok(templates.length > 0, `no grid template found for ${selector}`);
    for (const template of templates) {
      assert.doesNotMatch(template, /\b(auto|min-content|max-content|fit-content)\b/, template);
    }
  });
}
