// Feuilles de styles des boutons injectés dans les pages hôtes.
//
// Les tailles et les fonds recopient ceux des boutons d'action natifs de chaque
// site (relevés au 2026-11 sur YouTube watch, X et la colonne TikTok) : le
// bouton ne doit pas détonner à côté de ses voisins. Les constantes exportées
// sont la source des déclarations CSS et, pour le logo, du balisage produit par
// `brand-logo.ts` — un seul endroit à corriger quand un hôte change de design.

// YouTube : boutons d'action 40 × 40 (radius 20) au fond tonal
// rgba(255, 255, 255, 0.1) en sombre, rgba(0, 0, 0, 0.05) en clair ; icônes de
// 24 px. La pastille Shorts garde ses 48 px (rail du lecteur vertical).
export const YOUTUBE_BUTTON_SIZE_PX = 40;
export const YOUTUBE_BUTTON_LOGO_SIZE_PX = 24;

// X : boutons d'action 34 × 34, fond transparent — le cercle de survol est
// peint sur un enfant — et icônes de ~19 px.
export const TWITTER_BUTTON_SIZE_PX = 34;
export const TWITTER_BUTTON_LOGO_SIZE_PX = 19;

// TikTok : pastilles de la colonne d'actions 48 × 48 au fond translucide
// rgba(255, 255, 255, 0.13), icônes de 21 px.
export const TIKTOK_BUTTON_SIZE_PX = 48;
export const TIKTOK_BUTTON_LOGO_SIZE_PX = 21;

export const youtubeInpageStyles = `
:root {
  --lce-youtube-tonal-wash: rgba(0, 0, 0, 0.05);
  --lce-youtube-tonal-wash-hover: rgba(0, 0, 0, 0.1);
}
html[dark] {
  --lce-youtube-tonal-wash: rgba(255, 255, 255, 0.1);
  --lce-youtube-tonal-wash-hover: rgba(255, 255, 255, 0.2);
}
.lce-button-youtube {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: ${YOUTUBE_BUTTON_SIZE_PX}px;
  height: ${YOUTUBE_BUTTON_SIZE_PX}px;
  border: none;
  border-radius: 20px;
  padding: 0;
  font-family: "Roboto", "Arial", sans-serif;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.02em;
  cursor: pointer;
  pointer-events: auto;
  /* Fond tonal des boutons d'action YouTube : gris ardoise en thème clair,
     blanc translucide en thème sombre (html[dark]). */
  background: var(--lce-youtube-tonal-wash);
  color: inherit;
  transition: transform 120ms ease, background-color 180ms ease, color 180ms ease, box-shadow 180ms ease;
}
.lce-button-youtube:hover {
  background: var(--lce-youtube-tonal-wash-hover);
}
.lce-button-youtube:disabled {
  opacity: 1;
}
.lce-button-youtube-watch {
  flex: 0 0 auto;
  position: relative;
  z-index: 2;
}
.lce-button-youtube-shorts {
  width: 48px;
  height: 48px;
  /* 48 px : le rayon de 20 px du bouton watch ne suffit plus à faire un
     cercle, on garde la pastille ronde du rail Shorts. */
  border-radius: 999px;
}
.lce-button-youtube-shorts-floating,
.lce-button-youtube-watch-floating {
  position: fixed;
  z-index: 2147483646;
  pointer-events: auto !important;
  border-radius: 999px;
  /* Posée sur le lecteur, la puce garde son fond sombre quel que soit le
     thème de la page. */
  background: #272727;
  color: #fff;
}
.lce-button-youtube-shorts-floating:hover,
.lce-button-youtube-watch-floating:hover {
  background: #303030;
}
.lce-youtube-shorts-slot {
  display: flex;
  justify-content: center;
  width: 100%;
  margin-bottom: 12px;
  pointer-events: auto;
  z-index: 3;
}
.lce-youtube-watch-slot {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  /* Espacement de 8 px entre les groupes d'action YouTube, comme entre le
     segment J'aime et le bouton Partager. */
  margin-left: 8px;
  margin-right: 0;
  flex: 0 0 auto;
  pointer-events: auto !important;
}
.lce-button-youtube.is-loading {
  background: #fed54a;
  color: #0f0f0f;
  animation: lce-youtube-pulse 0.95s ease-in-out infinite;
}
.lce-button-youtube.is-success {
  background: #30d158;
  color: #0f0f0f;
  animation: lce-youtube-pulse-success 0.9s ease-in-out 2;
}
.lce-button-youtube.is-error {
  background: #ff453a;
  color: #fff;
  animation: lce-youtube-pulse-error 0.9s ease-in-out 2;
}
@keyframes lce-youtube-pulse {
  0% { transform: scale(1); box-shadow: 0 0 0 0 rgba(254, 213, 74, 0.45); }
  70% { transform: scale(1.06); box-shadow: 0 0 0 10px rgba(254, 213, 74, 0); }
  100% { transform: scale(1); box-shadow: 0 0 0 0 rgba(254, 213, 74, 0); }
}
@keyframes lce-youtube-pulse-success {
  0% { transform: scale(1); box-shadow: 0 0 0 0 rgba(48, 209, 88, 0.45); }
  70% { transform: scale(1.08); box-shadow: 0 0 0 12px rgba(48, 209, 88, 0); }
  100% { transform: scale(1); box-shadow: 0 0 0 0 rgba(48, 209, 88, 0); }
}
@keyframes lce-youtube-pulse-error {
  0% { transform: scale(1); box-shadow: 0 0 0 0 rgba(255, 69, 58, 0.45); }
  70% { transform: scale(1.08); box-shadow: 0 0 0 12px rgba(255, 69, 58, 0); }
  100% { transform: scale(1); box-shadow: 0 0 0 0 rgba(255, 69, 58, 0); }
}
`;

