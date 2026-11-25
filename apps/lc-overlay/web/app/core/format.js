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

// Discord's guild-icon acronym: the first letter of every word, in the case the
// name was typed in, with the English possessive dropped ("Serveur de babz" →
// "Sdb", "Bob's server" → "Bs"). Punctuation and emoji between the words stay,
// as in Discord ("Jean-Pierre" → "J-P", "L’Escale 🔱" → "L’E🔱"). Discord only
// knows [A-Za-z0-9_] as word characters; Unicode letters and digits count here
// too, so an accented name keeps its letter ("Été" → "É"), and the name is
// normalized first ("𝐒𝐮𝐩𝐞𝐫" → "S"). The empty string is Discord's answer for a
// missing name: callers choose their own fallback tile.
const ACRONYM_WORD = /[\p{L}\p{N}_]+/gu;
const ACRONYM_POSSESSIVE = /'s /g;

export function guildAcronym(name) {
  const text = name === null || name === undefined ? '' : String(name).normalize('NFKC');
  if (!text) {
    return '';
  }
  return text
    .replace(ACRONYM_POSSESSIVE, ' ')
    .replace(ACRONYM_WORD, (word) => firstGrapheme(word))
    .replace(/\s+/gu, '');
}

// An acronym has one letter per word, so a long name no longer fits the 44 px
// rail tile at its usual size: past four letters it shrinks like Discord's guild
// icon (16, 16, 14, 14, 12, 10, 8 px on Discord's 40 px tile). Four letters keep
// the rail's 13 px, the size of the initials before the acronym.
const ACRONYM_FONT_SIZES = [13, 13, 13, 13, 13, 11, 10, 8];

export function guildAcronymFontSize(acronym) {
  const length = graphemes(acronym === null || acronym === undefined ? '' : String(acronym)).length;
  return ACRONYM_FONT_SIZES[Math.min(length, ACRONYM_FONT_SIZES.length - 1)];
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
    case 'member_banned':
      return `Un admin t’a banni de ${quoted('une guilde')}. La guilde est retirée de LiveChat.`;
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

// Human-readable size in French: "1,2 Go", "512 Mo", "0 o". Decimals are only
// kept when they carry information ("5 Go", not "5,0 Go").
export function formatBytes(bytes) {
  if (bytes === null || bytes === undefined) {
    return null;
  }
  const value = Number(bytes);
  if (!Number.isFinite(value) || value < 0) {
    return null;
  }
  if (value < 1024 * 1024) {
    return `${Math.round(value / 1024)} Ko`;
  }
  const gigabytes = value / (1024 * 1024 * 1024);
  if (gigabytes >= 1) {
    return `${gigabytes.toLocaleString('fr-FR', { maximumFractionDigits: 1 })} Go`;
  }
  return `${(value / (1024 * 1024)).toLocaleString('fr-FR', { maximumFractionDigits: 1 })} Mo`;
}

// Occupation du board de la guilde: "10 médias · 22 Mo sur 5 Go". `requireMax`
// is false where the line must still show what the board holds before a quota
// is set (the guild settings); the board toolbar hides the line instead.
export function formatBoardUsage(mediaBoard, { requireMax = true } = {}) {
  if (!mediaBoard) {
    return null;
  }
  const maxBytes = Number(mediaBoard.maxBytes);
  const max = Number.isFinite(maxBytes) && maxBytes > 0 ? formatBytes(maxBytes) : null;
  if (!max && requireMax) {
    return null;
  }
  const used = formatBytes(mediaBoard.usedBytes);
  if (!used) {
    return null;
  }
  const count = Number(mediaBoard.itemsCount) || 0;
  return `${count} média${count > 1 ? 's' : ''} · ${used}${max ? ` sur ${max}` : ''}`;
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

// Short date under a board card's title: « aujourd'hui », « hier », then
// « il y a N j » up to six days, then the day and short month ("3 janv.").
// `now` is a Date or a timestamp, so the helper stays pure and testable. An
// unreadable or future date is shown as today rather than as a negative age.
const SHORT_MONTHS = ['janv.', 'févr.', 'mars', 'avr.', 'mai', 'juin', 'juil.', 'août', 'sept.', 'oct.', 'nov.', 'déc.'];

export function relativeDateLabel(iso, now = Date.now()) {
  if (!iso) {
    return null;
  }
  const date = new Date(iso);
  const reference = now instanceof Date ? now : new Date(now);
  if (Number.isNaN(date.getTime()) || Number.isNaN(reference.getTime())) {
    return null;
  }
  const startOfDay = (value) => new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
  const days = Math.round((startOfDay(reference) - startOfDay(date)) / 86_400_000);
  if (days <= 0) {
    return "aujourd'hui";
  }
  if (days === 1) {
    return 'hier';
  }
  if (days <= 6) {
    return `il y a ${days} j`;
  }
  return `${date.getDate()} ${SHORT_MONTHS[date.getMonth()]}`;
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

// Longest display time a member can choose for an image, in seconds.
export const MAX_DISPLAY_DURATION_SEC = 600;
export const INVALID_DURATION_MESSAGE = 'La durée doit être un nombre entier entre 1 et 600.';

// Image display time typed in a form: empty means "guild default" (`null`),
// else whole seconds from 1 to MAX_DISPLAY_DURATION_SEC.
export function parseDisplayDuration(raw) {
  const text = raw === null || raw === undefined ? '' : String(raw).trim();
  if (text === '') {
    return { ok: true, value: null };
  }
  if (!/^\d+$/.test(text)) {
    return { ok: false };
  }
  const value = Number(text);
  if (value < 1 || value > MAX_DISPLAY_DURATION_SEC) {
    return { ok: false };
  }
  return { ok: true, value };
}
