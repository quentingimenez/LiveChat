// REST calls to the active guild, through the Rust shell.

import { invoke } from './ipc.js';
import { describeError, state } from './state.js';
import { notify } from './toast.js';

export async function api(method, path, body) {
  const guildId = state.activeGuildId;
  const response = await invoke('guild_api', { payload: { guildId, method, path, body: body ?? null } });
  if (!response.ok) {
    const code = response.body && response.body.error;
    const error = new Error(describeError(code, `Le serveur a répondu ${response.status}.`));
    error.code = code;
    error.status = response.status;
    throw error;
  }
  return response.body || {};
}

// Runs an action, shows its error as a toast and returns `undefined` on
// failure.
export async function run(action, successMessage) {
  try {
    const result = await action();
    if (successMessage) {
      notify(successMessage);
    }
    return result;
  } catch (error) {
    notify(typeof error === 'string' ? error : error.message || 'Action impossible.', true);
    return undefined;
  }
}

export async function mediaUrl(assetId) {
  const key = `${state.activeGuildId}:${assetId}`;
  if (!state.mediaUrls.has(key)) {
    state.mediaUrls.set(key, await invoke('guild_media_url', { payload: { guildId: state.activeGuildId, assetId } }));
  }
  return state.mediaUrls.get(key);
}
