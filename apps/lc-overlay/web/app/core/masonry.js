// Board masonry: every card keeps one width and takes the height of its
// media, like Discord's GIF picker. The grid has tiny rows (`ROW_PX`) and
// each card spans as many as its height plus the gap below it, so CSS grid
// packs the cards column by column without any layout script. Pure: also
// loaded by the Node tests.

// Mirrors `.grid` in styles/board.css.
export const CARD_WIDTH_PX = 196;
export const ROW_PX = 2;
export const GAP_PX = 14;
// `.card-title` under the thumbnail: top margin and one line of text.
export const TITLE_PX = 25;

// Very tall or very wide media are cropped to these ratios (width / height):
// a 9:16 phone video is shown whole, a panorama does not shrink to a strip.
export const MIN_RATIO = 9 / 16;
export const MAX_RATIO = 2;
export const DEFAULT_RATIO = 16 / 10;
// Audio has no picture: a short tile.
export const AUDIO_RATIO = 2;

export function cardRatio(media) {
  if (!media || media.kind === 'audio') {
    return media ? AUDIO_RATIO : DEFAULT_RATIO;
  }
  const width = Number(media.width);
  const height = Number(media.height);
  if (!(width > 0) || !(height > 0)) {
    return DEFAULT_RATIO;
  }
  return Math.min(MAX_RATIO, Math.max(MIN_RATIO, width / height));
}

// Rows of `ROW_PX` covered by a card of this ratio, gap included.
export function cardSpan(ratio) {
  const thumb = CARD_WIDTH_PX / ratio;
  return Math.ceil((thumb + TITLE_PX + GAP_PX) / ROW_PX);
}

// Inline style of a card: the thumbnail's ratio and the card's row span.
export function cardStyle(ratio) {
  return `--ratio: ${ratio.toFixed(4)}; --span: ${cardSpan(ratio)}`;
}
