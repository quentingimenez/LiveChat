import { describe, expect, it } from 'vitest';
import {
  TIKTOK_BUTTON_LOGO_SIZE_PX,
  TWITTER_BUTTON_LOGO_SIZE_PX,
  YOUTUBE_BUTTON_LOGO_SIZE_PX,
  tiktokInpageStyles,
  twitterInpageStyles,
  youtubeInpageStyles,
} from './inpage-styles';

// Ce que ce fichier protège : les boutons injectés doivent copier la taille et
// le style du bouton d'action natif qui les entoure, sur chaque hôte. Les
// valeurs attendues ci-dessous sont les relevés faits dans les pages elles-mêmes
// (novembre 2026) — boutons YouTube 40 px, icônes X ~19 px, pastilles TikTok
// 48 px translucides — et non une recopie de la feuille de styles.

const declarationsFor = (styles: string, selector: string): Record<string, string> => {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const withoutComments = styles.replace(/\/\*[\s\S]*?\*\//g, '');
  const block = new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(withoutComments);

  if (!block) {
    throw new Error(`bloc CSS introuvable : ${selector}`);
  }

  const declarations: Record<string, string> = {};

  for (const entry of block[1].split(';')) {
    const separator = entry.indexOf(':');

    if (separator === -1) {
      continue;
    }

    declarations[entry.slice(0, separator).trim()] = entry.slice(separator + 1).trim();
  }

  return declarations;
};

describe('feuille de styles YouTube', () => {
  it('prend la taille, le rayon et les icônes des boutons d’action de YouTube', () => {
    const button = declarationsFor(youtubeInpageStyles, '.lce-button-youtube');

    expect(button.width).toBe('40px');
    expect(button.height).toBe('40px');
    expect(button['border-radius']).toBe('20px');
    expect(YOUTUBE_BUTTON_LOGO_SIZE_PX).toBe(24);
  });

  it('emprunte le fond tonal YouTube, clair en thème clair, translucide en sombre', () => {
    const root = declarationsFor(youtubeInpageStyles, ':root');
    const dark = declarationsFor(youtubeInpageStyles, 'html[dark]');
    const button = declarationsFor(youtubeInpageStyles, '.lce-button-youtube');
    const hover = declarationsFor(youtubeInpageStyles, '.lce-button-youtube:hover');

    expect(root['--lce-youtube-tonal-wash']).toBe('rgba(0, 0, 0, 0.05)');
    expect(dark['--lce-youtube-tonal-wash']).toBe('rgba(255, 255, 255, 0.1)');
    expect(dark['--lce-youtube-tonal-wash-hover']).toBe('rgba(255, 255, 255, 0.2)');
    expect(button.background).toBe('var(--lce-youtube-tonal-wash)');
    expect(hover.background).toBe('var(--lce-youtube-tonal-wash-hover)');
  });

  it('garde le fond opaque des pastilles posées sur le lecteur', () => {
    const floating = declarationsFor(youtubeInpageStyles, '.lce-button-youtube-watch-floating');

    expect(floating.background).toBe('#272727');
    expect(floating['border-radius']).toBe('999px');
  });

  it('garde une pastille ronde pour le rail Shorts et le bouton flottant', () => {
    const shorts = declarationsFor(youtubeInpageStyles, '.lce-button-youtube-shorts');

    expect(shorts.width).toBe('48px');
    expect(shorts['border-radius']).toBe('999px');
  });

  it('espace le bouton de 8 px, comme entre les groupes d’action', () => {
    const slot = declarationsFor(youtubeInpageStyles, '.lce-youtube-watch-slot');

    expect(slot['margin-left']).toBe('8px');
    expect(slot['margin-right']).toBe('0');
  });

  it('garde un logo à l’échelle de la pastille (charte : ~60 %)', () => {
    expect(YOUTUBE_BUTTON_LOGO_SIZE_PX / 40).toBeGreaterThan(0.55);
    expect(YOUTUBE_BUTTON_LOGO_SIZE_PX / 40).toBeLessThan(0.65);
  });
});

describe('feuille de styles X', () => {
  it('prend la taille et la couleur des boutons d’action de X', () => {
    const button = declarationsFor(twitterInpageStyles, '.lce-button');

    expect(button.width).toBe('34px');
    expect(button.height).toBe('34px');
    expect(button.background).toBe('transparent');
    expect(button.color).toBe('#71767b');
  });

  it('ne rajoute pas de marge à celle de la barre d’action', () => {
    const slot = declarationsFor(twitterInpageStyles, '.lce-twitter-action-slot');

    expect(slot['margin-left']).toBe('0');
  });

  it('garde un logo de la taille des icônes X (~19 px)', () => {
    expect(TWITTER_BUTTON_LOGO_SIZE_PX).toBe(19);
    expect(TWITTER_BUTTON_LOGO_SIZE_PX / 34).toBeGreaterThan(0.5);
    expect(TWITTER_BUTTON_LOGO_SIZE_PX / 34).toBeLessThan(0.6);
  });
});

describe('feuille de styles TikTok', () => {
  it('prend la taille et le fond translucide des pastilles TikTok', () => {
    const button = declarationsFor(tiktokInpageStyles, '.lce-button-tiktok');

    expect(button.width).toBe('48px');
    expect(button.height).toBe('48px');
    expect(button['border-radius']).toBe('999px');
    expect(button['position']).toBe('relative');
    expect(button.background).toBe('rgba(255, 255, 255, 0.13)');
  });

  it('superpose une couche blanche au survol comme TikTok', () => {
    // Le natif ne change pas le fond de la pastille au survol : il empile une
    // couche rgba(255, 255, 255, 0.13) (div
    // .tux-interaction-container__overlay, pointer-events: none) sur le fond
    // existant. On reproduit cette couche avec un pseudo-élément au lieu de
    // foncer le fond du bouton.
    const hover = /\.lce-button-tiktok:hover\s*\{/.test(tiktokInpageStyles);

    expect(hover).toBe(false);

    const overlay = declarationsFor(tiktokInpageStyles, '.lce-button-tiktok::after');

    expect(overlay.inset).toBe('0');
    expect(overlay['pointer-events']).toBe('none');
    expect(overlay['border-radius']).toBe('inherit');
    expect(overlay.background).toBe('rgba(255, 255, 255, 0)');

    const hoverOverlay = declarationsFor(tiktokInpageStyles, '.lce-button-tiktok:hover::after');

    expect(hoverOverlay.background).toBe('rgba(255, 255, 255, 0.13)');
  });

  it('ne réduit plus le bouton dans la colonne d’actions', () => {
    const vertical = declarationsFor(tiktokInpageStyles, '.lce-tiktok-slot.is-vertical .lce-button-tiktok');

    expect(vertical.width).toBeUndefined();
    expect(vertical.height).toBeUndefined();
  });

  it('laisse le même espace visible au-dessus et en dessous du bouton', () => {
    const slot = declarationsFor(tiktokInpageStyles, '.lce-tiktok-slot.is-vertical');

    // Relevé sans extension sur /@clips.caminotv/video/7701074617191943457 :
    // avatar 48 suivi de 20 px de marge, badge « + » de 24 px qui déborde de
    // 12 px sous l'avatar, items natifs de 78 px (8 px de padding haut +
    // pastille 48 + compteur). Notre bouton n'ayant pas de compteur, 8 px de
    // padding + 48 + 8 px = 64 px, donc 16 px d'espace visible au-dessus
    // (badge) comme en dessous (8 + le padding de 8 px du voisin suivant).
    expect(slot.height).toBe('64px');
    expect(slot.padding).toBe('8px 0 0');
    expect(slot['box-sizing']).toBe('border-box');
  });

  it('prend la taille des cercles de l’en-tête ouvert depuis un profil', () => {
    const horizontal = declarationsFor(tiktokInpageStyles, '.lce-tiktok-slot.is-horizontal .lce-button-tiktok');

    // Relevé dans l'en-tête du tiktok ouvert depuis un profil : les boutons
    // natifs (j'aime, commenter, enregistrer) sont des cercles de 32 px au
    // fond rgba(255, 255, 255, 0.12).
    expect(horizontal.width).toBe('32px');
    expect(horizontal.height).toBe('32px');
    expect(horizontal.background).toBe('rgba(255, 255, 255, 0.12)');
    expect(horizontal.padding).toBe('0');
  });

  it('garde un logo à l’échelle de l’icône native (21 px dans 48)', () => {
    expect(TIKTOK_BUTTON_LOGO_SIZE_PX).toBe(21);
    expect(TIKTOK_BUTTON_LOGO_SIZE_PX / 48).toBeGreaterThan(0.4);
    expect(TIKTOK_BUTTON_LOGO_SIZE_PX / 48).toBeLessThan(0.5);
  });
});
