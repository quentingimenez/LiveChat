// Guild icons already known to be broken in this session.
//
// Discord builds the icon URL from the guild id and the picture hash, so a new
// picture means a new URL: remembering a failure cannot hide a later valid
// icon. The rail is rebuilt on every guild event, and inserting a dead <img>
// again replaced the tile's acronym only once the network error came back,
// which made the acronym blink at each guild switch.

const brokenIcons = new Set();

export function canLoadIcon(iconUrl) {
  return Boolean(iconUrl) && !brokenIcons.has(iconUrl);
}

export function rememberBrokenIcon(iconUrl) {
  if (iconUrl) {
    brokenIcons.add(iconUrl);
  }
}
