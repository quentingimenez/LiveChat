// Pure helpers for the "say a text" dialog (speech synthesis, the board's
// version of the Discord commands /dire and /cdire). No DOM, no imports that
// need a browser: this module also loads under Node for its unit tests.

import { serverStatusMessage } from './http-status.js';

// Longest text spoken at once, in characters once whitespace is collapsed.
// Mirrors MAX_SPEECH_CHARS of the server (apps/lc-server/src/media/tts.rs),
// which refuses a longer text with `text_too_long`.
export const SAY_MAX_CHARS = 1000;

// Longest text shown on the overlay with the speech (MAX_MEDIA_TEXT_CHARS).
export const SAY_CAPTION_MAX_CHARS = 500;

// The text as the server will speak it: every run of whitespace becomes one
// space, both ends trimmed.
export function normalizeSpeech(text) {
  return String(text ?? '')
    .split(/\s+/u)
    .filter(Boolean)
    .join(' ');
}

// Length counted like the server: Unicode characters, not UTF-16 units.
export function speechLength(text) {
  return Array.from(normalizeSpeech(text)).length;
}

// State of the character counter under the textarea.
export function speechCounter(text, max = SAY_MAX_CHARS) {
  const length = speechLength(text);
  return {
    length,
    max,
    label: `${length} / ${max}`,
    empty: length === 0,
    tooLong: length > max,
    valid: length > 0 && length <= max,
  };
}

// Body of `POST /overlay/say`, or `null` when the text cannot be sent.
export function sayPayload({ text, caption, anonymous } = {}) {
  if (!speechCounter(text).valid) {
    return null;
  }
  const payload = { text: normalizeSpeech(text) };
  const shown = String(caption ?? '').trim();
  if (shown) {
    payload.caption = Array.from(shown).slice(0, SAY_CAPTION_MAX_CHARS).join('');
  }
  if (anonymous) {
    payload.anonymous = true;
  }
  return payload;
}

// Readable message for a refused or failed request. `fallback` is the message
// already built for the error, if any.
export function sayErrorMessage(status, code, fallback) {
  if (code === 'text_too_long') {
    return `Texte trop long : ${SAY_MAX_CHARS} caractères au maximum.`;
  }
  if (code === 'invalid_payload') {
    return 'Écris le texte à faire dire.';
  }
  if (code === 'speech_failed') {
    return 'La synthèse vocale a échoué. Réessaie avec un texte plus court ou plus tard.';
  }
  if (code === 'read_only_session') {
    return 'Les invités ne peuvent pas faire parler le bot.';
  }
  if (code === 'forbidden') {
    return 'Ton rôle ne permet pas de faire parler le bot.';
  }
  if (fallback) {
    return fallback;
  }
  return status ? serverStatusMessage(status) : 'La demande a été refusée.';
}
