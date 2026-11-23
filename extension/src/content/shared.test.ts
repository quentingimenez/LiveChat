import { describe, expect, it, vi } from 'vitest';

import {
  onTrustedPageEvent,
  readActionTarget,
  rememberActionTarget,
  resolveActionButtonTarget,
  resolveAllowedActionTarget,
} from './shared';

// Ce que ce fichier protège : un script de la page hôte peut réécrire le DOM
// des boutons injectés et fabriquer des événements. Ces tests vérifient que
// seuls les événements réellement émis par l'utilisateur déclenchent un envoi,
// que la cible vient du registre de l'extension (jamais de `dataset`) et
// qu'elle reste limitée aux URL canoniques de la plateforme. Le Bearer reste
// attaché par le service worker à l'API configurée, pas à cette cible.

const youtubeWatchUrl = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ';

const fakeButton = (datasetTargetUrl: string): HTMLButtonElement =>
  ({ dataset: { targetUrl: datasetTargetUrl } }) as unknown as HTMLButtonElement;

const dispatchTrusted = (target: EventTarget, type: string): void => {
  const event = new Event(type);
  Object.defineProperty(event, 'isTrusted', { value: true });
  target.dispatchEvent(event);
};

describe('événements de la page hôte', () => {
  it('ignore un clic forgé par la page (dispatchEvent non trusted)', () => {
    const button = new EventTarget();
    const send = vi.fn();

    onTrustedPageEvent<MouseEvent>(button, 'click', send);
    button.dispatchEvent(new Event('click'));

    expect(send).not.toHaveBeenCalled();
  });

  it('laisse passer un clic réellement émis par l’utilisateur', () => {
    const button = new EventTarget();
    const send = vi.fn();

    onTrustedPageEvent<MouseEvent>(button, 'click', send);
    dispatchTrusted(button, 'click');

    expect(send).toHaveBeenCalledTimes(1);
  });

  it('refuse aussi un pointerdown forgé, qui ouvrirait la composition', () => {
    const button = new EventTarget();
    const openCompose = vi.fn();

    onTrustedPageEvent<PointerEvent>(button, 'pointerdown', openCompose);
    button.dispatchEvent(new Event('pointerdown'));

    expect(openCompose).not.toHaveBeenCalled();
  });
});

describe('cible d’envoi du bouton injecté', () => {
  it('ignore une cible réécrite dans dataset par la page', () => {
    const button = fakeButton('https://www.youtube.com/watch?v=forged00000');
    rememberActionTarget(button, youtubeWatchUrl);

    button.dataset.targetUrl = 'https://www.youtube.com/watch?v=forged00000';

    expect(readActionTarget(button)).toBe(youtubeWatchUrl);
    expect(resolveActionButtonTarget(button, 'youtube', null)).toBe(youtubeWatchUrl);
  });

  it('ne dérive jamais la cible du dataset quand le registre est vide', () => {
    const button = fakeButton('https://www.youtube.com/watch?v=forged00000');

    expect(readActionTarget(button)).toBeNull();
    expect(resolveActionButtonTarget(button, 'youtube', null)).toBeNull();
  });

  it('envoie la cible enregistrée, canonicalisée, sur un clic trusted', () => {
    const button = new EventTarget() as unknown as HTMLButtonElement;
    Object.defineProperty(button, 'dataset', {
      value: { targetUrl: 'https://www.youtube.com/watch?v=forged00000' },
    });
    const sent: string[] = [];
    rememberActionTarget(button, 'https://youtu.be/dQw4w9WgXcQ');

    onTrustedPageEvent<MouseEvent>(button, 'click', () => {
      const targetUrl = resolveActionButtonTarget(button, 'youtube', null);

      if (targetUrl) {
        sent.push(targetUrl);
      }
    });
    dispatchTrusted(button, 'click');

    expect(sent).toEqual([youtubeWatchUrl]);
  });
});

describe('allowlist des URL cibles', () => {
  it('accepte les URL YouTube', () => {
    expect(resolveAllowedActionTarget('youtube', 'https://youtu.be/dQw4w9WgXcQ')).toBe(youtubeWatchUrl);
  });

  it('refuse un hôte YouTube inconnu', () => {
    expect(resolveAllowedActionTarget('youtube', 'https://player.example/watch?v=dQw4w9WgXcQ')).toBeNull();
  });

  it('accepte les statuts X / Twitter', () => {
    expect(resolveAllowedActionTarget('twitter', 'https://x.com/user/status/123')).toBe(
      'https://x.com/user/status/123',
    );
  });

  it('refuse un hôte X / Twitter inconnu', () => {
    expect(resolveAllowedActionTarget('twitter', 'https://evil.example/user/status/123')).toBeNull();
  });

  it('accepte les vidéos TikTok', () => {
    expect(resolveAllowedActionTarget('tiktok', 'https://www.tiktok.com/@user/video/123456789012345')).toBe(
      'https://www.tiktok.com/@user/video/123456789012345',
    );
  });

  it('refuse un hôte TikTok inconnu', () => {
    expect(resolveAllowedActionTarget('tiktok', 'https://evil.example/@user/video/123456789012345')).toBeNull();
  });
});
