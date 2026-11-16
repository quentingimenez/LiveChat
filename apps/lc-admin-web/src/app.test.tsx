import { render, screen, waitFor } from '@testing-library/preact';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from './app';
import {
  auditFixture,
  guildDetailFixture,
  guildFixture,
  healthFixture,
  overviewFixture,
} from './test-fixtures';

const session = {
  method: 'secret' as const,
  user: { id: '1', name: 'Quentin', avatarUrl: null },
  expiresAt: '2026-11-22T12:00:00.000Z',
};

function response(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    text: async () => JSON.stringify(body),
  } as unknown as Response;
}

function mockAdminApi() {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === '/admin/auth/session') {
      return response(session);
    }
    if (url === '/admin/api/overview') {
      return response(overviewFixture);
    }
    if (url === '/admin/api/health') {
      return response(healthFixture);
    }
    if (url === '/admin/api/ingestions?limit=50') {
      return response({ items: [] });
    }
    if (url.startsWith('/admin/api/audit')) {
      return response({ items: auditFixture, nextBefore: null });
    }
    if (url === `/admin/api/guilds/${guildFixture.id}`) {
      return response(guildDetailFixture);
    }
    return response({ error: 'not_found' }, 404);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = '';
});

describe('App', () => {
  it('mounts the authenticated dashboard without crashing', async () => {
    mockAdminApi();
    window.scrollTo = vi.fn();

    render(<App />);

    expect(await screen.findByRole('heading', { name: 'Tableau de bord' })).toBeTruthy();
    expect((await screen.findAllByText('Les Zigotos du Dimanche')).length).toBeGreaterThan(0);
    expect(screen.getByText('Base SQLite')).toBeTruthy();
    await waitFor(() => expect(screen.getByText('Le chat qui tombe du canapé')).toBeTruthy());
  });

  it('renders every guild tab without crashing', async () => {
    mockAdminApi();
    window.scrollTo = vi.fn();
    window.location.hash = `#/guilds/${guildFixture.id}/overview`;

    render(<App />);

    expect(
      await screen.findByRole('heading', { name: 'Les Zigotos du Dimanche' }),
    ).toBeTruthy();

    for (const tab of [
      'members',
      'devices',
      'access',
      'board',
      'queue',
      'settings',
      'discord',
      'danger',
      'overview',
    ]) {
      window.location.hash = `#/guilds/${guildFixture.id}/${tab}`;
      window.dispatchEvent(new Event('hashchange'));
      await waitFor(() =>
        expect(
          screen.getByRole('heading', { name: 'Les Zigotos du Dimanche' }),
        ).toBeTruthy(),
      );
    }
  });

  it('renders every top-level screen without crashing', async () => {
    mockAdminApi();
    window.scrollTo = vi.fn();
    window.location.hash = '#/dashboard';

    render(<App />);
    expect(await screen.findByRole('heading', { name: 'Tableau de bord' })).toBeTruthy();

    const screens: Array<[string, string]> = [
      ['guilds', 'Guildes'],
      ['devices', 'Overlays'],
      ['codes', 'Codes et invitations'],
      ['ingest', 'Clients d’ingestion'],
      ['roles', 'Rôles et droits'],
      ['media', 'Médias et stockage'],
      ['activity', 'Journal'],
      ['settings', 'Paramètres du serveur'],
      ['dashboard', 'Tableau de bord'],
    ];

    for (const [hash, title] of screens) {
      window.location.hash = `#/${hash}`;
      window.dispatchEvent(new Event('hashchange'));
      await waitFor(() => expect(document.title).toContain(title));
    }
  });

  it('shows the login screen when the session is missing', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        response(
          { error: 'unauthorized', methods: { discord: true, secret: true } },
          401,
        ),
      ),
    );

    render(<App />);

    expect(await screen.findByRole('heading', { name: 'Connexion' })).toBeTruthy();
    expect(screen.getByRole('link', { name: /Se connecter avec Discord/ })).toBeTruthy();
  });
});
