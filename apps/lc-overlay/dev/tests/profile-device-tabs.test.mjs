// Onglets « Mes appareils » / « Ajouter un appareil » du profil.
//
// Régression : les générateurs de code occupaient la même page que la liste des
// appareils, allongeant le dialogue et son défilement. Ils vivent désormais dans
// un second panneau, masqué par défaut, et la liste des appareils ouvre le
// dialogue. Sans DOM dans les dépendances de l'overlay, le test couvre les
// helpers de sélection et les invariants de structure (profile.js) et de style
// (dialog.css) qui portent ce comportement.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { selectTab, tabIndexForKey } from '../../web/app/core/tabs.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

// Declarations of the first rule whose selector list contains `selector`.
function declarationsOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    const selectors = match[1]
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split(',')
      .map((part) => part.trim());
    if (selectors.includes(selector)) {
      return match[2];
    }
  }
  assert.fail(`règle ${selector} introuvable`);
}

const profile = read('views/profile.js');
const css = read('styles/dialog.css');

function fakeTab() {
  const attributes = new Map();
  return {
    hidden: false,
    setAttribute(name, value) {
      attributes.set(name, String(value));
    },
    getAttribute(name) {
      return attributes.has(name) ? attributes.get(name) : null;
    },
  };
}

function snapshot(tabs, panels) {
  return {
    tabs: tabs.map((tab) => [tab.getAttribute('aria-selected'), tab.getAttribute('tabindex')]),
    panels: panels.map((panel) => panel.hidden),
  };
}

test('selectTab rend un seul onglet et un seul panneau visibles', () => {
  const tabs = [fakeTab(), fakeTab()];
  const panels = [fakeTab(), fakeTab()];

  selectTab(tabs, panels, 1);
  assert.deepEqual(tabs.map((tab) => tab.getAttribute('aria-selected')), ['false', 'true']);
  assert.deepEqual(tabs.map((tab) => tab.getAttribute('tabindex')), ['-1', '0']);
  assert.deepEqual(panels.map((panel) => panel.hidden), [true, false]);

  selectTab(tabs, panels, 0);
  assert.deepEqual(tabs.map((tab) => tab.getAttribute('aria-selected')), ['true', 'false']);
  assert.deepEqual(tabs.map((tab) => tab.getAttribute('tabindex')), ['0', '-1']);
  assert.deepEqual(panels.map((panel) => panel.hidden), [false, true]);
});

test('selectTab ignore un index hors bornes', () => {
  const tabs = [fakeTab(), fakeTab()];
  const panels = [fakeTab(), fakeTab()];
  selectTab(tabs, panels, 0);
  const before = snapshot(tabs, panels);

  selectTab(tabs, panels, -1);
  selectTab(tabs, panels, 2);

  assert.deepEqual(snapshot(tabs, panels), before);
});

test('le clavier boucle et couvre les flèches, Home et End', () => {
  assert.equal(tabIndexForKey(0, 'ArrowRight', 2), 1);
  assert.equal(tabIndexForKey(1, 'ArrowRight', 2), 0);
  assert.equal(tabIndexForKey(0, 'ArrowLeft', 2), 1);
  assert.equal(tabIndexForKey(1, 'ArrowLeft', 2), 0);
  assert.equal(tabIndexForKey(0, 'ArrowDown', 2), 1);
  assert.equal(tabIndexForKey(1, 'ArrowUp', 2), 0);
  assert.equal(tabIndexForKey(1, 'Home', 2), 0);
  assert.equal(tabIndexForKey(0, 'End', 2), 1);
  assert.equal(tabIndexForKey(0, 'Enter', 2), null);
  assert.equal(tabIndexForKey(0, 'Tab', 2), null);
});

test('un seul onglet ne bouge pas et un groupe vide ne répond pas', () => {
  assert.equal(tabIndexForKey(0, 'ArrowRight', 1), 0);
  assert.equal(tabIndexForKey(0, 'ArrowLeft', 1), 0);
  assert.equal(tabIndexForKey(0, 'ArrowRight', 0), null);
  assert.equal(tabIndexForKey(0, 'Home', 0), null);
});

