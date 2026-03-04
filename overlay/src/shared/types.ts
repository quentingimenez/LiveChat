export type Dictionary<TValue> = Record<string, TValue>;

export type Maybe<TValue> = TValue | null;

export interface Logger {
  info(message: string, context?: unknown): void;
  warn(message: string, context?: unknown): void;
  error(message: string, context?: unknown): void;
}
