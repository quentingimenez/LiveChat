export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function toTrimmedString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : '';
}

export function toBoolean(value: unknown): boolean {
  return value === true;
}

export function toFiniteNumber(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}
