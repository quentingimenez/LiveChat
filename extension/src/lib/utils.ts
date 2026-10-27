/**
 * Centralized utility functions shared across the extension.
 */

/** Type guard: returns true when value is a non-empty (after trim) string. */
export const isNonEmptyString = (value: unknown): value is string => {
  return typeof value === 'string' && value.trim().length > 0;
};

/** Coercion: trims the string and returns it, or null when empty/non-string. */
export const toNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

/** Type guard: returns true when value is a plain object (not null, not array). */
export const isRecord = (value: unknown): value is Record<string, unknown> => {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
};
