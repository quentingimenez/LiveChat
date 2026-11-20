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
import { bytes, duration, plural } from '../../format';
import type { GuildTabProps } from './types';

const KIND_LABELS: Record<string, string> = {
  video: 'Vidéo',
  audio: 'Son',
  image: 'Image',
  text: 'Texte',
};

// Payload riche de `/ingest` (offset YouTube) : non décodé, à ne pas afficher.
const RICH_TEXT_PREFIX = '__LC_OVERLAY_RICH__::';
const ORIGIN_LABELS: Record<string, string> = {
  board: 'Board',
  once: 'Une fois',
  direct: 'Direct',
};

// Titre affichable : item de board, puis titre de la source (`sourceTitle`),
// puis légende. Un média joué une fois n'a pas d'item de board.
function jobName(job: QueueJob): string {
  if (job.title) {
    return job.title;
  }
  if (job.sourceTitle) {
    return job.sourceTitle;
  }
  if (job.text && !job.text.startsWith(RICH_TEXT_PREFIX)) {
    return job.text;
  }
  return 'Média';
}

// Origine lue sur le serveur, avec le repli des serveurs plus anciens.
function jobOrigin(job: QueueJob): string {
  return ORIGIN_LABELS[job.origin ?? ''] ?? (job.mediaAssetId ? 'Board' : 'Direct');
}

function sourceHost(sourceUrl: string | null | undefined): string | null {
  if (!sourceUrl) {
    return null;
  }
  try {
    const url = new URL(sourceUrl);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') {
      return null;
    }
    return url.hostname.replace(/^www\./, '') || null;
  } catch {
    return null;
  }
}

// Détails du média (format, dimensions, taille, source) sous le titre.
function jobDetails(job: QueueJob): string | null {
  const media = job.media;
  const parts: string[] = [];
  const kind = KIND_LABELS[media?.kind ?? job.kind ?? ''];
  if (kind) {
    parts.push(kind);
  }
  if (media?.durationSec || job.durationSec) {
    parts.push(duration(media?.durationSec || job.durationSec));
  }
  if (media?.width && media.height) {
    parts.push(`${media.width} × ${media.height}`);
  }
  if (media?.sizeBytes) {
    parts.push(bytes(media.sizeBytes));
  }
  const host = sourceHost(media?.sourceUrl);
  if (host) {
    parts.push(host);
  }
  return parts.length ? parts.join(' · ') : null;
}

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
              <td class="primary">
                {jobName(job)}
                {jobDetails(job) ? <div class="soft">{jobDetails(job)}</div> : null}
              </td>
              <td>
                <Badge>{KIND_LABELS[job.kind ?? ''] ?? job.kind ?? '—'}</Badge>
              </td>
              <td class="is-num">{duration(job.durationSec)}</td>
              <td>{job.authorName ? <Who name={job.authorName} /> : '—'}</td>
              <td class="soft">{jobOrigin(job)}</td>
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
