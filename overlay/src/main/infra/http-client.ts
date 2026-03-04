import http from 'node:http';
import https from 'node:https';

export interface HttpRequestJsonOptions {
  rejectUnauthorized?: boolean;
  timeoutMs?: number;
}

export interface HttpJsonResponse {
  statusCode: number;
  body: string;
}

const TLS_ERROR_CODES = new Set([
  'DEPTH_ZERO_SELF_SIGNED_CERT',
  'UNABLE_TO_VERIFY_LEAF_SIGNATURE',
  'SELF_SIGNED_CERT_IN_CHAIN',
  'CERT_HAS_EXPIRED',
  'ERR_TLS_CERT_ALTNAME_INVALID'
]);

function getErrorField(error: unknown, key: 'code' | 'message'): string {
  if (!error || typeof error !== 'object' || Array.isArray(error)) {
    return '';
  }

  const value = (error as Record<string, unknown>)[key];
  return typeof value === 'string' ? value : '';
}

export function isLikelyTlsError(error: unknown): boolean {
  if (!error) {
    return false;
  }

  const code = getErrorField(error, 'code').toUpperCase();
  const message = getErrorField(error, 'message').toLowerCase();

  return TLS_ERROR_CODES.has(code) || message.includes('certificate') || message.includes('tls');
}

export function formatNetworkError(error: unknown, endpoint: string): string {
  const code = getErrorField(error, 'code').toUpperCase();
  const message = getErrorField(error, 'message') || 'unknown network error';

  if (code === 'ENOTFOUND') {
    return `dns_unreachable (${endpoint})`;
  }

  if (code === 'ECONNREFUSED') {
    return `connection_refused (${endpoint})`;
  }

  if (code === 'ETIMEDOUT' || code === 'ECONNABORTED') {
    return `request_timeout (${endpoint})`;
  }

  if (isLikelyTlsError(error)) {
    return `tls_error (${message})`;
  }

  return `network_error (${code || 'UNKNOWN'}: ${message})`;
}

export function httpRequestJson(
  url: string,
  payload: unknown,
  options: HttpRequestJsonOptions = {}
): Promise<HttpJsonResponse> {
  const target = new URL(url);
  const isHttps = target.protocol === 'https:';
  const client = isHttps ? https : http;
  const requestBody = JSON.stringify(payload);

  return new Promise((resolve, reject) => {
    const req = client.request(
      {
        protocol: target.protocol,
        hostname: target.hostname,
        port: target.port || (isHttps ? 443 : 80),
        path: `${target.pathname}${target.search}`,
        method: 'POST',
        rejectUnauthorized: options.rejectUnauthorized !== false,
        timeout: options.timeoutMs || 10000,
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(requestBody)
        }
      },
      (res) => {
        let responseBody = '';

        res.setEncoding('utf8');
        res.on('data', (chunk) => {
          responseBody += chunk;
        });
        res.on('end', () => {
          resolve({
            statusCode: res.statusCode || 0,
            body: responseBody
          });
        });
      }
    );

    req.on('timeout', () => {
      req.destroy(Object.assign(new Error('request_timeout'), { code: 'ETIMEDOUT' }));
    });

    req.on('error', (error) => {
      reject(error);
    });

    req.write(requestBody);
    req.end();
  });
}
