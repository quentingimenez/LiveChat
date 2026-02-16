import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildIngestPayload, sendToIngest } from './ingestClient';
import type { ExtensionSettings } from './settings';

const settings: ExtensionSettings = {
  apiUrl: 'https://bot.example.com',
  ingestToken: 'ingest-token',
  guildId: '1234567890',
  authorName: 'LiveChat Extension',
  authorImage: 'https://cdn.example.com/avatar.png',
};

describe('ingest client', () => {
  afterEach(() => {
    vi.restoreAllMocks();
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
      authorName: 'LiveChat Extension',
      authorImage: 'https://cdn.example.com/avatar.png',
    });
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
      authorName: 'LiveChat Extension',
      authorImage: 'https://cdn.example.com/avatar.png',
    });
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
});
