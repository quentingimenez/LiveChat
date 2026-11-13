// Main view router: board, queue or admin of the active guild.

import { h } from '../core/dom.js';
import { state } from '../core/state.js';
import { renderAdmin } from './admin.js';
import { renderBoard } from './board.js';
import { openJoinDialog } from './join.js';
import { renderQueue } from './queue.js';

export function renderView() {
  const view = document.getElementById('view');
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
  } else {
    view.replaceChildren(renderBoard());
  }
}
