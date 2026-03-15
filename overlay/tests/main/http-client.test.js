const test = require('node:test');
const assert = require('node:assert/strict');

const { isLikelyTlsError, formatNetworkError } = require('../../dist/main/infra/http-client.js');

test('http-client isLikelyTlsError returns false for falsy values', () => {
  assert.equal(isLikelyTlsError(null), false);
  assert.equal(isLikelyTlsError(undefined), false);
  assert.equal(isLikelyTlsError(0), false);
  assert.equal(isLikelyTlsError(''), false);
});

test('http-client isLikelyTlsError detects TLS error codes', () => {
  assert.equal(isLikelyTlsError({ code: 'DEPTH_ZERO_SELF_SIGNED_CERT' }), true);
  assert.equal(isLikelyTlsError({ code: 'UNABLE_TO_VERIFY_LEAF_SIGNATURE' }), true);
  assert.equal(isLikelyTlsError({ code: 'SELF_SIGNED_CERT_IN_CHAIN' }), true);
  assert.equal(isLikelyTlsError({ code: 'CERT_HAS_EXPIRED' }), true);
  assert.equal(isLikelyTlsError({ code: 'ERR_TLS_CERT_ALTNAME_INVALID' }), true);
});

test('http-client isLikelyTlsError detects TLS via message keywords', () => {
  assert.equal(isLikelyTlsError({ message: 'certificate verify failed' }), true);
  assert.equal(isLikelyTlsError({ message: 'TLS handshake failure' }), true);
});

test('http-client isLikelyTlsError returns false for non-TLS errors', () => {
  assert.equal(isLikelyTlsError({ code: 'ECONNREFUSED' }), false);
  assert.equal(isLikelyTlsError({ message: 'connection reset' }), false);
  assert.equal(isLikelyTlsError(new Error('some random error')), false);
});

test('http-client formatNetworkError formats ENOTFOUND', () => {
  const result = formatNetworkError({ code: 'ENOTFOUND' }, 'https://api.example.com');
  assert.equal(result, 'dns_unreachable (https://api.example.com)');
});

test('http-client formatNetworkError formats ECONNREFUSED', () => {
  const result = formatNetworkError({ code: 'ECONNREFUSED' }, 'https://api.example.com');
  assert.equal(result, 'connection_refused (https://api.example.com)');
});

test('http-client formatNetworkError formats ETIMEDOUT', () => {
  const result = formatNetworkError({ code: 'ETIMEDOUT' }, 'https://api.example.com');
  assert.equal(result, 'request_timeout (https://api.example.com)');
});

test('http-client formatNetworkError formats ECONNABORTED', () => {
  const result = formatNetworkError({ code: 'ECONNABORTED' }, 'https://api.example.com');
  assert.equal(result, 'request_timeout (https://api.example.com)');
});

test('http-client formatNetworkError formats TLS errors', () => {
  const result = formatNetworkError(
    { code: 'DEPTH_ZERO_SELF_SIGNED_CERT', message: 'self signed certificate' },
    'https://api.example.com'
  );
  assert.equal(result, 'tls_error (self signed certificate)');
});

test('http-client formatNetworkError formats unknown errors with code', () => {
  const result = formatNetworkError(
    { code: 'ESOMETHING', message: 'weird error' },
    'https://api.example.com'
  );
  assert.equal(result, 'network_error (ESOMETHING: weird error)');
});

test('http-client formatNetworkError handles errors without code', () => {
  const result = formatNetworkError({ message: 'some failure' }, 'https://api.example.com');
  assert.equal(result, 'network_error (UNKNOWN: some failure)');
});

test('http-client formatNetworkError handles non-object errors', () => {
  const result = formatNetworkError('string error', 'https://api.example.com');
  assert.equal(result, 'network_error (UNKNOWN: unknown network error)');
});

test('http-client formatNetworkError handles null error', () => {
  const result = formatNetworkError(null, 'https://api.example.com');
  assert.equal(result, 'network_error (UNKNOWN: unknown network error)');
});
