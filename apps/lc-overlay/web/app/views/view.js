// Main view router: settings page, or board, queue, admin or settings of the
// active guild.

import { h } from '../core/dom.js';
import { fitSingleLineDescriptions } from '../core/measure.js';
import { boardScrollToKeep } from '../core/scroll-memory.js';
import { state } from '../core/state.js';
import { renderAdmin } from './admin.js';
import { renderBoard } from './board.js';
import { openJoinDialog } from './join.js';
import { renderGuildSettings } from './guild-settings.js';
import { renderQueue } from './queue.js';
import { renderSettings } from './settings.js';

export function renderView() {
  const view = document.getElementById('view');
  // The board is repainted often (selection mode, refresh, filters). WebView2
  // may clamp the scroller back to the top while its children are replaced, so
  // the position is saved around the swap and put back once the new content is
  // in place. Switching view (tab, page, guild) still starts at the top.
  const hadBoard = view.querySelector('.board') !== null;
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
  const keepScroll = boardScrollToKeep({
    hadBoard,
    hasBoard: view.querySelector('.board') !== null,
    scrollTop,
  });
  if (keepScroll !== null) {
    view.scrollTop = keepScroll;
  }
}
