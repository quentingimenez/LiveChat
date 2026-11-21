// Logo et libellé du bouton d'envoi injecté dans les pages (X, TikTok, YouTube).
//
// Le bouton ne porte plus le sigle « LC » : au repos, il montre le logo
// LiveChat. On reprend le dessin simplifié de la charte
// (docs/brand/README.md) — traits de 28, bulle de 138 × 114 — parce qu'il
// reste lisible à 16 px, là où le dessin complet est interdit sous 48 px.
//
// Les coins du cadrage sont en `currentColor` : le logo emprunte la couleur de
// texte du bouton, donc celle du thème de la page hôte (coins Craie sur fond
// sombre, Ardoise sur fond clair), comme `logo.svg` / `logo-light.svg` sur la
// board. La bulle reste ambre sur tous les fonds, et le triangle de lecture
// garde son encre ambre.
//
// Le cadre du dessin occupe x 16 → 246 et y 10 → 240, traits compris, sur la
// grille de 256 : le viewBox recadre sur ces bornes pour que le logo remplisse
// la taille demandée. La marge de protection de la charte (1/8 de la taille)
// vient de la pastille du bouton, dont le logo n'occupe que ~60 %.

export type InjectedButtonState = 'idle' | 'loading' | 'success' | 'error';

// En dessous de 16 px, la charte interdit le logo : on relève la taille
// demandée plutôt que de rendre la bulle illisible.
const MIN_LOGO_SIZE_PX = 16;

// Pastilles de texte des états d'envoi : l'échec et le succès durent ~2 s, le
// logo revient ensuite.
const BUTTON_TEXT_BY_STATE: Record<Exclude<InjectedButtonState, 'idle'>, string> = {
  loading: '...',
  success: 'OK',
  error: 'ER',
};

const LOGO_PATHS = [
  '<path d="M30 118V68H80"/>',
  '<path d="M30 176V226H80"/>',
  '<path d="M148 226H198V176"/>',
].join('');

export const buttonLogoMarkup = (sizePx: number): string => {
  const size = Math.max(MIN_LOGO_SIZE_PX, Math.round(sizePx));

  // Le SVG est décoratif : le nom du bouton vient de son `title` et de son
  // `aria-label` (voir `setButtonLabel`), pas du dessin.
  return [
    `<svg class="lce-logo" viewBox="16 10 230 230" width="${size}" height="${size}"`,
    ' aria-hidden="true" focusable="false">',
    `<g fill="none" stroke="currentColor" stroke-width="28" stroke-linecap="round"`,
    ' stroke-linejoin="round">',
    LOGO_PATHS,
    '</g>',
    '<rect x="108" y="10" width="138" height="114" rx="32" fill="#F2B544"/>',
    '<path d="M140 110L104 160L182 118Z" fill="#F2B544" stroke="#F2B544"',
    ' stroke-width="8" stroke-linejoin="round"/>',
    '<path d="M160 42L200 67L160 92Z" fill="#2A1D02" stroke="#2A1D02"',
    ' stroke-width="9" stroke-linejoin="round"/>',
    '</svg>',
  ].join('');
};

// Contenu du bouton : le logo au repos, la pastille de texte pendant l'envoi et
// quand il se termine. Le contenu est du balisage constant, jamais une entrée
// utilisateur.
export const renderButtonContent = (
  button: HTMLButtonElement,
  state: InjectedButtonState,
  logoSizePx: number,
): void => {
  if (state === 'idle') {
    button.innerHTML = buttonLogoMarkup(logoSizePx);
    return;
  }

  button.textContent = BUTTON_TEXT_BY_STATE[state];
};

// Un bouton réduit à un dessin n'a plus de texte visible : le libellé part dans
// l'infobulle et dans le nom accessible, que les lecteurs d'écran annoncent.
export const setButtonLabel = (button: HTMLButtonElement, label: string): void => {
  button.title = label;
  button.setAttribute('aria-label', label);
};
