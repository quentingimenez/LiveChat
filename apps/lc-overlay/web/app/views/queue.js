// Queue tab and "on air" bar.

import { api } from '../core/api.js';
import { performAction } from '../core/action-ui.js';
import { openContextMenu } from '../core/contextmenu.js';
import { h, icon } from '../core/dom.js';
import { formatDuration, relativeTime } from '../core/format.js';
import { invoke } from '../core/ipc.js';
import { jobMeta, jobOriginLabel, jobOriginTitle, jobTitle } from '../core/job-format.js';
import { queueClock } from '../core/queue-clock.js';
import { can, describeError, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { confirmDialog } from '../core/ui.js';
import { loadQueue, refreshQueueAutomatically } from '../data.js';
import { renderHeader } from './header.js';
import { renderView } from './view.js';

// Where the job comes from: a board item ("board"), a one-off play / client
// ingest without a board item ("once") or a text sent straight to the queue
// ("direct"). Hidden when the server does not say (older server).
function originBadge(job) {
  const label = jobOriginLabel(job);
  if (!label) {
    return null;
  }
  return h('span', { class: 'badge queue-origin', title: jobOriginTitle(job) || '' }, label);
}

// First line: the resolved title and its origin. Second line: the author
// and/or the media details (`jobMeta`), so a one-off play or an ingest keeps
// its format, size and source in the queue and in the history.
function jobWhat(job, author) {
  const details = [author ? `Ajouté par ${author}` : null, jobMeta(job)].filter(Boolean).join(' · ');
  return h(
    'div',
    { class: 'queue-what' },
    h('div', { class: 'queue-title' }, h('span', { class: 'ellipsis' }, jobTitle(job)), originBadge(job)),
    details ? h('div', { class: 'sub ellipsis' }, details) : null,
  );
}

// Stop actions offered for the playing media. « Ici » only clears this
// device's overlay, so it needs no permission; « pour la guilde » stops the
// guild's playback and requires the `stop` permission.
export function stopActions(canStopForAll) {
  const actions = [
    {
      id: 'local',
      label: 'Arrêter ici',
      title: 'Arrête la lecture sur cet appareil seulement',
      onPress: (control) => stopPlaybackLocally(control),
      onSelect: () => stopPlaybackLocally(),
    },
  ];
  if (canStopForAll) {
    actions.push({
      id: 'all',
      label: 'Arrêter pour la guilde',
      title: 'Arrête la lecture pour tous les overlays de la guilde',
      onPress: (control) => stopPlayback(control),
      onSelect: () => stopPlayback(),
    });
  }
  return actions;
}

function stopButtons() {
  return stopActions(can('stop')).map((action) =>
    h(
      'button',
      {
        class: action.id === 'local' ? 'btn is-quiet' : 'btn is-danger',
        title: action.title,
        onClick: (event) => action.onPress(event.currentTarget),
      },
      icon('stop'),
      action.label,
    ),
  );
}

function openStopMenu(event) {
  openContextMenu(
    event,
    stopActions(can('stop')).map((action) => ({ label: action.label, icon: 'stop', onSelect: action.onSelect })),
  );
}

function openPendingMenu(event, job) {
  // Without the moderation permission there is no action, so no menu.
  if (!can('moderateQueue')) {
    openContextMenu(event, []);
    return;
  }
  openContextMenu(event, [
    { heading: jobTitle(job) },
    { label: 'Retirer', icon: 'close', onSelect: () => removeJob(job) },
    'separator',
    { label: 'Vider…', icon: 'remove', danger: true, onSelect: () => clearQueue() },
  ]);
}

// The on-air bar is reused (only its children are replaced): bind its menu once.
let onAirMenuBound = false;

// The history is a <details> rebuilt on every queue render: remember whether the
// user opened it (and for which guild) so a refresh does not close it.
let historyOpenGuildId = null;
let historyOpen = false;

// One-second tick: update only the clock and the progress bar. A full
// `renderView()` here would close an open history and drop keyboard focus.
export function tickQueueClock(now = Date.now()) {
  const playing = state.queue.playing;
  if (!playing) {
    return;
  }
  const clock = queueClock(playing, now);
  const time = document.querySelector('[data-queue-clock]');
  if (time) {
    time.textContent = clock.text;
  }
  const bar = document.querySelector('[data-queue-progress]');
  if (bar) {
    bar.style.width = `${Math.round(clock.ratio * 100)}%`;
  }
}

function bindOnAirMenu() {
  if (onAirMenuBound) {
    return;
  }
  const bar = document.getElementById('onair');
  if (!bar) {
    return;
  }
  bar.addEventListener('contextmenu', openStopMenu);
  onAirMenuBound = true;
}

function queueSkeleton() {
  return h(
    'div',
    { class: 'list queue-skeleton', role: 'status', 'aria-busy': 'true' },
    h('span', { class: 'sr-only' }, 'Chargement de la file…'),
    Array.from({ length: 3 }, () =>
      h('div', { class: 'line is-queue is-skeleton' }, h('span', { class: 'skeleton-block' })),
    ),
  );
}

function retryQueue() {
  if (!state.queueError) {
    return;
  }
  state.queueLoaded = false;
  state.queueError = null;
  renderView();
  loadQueue();
}

export function renderQueue() {
  const { playing, pending } = state.queue;
  // Before the first read, a queue is not empty: it is unknown.
  if (!state.queueLoaded && !state.queueError) {
    return h('div', null, queueSkeleton(), renderHistory());
  }
  const blocks = [];
  if (playing) {
    const clock = queueClock(playing);
    blocks.push(
      h(
        'div',
        { class: 'now', onContextMenu: openStopMenu },
        h(
          'div',
          { class: 'row' },
          h('strong', { class: 'ellipsis' }, jobTitle(playing)),
          originBadge(playing),
          playing.authorName ? h('span', { class: 'meta' }, `par ${playing.authorName}`) : null,
          h('span', { class: 'spacer' }),
          h('span', { class: 'meta queue-time', 'data-queue-clock': '' }, clock.text),
        ),
        jobMeta(playing) ? h('div', { class: 'sub ellipsis' }, jobMeta(playing)) : null,
        h('div', { class: 'progress' }, h('span', { 'data-queue-progress': '', style: `width:${Math.round(clock.ratio * 100)}%` })),
      ),
    );
  }
  if (state.queueError && !playing && pending.length === 0) {
    blocks.push(
      h(
        'div',
        { class: 'empty' },
        h('h2', null, 'File indisponible'),
        h('p', null, state.queueError),
        h('button', { class: 'btn is-primary', onClick: retryQueue }, 'Réessayer'),
      ),
    );
  } else if (pending.length === 0) {
    blocks.push(
      h(
        'div',
        { class: 'empty' },
        h('h2', null, playing ? 'Rien d’autre en attente' : 'La file est vide'),
      ),
    );
  } else {
    const total = pending.reduce((sum, job) => sum + (job.durationSec || 0), 0);
    blocks.push(
      h(
        'div',
        { class: 'list' },
        pending.map((job, index) =>
          h(
            'div',
            { class: 'line is-queue', onContextMenu: (event) => openPendingMenu(event, job) },
            h('span', { class: 'meta' }, index + 1),
            jobWhat(job, job.authorName),
            h('span', { class: 'meta' }, formatDuration(job.durationSec) || ''),
            can('moderateQueue')
              ? h(
                  'button',
                  {
                    class: 'btn is-quiet icon-btn',
                    'aria-label': `Retirer ${jobTitle(job)} de la file`,
                    title: 'Retirer',
                    onClick: (event) => removeJob(job, event.currentTarget),
                  },
                  icon('close'),
                )
              : h('span'),
          ),
        ),
      ),
      h(
        'div',
        { class: 'row is-wrap queue-foot' },
        // Pending jobs often have no duration yet (it is fixed when playback starts).
        h(
          'span',
          { class: 'meta' },
          formatDuration(total)
            ? `${pending.length} média${pending.length > 1 ? 's' : ''} en attente · ${formatDuration(total)} au total`
            : `${pending.length} média${pending.length > 1 ? 's' : ''} en attente`,
        ),
        h('span', { class: 'spacer' }),
        can('moderateQueue') ? h('button', { class: 'btn is-danger', onClick: clearQueue }, 'Vider…') : null,
      ),
    );
  }
  if (state.queueError && (playing || pending.length > 0)) {
    blocks.push(
      h(
        'div',
        { class: 'board-banner is-stale', role: 'status' },
        h('span', null, 'Impossible d’actualiser la file.'),
        h('button', { class: 'btn is-quiet', onClick: retryQueue }, 'Réessayer'),
      ),
    );
  }
  return h('div', null, blocks, renderHistory());
}

// Last finished/failed plays of the guild, collapsed by default. The status is
// drawn as a coloured glyph so the row stays readable without an icon font.
function renderHistory() {
  if (!can('viewQueue') || state.queueHistory.length === 0) {
    return null;
  }
  if (state.activeGuildId !== historyOpenGuildId) {
    historyOpenGuildId = state.activeGuildId;
    historyOpen = false;
  }
  const items = state.queueHistory.slice(0, 30);
  const rows = items.map((job) => {
    const failed = job.status === 'failed';
    return h(
      'div',
      { class: 'line is-history' },
      h('span', { class: `history-status ${failed ? 'is-err' : 'is-ok'}`, title: failed ? 'Échec' : 'Terminé' }, failed ? icon('close') : icon('check')),
      jobWhat(job, null),
      h('span', { class: 'sub ellipsis' }, job.authorName ? `par ${job.authorName}` : '—'),
      h('span', { class: 'sub', title: job.finishedAt || '' }, relativeTime(job.finishedAt)),
    );
  });
  return h(
    'details',
    {
      class: 'queue-history',
      open: historyOpen,
      onToggle: (event) => {
        if (event.target === event.currentTarget) {
          historyOpen = event.currentTarget.open;
        }
      },
    },
    h('summary', null, `Historique (${items.length})`),
    h('div', { class: 'list' }, rows),
  );
}

export async function stopPlayback(control) {
  await performAction(control, 'queue-stop', async () => {
    const response = await invoke('media_board_stop', { payload: { guildId: state.activeGuildId } });
    if (response && !response.ok) {
      throw new Error(describeError(response.reason));
    }
    setTimeout(refreshQueueAutomatically, 400);
  });
}

// Stops the media on this device only: the guild keeps playing it, so the
// queue is left as it is.
export async function stopPlaybackLocally(control) {
  await performAction(control, 'queue-stop-local', async () => {
    const response = await invoke('media_board_stop_local');
    if (!response) {
      return;
    }
    if (response.ok) {
      notify('Média arrêté sur cet appareil.');
    } else if (response.reason === 'nothing_playing') {
      notify('Aucun média à l’écran sur cet appareil.');
    } else {
      throw new Error(describeError(response.reason));
    }
  });
}

function paintQueue() {
  renderHeader();
  renderOnAir();
  renderView();
}

export async function removeJob(job, control) {
  const index = state.queue.pending.findIndex((entry) => entry.id === job.id);
  const removed = index >= 0 ? state.queue.pending[index] : null;
  await performAction(control, `queue-remove:${job.id}`, () => api('DELETE', `/overlay/queue/${encodeURIComponent(job.id)}`), {
    // The row leaves at once; a refusal puts it back at its place.
    optimistic: () => {
      if (!removed) {
        return;
      }
      const at = state.queue.pending.findIndex((entry) => entry.id === job.id);
      if (at >= 0) {
        state.queue.pending.splice(at, 1);
        paintQueue();
      }
    },
    rollback: () => {
      if (!removed || state.queue.pending.some((entry) => entry.id === job.id)) {
        return;
      }
      state.queue.pending.splice(Math.min(index, state.queue.pending.length), 0, removed);
      paintQueue();
    },
    onSuccess: () => loadQueue(),
  });
}

export async function clearQueue() {
  const count = state.queue.pending.length;
  if (count === 0) {
    return;
  }
  const plural = count > 1;
  confirmDialog(
    'Vider la file d’attente ?',
    `Les ${count} média${plural ? 's' : ''} en attente ser${plural ? 'ont' : 'a'} retiré${plural ? 's' : ''}. Le média à l’antenne n’est pas concerné.`,
    'Vider',
    async () => {
      await performAction(null, 'queue-clear', () => api('DELETE', '/overlay/queue'), {
        success: 'File vidée.',
        onSuccess: () => loadQueue(),
      });
    },
  );
}

export function renderOnAir() {
  bindOnAirMenu();
  const bar = document.getElementById('onair');
  const playing = state.queue.playing;
  if (!playing || !state.activeGuildId) {
    bar.hidden = true;
    return;
  }
  bar.hidden = false;
  // replaceChildren() would render a null child as the text "null": filter first.
  const children = [
    h('span', { class: 'pulse' }),
    h('span', { class: 'title ellipsis', title: jobTitle(playing) }, jobTitle(playing)),
    playing.authorName ? h('span', { class: 'meta onair-author' }, `par ${playing.authorName}`) : null,
    h('span', { class: 'spacer' }),
    state.queue.pending.length
      ? h(
          'span',
          { class: 'meta', title: `${state.queue.pending.length} média${state.queue.pending.length > 1 ? 's' : ''} en attente` },
          `${state.queue.pending.length} média${state.queue.pending.length > 1 ? 's' : ''} en attente`,
        )
      : null,
    ...stopButtons(),
  ];
  bar.replaceChildren(...children.filter(Boolean));
}
