// IPC with the Rust shell. A guild token never leaves the shell: every REST
// call goes through the `guild_api` command.

const internals = window.__TAURI_INTERNALS__ || null;

export function invoke(command, args) {
  if (!internals) {
    return Promise.reject(new Error('Application ouverte hors de LiveChat.'));
  }
  return internals.invoke(command, args);
}

export function listen(eventName, handler) {
  if (!internals) {
    return;
  }
  const id = internals.transformCallback((event) => handler(event && event.payload), false);
  invoke('plugin:event|listen', { event: eventName, target: { kind: 'Any' }, handler: id }).catch(() => {});
}
