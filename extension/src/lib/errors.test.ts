import { describe, expect, it } from 'vitest';
import { parseIngestErrorBody, mapHttpFailure, mapNetworkFailure } from './errors';

const RAW_YTDLP =
  'ERROR: [youtube] 6GvU1WyGMok: Sign in to confirm your age. This video may be ' +
  'inappropriate for some users. Use --cookies-from-browser or --cookies for the ' +
  'authentication. See https://github.com/yt-dlp/yt-dlp/wiki/FAQ';

describe('parseIngestErrorBody', () => {
  it('returns empty object for non-record values', () => {
    expect(parseIngestErrorBody(null)).toEqual({});
    expect(parseIngestErrorBody(undefined)).toEqual({});
    expect(parseIngestErrorBody('string')).toEqual({});
    expect(parseIngestErrorBody(42)).toEqual({});
    expect(parseIngestErrorBody([])).toEqual({});
  });

  it('extracts only the machine fields, never the server message', () => {
    const result = parseIngestErrorBody({
      error: 'media_ingestion_failed',
      code: 'PRIVATE_OR_AUTH_REQUIRED',
      message: RAW_YTDLP
    });

    expect(result).toEqual({
      error: 'media_ingestion_failed',
      code: 'PRIVATE_OR_AUTH_REQUIRED'
    });
    expect(result).not.toHaveProperty('message');
  });

  it('returns undefined for empty or whitespace-only fields', () => {
    const result = parseIngestErrorBody({
      error: '',
      code: '   ',
      message: null
    });

    expect(result.error).toBeUndefined();
    expect(result.code).toBeUndefined();
  });

  it('ignores non-string field values', () => {
    const result = parseIngestErrorBody({
      error: 123,
      code: true,
      message: {}
    });

    expect(result.error).toBeUndefined();
    expect(result.code).toBeUndefined();
  });
});

describe('mapHttpFailure', () => {
  it('maps 400 to INVALID_PAYLOAD', () => {
    const result = mapHttpFailure(400, {});
    expect(result.code).toBe('INVALID_PAYLOAD');
    expect(result.status).toBe(400);
  });

  it('maps invalid_payload error label to INVALID_PAYLOAD', () => {
    const result = mapHttpFailure(200, { error: 'invalid_payload' });
    expect(result.code).toBe('INVALID_PAYLOAD');
  });

  it('maps 401 to UNAUTHORIZED', () => {
    const result = mapHttpFailure(401, {});
    expect(result.code).toBe('UNAUTHORIZED');
  });

  it('maps an expired ingest token to a re-pairing message', () => {
    const result = mapHttpFailure(401, { error: 'ingest_token_expired' });
    expect(result.code).toBe('UNAUTHORIZED');
    expect(result.message).toContain('Ré-appairez');
  });

  it('maps 422 to a local message for the legacy media code', () => {
    const result = mapHttpFailure(422, {
      error: 'media_ingestion_failed',
      code: 'PRIVATE_OR_AUTH_REQUIRED',
      message: RAW_YTDLP
    });

    expect(result.code).toBe('MEDIA_INGESTION_FAILED');
    expect(result.message).toBe('Ce média est privé ou demande une connexion.');
    expect(result.details).toBe('PRIVATE_OR_AUTH_REQUIRED');
  });

  it('never surfaces the raw server message, even with an unknown media code', () => {
    const result = mapHttpFailure(422, {
      error: 'media_ingestion_failed',
      code: 'SOMETHING_NEW',
      message: RAW_YTDLP
    });

    expect(result.message).toBe('Le média n’a pas pu être ingéré par le serveur.');
    expect(result.message).not.toContain('ERROR:');
    expect(result.message).not.toContain('yt-dlp');
    expect(result.message).not.toContain('cookies-from-browser');
  });

  it('maps ingest_api_disabled to INGEST_DISABLED', () => {
    const result = mapHttpFailure(503, { error: 'ingest_api_disabled' });
    expect(result.code).toBe('INGEST_DISABLED');
  });

  it('maps ingest_unavailable to SERVER_ERROR with a temporary message', () => {
    const result = mapHttpFailure(503, { error: 'ingest_unavailable' });
    expect(result.code).toBe('SERVER_ERROR');
    expect(result.message).toContain('momentanément indisponible');
  });

  it('maps guild_mismatch to INVALID_CONFIG', () => {
    const result = mapHttpFailure(403, { error: 'guild_mismatch' });
    expect(result.code).toBe('INVALID_CONFIG');
  });

  it('maps 500+ to SERVER_ERROR', () => {
    const result = mapHttpFailure(500, {});
    expect(result.code).toBe('SERVER_ERROR');
  });

  it('maps 429 to UNKNOWN with a rate-limit message', () => {
    const result = mapHttpFailure(429, {});
    expect(result.code).toBe('UNKNOWN');
    expect(result.message).toContain('Trop d’envois');
  });

  it('maps an unrecognized status to UNKNOWN without echoing the body', () => {
    const result = mapHttpFailure(418, { message: 'I am a teapot' });
    expect(result.code).toBe('UNKNOWN');
    expect(result.message).toBe('Erreur serveur inattendue (418).');
    expect(result.message).not.toContain('teapot');
  });

  it('uses a status-based fallback for an unrecognized status without body', () => {
    const result = mapHttpFailure(418, {});
    expect(result.code).toBe('UNKNOWN');
    expect(result.message).toContain('418');
  });
});

describe('mapNetworkFailure', () => {
  it('maps AbortError to TIMEOUT', () => {
    const abort = new DOMException('Aborted', 'AbortError');
    const result = mapNetworkFailure(abort);
    expect(result.code).toBe('TIMEOUT');
  });

  it('maps "failed to fetch" errors to NETWORK_ERROR', () => {
    const result = mapNetworkFailure(new Error('Failed to fetch'));
    expect(result.code).toBe('NETWORK_ERROR');
    expect(result.message).toContain('contacter');
  });

  it('maps "networkerror" errors to NETWORK_ERROR', () => {
    const result = mapNetworkFailure(new TypeError('NetworkError when attempting to fetch'));
    expect(result.code).toBe('NETWORK_ERROR');
  });

  it('maps unknown errors to NETWORK_ERROR with details', () => {
    const result = mapNetworkFailure(new Error('something unexpected'));
    expect(result.code).toBe('NETWORK_ERROR');
    expect(result.details).toBe('something unexpected');
  });

  it('handles non-Error values', () => {
    const result = mapNetworkFailure('just a string');
    expect(result.code).toBe('NETWORK_ERROR');
  });

  it('handles null/undefined', () => {
    const result = mapNetworkFailure(null);
    expect(result.code).toBe('NETWORK_ERROR');
  });
});
