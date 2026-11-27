// Main view router: settings page, or board, queue, admin or settings of the
// active guild.

import { h } from '../core/dom.js';
import { fitSingleLineDescriptions } from '../core/measure.js';
import { viewScrollToKeep } from '../core/scroll-memory.js';
import { state } from '../core/state.js';
import { renderAdmin } from './admin.js';
import { closeDetail, renderBoard } from './board.js';
import { openJoinDialog } from './join.js';
import { renderGuildSettings } from './guild-settings.js';
import { renderQueue } from './queue.js';
import { renderSettings } from './settings.js';

// Identity of the view currently in #view. Repainting the same view keeps its
// position; a real change (page, guild, tab, admin section) starts at the top.
let paintedView = null;

function shownViewKey() {
  if (state.page === 'settings') {
    return 'settings';
  }
  if (!state.activeGuildId) {
    return 'join';
  }
  const section = state.tab === 'admin' ? state.adminTab || '' : '';
  return `guild:${state.activeGuildId}:${state.tab}:${section}`;
}

export function renderView() {
  const view = document.getElementById('view');
  // The media detail window is appended to the window, not to the view: it
  // must not stay over another tab. Leaving the board closes it (the board's
  // own refreshes keep it open and only refresh its information column).
  const showsBoard = state.page !== 'settings' && Boolean(state.activeGuildId) && state.tab === 'board';
  if (!showsBoard) {
    closeDetail();
  }
  // A repaint of the same view can move the scroller (WebView2 clamps it back to
  // the top on some repaints, Chromium's scroll anchoring shifts it while the
  // children are replaced), so the position is saved around the swap and put
  // back once the new content is in place. Switching view (tab, page, guild,
  // admin section) still starts at the top.
  const previousView = paintedView;
  const nextView = shownViewKey();
  const scrollTop = view.scrollTop;
  if (state.page === 'settings') {
    view.replaceChildren(renderSettings());
  } else if (!state.activeGuildId) {
    view.replaceChildren(
      h(
        'div',
        { class: 'empty' },
        h('h2', null, 'Rejoins ta première guilde'),
        h('p', null, "Un admin te donne un code. Il fixe ton rôle : admin, membre ou invité."),
        h('button', { class: 'btn is-air', onClick: openJoinDialog }, 'Saisir un code'),
      ),
    );
  } else if (state.tab === 'queue') {
    view.replaceChildren(renderQueue());
  } else if (state.tab === 'admin') {
    view.replaceChildren(renderAdmin());
  } else if (state.tab === 'settings') {
    view.replaceChildren(renderGuildSettings());
  } else {
    view.replaceChildren(renderBoard());
  }
  // The 62ch measure is a fallback: the sentences that fit stay on one line.
  fitSingleLineDescriptions(view);
  const keepScroll = viewScrollToKeep({ previousView, nextView, scrollTop });
  view.scrollTop = keepScroll;
  paintedView = nextView;
}
