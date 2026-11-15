// Queue tab and "on air" bar.

import { api, run } from '../core/api.js';
import { openContextMenu } from '../core/contextmenu.js';
import { h, icon } from '../core/dom.js';
import { formatDuration } from '../core/format.js';
import { invoke } from '../core/ipc.js';
import { can, describeError, KIND_LABELS, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { loadQueue, refreshQueueAutomatically } from '../data.js';

export function jobTitle(job) {
  return job.title || job.text || (job.kind ? KIND_LABELS[job.kind] : null) || 'Média';
}

// Stop actions offered for the playing media. « Chez moi » only clears this
// device's overlay, so it needs no permission; « pour tous » stops the
// guild's playback and requires the `stop` permission.
export function stopActions(canStopForAll) {
  const actions = [{ id: 'local', label: 'Arrêter chez moi', title: 'Arrêter le média sur cet appareil seulement', onSelect: stopPlaybackLocally }];
  if (canStopForAll) {
    actions.push({ id: 'all', label: 'Arrêter pour tous', title: 'Arrêter le média pour toute la guilde', onSelect: stopPlayback });
  }
  return actions;
}

function stopButtons() {
  return stopActions(can('stop')).map((action) =>
    h('button', { class: action.id === 'local' ? 'btn is-quiet' : 'btn', title: action.title, onClick: action.onSelect }, icon('stop'), action.label),
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
    { label: 'Retirer de la file', icon: 'close', onSelect: () => removeJob(job) },
    'separator',
    { label: 'Vider la file', icon: 'remove', danger: true, onSelect: () => clearQueue() },
  ]);
}

// The on-air bar is reused (only its children are replaced): bind its menu once.
let onAirMenuBound = false;

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

export function renderQueue() {
  const { playing, pending } = state.queue;
  const blocks = [];
  if (playing) {
    const started = playing.startedAt ? new Date(playing.startedAt).getTime() : null;
    const elapsed = started ? Math.max(0, (Date.now() - started) / 1000) : 0;
    const ratio = playing.durationSec > 0 ? Math.min(1, elapsed / playing.durationSec) : 0;
    blocks.push(
      h(
        'div',
        { class: 'now', onContextMenu: openStopMenu },
        h(
          'div',
          { class: 'row' },
          h('strong', { class: 'ellipsis' }, jobTitle(playing)),
          playing.authorName ? h('span', { class: 'meta' }, `par ${playing.authorName}`) : null,
          h('span', { class: 'spacer' }),
          h('span', { class: 'meta' }, `${formatDuration(Math.min(elapsed, playing.durationSec)) || '0:00'} / ${formatDuration(playing.durationSec) || '0:00'}`),
          stopButtons(),
        ),
        h('div', { class: 'progress' }, h('span', { style: `width:${Math.round(ratio * 100)}%` })),
      ),
    );
  }
  if (pending.length === 0) {
    blocks.push(
      h(
        'div',
        { class: 'empty' },
        h('h2', null, playing ? 'Rien d’autre en attente' : 'La file est vide'),
        h('p', null, 'Les médias envoyés à la guilde apparaissent ici avant de passer à l’écran.'),
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
            h('div', { class: 'ellipsis' }, jobTitle(job), job.authorName ? h('span', { class: 'sub' }, `  par ${job.authorName}`) : null),
            h('span', { class: 'meta' }, formatDuration(job.durationSec) || ''),
            can('moderateQueue')
              ? h('button', { class: 'btn is-quiet', 'aria-label': `Retirer ${jobTitle(job)} de la file`, title: 'Retirer de la file', onClick: () => removeJob(job) }, icon('close'))
              : h('span'),
          ),
        ),
      ),
      h(
        'div',
        { class: 'row', style: 'margin-top:14px' },
        // Pending jobs often have no duration yet (it is fixed when playback starts).
        h('span', { class: 'meta' }, formatDuration(total) ? `${pending.length} en attente, ${formatDuration(total)} au total` : `${pending.length} en attente`),
        h('span', { class: 'spacer' }),
        can('moderateQueue') ? h('button', { class: 'btn is-danger', onClick: clearQueue }, 'Vider la file') : null,
      ),
    );
  }
  return h('div', null, blocks);
}

export async function stopPlayback() {
  const response = await run(() => invoke('media_board_stop', { payload: { guildId: state.activeGuildId } }));
  if (response && !response.ok) {
    notify(describeError(response.reason), true);
  }
  setTimeout(refreshQueueAutomatically, 400);
}

// Stops the media on this device only: the guild keeps playing it, so the
// queue is left as it is.
export async function stopPlaybackLocally() {
  const response = await run(() => invoke('media_board_stop_local'));
  if (!response) {
    return;
  }
  if (response.ok) {
    notify('Média arrêté sur cet appareil.');
  } else if (response.reason === 'nothing_playing') {
    notify('Aucun média à l’écran sur cet appareil.');
  } else {
    notify(describeError(response.reason), true);
  }
}

export async function removeJob(job) {
  await run(() => api('DELETE', `/overlay/queue/${encodeURIComponent(job.id)}`));
  loadQueue();
}

export async function clearQueue() {
  await run(() => api('DELETE', '/overlay/queue'), 'File vidée.');
  loadQueue();
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
    h('span', { class: 'title ellipsis' }, jobTitle(playing)),
    playing.authorName ? h('span', { class: 'meta' }, `par ${playing.authorName}`) : null,
    h('span', { class: 'spacer' }),
    state.queue.pending.length ? h('span', { class: 'meta' }, `${state.queue.pending.length} en attente`) : null,
    ...stopButtons(),
  ];
  bar.replaceChildren(...children.filter(Boolean));
}
