import type { Logger } from '../../shared/types';

export const logger: Logger = {
  info(message: string, context?: unknown): void {
    if (context === undefined) {
      console.info(message);
      return;
    }

    console.info(message, context);
  },

  warn(message: string, context?: unknown): void {
    if (context === undefined) {
      console.warn(message);
      return;
    }

    console.warn(message, context);
  },

  error(message: string, context?: unknown): void {
    if (context === undefined) {
      console.error(message);
      return;
    }

    console.error(message, context);
  }
};
