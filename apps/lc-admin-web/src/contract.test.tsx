// Contract tests: every screen is rendered from responses produced by the real
// lc-server router (apps/lc-server/src/http/admin/tests_fixtures.rs writes
// src/api/fixtures/*.json, and fails when the server shape drifts from them).
//
// These tests do not use hand-written payloads: if the console reads a field
// the server does not send, or misreads a type, the rendered text shows it
// ("undefined", "NaN", "Invalid Date", "[object Object]") or the seeded values
// are missing from the page.

import { cleanup, render, screen, waitFor } from '@testing-library/preact';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './app';

import audit from './api/fixtures/audit.json';
import guildDetail from './api/fixtures/guild-detail.json';
import guildDetailStandalone from './api/fixtures/guild-detail-standalone.json';
import health from './api/fixtures/health.json';
import ingestAuthors from './api/fixtures/ingest-authors.json';
import ingestions from './api/fixtures/ingestions.json';
import mediaBoardItems from './api/fixtures/media-board-items.json';
import overview from './api/fixtures/overview.json';
import pairingCodes from './api/fixtures/pairing-codes.json';
import rights from './api/fixtures/rights.json';
import runtimeSettings from './api/fixtures/runtime-settings.json';
import session from './api/fixtures/session.json';
import sessionUnauthorized from './api/fixtures/session-unauthorized.json';

const GUILD = guildDetail.id;
const STANDALONE = guildDetailStandalone.id;

// Values seeded by tests_fixtures.rs.
const GUILD_NAME = 'Les Copains du Jeudi';
const STANDALONE_NAME = 'Studio LiveChat';

const BAD_TEXT = ['undefined', 'NaN', 'Invalid Date', '[object Object]'];

function response(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: new Headers({ 'content-type': 'application/json' }),
    text: async () => JSON.stringify(body),
    json: async () => body,
  } as unknown as Response;
}

/** Real-server responses, keyed by path without the query string. */
const ROUTES: Record<string, unknown> = {
  '/admin/auth/session': session,
  '/admin/api/overview': overview,
  [`/admin/api/guilds/${GUILD}`]: guildDetail,
  [`/admin/api/guilds/${STANDALONE}`]: guildDetailStandalone,
  [`/admin/api/guilds/${GUILD}/media-board/items`]: mediaBoardItems,
  '/admin/api/health': health,
  '/admin/api/ingestions': ingestions,
  '/admin/api/audit': audit,
  '/admin/api/pairing-codes': pairingCodes,
  '/admin/api/ingest-authors': ingestAuthors,
  '/admin/api/runtime-settings': runtimeSettings,
  '/admin/api/rights': rights,
};

const requested: string[] = [];

function serve(routes: Record<string, unknown>) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://console.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      requested.push(`${method} ${url.pathname}`);
      if (method === 'GET' && url.pathname in routes) {
        return response(routes[url.pathname]);
      }
      // Routes not covered by fixtures yet (rights, events, mutations).
      return response({ error: 'not_found' }, 404);
    }),
  );
}

function pageText(): string {
  return document.body.textContent ?? '';
}

function expectCleanText() {
  const text = pageText();
  for (const bad of BAD_TEXT) {
    expect(text, `page shows "${bad}"`).not.toContain(bad);
  }
}

async function visit(hash: string, expected: string[]) {
  window.location.hash = hash;
  window.dispatchEvent(new Event('hashchange'));
  for (const value of expected) {
    await waitFor(() => expect(pageText(), `${hash} shows "${value}"`).toContain(value), {
      timeout: 3000,
    });
  }
  expectCleanText();
}

beforeEach(() => {
  requested.length = 0;
  window.scrollTo = vi.fn();
  // EventSource is not in jsdom: the console must fall back to polling.
  vi.stubGlobal('EventSource', undefined);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  window.location.hash = '';
});

describe('console against real server responses', () => {
  it('renders the top-level screens with the seeded data', async () => {
    serve(ROUTES);
    window.location.hash = '#/dashboard';
    render(<App />);

    // Health shows short French values, not the server's raw detail.
    await visit('#/dashboard', [GUILD_NAME, 'Accessible', 'Aucun échec sur 1 h']);
    expect(pageText()).not.toContain('database reachable');
    // 1 active pairing code + 2 active invitations (ABCDEFGH and the admin
    // invitation of the standalone guild).
    const codesTile = Array.from(document.querySelectorAll('.tile')).find((tile) =>
      tile.textContent?.includes('Codes actifs'),
    );
    expect(codesTile?.querySelector('.value')?.textContent).toBe('3');
    await visit('#/guilds', [GUILD_NAME, STANDALONE_NAME]);
    await visit('#/devices', ['PC-Salon', 'PC-Bureau', 'PC-Portable']);
    await visit('#/codes', ['JKLMNPQR']);
    await visit('#/ingest', ['Extension Firefox']);
    await visit('#/media', [GUILD_NAME]);
    await visit('#/roles', ['Lancer une lecture', 'Gérer les membres']);
    await visit('#/activity', ['guild.settings']);
    await visit('#/settings', []);
  });

  it('renders every guild tab with the seeded data', async () => {
    serve(ROUTES);
    window.location.hash = `#/guilds/${GUILD}/overview`;
    render(<App />);

    await visit(`#/guilds/${GUILD}/overview`, [GUILD_NAME]);
    await visit(`#/guilds/${GUILD}/members`, ['Quentin', 'Maëlle', 'Théo']);
    await visit(`#/guilds/${GUILD}/devices`, ['PC-Salon', 'PC-Portable']);
    await visit(`#/guilds/${GUILD}/access`, ['ABCD-EFGH']);
    await visit(`#/guilds/${GUILD}/board`, ['Clip du but', 'Applaudissements', 'Mème']);
    await visit(`#/guilds/${GUILD}/queue`, ['Bonne soirée à tous']);
    await visit(`#/guilds/${GUILD}/settings`, []);
    await visit(`#/guilds/${GUILD}/discord`, [GUILD]);
    await visit(`#/guilds/${GUILD}/danger`, [GUILD]);
  });

  it('renders the standalone guild', async () => {
    serve(ROUTES);
    window.location.hash = `#/guilds/${STANDALONE}/overview`;
    render(<App />);
    await visit(`#/guilds/${STANDALONE}/overview`, [STANDALONE_NAME]);
    await visit(`#/guilds/${STANDALONE}/discord`, [STANDALONE_NAME]);
  });

  it('offers exactly the login methods of the real 401 body', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => response(sessionUnauthorized, 401)),
    );
    window.location.hash = '#/dashboard';
    render(<App />);
    // The test server only enables the secret login.
    expect(await screen.findByLabelText('Secret d’administration')).toBeTruthy();
    expect(screen.queryByText(/Se connecter avec Discord/)).toBeNull();
    expectCleanText();
  });
});
