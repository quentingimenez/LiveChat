import { useState } from 'preact/hooks';
import {
  clearQueue,
  isUnavailable,
  removeQueueJob,
  reorderQueue,
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
import { moveTo } from '../../queue-order';
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
  const [reorderBusy, setReorderBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);
  // Glisser-déposer : la ligne en cours de glissement (`dragJobId`) et la ligne
  // survolée (`overJobId`). Le serveur reçoit toujours la file entière.
  const [dragJobId, setDragJobId] = useState<string | null>(null);
  const [overJobId, setOverJobId] = useState<string | null>(null);

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

  // Envoie l'ordre complet de la file puis recharge l'état serveur. Une file
  // modifiée entre-temps est refusée (409) : l'erreur est affichée et la console
  // garde l'ordre qu'elle montre, que le prochain rafraîchissement corrige.
  const sendOrder = async (ordered: QueueJob[]) => {
    if (reorderBusy) {
      return;
    }
    setReorderBusy(true);
    setActionError(null);
    try {
      await reorderQueue(guild.id, ordered.map((entry) => entry.id));
      await refresh();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
      // La file a pu changer pendant le glissement (un média est passé en
      // lecture, un autre appareil a réordonné) : on se recale sur le serveur.
      await refresh().catch(() => {});
    } finally {
      setReorderBusy(false);
    }
  };

  const resetDrag = () => {
    setDragJobId(null);
    setOverJobId(null);
  };

  // Déplacement clavier (flèches haut/bas sur la poignée) : un cran.
  const move = async (job: QueueJob, delta: number) => {
    const index = pending.findIndex((entry) => entry.id === job.id);
    const target = index + delta;
    if (index < 0 || target < 0 || target >= pending.length) {
      return;
    }
    const ordered = [...pending];
    const [moved] = ordered.splice(index, 1);
    if (!moved) {
      return;
    }
    ordered.splice(target, 0, moved);
    await sendOrder(ordered);
  };

  // Dépôt sur une ligne : le média prend la place de la cible, les autres
  // glissent autour. L'index de la cible est calculé avant le retrait.
  const dropOn = async (target: QueueJob) => {
    const fromId = dragJobId;
    resetDrag();
    if (!fromId || fromId === target.id) {
      return;
    }
    await sendOrder(moveTo(pending, fromId, target.id));
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
            <th class="is-num queue-grip-head">#</th>
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
            <tr
              class={[
                job.status === 'playing' ? 'is-air' : '',
                dragJobId === job.id ? 'is-dragging' : '',
                overJobId === job.id ? 'is-drop-target' : '',
              ]
                .filter(Boolean)
                .join(' ')}
              key={job.id}
              draggable={!reorderBusy && job.status !== 'playing'}
              onDragStart={(event) => {
                if (job.status === 'playing' || !event.dataTransfer) {
                  return;
                }
                setDragJobId(job.id);
                event.dataTransfer.effectAllowed = 'move';
                event.dataTransfer.setData('text/plain', job.id);
                // Fond appliqué tout de suite pour que l'image glissée soit opaque.
                const row = event.currentTarget;
                row.classList.add('is-dragging');
                void row.offsetWidth;
                event.dataTransfer.setDragImage(row, 16, 16);
              }}
              onDragEnd={resetDrag}
              onDragOver={(event) => {
                if (!dragJobId || dragJobId === job.id || job.status === 'playing') {
                  return;
                }
                event.preventDefault();
                if (event.dataTransfer) {
                  event.dataTransfer.dropEffect = 'move';
                }
                setOverJobId(job.id);
              }}
              onDragLeave={(event) => {
                if (overJobId === job.id && !event.currentTarget.contains(event.relatedTarget as Node | null)) {
                  setOverJobId(null);
                }
              }}
              onDrop={(event) => {
                event.preventDefault();
                void dropOn(job);
              }}
            >
              <td class="is-num queue-grip-cell">
                {job.status === 'playing' ? (
                  <span class="queue-grip-spacer" aria-hidden="true" />
                ) : (
                  <button
                    class="queue-grip"
                    type="button"
                    title="Glisser pour réordonner"
                    aria-label={`Réordonner ${jobName(job)}`}
                    disabled={reorderBusy}
                    onKeyDown={(event) => {
                      if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
                        event.preventDefault();
                        void move(job, event.key === 'ArrowUp' ? -1 : 1);
                      }
                    }}
                  >
                    <Icon name="menu" />
                  </button>
                )}
                <span class="soft">{index + 1}</span>
              </td>
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
                  <span class="queue-actions">
                    <button class="btn is-small is-quiet is-danger" type="button" onClick={() => void remove(job)}>
                      Retirer
                    </button>
                  </span>
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