export const twitterInpageStyles = `
.lce-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: ${TWITTER_BUTTON_SIZE_PX}px;
  height: ${TWITTER_BUTTON_SIZE_PX}px;
  border: none;
  border-radius: 999px;
  padding: 0;
  font-family: "Segoe UI", "Helvetica Neue", sans-serif;
  font-size: 11px;
  font-weight: 700;
  cursor: pointer;
  background: transparent;
  color: #71767b;
  transition: background-color 140ms ease, color 140ms ease, transform 120ms ease;
}
.lce-button:hover { background: rgba(29, 155, 240, 0.1); color: #1d9bf0; }
.lce-button:disabled { opacity: 1; }
.lce-button.is-loading {
  background: rgba(29, 155, 240, 0.1);
  color: #1d9bf0;
}
.lce-button.is-success {
  background: #30d158;
  color: #0f0f0f;
}
.lce-button.is-error {
  background: #ff453a;
  color: #fff;
}
.lce-button-floating { position: fixed; right: 20px; bottom: 24px; z-index: 2147483646; }
.lce-twitter-action-slot {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  /* La barre d'actions X espace déjà ses enfants de 4 px : pas de marge en
     plus, sinon le bouton s'écarte du bouton Partager. */
  margin-left: 0;
  flex: 0 0 auto;
}
`;

export const tiktokInpageStyles = `
.lce-button-tiktok {
  position: relative;
  width: ${TIKTOK_BUTTON_SIZE_PX}px;
  height: ${TIKTOK_BUTTON_SIZE_PX}px;
  border: none;
  border-radius: 999px;
  /* Pastille translucide de la colonne d'actions TikTok, identique aux icônes
     natives qui partagent le même fond au-dessus de la vidéo. */
  background: rgba(255, 255, 255, 0.13);
  color: #fff;
  font-family: "TikTokDisplayFont", "ProximaNova", "Segoe UI", sans-serif;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.02em;
  cursor: pointer;
  pointer-events: auto;
  transition: transform 120ms ease, background-color 180ms ease, color 180ms ease, box-shadow 180ms ease;
}
.lce-button-tiktok::after {
  /* Le survol natif n'éclaircit pas la pastille elle-même : TikTok ajoute une
     couche blanche à 13 % au-dessus de l'icône (div
     .tux-interaction-container__overlay, pointer-events: none). On superpose
     la même couche pour obtenir le même composite (0.13 sur 0.13). */
  content: "";
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: rgba(255, 255, 255, 0);
  pointer-events: none;
}
.lce-button-tiktok:hover::after {
  background: rgba(255, 255, 255, 0.13);
}
.lce-button-tiktok:disabled {
  opacity: 1;
}
.lce-button-tiktok.is-loading {
  background: #fed54a;
  color: #0f0f0f;
}
.lce-button-tiktok.is-success {
  background: #25f4ee;
  color: #0f0f0f;
}
.lce-button-tiktok.is-error {
  background: #fe2c55;
  color: #fff;
}
.lce-tiktok-slot {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-start;
  margin-top: 0;
  pointer-events: auto;
}
.lce-tiktok-slot.is-vertical {
  /* Dans la colonne, chaque pastille native occupe un item de 78 px : 8 px de
     padding haut, la pastille de 48 px, puis le compteur. Notre bouton n'a pas
     de compteur, donc l'espace visible autour de lui se mesure à la pastille
     voisine : le badge « + » de l'avatar descend 12 px sous sa boîte (24 px de
     haut pour 12 px de débord), ce qui laisse 16 px entre le badge et notre
     cercle. On s'arrête 8 px après le cercle : 8 px + les 8 px de padding de
     l'item suivant = 16 px en dessous, soit le même espace visible. */
  height: 64px;
  padding: 8px 0 0;
  box-sizing: border-box;
  margin-top: 0;
  margin-bottom: 0;
}
.lce-tiktok-slot.is-vertical .lce-button-tiktok {
  font-size: 10px;
}
.lce-tiktok-slot.is-horizontal {
  margin-top: 0;
  margin-bottom: 0;
}
.lce-tiktok-slot.is-horizontal .lce-button-tiktok {
  /* L'en-tête du tiktok ouvert depuis un profil aligne des cercles de 32 px
     (rgba(255, 255, 255, 0.12) sur SpanIconWrapper) : la pastille de 48 px y
     déborderait, donc on reprend exactement le gabarit voisin. */
  width: 32px;
  height: 32px;
  padding: 0;
  font-size: 10px;
  background: rgba(255, 255, 255, 0.12);
}
`;
