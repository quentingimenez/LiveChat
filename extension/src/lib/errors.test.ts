import { describe, expect, it } from 'vitest';
import { parseIngestErrorBody, mapHttpFailure, mapNetworkFailure } from './errors';

describe('parseIngestErrorBody', () => {
  it('returns empty object for non-record values', () => {
    expect(parseIngestErrorBody(null)).toEqual({});
    expect(parseIngestErrorBody(undefined)).toEqual({});
    expect(parseIngestErrorBody('string')).toEqual({});
    expect(parseIngestErrorBody(42)).toEqual({});
    expect(parseIngestErrorBody([])).toEqual({});
  });

  it('extracts error, code and message from a valid body', () => {
    const result = parseIngestErrorBody({
      error: 'invalid_payload',
      code: 'ERR_001',
      message: 'Something went wrong',
    });

    expect(result.error).toBe('invalid_payload');
    expect(result.code).toBe('ERR_001');
    expect(result.message).toBe('Something went wrong');
  });

  it('returns undefined for empty or whitespace-only fields', () => {
    const result = parseIngestErrorBody({
      error: '',
      code: '   ',
      message: null,
    });

    expect(result.error).toBeUndefined();
    expect(result.code).toBeUndefined();
    expect(result.message).toBeUndefined();
  });

  it('ignores non-string field values', () => {
    const result = parseIngestErrorBody({
      error: 123,
      code: true,
      message: {},
    });

    expect(result.error).toBeUndefined();
    expect(result.code).toBeUndefined();
    expect(result.message).toBeUndefined();
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

  it('maps unauthorized error label to UNAUTHORIZED', () => {
    const result = mapHttpFailure(200, { error: 'unauthorized' });
    expect(result.code).toBe('UNAUTHORIZED');
  });

  it('maps 422 to MEDIA_INGESTION_FAILED', () => {
    const result = mapHttpFailure(422, { message: 'video too large' });
    expect(result.code).toBe('MEDIA_INGESTION_FAILED');
    expect(result.message).toBe('video too large');
  });

  it('maps 503 to INGEST_DISABLED', () => {
    const result = mapHttpFailure(503, {});
    expect(result.code).toBe('INGEST_DISABLED');
  });

  it('maps 500+ to SERVER_ERROR', () => {
    const result = mapHttpFailure(500, {});
    expect(result.code).toBe('SERVER_ERROR');
  });

  it('maps unrecognized status to UNKNOWN', () => {
    const result = mapHttpFailure(418, { message: 'I am a teapot' });
    expect(result.code).toBe('UNKNOWN');
    expect(result.message).toBe('I am a teapot');
  });

  it('uses fallback message for unrecognized status without body message', () => {
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
