import { useMemo, useState } from 'preact/hooks';
import {
  cleanMediaOrphans,
  getIngestions,
  isUnavailable,
  purgeMediaCache,
} from '../api/client';
import type { Ingestion } from '../api/types';
import { ConfirmDialog } from '../components/Dialog';
import { Layout } from '../components/Layout';
import { Icon } from '../components/icons';
import {
  Badge,
  Card,
  CardHead,
  Empty,
  ErrorNotice,
  Loading,
  Meter,
  PageHead,
  Table,
  Unavailable,
} from '../components/ui';
import { useApp } from '../context';
import { describeError } from '../errors';
import { ago, bytes, plural } from '../format';
import { useAsync } from '../use-async';

const INGESTION_STATUS: Record<string, { label: string; tone: 'ok' | 'err' | 'info' | 'neutral' }> = {
  pending: { label: 'En attente', tone: 'neutral' },
  running: { label: 'En cours', tone: 'info' },
  processing: { label: 'En cours', tone: 'info' },
  done: { label: 'Prêt', tone: 'ok' },
  ready: { label: 'Prêt', tone: 'ok' },
  failed: { label: 'Échec', tone: 'err' },
};

function ingestionDetail(item: Ingestion): string {
  if (item.error) {
    return item.error;
  }
  if (item.status === 'running' || item.status === 'pending') {
    return item.durationMs ? `${(item.durationMs / 1000).toFixed(0)} s` : 'En cours…';
  }
  return item.source;
}

