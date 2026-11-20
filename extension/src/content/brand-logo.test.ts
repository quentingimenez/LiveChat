import { describe, expect, it } from 'vitest';

import {
  buttonLogoMarkup,
  renderButtonContent,
  setButtonLabel,
  type InjectedButtonState,
} from './brand-logo';

// Ce que ce fichier protège : le bouton d'envoi injecté montre le logo du
// produit au repos — plus le sigle « LC » — dans le dessin simplifié de la
// charte, avec des coins qui suivent le thème de la page ; les états d'envoi
// gardent leur pastille de texte et le bouton reste nommé pour les lecteurs
// d'écran malgré la disparition du texte visible.

const markup = buttonLogoMarkup(24);

const viewBox = (source: string): number[] => {
  const match = /viewBox="([-\d.]+) ([-\d.]+) ([\d.]+) ([\d.]+)"/.exec(source);

  if (!match) {
    throw new Error(`viewBox introuvable dans : ${source}`);
  }

  return match.slice(1).map(Number);
};

describe('logo du bouton injecté', () => {
  it('reprend le dessin simplifié de la charte, pas le complet', () => {
    // Dessin complet : traits de 18 et bulle de 116 ; simplifié : 28 et 138.
    expect(markup).toContain('stroke-width="28"');
    expect(markup).toContain('width="138"');
    expect(markup).toContain('height="114"');
    expect(markup).not.toContain('stroke-width="18"');
  });

  it('laisse les coins suivre le thème et garde la bulle ambre', () => {
    expect(markup).toContain('stroke="currentColor"');
    expect(markup).toContain('fill="#F2B544"');
    expect(markup).toContain('fill="#2A1D02"');
    // Aucune couleur de thème figée : les coins Craie ou Ardoise viennent de la
    // couleur de texte du bouton, donc de la page hôte.
    expect(markup).not.toContain('#E9EBF3');
    expect(markup).not.toContain('#171B25');
  });

  it('est décoratif et sans identifiant, pour cohabiter avec plusieurs boutons', () => {
    expect(markup).toContain('aria-hidden="true"');
    expect(markup).toContain('focusable="false"');
    expect(markup).not.toContain('id=');
    expect(markup).not.toContain('<title');
  });

  it('taille le logo en carré et ne descend pas sous 16 px', () => {
    expect(markup).toContain('width="24" height="24"');
    expect(buttonLogoMarkup(12)).toContain('width="16" height="16"');
    expect(buttonLogoMarkup(21.6)).toContain('width="22" height="22"');

    const [, , width, height] = viewBox(markup);
    expect(width).toBe(height);
  });

  it('cadre le dessin sur ses bornes d’encre, traits compris', () => {
    // Le contour du logo va de x 16 à 246 et de y 10 à 240 sur la grille de 256.
    expect(viewBox(markup)).toEqual([16, 10, 230, 230]);
  });
});

describe('contenu du bouton', () => {
  const createButtonStub = (): { button: HTMLButtonElement; writes: string[] } => {
    const writes: string[] = [];
    const button = {
      set innerHTML(value: string) {
        writes.push(`html:${value}`);
      },
      set textContent(value: string) {
        writes.push(`text:${value}`);
      },
    } as unknown as HTMLButtonElement;

    return { button, writes };
  };

  it('affiche le logo au repos et le texte pendant l’envoi', () => {
    const { button, writes } = createButtonStub();

    renderButtonContent(button, 'idle', 24);
    expect(writes).toEqual([`html:${buttonLogoMarkup(24)}`]);

    renderButtonContent(button, 'loading', 24);
    renderButtonContent(button, 'success', 24);
    renderButtonContent(button, 'error', 24);
    expect(writes.slice(1)).toEqual(['text:...', 'text:OK', 'text:ER']);
  });

  it('ne montre plus le sigle « LC » dans aucun état', () => {
    const states: InjectedButtonState[] = ['idle', 'loading', 'success', 'error'];

    for (const state of states) {
      const { button, writes } = createButtonStub();
      renderButtonContent(button, state, 24);
      expect(writes.join('')).not.toContain('LC');
    }
  });
});

describe('libellé du bouton', () => {
  it('nomme le bouton par son infobulle et son nom accessible', () => {
    const attributes = new Map<string, string>();
    const button = {
      title: '',
      setAttribute: (name: string, value: string) => attributes.set(name, value),
    } as unknown as HTMLButtonElement;

    setButtonLabel(button, 'Envoyer la vidéo YouTube vers LiveChat');

    expect(button.title).toBe('Envoyer la vidéo YouTube vers LiveChat');
    expect(attributes.get('aria-label')).toBe('Envoyer la vidéo YouTube vers LiveChat');
  });
});
