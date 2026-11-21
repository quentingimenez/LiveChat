// Rules of the guild rail.

// A double click on a guild tile opens that guild's board, whichever tab was
// open (queue, administration, guild settings, device settings). The two
// clicks have already asked the shell to select the guild: this only moves the
// view, and says whether it must be painted now.
//
// A switch in flight paints its own header and board once the new guild's info
// lands: painting here would show the previous guild's board under the new
// guild's name, without the tab fallback of its permissions.
export function openGuildBoard(appState, guildId) {
  appState.page = 'guild';
  appState.tab = 'board';
  return !appState.switchingGuild && guildId === appState.activeGuildId;
}
