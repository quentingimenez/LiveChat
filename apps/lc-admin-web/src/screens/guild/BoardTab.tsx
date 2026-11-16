import { useEffect, useMemo, useState } from 'preact/hooks';
import {
  clearMediaBoard,
  deleteMediaBoardItem,
  getMediaBoardItems,
  isUnavailable,
  editMediaBoardItem,
  patchMediaBoardItem,
  updateGuildSettings,
} from '../../api/client';
import type { MediaBoardItem } from '../../api/types';
import { ConfirmDialog, Dialog } from '../../components/Dialog';
import { Icon } from '../../components/icons';
import {
  Badge,
  Card,
  CardHead,
  Empty,
  ErrorNotice,
  Loading,
  Meter,
  Switch,
  Table,
  Unavailable,
} from '../../components/ui';
import { describeError } from '../../errors';
import { ago, bytes, duration, plural } from '../../format';
import { useAsync } from '../../use-async';
import type { GuildTabProps } from './types';

const KIND_LABELS: Record<string, string> = {
  video: 'Vidéo',
  audio: 'Son',
  image: 'Image',
  text: 'Texte',
};

export function BoardTab({ guild, detail, refresh, notify }: GuildTabProps) {
  const items = useAsync((signal) => getMediaBoardItems(guild.id, signal), [guild.id]);
  const [quota, setQuota] = useState('');
  const [quotaBusy, setQuotaBusy] = useState(false);
  const [quotaError, setQuotaError] = useState<unknown>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearBusy, setClearBusy] = useState(false);
  const [editTarget, setEditTarget] = useState<MediaBoardItem | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<MediaBoardItem | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);

  useEffect(() => {
    if (detail?.boardQuotaBytes != null) {
      setQuota(String(Math.round((detail.boardQuotaBytes / (1024 * 1024)) * 10) / 10));
    } else {
      setQuota('');
    }
  }, [detail?.boardQuotaBytes]);

  const board = items.data?.items ?? [];
  const kinds = useMemo(() => {
    const counts = { video: 0, audio: 0, image: 0 };
    for (const item of board) {
      const kind = item.media.kind;
      if (kind === 'video' || kind === 'audio' || kind === 'image') {
        counts[kind] += 1;
      }
    }
    return counts;
  }, [board]);
  const mediaUnavailable = isUnavailable(items.error);

  const applyQuota = async () => {
    if (quotaBusy) {
      return;
    }
    setQuotaBusy(true);
    setQuotaError(null);
    try {
      const parsed = quota.trim() === '' ? null : Number(quota);
      await updateGuildSettings(guild.id, {
        boardQuotaMb:
          parsed == null || !Number.isFinite(parsed) ? null : Math.max(1, Math.round(parsed * 1024)),
      });
      notify('Quota de la guilde enregistré.');
      await refresh();
    } catch (error) {
      setQuotaError(error);
      notify(describeError(error), 'error');
    } finally {
      setQuotaBusy(false);
    }
  };

  const clear = async () => {
    if (clearBusy) {
      return;
    }
    setClearBusy(true);
    setActionError(null);
    try {
      const result = await clearMediaBoard(guild.id, guild.id);
      setConfirmClear(false);
      notify(
        `Board vidé : ${plural(
          result.deleted.mediaBoardItems,
          'média supprimé',
          'médias supprimés',
        )} (${plural(result.deleted.orphanMediaAssets, 'fichier orphelin', 'fichiers orphelins')}).`,
      );
      items.reload();
      await refresh();
    } catch (error) {
      setConfirmClear(false);
      setActionError(error);
      notify(describeError(error), 'error');
    } finally {
      setClearBusy(false);
    }
  };

  const remove = async () => {
    if (!deleteTarget || deleteBusy) {
      return;
    }
    setDeleteBusy(true);
    setActionError(null);
    try {
      await deleteMediaBoardItem(guild.id, deleteTarget.id);
      notify(`« ${deleteTarget.title} » supprimé du board.`);
      setDeleteTarget(null);
      items.reload();
      await refresh();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    } finally {
      setDeleteBusy(false);
    }
  };

  return (
    <div class="stack">
      <div class="grid is-2">
        <Card>
          <CardHead title="Occupation" />
          <div class="card-body stack">
            <div class="meter-row">
              <span>{guild.mediaBoard.itemsCount} médias</span>
              <span class="num soft">
                {bytes(guild.mediaBoard.usedBytes)} / {bytes(guild.mediaBoard.maxBytes)}
              </span>
              <Meter
                used={guild.mediaBoard.usedBytes}
                max={guild.mediaBoard.maxBytes || 1}
                label="Board"
              />
            </div>
            <dl class="kv">
              <dt>Vidéos</dt>
              <dd class="num">{items.data ? kinds.video : '—'}</dd>
              <dt>Sons</dt>
              <dd class="num">{items.data ? kinds.audio : '—'}</dd>
              <dt>Images</dt>
              <dd class="num">{items.data ? kinds.image : '—'}</dd>
            </dl>
          </div>
        </Card>
        <Card>
          <CardHead title="Quota de cette guilde" />
          <div class="card-body stack">
            <p class="soft">
              Par défaut, la limite du serveur s’applique (
              <span class="mono">MEDIA_BOARD_MAX_TOTAL_MB</span>,{' '}
              {bytes(guild.mediaBoard.maxBytes)}).
            </p>
            <div class="row is-wrap">
              <label for="board-quota">Quota propre (Go)</label>
              <input
                class="field is-inline is-narrow num"
                id="board-quota"
                type="number"
                min={1}
                max={200}
                placeholder={String(Math.round(guild.mediaBoard.maxBytes / (1024 * 1024 * 1024)))}
                value={quota}
                onInput={(event) => setQuota(event.currentTarget.value)}
              />
              <button class="btn" type="button" onClick={() => void applyQuota()} disabled={quotaBusy}>
                {quotaBusy ? 'Enregistrement…' : 'Appliquer'}
              </button>
            </div>
            {isUnavailable(quotaError) ? (
              <Unavailable detail="Le quota par guilde (PATCH /admin/api/guilds/{id}/settings) n’est pas exposé par ce serveur." />
            ) : quotaError ? (
              <ErrorNotice error={quotaError} />
            ) : null}
            <div class="row is-wrap">
              <button
                class="btn is-danger"
                type="button"
                onClick={() => setConfirmClear(true)}
              >
                <Icon name="trash" />
                Vider le board
              </button>
            </div>
          </div>
        </Card>
      </div>

      {isUnavailable(actionError) ? (
        <Unavailable detail="Cette action sur le board n’est pas exposée par ce serveur." />
      ) : actionError ? (
        <ErrorNotice error={actionError} title="Action impossible" />
      ) : null}

      <Card>
        <CardHead title="Derniers ajouts" meta="aperçu" />
        {items.loading && !items.data ? (
          <Loading label="Chargement du board…" />
        ) : mediaUnavailable ? (
          <div class="card-body">
            <Unavailable detail="La liste des médias (GET /admin/api/guilds/{id}/media-board/items) n’est pas exposée par ce serveur. L’occupation et le vidage restent disponibles." />
          </div>
        ) : items.error ? (
          <div class="card-body">
            <ErrorNotice error={items.error} onRetry={items.reload} />
          </div>
        ) : board.length ? (
          <Table>
            <thead>
              <tr>
                <th>Titre</th>
                <th>Type</th>
                <th class="is-num">Durée</th>
                <th class="is-num">Taille</th>
                <th>Ajouté par</th>
                <th>Ajouté</th>
                <th>
                  <span class="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {board.map((item) => (
                <tr key={item.id}>
                  <td class="primary">
                    {item.title}
                  </td>
                  <td>
                    <Badge>{KIND_LABELS[item.media.kind] ?? item.media.kind}</Badge>
                  </td>
                  <td class="is-num">{duration(item.media.durationSec)}</td>
                  <td class="is-num">{bytes(item.media.sizeBytes)}</td>
                  <td class="soft">{item.createdByName}</td>
                  <td class="soft">{ago(item.createdAt)}</td>
                  <td class="is-actions">
                    <button class="btn is-small" type="button" onClick={() => setEditTarget(item)}>
                      Modifier
                    </button>
                    <button
                      class="btn is-small is-quiet is-danger"
                      type="button"
                      onClick={() => setDeleteTarget(item)}
                    >
                      Supprimer
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </Table>
        ) : (
          <Empty>Board vide.</Empty>
        )}
      </Card>

      <ConfirmDialog
        open={confirmClear}
        title="Vider le board média ?"
        body={`Les ${guild.mediaBoard.itemsCount} médias de « ${guild.name} » et leurs fichiers (${bytes(
          guild.mediaBoard.usedBytes,
        )}) sont supprimés. Irréversible.`}
        confirmLabel="Vider le board"
        danger
        expect={guild.id}
        busy={clearBusy}
        onCancel={() => setConfirmClear(false)}
        onConfirm={() => void clear()}
      />

      <ConfirmDialog
        open={deleteTarget !== null}
        title={deleteTarget ? `Supprimer « ${deleteTarget.title} » ?` : 'Supprimer ce média ?'}
        body="Le média et son fichier sont retirés du board. Irréversible."
        confirmLabel="Supprimer"
        danger
        busy={deleteBusy}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={() => void remove()}
      />

      {editTarget ? (
        <MediaEditDialog
          guildId={guild.id}
          item={editTarget}
          onClose={() => setEditTarget(null)}
          onSaved={() => {
            items.reload();
            void refresh();
          }}
          notify={notify}
        />
      ) : null}
    </div>
  );
}

