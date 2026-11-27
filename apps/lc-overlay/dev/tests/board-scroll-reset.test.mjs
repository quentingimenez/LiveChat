// Protège le routeur de la vue contre le défilement reporté d'un onglet à
// l'autre : ouvrir les réglages ou un autre onglet de la guilde doit repartir
// du haut, tandis qu'une repeinte de la vue courante garde la position. Le
// faux DOM ne reproduit que le comportement du navigateur qui compte ici :
// remplacer les enfants d'un conteneur ne remet pas `scrollTop` à zéro.
//
// Chaque fichier de test tourne dans son propre processus (node --test isole
// les fichiers), donc les globaux définis ici ne fuient pas dans les autres.

import assert from 'node:assert/strict';
import { test } from 'node:test';

class FakeNode {
  constructor(tag) {
    this.tagName = String(tag).toUpperCase();
    this.children = [];
    this.style = {};
    this.dataset = {};
    this.attributes = {};
    this.listeners = {};
    this.className = '';
    this.textContent = '';
    this.value = '';
    this.checked = false;
    this.disabled = false;
    this.hidden = false;
    this.scrollTop = 0;
    this.scrollHeight = 0;
    this.clientHeight = 0;
    this.clientWidth = 0;
    this.isConnected = true;
    this.parentElement = null;
    this.classList = fakeClassList();
  }

  append(...nodes) {
    for (const node of nodes) {
      if (node && typeof node === 'object') {
        node.parentElement = this;
      }
      this.children.push(node);
    }
  }

  appendChild(node) {
    this.append(node);
    return node;
  }

  replaceChildren(...nodes) {
    this.children = [];
    this.append(...nodes);
  }

  insertBefore(node) {
    this.append(node);
  }

  remove() {}

  setAttribute(name, value) {
    this.attributes[name] = value;
    if (name === 'class') {
      this.className = String(value);
    }
  }

  removeAttribute(name) {
    delete this.attributes[name];
  }

  getAttribute(name) {
    return this.attributes[name] ?? null;
  }

  addEventListener(type, handler) {
    (this.listeners[type] ||= []).push(handler);
  }

  removeEventListener() {}

  dispatchEvent() {
    return true;
  }

  closest(selector) {
    let node = this;
    while (node) {
      if (node.matches?.(selector)) return node;
      node = node.parentElement;
    }
    return null;
  }

  matches(selector) {
    if (!selector.startsWith('.')) return false;
    const name = selector.slice(1);
    return this.classList.contains(name) || String(this.className).split(/\s+/).includes(name);
  }

  querySelectorAll(selector) {
    const found = [];
    const walk = (node) => {
      for (const child of node.children || []) {
        if (child.matches?.(selector)) found.push(child);
        walk(child);
      }
    };
    walk(this);
    return found;
  }

  querySelector(selector) {
    return this.querySelectorAll(selector)[0] || null;
  }

  focus() {}

  blur() {}

  click() {}

  scrollIntoView() {}

  scrollTo() {}

  getBoundingClientRect() {
    return { width: 0, height: 0, top: 0, left: 0 };
  }

  setPointerCapture() {}

  releasePointerCapture() {}

  get firstElementChild() {
    return this.children.find((child) => child instanceof FakeNode) || null;
  }
}

function fakeClassList() {
  const names = new Set();
  return {
    add: (...values) => values.forEach((value) => names.add(value)),
    remove: (...values) => values.forEach((value) => names.delete(value)),
    toggle: (value, force) => {
      const on = force === undefined ? !names.has(value) : force;
      if (on) names.add(value);
      else names.delete(value);
      return on;
    },
    contains: (value) => names.has(value),
  };
}

const view = new FakeNode('main');
view.className = 'view';

