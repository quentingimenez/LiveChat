// `perform` with the app's default feedback: a toast on failure, an optional
// success message, and per-control pending styling.

import { actionErrorMessage, perform } from './action.js';
import { notify } from './toast.js';

export function performAction(control, key, work, options = {}) {
  return perform(control, key, work, {
    ...options,
    onSuccess: async (result) => {
      if (options.onSuccess) {
        await options.onSuccess(result);
      }
      const message = typeof options.success === 'function' ? options.success(result) : options.success;
      if (message) {
        notify(message);
      }
    },
    onError: async (error) => {
      if (options.onError) {
        await options.onError(error);
      } else {
        notify(actionErrorMessage(error), true);
      }
    },
  });
}
