// Pure formatting helpers (no DOM, no state).

const graphemeSegmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
// Words are separated by spaces, apostrophes, hyphens, underscores and dots.
const WORD_SEPARATORS = /[\s'’\-_.]+/u;
const LETTER_OR_DIGIT = /^[\p{L}\p{N}]/u;

function graphemes(text) {
  return Array.from(graphemeSegmenter.segment(text), (part) => part.segment);
}

// First grapheme of a string, never half of a surrogate pair. Used for avatar
// letters, where a single code unit could split an emoji.
export function firstGrapheme(text) {
  if (text === null || text === undefined) {
    return '';
  }
  return graphemes(String(text))[0] || '';
}

export function initials(name) {
  const text = name === null || name === undefined ? '' : String(name).normalize('NFKC').trim();
  if (!text) {
    return '?';
  }
  // Keep the words that start with a letter or a digit: punctuation and emoji
  // words must not steal the initials ("L’Escale 🔱" → "LE").
  const words = text.split(WORD_SEPARATORS).filter(Boolean);
  const kept = words.filter((word) => LETTER_OR_DIGIT.test(word));
  let letters;
  if (kept.length >= 2) {
    letters = graphemes(kept[0])[0] + graphemes(kept[1])[0];
  } else if (kept.length === 1) {
    const parts = graphemes(kept[0]);
    // A second grapheme that is not a letter or a digit is no initial.
    letters = parts.length > 1 && LETTER_OR_DIGIT.test(parts[1]) ? parts[0] + parts[1] : parts[0];
  } else {
    // A lone emoji is still a readable avatar.
    letters = graphemes(text)[0];
  }
  return letters.toLocaleUpperCase('fr-FR');
}

// Message of a guild removal notice. The shell removes a guild on its own when
// it is deleted, when the member is removed or when the device is revoked.
export function removalMessage(removal = {}) {
  const { guildName, reason } = removal || {};
  const name = guildName === null || guildName === undefined ? '' : String(guildName).trim();
  // A missing name is not quoted: « Une guilde » would read as a guild name.
  const quoted = (fallback) => (name ? `« ${name} »` : fallback);
  switch (reason) {
    case 'guild_deleted':
      return `${quoted('Une guilde')} a été supprimée par un admin et retirée de LiveChat.`;
    case 'member_removed':
      return `Un admin t’a retiré de ${quoted('une guilde')}. La guilde est retirée de LiveChat.`;
    case 'revoked':
      return `Cet appareil a été révoqué dans ${quoted('une guilde')}. La guilde est retirée de LiveChat.`;
    case 'invalid_token':
    default:
      return `${quoted('Une guilde')} ne reconnaît plus cet appareil. La guilde est retirée de LiveChat.`;
  }
}

export function formatDuration(seconds) {
  if (!seconds || seconds <= 0) {
    return null;
  }
  const total = Math.round(seconds);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
}

export function relativeTime(iso) {
  if (!iso) {
    return 'Jamais vu';
  }
  const minutes = Math.round((Date.now() - new Date(iso).getTime()) / 60000);
  if (minutes < 2) {
    return "À l'instant";
  }
  if (minutes < 60) {
    return `Il y a ${minutes} min`;
  }
  if (minutes < 1440) {
    return `Il y a ${Math.round(minutes / 60)} h`;
  }
  return `Il y a ${Math.round(minutes / 1440)} j`;
}

export function formatCode(code) {
  return code.length === 8 ? `${code.slice(0, 4)}-${code.slice(4)}` : code;
}

// Signed media URLs expire: reload the board five minutes before the earliest
// expiry of the loaded items. Returns the setTimeout delay in ms, or null when
// no item carries a usable expiry.
export const MEDIA_URL_REFRESH_MARGIN_MS = 5 * 60 * 1000;

export function mediaRefreshDelay(items, now = Date.now()) {
  let earliest = null;
  for (const item of items || []) {
    const expiresAt = Number(item && item.mediaUrlExpiresAt);
    if (!Number.isFinite(expiresAt) || expiresAt <= 0) {
      continue;
    }
    if (earliest === null || expiresAt < earliest) {
      earliest = expiresAt;
    }
  }
  if (earliest === null) {
    return null;
  }
  return Math.max(0, earliest - MEDIA_URL_REFRESH_MARGIN_MS - now);
}