test('la liste des appareils ouvre le dialogue et l’ajout est masqué', () => {
  assert.ok(profile.includes('let profileDialogCount = 0;'));
  assert.ok(profile.includes('const tabSetId = `profile-tabs-${++profileDialogCount}`;'));

  const devices = profile.indexOf('const devicesPanel = h(');
  const add = profile.indexOf('const addPanel = h(');
  const panels = profile.indexOf('const tabPanels');
  assert.ok(devices !== -1 && add !== -1 && panels !== -1, 'panneaux des onglets introuvables');
  assert.ok(devices < add && add < panels, 'l’ordre des panneaux a changé');
  assert.match(profile.slice(add, panels), /hidden: true/);
  assert.ok(profile.includes('selectDeviceTab(0);'));

  const firstTab = profile.indexOf("'Mes appareils'");
  const secondTab = profile.indexOf("'Ajouter un appareil'");
  assert.ok(firstTab !== -1 && secondTab !== -1, 'libellés des onglets introuvables');
  assert.ok(firstTab < secondTab, '« Mes appareils » doit être le premier onglet');
});

test('les onglets exposent les rôles et les liens ARIA attendus', () => {
  assert.ok(profile.includes("role: 'tablist'"));
  assert.ok(profile.includes("'aria-label': 'Appareils'"));
  assert.ok(profile.includes("role: 'tab'"));
  assert.ok(profile.includes("role: 'tabpanel'"));
  assert.ok(profile.includes('id: `${tabSetId}-devices`'));
  assert.ok(profile.includes('id: `${tabSetId}-add`'));
  assert.ok(profile.includes("'aria-labelledby': `${tabSetId}-devices-tab`"));
  assert.ok(profile.includes("'aria-labelledby': `${tabSetId}-add-tab`"));
  assert.ok(profile.includes("'aria-controls': `${tabSetId}-devices`"));
  assert.ok(profile.includes("'aria-controls': `${tabSetId}-add`"));
  assert.ok(profile.includes("'aria-selected': 'true'"));
  assert.ok(profile.includes("'aria-selected': 'false'"));
  assert.ok(profile.includes("tabindex: '0'"));
  assert.ok(profile.includes("tabindex: '-1'"));
});

test('les flèches changent d’onglet et déplacent le focus', () => {
  assert.ok(profile.includes('onClick: () => selectDeviceTab(0)'));
  assert.ok(profile.includes('onClick: () => selectDeviceTab(1)'));
  assert.ok(profile.includes('onKeydown: (event) => onDeviceTabKey(event, 0)'));
  assert.ok(profile.includes('onKeydown: (event) => onDeviceTabKey(event, 1)'));
  assert.ok(profile.includes('event.preventDefault();'));
  assert.ok(profile.includes('tabButtons[next].focus();'));
});

test('les onglets et le panneau masqué ont leur style', () => {
  const bar = declarationsOf(css, '.profile-dialog .profile-tabs');
  assert.match(bar, /display:\s*flex/);
  assert.match(bar, /gap:\s*4px/);
  assert.match(bar, /background:\s*var\(--slate-900\)/);

  const tab = declarationsOf(css, '.profile-dialog .profile-tab');
  assert.match(tab, /flex:\s*1/);
  assert.match(tab, /justify-content:\s*center/);
  assert.match(tab, /color:\s*var\(--text-soft\)/);

  const active = declarationsOf(css, ".profile-dialog .profile-tab[aria-selected='true']");
  assert.match(active, /background:\s*var\(--slate-600\)/);
  assert.match(active, /color:\s*var\(--text\)/);

  // Le survol de `.btn.is-quiet` est plus spécifique que l'état sélectionné :
  // la règle de survol dédiée doit reprendre la même surface.
  const activeHover = declarationsOf(css, ".profile-dialog .profile-tab[aria-selected='true']:hover");
  assert.match(activeHover, /background:\s*var\(--slate-600\)/);

  const hidden = declarationsOf(css, '.profile-dialog .profile-tabpanel[hidden]');
  assert.match(hidden, /display:\s*none/);
});
