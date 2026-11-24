// REST calls to the active guild, through the Rust shell.

import { beginActivity, endActivity } from './activity.js';
import { serverStatusMessage } from './http-status.js';
import { invoke } from './ipc.js';
import { withDeadline } from './query.js';
import { describeError, state } from './state.js';
import { notify } from './toast.js';

// A read that never lands must not keep a skeleton up for the shell's 120 s
// timeout. Writes keep the shell timeout: their control shows the wait.
const READ_TIMEOUT_MS = 15_000;

// `options.silent` skips the activity bar (background prefetch).
export async function apiForGuild(guildId, method, path, body, options = {}) {
  const silent = Boolean(options && options.silent === true);
  const timeoutMs =
    options && Number.isFinite(options.timeoutMs) && options.timeoutMs > 0
      ? options.timeoutMs
      : null;
  if (!silent) {
    beginActivity();
  }
  try {
    const pending = invoke('guild_api', {
      payload: { guildId, method, path, body: body ?? null },
    });
    const response = await (method === 'GET' || timeoutMs
      ? withDeadline(pending, timeoutMs || READ_TIMEOUT_MS)
      : pending);
    if (!response) {
      throw new Error('Le serveur n’a pas répondu.');
    }
    if (!response.ok) {
      const code = response.body && response.body.error;
      const error = new Error(describeError(code, serverStatusMessage(response.status)));
      error.code = code;
      error.status = response.status;
      throw error;
    }
    return response.body || {};
  } catch (error) {
    if (error && error.code === 'timeout') {
      const timeout = new Error(describeError('timeout'));
      timeout.code = 'timeout';
      throw timeout;
    }
    if (error instanceof Error) {
      throw error;
    }
    throw new Error(typeof error === 'string' ? error : 'Le serveur n’a pas répondu.');
  } finally {
    if (!silent) {
      endActivity();
    }
  }
}

export function api(method, path, body) {
  return apiForGuild(state.activeGuildId, method, path, body);
}

// Global, read-only cache probe used by the add dialog. The server answers
// only whether it can reuse a media for this URL: no title, id or guild.
export async function lookupMediaCache(guildId, url) {
  const body = await apiForGuild(guildId, 'POST', '/overlay/media-board/cache', { url }, {
    silent: true,
    timeoutMs: 5000,
  });
  return body.cached === true;
}

// Runs an action, shows its error as a toast and returns `undefined` on
// failure.
//
// Most buttons still call `run` directly; the action bar and the pending state
// are applied to the focused button so every one of them acknowledges the
// click, without threading a control through every call site.
export async function run(action, successMessage) {
  const control =
    typeof document !== 'undefined' && document.activeElement && document.activeElement.tagName === 'BUTTON'
      ? document.activeElement
      : null;
  const wasDisabled = Boolean(control && control.disabled);
  let slowTimer = null;
  if (control) {
    control.disabled = true;
    control.setAttribute('aria-busy', 'true');
    control.setAttribute('data-pending', 'true');
    slowTimer = setTimeout(() => {
      if (control.isConnected) {
        control.classList.add('is-slow');
      }
    }, 5000);
  }
  try {
    const result = await action();
    if (successMessage) {
      notify(successMessage);
    }
    return result;
  } catch (error) {
    notify(typeof error === 'string' ? error : error.message || 'Action impossible.', true);
    return undefined;
  } finally {
    if (slowTimer) {
      clearTimeout(slowTimer);
    }
    if (control && control.isConnected) {
      control.disabled = wasDisabled;
      control.removeAttribute('aria-busy');
      control.removeAttribute('data-pending');
      control.classList.remove('is-slow');
    }
  }
}
