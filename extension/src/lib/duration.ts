/** Shortest display time a member can choose for an image, in seconds. */
export const MIN_DISPLAY_DURATION_SEC = 1;
/** Longest display time a member can choose for an image, in seconds. */
export const MAX_DISPLAY_DURATION_SEC = 600;

/** Message shown when the duration field holds an invalid value. */
export const INVALID_DURATION_MESSAGE = 'La durée doit être un nombre entier entre 1 et 600.';

export type ParsedDisplayDuration = { ok: true; value: number | null } | { ok: false };

/**
 * Reads the image display time typed in a send form: empty means "guild
 * default" (`null`), else whole seconds within the bounds.
 */
export const parseDisplayDuration = (raw: string): ParsedDisplayDuration => {
  const trimmed = raw.trim();
  if (trimmed === '') {
    return { ok: true, value: null };
  }
  if (!/^\d+$/.test(trimmed)) {
    return { ok: false };
  }
  const value = Number(trimmed);
  if (value < MIN_DISPLAY_DURATION_SEC || value > MAX_DISPLAY_DURATION_SEC) {
    return { ok: false };
  }
  return { ok: true, value };
};
