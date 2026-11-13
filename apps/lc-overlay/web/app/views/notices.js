// Guild removal notices sent by the Rust shell.
//
// The shell removes a guild by itself when it is deleted, when the member is
// excluded or when the device is revoked. It keeps the pending removals and
// drains them on `board_take_guild_removals`; the command empties the list, so
// the calls are chained instead of running in parallel: a removal is shown
// only once.

import { removalMessage } from '../core/format.js';
import { invoke, listen } from '../core/ipc.js';
import { toast } from '../core/toast.js';
import { applyGuilds } from '../data.js';

let chain = Promise.resolve();

async function takeRemovals() {
  let removals;
  try {
    removals = await invoke('board_take_guild_removals');
  } catch {
    // Older shell or dev harness: no removal list to read.
    return;
  }
  if (!Array.isArray(removals) || removals.length === 0) {
    return;
  }
  for (const removal of removals) {
    toast({ message: removalMessage(removal || {}), kind: 'info', timeout: null });
  }
  try {
    applyGuilds(await invoke('guilds_list'));
  } catch {
    // The next overlay://guilds event refreshes the rail anyway.
  }
}

export function initGuildRemovalNotices() {
  const refresh = () => {
    chain = chain.then(takeRemovals).catch(() => {});
  };
  listen('overlay://guild-removed', refresh);
  refresh();
}
