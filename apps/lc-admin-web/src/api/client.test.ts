import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ApiError,
  createGuild,
  getGuild,
  getOverview,
  isUnavailable,
  loginWithSecret,
  setUnauthorizedHandler,
  updateGuildSettings,
} from './client';

function mockResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    text: async () => (body === undefined ? '' : JSON.stringify(body)),
  } as unknown as Response;
}

let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  setUnauthorizedHandler(null);
});

describe('API client', () => {
  it('parses JSON and sends same-origin cookie credentials', async () => {
    fetchMock.mockResolvedValue(mockResponse({ generatedAt: '2026-11-15T12:00:00.000Z' }));

    const overview = await getOverview();

    expect(overview.generatedAt).toBe('2026-11-15T12:00:00.000Z');
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/admin/api/overview');
    expect(init).toMatchObject({ method: 'GET', credentials: 'same-origin' });
  });

  it('carries the server error code on failures', async () => {
    fetchMock.mockResolvedValue(mockResponse({ error: 'invalid_payload' }, 400));

    let caught: unknown;
    try {
      await updateGuildSettings('42', { name: 'x' });
    } catch (error) {
      caught = error;
    }

    expect(caught).toBeInstanceOf(ApiError);
    expect(caught).toMatchObject({ status: 400, code: 'invalid_payload' });
  });

  it('flags 404 and 405 responses as unavailable', async () => {
    fetchMock.mockResolvedValueOnce(mockResponse({ error: 'guild_not_found' }, 404));
    const notFound = await getGuild('42').catch((error: unknown) => error);
    expect(isUnavailable(notFound)).toBe(true);

    fetchMock.mockResolvedValueOnce(mockResponse({ error: 'method_not_allowed' }, 405));
    const notAllowed = await getGuild('42').catch((error: unknown) => error);
    expect(isUnavailable(notAllowed)).toBe(true);

    fetchMock.mockResolvedValueOnce(mockResponse({ error: 'internal' }, 500));
    const internal = await getGuild('42').catch((error: unknown) => error);
    expect(isUnavailable(internal)).toBe(false);
  });

  it('calls the unauthorized handler on a later 401', async () => {
    const handler = vi.fn();
    setUnauthorizedHandler(handler);
    fetchMock.mockResolvedValue(mockResponse({ error: 'unauthorized' }, 401));

    await expect(getOverview()).rejects.toMatchObject({ status: 401 });
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it('does not redirect on a wrong login secret', async () => {
    const handler = vi.fn();
    setUnauthorizedHandler(handler);
    fetchMock.mockResolvedValue(mockResponse({ error: 'invalid_secret' }, 401));

    await expect(loginWithSecret('wrong')).rejects.toMatchObject({ code: 'invalid_secret' });
    expect(handler).not.toHaveBeenCalled();
  });

  it('sends JSON with the right content type on mutations', async () => {
    fetchMock.mockResolvedValue(mockResponse({ created: true }));

    await createGuild({ name: 'Soirée jeux' });

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit & { headers: Record<string, string> }];
    expect(url).toBe('/admin/api/guilds');
    expect(init.method).toBe('POST');
    expect(init.credentials).toBe('same-origin');
    expect(init.headers).toMatchObject({ 'Content-Type': 'application/json' });
    expect(JSON.parse(String(init.body))).toEqual({ name: 'Soirée jeux' });
  });

  it('accepts 204 responses without a body', async () => {
    fetchMock.mockResolvedValue(mockResponse(undefined, 204));
    await expect(loginWithSecret('good')).resolves.toBeUndefined();
  });
});
