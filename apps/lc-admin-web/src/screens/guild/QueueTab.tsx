import { useState } from 'preact/hooks';
import {
  clearQueue,
  isUnavailable,
  removeQueueJob,
  stopGuild,
} from '../../api/client';
import type { QueueJob } from '../../api/types';
import { ConfirmDialog } from '../../components/Dialog';
import { Icon } from '../../components/icons';
import {
  Badge,
  Empty,
  ErrorNotice,
  Loading,
  Table,
  Unavailable,
  Who,
} from '../../components/ui';
import { describeError } from '../../errors';
import { duration, plural } from '../../format';
import type { GuildTabProps } from './types';

const KIND_LABELS: Record<string, string> = {
  video: 'Vidéo',
  audio: 'Son',
  image: 'Image',
  text: 'Texte',
};

export function QueueTab({ guild, detail, detailUnavailable, refresh, notify }: GuildTabProps) {
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearBusy, setClearBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);

  const queue = detail?.queue;
  const playing = queue?.playing ?? null;
  const pending = queue?.pending ?? [];
  const rows: QueueJob[] = playing ? [playing, ...pending] : pending;

  const stop = async () => {
    try {
      await stopGuild(guild.id);
      notify('Lecture arrêtée.');
      await refresh();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    }
  };

  const remove = async (job: QueueJob) => {
    try {
      await removeQueueJob(guild.id, job.id);
      notify('Média retiré de la file.');
      await refresh();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    }
  };

  const clear = async () => {
    if (clearBusy) {
      return;
    }
    setClearBusy(true);
    setActionError(null);
    try {
      await clearQueue(guild.id);
      setConfirmClear(false);
      notify('File vidée.');
      await refresh();
    } catch (error) {
      setConfirmClear(false);
      setActionError(error);
      notify(describeError(error), 'error');
    } finally {
      setClearBusy(false);
    }
  };

  let body;
  if (detailUnavailable) {
    body = (
      <Unavailable detail="La file d’attente (GET /admin/api/guilds/{id}) n’est pas exposée par ce serveur." />
    );
  } else if (!detail) {
    body = <Loading label="Chargement de la file…" />;
  } else if (!rows.length) {
    body = (
      <div class="card">
        <Empty>Aucune lecture en cours ni en attente.</Empty>
      </div>
    );
  } else {
    body = (
      <Table>
        <thead>
          <tr>
            <th class="is-num">#</th>
            <th>Média</th>
            <th>Type</th>
            <th class="is-num">Durée</th>
            <th>Demandé par</th>
            <th>Origine</th>
            <th>État</th>
            <th>
              <span class="sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((job, index) => (
            <tr class={job.status === 'playing' ? 'is-air' : ''} key={job.id}>
              <td class="is-num soft">{index + 1}</td>
              <td class="primary">{job.title || job.text || 'Média'}</td>
              <td>
                <Badge>{KIND_LABELS[job.kind ?? ''] ?? job.kind ?? '—'}</Badge>
              </td>
              <td class="is-num">{duration(job.durationSec)}</td>
              <td>{job.authorName ? <Who name={job.authorName} /> : '—'}</td>
              <td class="soft">{job.mediaAssetId ? 'Board' : 'Direct'}</td>
              <td>
                {job.status === 'playing' ? (
                  <Badge tone="air">À l’antenne</Badge>
                ) : (
                  <Badge>En attente</Badge>
                )}
              </td>
              <td class="is-actions">
                {job.status === 'playing' ? null : (
                  <button
                    class="btn is-small is-quiet is-danger"
                    type="button"
                    onClick={() => void remove(job)}
                  >
                    Retirer
                  </button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </Table>
    );
  }

  return (
    <>
      <div class="toolbar">
        <p class="soft">
          {rows.length
            ? `${plural(rows.length, 'média', 'médias')} dans la file, dont celui à l’antenne.`
            : 'La file est vide.'}
        </p>
        <span class="spacer" />
        <button class="btn" type="button" onClick={() => setConfirmClear(true)} disabled={pending.length === 0}>
          Vider la file
        </button>
        <button class="btn" type="button" onClick={() => void stop()} disabled={!playing}>
          <Icon name="stop" />
          Arrêter la lecture
        </button>
      </div>

      {isUnavailable(actionError) ? (
        <Unavailable detail="Cette action sur la file n’est pas exposée par ce serveur." />
      ) : actionError ? (
        <ErrorNotice error={actionError} title="Action impossible" />
      ) : null}

      {body}

      <p class="table-foot">
        Historique : {guild.playback.done} lectures terminées, {guild.playback.failed} en échec
        (conservées 24 h).
      </p>

      <ConfirmDialog
        open={confirmClear}
        title="Vider la file d’attente ?"
        body={`Les ${pending.length} médias en attente sont retirés. La lecture en cours continue.`}
        confirmLabel="Vider la file"
        busy={clearBusy}
        onCancel={() => setConfirmClear(false)}
        onConfirm={() => void clear()}
      />
    </>
  );
}
