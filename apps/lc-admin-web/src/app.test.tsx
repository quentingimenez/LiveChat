import { fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from './app';
import runtimeSettingsFixture from './api/fixtures/runtime-settings.json';
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

function mockAdminApi(userName = session.user.name) {
  const currentSession = { ...session, user: { ...session.user, name: userName } };
  const fetchMock = vi.fn(async (input: RequestInfo | URL, _init?: RequestInit) => {
    const url = String(input);
    if (url === '/admin/auth/session') {
      return response(currentSession);
    }
    if (url === '/admin/api/overview') {
      return response(overviewFixture);
    }
    if (url === '/admin/api/health') {
      return response(healthFixture);
    }
    if (url === '/admin/api/runtime-settings') {
      return response(runtimeSettingsFixture);
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
    if (url.endsWith('/members/member-2/ban')) {
      return response({ banned: true, ban: guildDetailFixture.bans.items[0] });
    }
    if (url.endsWith('/bans/ban-1')) {
      return response({ unbanned: true });
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

  it('draws one initial when the signed-in name starts with an astral letter', async () => {
    mockAdminApi('𝐒𝐢𝐦𝐨𝐧');
    window.scrollTo = vi.fn();

    render(<App />);

    const name = await screen.findByText('𝐒𝐢𝐦𝐨𝐧');
    const avatar = name.closest('.user')?.querySelector('.avatar');
    expect(avatar?.textContent).toBe('S');
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

  it('bans a member and lifts a ban from the members tab', async () => {
    const fetchMock = mockAdminApi();
    window.scrollTo = vi.fn();
    window.location.hash = `#/guilds/${guildFixture.id}/members`;

    render(<App />);

    // The banned person is listed with who banned them.
    expect(await screen.findByText('Troll')).toBeTruthy();
    expect(screen.getByText('999999999999999999')).toBeTruthy();

    const banButtons = await screen.findAllByRole('button', { name: 'Bannir' });
    fireEvent.click(banButtons[1]);
    expect(await screen.findByText('Bannir Yanis ?')).toBeTruthy();
    // Yanis has no Discord account: the dialog says a ban cannot keep them out.
    expect(screen.getByText(/Sans compte Discord lié/)).toBeTruthy();
    const confirmBan = screen.getAllByRole('button', { name: 'Bannir' }).at(-1);
    fireEvent.click(confirmBan as HTMLElement);
    await waitFor(() =>
      expect(
        fetchMock.mock.calls.some(
          ([url, init]) =>
            String(url) === `/admin/api/guilds/${guildFixture.id}/members/member-2/ban` &&
            (init as RequestInit | undefined)?.method === 'POST',
        ),
      ).toBe(true),
    );

    await waitFor(() => expect(screen.queryByText('Bannir Yanis ?')).toBeNull());

    fireEvent.click(screen.getByRole('button', { name: 'Lever' }));
    expect(await screen.findByText('Lever le bannissement de Troll ?')).toBeTruthy();
    fireEvent.click(screen.getAllByRole('button', { name: 'Lever' }).at(-1) as HTMLElement);
    await waitFor(() =>
      expect(
        fetchMock.mock.calls.some(
          ([url, init]) =>
            String(url) === `/admin/api/guilds/${guildFixture.id}/bans/ban-1` &&
            (init as RequestInit | undefined)?.method === 'DELETE',
        ),
      ).toBe(true),
    );
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

  it('saves the YouTube cookie from the settings screen', async () => {
    const fetchMock = mockAdminApi();
    window.scrollTo = vi.fn();
    window.location.hash = '#/settings';

    render(<App />);

    const field = await screen.findByLabelText('Cookie YouTube');
    fireEvent.input(field, { target: { value: 'SAPISID=runtime-secret' } });
    await waitFor(() =>
      expect(
        (screen.getByRole('button', { name: 'Enregistrer les paramètres' }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Enregistrer les paramètres' }));

    await waitFor(() => {
      const patch = fetchMock.mock.calls.find(
        ([url, init]) =>
          String(url) === '/admin/api/runtime-settings' &&
          (init as RequestInit | undefined)?.method === 'PATCH',
      );
      expect(patch).toBeTruthy();
      expect(JSON.parse(String((patch?.[1] as RequestInit | undefined)?.body))).toMatchObject({
        youtubeCookie: 'SAPISID=runtime-secret',
      });
    });
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
