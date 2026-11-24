import { describe, expect, it } from 'vitest';
import { redactSecrets } from './redact';

describe('masquage des secrets', () => {
  it('retire le jeton d’un message d’erreur', () => {
    expect(redactSecrets('Échec avec le jeton super-secret-token', ['super-secret-token'])).toBe(
      'Échec avec le jeton [redacted]',
    );
  });

  it('retire la forme Bearer du jeton', () => {
    expect(redactSecrets('Bearer abc123 refusé', ['abc123'])).toBe('[redacted] refusé');
  });

  it('retire chaque secret fourni', () => {
    expect(redactSecrets('a=b et c=d', ['b', 'd'])).toBe('a=[redacted] et c=[redacted]');
  });

  it('ignore les secrets vides ou non chaînes', () => {
    expect(redactSecrets('message sensible', ['', null as unknown as string])).toBe('message sensible');
  });
});