function MediaEditDialog({
  guildId,
  item,
  onClose,
  onSaved,
  notify,
}: {
  guildId: string;
  item: MediaBoardItem;
  onClose: () => void;
  onSaved: () => void;
  notify: (message: string, kind?: 'ok' | 'error') => void;
}) {
  const [title, setTitle] = useState(item.title);
  const [trimStart, setTrimStart] = useState('0');
  const [trimEnd, setTrimEnd] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const timed = item.media.kind !== 'image';
  const maxDuration = item.media.durationSec || 0;

  const save = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const newTitle = title.trim() || item.title;
      const start = Math.max(0, Number(trimStart) || 0);
      const parsedEnd = trimEnd.trim() === '' ? maxDuration : Number(trimEnd);
      const end = Number.isFinite(parsedEnd) ? Math.min(parsedEnd, maxDuration) : maxDuration;
      const cut = timed && maxDuration > 0 && (start > 0 || end < maxDuration);
      if (cut && end <= start) {
        setError(new Error('La fin de la découpe doit être après son début.'));
        return;
      }
      if (newTitle !== item.title) {
        await patchMediaBoardItem(guildId, item.id, { title: newTitle });
      }
      if (cut) {
        // The cut is re-encoded by the media editor and replaces the file; the
        // board updates when the ingestion finishes.
        await editMediaBoardItem(guildId, item.id, {
          replace: true,
          recipe: { segments: [[start, end]] },
        });
        notify(`Découpe de « ${newTitle} » en cours.`);
      } else {
        notify(`« ${newTitle} » enregistré.`);
      }
      onSaved();
      onClose();
    } catch (caught) {
      setError(caught);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      title="Modifier le média"
      subtitle={`${KIND_LABELS[item.media.kind] ?? item.media.kind} · ajouté par ${item.createdByName}`}
      onClose={onClose}
      footer={
        <>
          <button type="button" class="btn" onClick={onClose} disabled={busy}>
            Annuler
          </button>
          <button type="button" class="btn is-primary" onClick={() => void save()} disabled={busy}>
            {busy ? 'Enregistrement…' : 'Enregistrer'}
          </button>
        </>
      }
    >
      <div class="stack">
        <div>
          <label for="md-title">Titre</label>
          <input
            class="field is-after-label"
            id="md-title"
            maxlength={120}
            value={title}
            onInput={(event) => setTitle(event.currentTarget.value)}
          />
        </div>
        {timed ? (
          <div>
            <span class="lbl">Découpe (s)</span>
            <div class="row">
              <label class="sr-only" for="md-start">
                Début
              </label>
              <input
                class="field is-narrow num"
                id="md-start"
                type="number"
                min={0}
                max={maxDuration}
                step={0.1}
                value={trimStart}
                onInput={(event) => setTrimStart(event.currentTarget.value)}
              />
              <span class="soft">à</span>
              <label class="sr-only" for="md-end">
                Fin
              </label>
              <input
                class="field is-narrow num"
                id="md-end"
                type="number"
                min={0}
                max={maxDuration}
                step={0.1}
                placeholder={String(maxDuration)}
                value={trimEnd}
                onInput={(event) => setTrimEnd(event.currentTarget.value)}
              />
              <span class="soft">sur {maxDuration} s</span>
            </div>
            <p class="hint">
              Le média est réencodé et remplacé par la plage gardée. L’éditeur complet (forme
              d’onde, vitesse, recadrage, illustration) reste dans l’overlay.
            </p>
          </div>
        ) : null}
      </div>
      {error ? (
        isUnavailable(error) ? (
          <Unavailable detail="La modification d’un média (PATCH /admin/api/guilds/{id}/media-board/items/{itemId}) n’est pas exposée par ce serveur." />
        ) : (
          <div class="inline-error">{describeError(error)}</div>
        )
      ) : null}
    </Dialog>
  );
}