globalThis.window = globalThis;
globalThis.Node = FakeNode;
globalThis.document = {
  getElementById: (id) => (id === 'view' ? view : new FakeNode('div')),
  querySelector: () => null,
  querySelectorAll: () => [],
  createElement: (tag) => new FakeNode(tag),
  createElementNS: (namespace, tag) => new FakeNode(tag),
  createTextNode: (text) => {
    const node = new FakeNode('#text');
    node.textContent = String(text);
    return node;
  },
  addEventListener() {},
  removeEventListener() {},
  documentElement: new FakeNode('html'),
  head: new FakeNode('head'),
  body: new FakeNode('body'),
  hasFocus: () => true,
  hidden: false,
  visibilityState: 'visible',
  activeElement: null,
};
globalThis.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
globalThis.sessionStorage = globalThis.localStorage;
globalThis.matchMedia = () => ({ matches: false, addEventListener() {}, addListener() {} });
globalThis.requestAnimationFrame = () => 0;
globalThis.cancelAnimationFrame = () => {};
globalThis.ResizeObserver = class {
  observe() {}
  disconnect() {}
};
globalThis.IntersectionObserver = class {
  observe() {}
  disconnect() {}
};
globalThis.MutationObserver = class {
  observe() {}
  disconnect() {}
};
Object.defineProperty(globalThis, 'navigator', {
  configurable: true,
  value: {
    userAgent: 'node',
    language: 'fr',
    languages: ['fr'],
    clipboard: { writeText: async () => {} },
  },
});
globalThis.location = { href: 'http://localhost/', reload() {}, search: '' };
globalThis.HTMLElement = class {};
globalThis.CustomEvent = class {
  constructor(type, init) {
    Object.assign(this, init);
    this.type = type;
  }
};
globalThis.Event = class {
  constructor(type, init) {
    Object.assign(this, init);
    this.type = type;
  }
};
globalThis.getComputedStyle = () => ({ paddingLeft: '0', paddingRight: '0', getPropertyValue: () => '' });
globalThis.addEventListener = () => {};
globalThis.removeEventListener = () => {};
globalThis.innerWidth = 1200;
globalThis.innerHeight = 900;
globalThis.fetch = async () => ({ ok: true, json: async () => ({}) });

const { state } = await import('../../web/app/core/state.js');
const { renderView } = await import('../../web/app/views/view.js');
const { loadDeviceSettings } = await import('../../web/app/views/settings.js');

// No shell bridge here: this records the settings error once, so the renders
// below stay synchronous instead of queueing a settings load each time.
await loadDeviceSettings();

function resetState() {
  state.page = 'guild';
  state.activeGuildId = 'g1';
  state.tab = 'board';
  state.info = {
    guild: { id: 'g1', name: 'Guilde test', role: 'member' },
    permissions: { viewBoard: true, viewQueue: true, manageGuild: false, admin: false },
  };
  state.infoLoaded = true;
  state.items = [];
  state.itemsLoaded = true;
  state.queue = { playing: null, pending: [], at: 0 };
  state.queueLoaded = true;
  state.members = [];
  state.devices = [];
  state.invites = [];
  state.auditEntries = [];
  state.bindings = {};
  state.profile = { displayName: 'Test', avatar: null };
  state.guilds = [{ membership: { guildId: 'g1', guildName: 'Guilde test' } }];
  view.scrollTop = 0;
}

function showBoard() {
  resetState();
  renderView();
}

test('ouvrir la page Réglages depuis la board repart du haut', () => {
  showBoard();
  view.scrollTop = 800;
  state.page = 'settings';
  renderView();
  assert.equal(view.querySelector('.board'), null);
  assert.equal(view.scrollTop, 0);
});

test('ouvrir les réglages de la guilde depuis la board repart du haut', () => {
  showBoard();
  view.scrollTop = 800;
  state.tab = 'settings';
  renderView();
  assert.equal(view.querySelector('.board'), null);
  assert.equal(view.scrollTop, 0);
});

test('repeindre la board garde la position de lecture', () => {
  showBoard();
  view.scrollTop = 800;
  renderView();
  assert.equal(view.scrollTop, 800);
});

test('changer de guilde repart du haut, même vers une autre board', () => {
  showBoard();
  view.scrollTop = 800;
  state.activeGuildId = 'g2';
  state.info = {
    guild: { id: 'g2', name: 'Guilde deux', role: 'member' },
    permissions: { viewBoard: true, viewQueue: true, manageGuild: false, admin: false },
  };
  state.guilds = [
    { membership: { guildId: 'g1', guildName: 'Guilde test' } },
    { membership: { guildId: 'g2', guildName: 'Guilde deux' } },
  ];
  renderView();
  assert.notEqual(view.querySelector('.board'), null);
  assert.equal(view.scrollTop, 0);
});

test('changer de section d’administration repart du haut', () => {
  showBoard();
  state.info.permissions = {
    viewBoard: true,
    viewQueue: true,
    manageGuild: true,
    admin: true,
    manageMembers: true,
    manageInvites: true,
    manageSettings: true,
  };
  state.tab = 'admin';
  state.adminTab = 'members';
  renderView();
  assert.equal(view.scrollTop, 0);
  view.scrollTop = 400;
  state.adminTab = 'devices';
  renderView();
  assert.equal(view.scrollTop, 0);
});

test('repeindre les réglages garde la position de défilement', () => {
  resetState();
  state.tab = 'settings';
  renderView();
  view.scrollTop = 320;
  renderView();
  assert.equal(view.scrollTop, 320);
});