export function MediaScreen() {
  const { route, overview, overviewError, refreshOverview, notify, liveIngestions } = useApp();
  const [confirmPurge, setConfirmPurge] = useState(false);
  const [purgeBusy, setPurgeBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);
  const ingestions = useAsync((signal) => getIngestions(50, signal), []);

  const ingestionsItems = useMemo(() => {
    const base = ingestions.data?.items ?? [];
    const known = new Set(base.map((item) => item.id));
    return [...liveIngestions.filter((item) => !known.has(item.id)), ...base].slice(0, 100);
  }, [ingestions.data, liveIngestions]);

  const guilds = overview?.guilds ?? [];
  const totalBoardBytes = guilds.reduce((sum, guild) => sum + guild.mediaBoard.usedBytes, 0);
  const totalItems = guilds.reduce((sum, guild) => sum + guild.mediaBoard.itemsCount, 0);
  const boardMax = guilds.reduce((max, guild) => Math.max(max, guild.mediaBoard.maxBytes), 0);
  const sortedGuilds = [...guilds].sort(
    (left, right) => right.mediaBoard.usedBytes - left.mediaBoard.usedBytes,
  );

  const effectiveActionError = actionError;
  const actionUnavailable = isUnavailable(effectiveActionError);

  const purge = async () => {
    setPurgeBusy(true);
    try {
      const result = await purgeMediaCache();
      setConfirmPurge(false);
      setActionError(null);
      notify(
        `${plural(result.removedFiles, 'fichier supprimé', 'fichiers supprimés')} (${bytes(result.freedBytes)}).`,
      );
      await refreshOverview();
    } catch (error) {
      setConfirmPurge(false);
      setActionError(error);
      notify(describeError(error), 'error');
    } finally {
      setPurgeBusy(false);
    }
  };

  const cleanOrphans = async () => {
    try {
      const result = await cleanMediaOrphans();
      setActionError(null);
      notify(
        `${plural(result.removedFiles, 'fichier orphelin supprimé', 'fichiers orphelins supprimés')} (${bytes(result.freedBytes)}).`,
      );
      await refreshOverview();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    }
  };

  let content;
  if (!overview) {
    content = overviewError ? (
      <ErrorNotice error={overviewError} onRetry={() => void refreshOverview()} />
    ) : (
      <Loading label="Chargement du stockage…" />
    );
  } else {
    content = (
      <div class="stack">
        <div class="grid is-tiles">
          <div class="card tile">
            <div class="label">Cache média</div>
            <div class="value">{bytes(overview.cache.usedBytes)}</div>
            <div class="foot has-meter">
              <Meter used={overview.cache.usedBytes} max={overview.cache.maxBytes} label="Cache" />
              sur {bytes(overview.cache.maxBytes)} · expiration selon la configuration
            </div>
          </div>
          <div class="card tile">
            <div class="label">Boards (total)</div>
            <div class="value">{bytes(totalBoardBytes)}</div>
            <div class="foot">
              {plural(totalItems, 'média', 'médias')}
              {boardMax ? ` · quota ${bytes(boardMax)} par guilde` : ''}
            </div>
          </div>
          <div class="card tile">
            <div class="label">Guildes occupées</div>
            <div class="value">{guilds.filter((guild) => guild.mediaBoard.itemsCount > 0).length}</div>
            <div class="foot">sur {plural(guilds.length, 'guilde', 'guildes')}</div>
          </div>
          <div class="card tile">
            <div class="label">Fichiers orphelins</div>
            <div class="value">—</div>
            <div class="foot">
              <button class="btn is-small" type="button" onClick={() => void cleanOrphans()}>
                Nettoyer
              </button>
            </div>
          </div>
        </div>

        {actionUnavailable ? (
          <Unavailable detail="La route de purge ou de nettoyage n’est pas exposée par ce serveur." />
        ) : effectiveActionError ? (
          <ErrorNotice error={effectiveActionError} title="Action indisponible" />
        ) : null}

        <Card>
          <CardHead title="Occupation par guilde" />
          {sortedGuilds.length ? (
            <Table>
              <thead>
                <tr>
                  <th>Guilde</th>
                  <th class="is-num">Médias</th>
                  <th>Occupation</th>
                  <th class="is-num">Quota</th>
                  <th>
                    <span class="sr-only">Actions</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {sortedGuilds.map((guild) => (
                  <tr key={guild.id}>
                    <td>
                      <a class="primary" href={`#/guilds/${encodeURIComponent(guild.id)}/board`}>
                        {guild.name}
                      </a>
                    </td>
                    <td class="is-num">{guild.mediaBoard.itemsCount}</td>
                    <td>
                      <span class="mini-meter is-wide">
                        <Meter
                          used={guild.mediaBoard.usedBytes}
                          max={guild.mediaBoard.maxBytes || 1}
                          label="Board"
                        />
                      </span>
                      <span class="num soft">
                        {bytes(guild.mediaBoard.usedBytes)} · {guild.mediaBoard.usedPercent} %
                      </span>
                    </td>
                    <td class="is-num soft">{bytes(guild.mediaBoard.maxBytes)}</td>
                    <td class="is-actions">
                      <a class="btn is-small" href={`#/guilds/${encodeURIComponent(guild.id)}/board`}>
                        Gérer
                      </a>
                    </td>
                  </tr>
                ))}
              </tbody>
            </Table>
          ) : (
            <Empty>Aucune guilde.</Empty>
          )}
        </Card>

        <Card>
          <CardHead title="Ingestions récentes" meta="téléchargement et transcodage" />
          {ingestions.loading && !ingestions.data ? (
            <Loading label="Chargement des ingestions…" />
          ) : isUnavailable(ingestions.error) ? (
            <div class="card-body">
              <Unavailable detail="L’historique des ingestions (GET /admin/api/ingestions) n’est pas encore exposé par ce serveur." />
            </div>
          ) : ingestions.error ? (
            <div class="card-body">
              <ErrorNotice error={ingestions.error} onRetry={ingestions.reload} />
            </div>
          ) : ingestionsItems.length ? (
            <Table>
              <thead>
                <tr>
                  <th>Quand</th>
                  <th>Média</th>
                  <th>Guilde</th>
                  <th>Origine</th>
                  <th>État</th>
                  <th>Détail</th>
                </tr>
              </thead>
              <tbody>
                {ingestionsItems.map((item) => {
                  const status = INGESTION_STATUS[item.status] ?? {
                    label: item.status,
                    tone: 'neutral' as const,
                  };
                  return (
                    <tr key={item.id}>
                      <td class="soft">{ago(item.startedAt)}</td>
                      <td class="primary">{item.url || item.source}</td>
                      <td>
                        <a href={`#/guilds/${encodeURIComponent(item.guildId)}`}>{item.guildName}</a>
                      </td>
                      <td class="soft">{item.source}</td>
                      <td>
                        <Badge tone={status.tone}>{status.label}</Badge>
                      </td>
                      <td class="soft is-wrap">{ingestionDetail(item)}</td>
                    </tr>
                  );
                })}
              </tbody>
            </Table>
          ) : (
            <Empty>Aucune ingestion récente.</Empty>
          )}
        </Card>
      </div>
    );
  }

  return (
    <Layout route={route} title="Médias et stockage" crumbs={[{ label: 'Médias et stockage' }]}>
      <PageHead
        title="Médias et stockage"
        sub="Cache de lecture, boards par guilde et ingestions récentes."
        actions={
          <button class="btn is-danger" type="button" onClick={() => setConfirmPurge(true)}>
            <Icon name="trash" />
            Purger le cache
          </button>
        }
      />
      {content}

      <ConfirmDialog
        open={confirmPurge}
        title="Purger le cache média ?"
        body={`${bytes(overview?.cache.usedBytes ?? 0)} de médias hors board sont supprimés. Ceux des lectures en cours sont conservés ; les autres seront retéléchargés à la demande.`}
        confirmLabel="Purger le cache"
        danger
        busy={purgeBusy}
        onCancel={() => setConfirmPurge(false)}
        onConfirm={() => void purge()}
      />
    </Layout>
  );
}
