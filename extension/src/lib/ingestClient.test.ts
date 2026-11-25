import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { buildIngestPayload, sendToIngest } from './ingestClient';
import type { ExtensionSettings } from './settings';

const settings: ExtensionSettings = {
  apiUrl: 'https://livechat.example.com',
  ingestToken: 'ingest-token',
  guildId: '1234567890',
};

describe('ingest client', () => {
  const getPlatformInfo = vi.fn(async () => ({ os: 'win', arch: 'x86-64' }));

  beforeEach(() => {
    getPlatformInfo.mockClear();
    vi.stubGlobal('browser', { runtime: { getPlatformInfo } });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('construit le payload quick', () => {
    const payload = buildIngestPayload(
      {
        mode: 'quick',
        url: 'https://youtu.be/abc123',
      },
      settings,
    );

    expect(payload).toEqual({
      guildId: '1234567890',
      url: 'https://www.youtube.com/watch?v=abc123',
    });
  });

  it('construit le payload quick en conservant le timestamp YouTube', () => {
    const payload = buildIngestPayload(
      {
        mode: 'quick',
        url: 'https://youtu.be/abc123?si=foo&t=108',
      },
      settings,
    );

    expect(payload).toEqual({
      guildId: '1234567890',
      url: 'https://www.youtube.com/watch?v=abc123&t=108',
    });
  });

  it('envoie durationSec en mode compose quand elle est fournie', () => {
    const payload = buildIngestPayload(
      {
        mode: 'compose',
        url: 'https://x.com/livechat/status/1',
        durationSec: 8,
      },
      settings,
    );

    expect(payload?.durationSec).toBe(8);
  });

  it("n'envoie pas durationSec sans durée", () => {
    const payload = buildIngestPayload(
      {
        mode: 'compose',
        url: 'https://x.com/livechat/status/1',
      },
      settings,
    );

    expect(payload).not.toHaveProperty('durationSec');
  });

  it('construit le payload compose avec forceRefresh', () => {
    const payload = buildIngestPayload(
      {
        mode: 'compose',
        url: 'https://x.com/livechat/status/2020921090097164393?s=20',
        text: 'hello',
        forceRefresh: true,
      },
      settings,
    );

    expect(payload).toEqual({
      guildId: '1234567890',
      url: 'https://x.com/livechat/status/2020921090097164393',
      text: 'hello',
      forceRefresh: true,
      saveToBoard: false,
    });
  });

  it("n'envoie pas le texte quand le média va sur le board", () => {
    const payload = buildIngestPayload(
      {
        mode: 'compose',
        url: 'https://x.com/livechat/status/2020921090097164393',
        text: 'hello',
        saveToBoard: true,
      },
      settings,
    );

    expect(payload).not.toHaveProperty('text');
    expect(payload?.saveToBoard).toBe(true);
  });

  it('mappe une erreur 401', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ error: 'unauthorized' }), {
          status: 401,
          headers: { 'content-type': 'application/json' },
        }),
      ),
    );

    const result = await sendToIngest(
      {
        mode: 'quick',
        url: 'https://www.youtube.com/watch?v=abc123',
      },
      settings,
    );

    expect(result.ok).toBe(false);

    if (!result.ok) {
      expect(result.error.code).toBe('UNAUTHORIZED');
    }
  });

  it('ne laisse pas fuir le jeton dans les logs ni dans le message d’erreur réseau', async () => {
    const token = settings.ingestToken;
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const logSpy = vi.spyOn(console, 'log').mockImplementation(() => undefined);
    vi.stubGlobal(
      'fetch',
      vi.fn().mockRejectedValue(new Error(`Échec pour Authorization: Bearer ${token} sur ${settings.apiUrl}`)),
    );

    const result = await sendToIngest(
      {
        mode: 'quick',
        url: 'https://www.youtube.com/watch?v=abc123',
      },
      settings,
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(JSON.stringify(result.error)).not.toContain(token);
    }
    for (const spy of [errorSpy, warnSpy, logSpy]) {
      const logs = JSON.stringify(spy.mock.calls);
      expect(logs, `Le jeton apparaît dans un log console (${spy.mock.calls.length} appels)`).not.toContain(token);
    }
  });

  it('retourne un succès 201', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ accepted: true, jobId: 'job-123' }), {
          status: 201,
          headers: { 'content-type': 'application/json' },
        }),
      ),
    );

    const result = await sendToIngest(
      {
        mode: 'quick',
        url: 'https://www.youtube.com/watch?v=abc123',
      },
      settings,
    );

    expect(result.ok).toBe(true);

    if (result.ok) {
      expect(result.jobId).toBe('job-123');
    }
  });

  const jsonResponse = (status: number, body: unknown): Response =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });

  const quick = { mode: 'quick' as const, url: 'https://www.youtube.com/watch?v=abc123' };

  it('suit une ingestion acceptée en 202 jusqu’au job', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(jsonResponse(202, { accepted: true, ingestionId: 'ing-1' }))
      .mockResolvedValueOnce(jsonResponse(200, { ingestionId: 'ing-1', status: 'processing' }))
      .mockResolvedValueOnce(
        jsonResponse(200, { ingestionId: 'ing-1', status: 'ready', jobId: 'job-123456789' }),
      );
    vi.stubGlobal('fetch', fetchMock);

    const result = await sendToIngest(quick, settings, 5000, 0);

    expect(result).toEqual({
      ok: true,
      status: 200,
      jobId: 'job-123456789',
      message: 'Envoyé (job: job-1234...)',
    });
    expect(fetchMock).toHaveBeenCalledTimes(3);
    const [statusUrl, statusInit] = fetchMock.mock.calls[1];
    expect(statusUrl).toBe('https://livechat.example.com/ingest/ingestions/ing-1');
    expect(statusInit.method).toBe('GET');
    expect(statusInit.headers.Authorization).toBe('Bearer ingest-token');
  });

  it('annonce un ajout au board prêt', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(jsonResponse(202, { accepted: true, ingestionId: 'ing-2' }))
        .mockResolvedValueOnce(
          jsonResponse(200, { ingestionId: 'ing-2', status: 'ready', created: true, itemId: 'item-1' }),
        ),
    );

    const result = await sendToIngest(quick, settings, 5000, 0);

    expect(result).toEqual({ ok: true, status: 200, jobId: null, message: 'Ajouté au board LiveChat.' });
  });

  it('traduit un échec d’ingestion avec le code média', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(jsonResponse(202, { accepted: true, ingestionId: 'ing-3' }))
        .mockResolvedValueOnce(
          jsonResponse(200, {
            ingestionId: 'ing-3',
            status: 'failed',
            error: {
              code: 'media_ingestion_failed',
              message: 'yt-dlp: ERROR sign in',
              mediaCode: 'PRIVATE_OR_AUTH_REQUIRED',
            },
          }),
        ),
    );

    const result = await sendToIngest(quick, settings, 5000, 0);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error.code).toBe('MEDIA_INGESTION_FAILED');
      expect(result.error.message).toBe('Ce média est privé ou demande une connexion.');
      expect(result.error.message).not.toContain('yt-dlp');
      expect(result.error.details).toBe('PRIVATE_OR_AUTH_REQUIRED');
    }
  });

  it('signale une ingestion dont le serveur a perdu la trace', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(jsonResponse(202, { accepted: true, ingestionId: 'ing-4' }))
        .mockResolvedValueOnce(jsonResponse(404, { error: 'ingestion_not_found' })),
    );

    const result = await sendToIngest(quick, settings, 5000, 0);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error.code).toBe('SERVER_ERROR');
      expect(result.error.message).toContain('suivi');
    }
  });

  it('reste un succès quand le média est encore en préparation au délai', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(jsonResponse(202, { accepted: true, ingestionId: 'ing-5' }))
        .mockImplementation(async () => jsonResponse(200, { ingestionId: 'ing-5', status: 'processing' })),
    );

    const result = await sendToIngest(quick, settings, 50, 5);

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.jobId).toBeNull();
      expect(result.message).toContain('prépare encore');
    }
  });

  it('annonce un envoi terminé sans job ni élément', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(jsonResponse(202, { accepted: true, ingestionId: 'ing-6' }))
        .mockResolvedValueOnce(jsonResponse(200, { ingestionId: 'ing-6', status: 'ready' })),
    );

    const result = await sendToIngest(quick, settings, 5000, 0);

    expect(result).toEqual({ ok: true, status: 200, jobId: null, message: 'Envoyé vers LiveChat.' });
  });

  it('traduit une erreur HTTP pendant le suivi', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(jsonResponse(202, { accepted: true, ingestionId: 'ing-7' }))
        .mockResolvedValueOnce(jsonResponse(401, { error: 'ingest_token_expired' })),
    );

    const result = await sendToIngest(quick, settings, 5000, 0);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error.code).toBe('UNAUTHORIZED');
      expect(result.error.status).toBe(401);
    }
  });

  it('signale une activité d’extension avant chaque attente du suivi', async () => {
    vi.useFakeTimers();
    try {
      const fetchMock = vi
        .fn()
        .mockResolvedValueOnce(jsonResponse(202, { accepted: true, ingestionId: 'ing-9' }))
        .mockImplementation(async () => jsonResponse(200, { ingestionId: 'ing-9', status: 'processing' }));
      vi.stubGlobal('fetch', fetchMock);

      const pending = sendToIngest(quick, settings, 3000, 1000);
      // Chrome stops an idle service worker after 30 s: a fetch alone does not
      // count as activity, an extension API call does.
      await vi.advanceTimersByTimeAsync(0);
      expect(getPlatformInfo).toHaveBeenCalledTimes(1);
      expect(fetchMock).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(1000);
      expect(fetchMock).toHaveBeenCalledTimes(2);
      expect(getPlatformInfo).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(2000);

      expect((await pending).ok).toBe(true);
      expect(fetchMock).toHaveBeenCalledTimes(4);
      expect(getPlatformInfo).toHaveBeenCalledTimes(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it('attend l’intervalle entre deux lectures sans dépasser le délai', async () => {
    vi.useFakeTimers();
    try {
      const fetchMock = vi
        .fn()
        .mockResolvedValueOnce(jsonResponse(202, { accepted: true, ingestionId: 'ing-8' }))
        .mockImplementation(async () => jsonResponse(200, { ingestionId: 'ing-8', status: 'processing' }));
      vi.stubGlobal('fetch', fetchMock);

      const pending = sendToIngest(quick, settings, 100, 80);
      await vi.advanceTimersByTimeAsync(79);
      // Only the submission: the first read waits for the interval.
      expect(fetchMock).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(1);
      expect(fetchMock).toHaveBeenCalledTimes(2);
      // The last wait is cut to the time left before the deadline (20 ms).
      await vi.advanceTimersByTimeAsync(20);
      expect(fetchMock).toHaveBeenCalledTimes(3);

      const result = await pending;
      expect(result.ok).toBe(true);
      expect(fetchMock).toHaveBeenCalledTimes(3);
    } finally {
      vi.useRealTimers();
    }
  });
});
