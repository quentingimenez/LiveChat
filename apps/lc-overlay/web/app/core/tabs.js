// ARIA tab lists. The caller builds the tab and panel elements; these helpers
// only move the selected state, so the same rules drive clicks and the
// keyboard without duplicating them in each view.

// Index a tab key moves to, or null when the key is not a tab command.
export function tabIndexForKey(current, key, count) {
  if (count <= 0) {
    return null;
  }
  if (key === 'ArrowRight' || key === 'ArrowDown') {
    return (current + 1) % count;
  }
  if (key === 'ArrowLeft' || key === 'ArrowUp') {
    return (current - 1 + count) % count;
  }
  if (key === 'Home') {
    return 0;
  }
  if (key === 'End') {
    return count - 1;
  }
  return null;
}

// Makes `index` the only selected tab and the only visible panel.
export function selectTab(tabs, panels, index) {
  if (index < 0 || index >= tabs.length) {
    return;
  }
  tabs.forEach((tab, position) => {
    tab.setAttribute('aria-selected', position === index ? 'true' : 'false');
    tab.setAttribute('tabindex', position === index ? '0' : '-1');
  });
  panels.forEach((panel, position) => {
    panel.hidden = position !== index;
  });
}

// The tab to show after switching guilds: « Administration » stays selected
// when the newly active guild grants it too, and falls back to the board when
// it does not (the rail switches to guilds where you are only a member).
export function tabAfterGuildSwitch(tab, canAdmin) {
  return tab === 'admin' && !canAdmin ? 'board' : tab;
}

// The queue badge painted in the header: while a guild switch is in flight the
// queue already belongs to the next guild (its list was reset), so the previous
// count stays up until the new guild's queue is read. The badge keeps its place
// instead of vanishing and pushing the tabs on its right, then coming back.
export function queueBadgeCount(pending, lastShown, switching) {
  return switching ? lastShown : pending;
}
