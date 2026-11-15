// Main view router: settings page, or board, queue, admin or settings of the
// active guild.

import { h } from '../core/dom.js';
import { state } from '../core/state.js';
import { renderAdmin } from './admin.js';
import { renderBoard } from './board.js';
import { openJoinDialog } from './join.js';
import { renderGuildSettings } from './guild-settings.js';
import { renderQueue } from './queue.js';
import { renderSettings } from './settings.js';

export function renderView() {
  const view = document.getElementById('view');
  if (state.page === 'settings') {
    view.replaceChildren(renderSettings());
    return;
  }
  if (!state.activeGuildId) {
    view.replaceChildren(
      h(
        'div',
        { class: 'empty' },
        h('h2', null, 'Rejoins ta première guilde'),
        h('p', null, "Un admin te donne un code. Il fixe ton rôle : admin, membre ou invité."),
        h('button', { class: 'btn is-air', onClick: openJoinDialog }, 'Saisir un code'),
      ),
    );
    return;
  }
  if (state.tab === 'queue') {
    view.replaceChildren(renderQueue());
  } else if (state.tab === 'admin') {
    view.replaceChildren(renderAdmin());
  } else if (state.tab === 'settings') {
    view.replaceChildren(renderGuildSettings());
  } else {
    view.replaceChildren(renderBoard());
  }
}
