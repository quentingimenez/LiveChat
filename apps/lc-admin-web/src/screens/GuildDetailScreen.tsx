import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { createAdminCode, getGuild, isUnavailable, stopGuild } from '../api/client';
import type { GuildDetail, GuildEntry } from '../api/types';
import { SecretDialog } from '../components/Dialog';
import { Layout } from '../components/Layout';
import { Icon } from '../components/icons';
import {
  Badge,
  Card,
  CopyButton,
  Empty,
  ErrorNotice,
  GuildIcon,
  Loading,
  Unavailable,
} from '../components/ui';
import { useApp } from '../context';
import { describeError } from '../errors';
import { GUILD_TABS, type GuildTab } from '../router';
import { useAsync } from '../use-async';
import { AccessTab } from './guild/AccessTab';
import { BoardTab } from './guild/BoardTab';
import { DangerTab } from './guild/DangerTab';
import { DevicesTab } from './guild/DevicesTab';
import { DiscordTab } from './guild/DiscordTab';
import { MembersTab } from './guild/MembersTab';
import { OverviewTab } from './guild/OverviewTab';
import { QueueTab } from './guild/QueueTab';
import { SettingsTab } from './guild/SettingsTab';
import { GuildKindBadge } from './guild/shared';
import type { GuildTabProps } from './guild/types';

const TAB_LABELS: Record<GuildTab, string> = {
  overview: 'Vue d’ensemble',
  members: 'Membres',
  devices: 'Overlays',
  access: 'Invitations et codes',
  board: 'Board média',
  queue: 'File d’attente',
  settings: 'Réglages',
  discord: 'Discord',
  danger: 'Zone de danger',
};

export function GuildDetailScreen() {
  const {
    route,
    overview,
    refreshOverview,
    notify,
  } = useApp();
  const guildId = route.guildId;
  const tab: GuildTab = route.tab ?? 'overview';
  const detailState = useAsync(
    (signal) =>
      guildId
        ? getGuild(guildId, signal)
        : Promise.reject(new Error('Identifiant de guilde absent.')),
    [guildId],
  );
  // Ignore data fetched for the previous guild while the new detail loads.
  const detail: GuildDetail | null =
    detailState.data && detailState.data.id === guildId ? detailState.data : null;
  const overviewGuild = useMemo(
    () => overview?.guilds.find((entry) => entry.id === guildId) ?? null,
    [guildId, overview],
  );
  const guild: GuildEntry | null = detail ?? overviewGuild;
  const detailUnavailable = isUnavailable(detailState.error);
  const [adminCode, setAdminCode] = useState<string | null>(null);
  const [actionError, setActionError] = useState<unknown>(null);

  // The contract says details are reread when an `overview` SSE event arrives.
  const overviewStamp = overview?.generatedAt;
  const lastStamp = useRef<string | null>(null);
  useEffect(() => {
    if (!overviewStamp || tab === 'settings' || tab === 'board') {
      return;
    }
    if (lastStamp.current === null) {
      lastStamp.current = overviewStamp;
      return;
    }
    if (lastStamp.current !== overviewStamp) {
      lastStamp.current = overviewStamp;
      detailState.reload();
    }
  }, [overviewStamp, tab, detailState.reload]);

  if (!guildId) {
    return (
      <Layout route={route} title="Guilde" crumbs={[{ label: 'Guildes', href: '#/guilds' }]}>
        <Card>
          <Empty>
            Guilde introuvable. <a href="#/guilds">Retour à la liste</a>
          </Empty>
        </Card>
      </Layout>
    );
  }

  if (!guild) {
    return (
      <Layout
        route={route}
        title="Guilde"
        crumbs={[{ label: 'Guildes', href: '#/guilds' }, { label: 'Guilde' }]}
      >
        {detailState.loading ? (
          <Loading label="Chargement de la guilde…" />
        ) : (
          <Card>
            <Empty>
              Cette guilde n’existe plus. <a href="#/guilds">Retour à la liste</a>
            </Empty>
            {detailUnavailable ? (
              <p class="hint">
                Indisponible sur ce serveur : le détail d’une guilde (GET /admin/api/guilds/&#123;id&#125;)
                n’est pas exposé.
              </p>
            ) : null}
          </Card>
        )}
      </Layout>
    );
  }

  const playing = guild.playback.playing > 0;
  const refresh = async () => {
    await refreshOverview();
    detailState.reload();
  };

  const stop = async () => {
    setActionError(null);
    try {
      await stopGuild(guild.id);
      notify('Lecture arrêtée.');
      await refresh();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    }
  };

  const requestAdminCode = async () => {
    setActionError(null);
    try {
      const response = await createAdminCode(guild.id);
      setAdminCode(response.adminInvite.code);
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    }
  };

  const counts: Partial<Record<GuildTab, number>> = {
    members: detail?.members?.count,
    devices: guild.overlays.total,
    access: detail
      ? detail.invites.items.filter((invite) => invite.isActive).length
      : undefined,
    board: guild.mediaBoard.itemsCount,
    queue: detail ? detail.queue.pending.length + (detail.queue.playing ? 1 : 0) : undefined,
  };

  const tabProps: GuildTabProps = {
    guild,
    detail,
    detailUnavailable,
    refresh,
    notify,
  };

  let tabContent;
  switch (tab) {
    case 'members':
      tabContent = <MembersTab {...tabProps} />;
      break;
    case 'devices':
      tabContent = <DevicesTab {...tabProps} />;
      break;
    case 'access':
      tabContent = <AccessTab {...tabProps} />;
      break;
    case 'board':
      tabContent = <BoardTab {...tabProps} />;
      break;
    case 'queue':
      tabContent = <QueueTab {...tabProps} />;
      break;
    case 'settings':
      tabContent = <SettingsTab key={guild.id} {...tabProps} />;
      break;
    case 'discord':
      tabContent = <DiscordTab {...tabProps} />;
      break;
    case 'danger':
      tabContent = <DangerTab {...tabProps} />;
      break;
    default:
      tabContent = <OverviewTab {...tabProps} />;
  }

  return (
    <Layout
      route={route}
      title={guild.name}
      crumbs={[{ label: 'Guildes', href: '#/guilds' }, { label: guild.name }]}
    >
      <div class="page-head">
        <div>
          <h1 class="guild-title">
            <GuildIcon name={guild.name} kind={guild.discord.kind} />
            {guild.name}
          </h1>
          <div class="guild-id">
            <GuildKindBadge kind={guild.discord.kind} />
            {playing ? <Badge tone="air">À l’antenne</Badge> : null}
            <span class="mono soft">{guild.id}</span>
            <CopyButton text={guild.id} label="Copier l’identifiant" iconOnly />
          </div>
        </div>
        <div class="actions">
          <button class="btn" type="button" onClick={() => void requestAdminCode()}>
            <Icon name="key" />
            Code admin
          </button>
          <button class="btn" type="button" onClick={() => void stop()} disabled={!playing}>
            <Icon name="stop" />
            Arrêter la lecture
          </button>
        </div>
      </div>

      {isUnavailable(actionError) ? (
        <Unavailable detail="Cette action de la guilde n’est pas exposée par ce serveur." />
      ) : actionError ? (
        <ErrorNotice error={actionError} title="Action impossible" />
      ) : null}

      <nav class="tabs" aria-label="Sections de la guilde">
        {GUILD_TABS.map((key) => {
          const count = counts[key];
          return (
            <a
              key={key}
              href={`#/guilds/${encodeURIComponent(guild.id)}/${key}`}
              class={key === 'danger' ? 'is-danger' : undefined}
              aria-current={key === tab ? 'page' : undefined}
            >
              {TAB_LABELS[key]}
              {count != null ? <span class="count">{count}</span> : null}
            </a>
          );
        })}
      </nav>

      {detailUnavailable ? (
        <p class="hint">
          Indisponible sur ce serveur : le détail enrichi de la guilde (GET /admin/api/guilds/&#123;id&#125;)
          n’est pas exposé. Les données de l’aperçu et les routes encore disponibles restent
          utilisables.
        </p>
      ) : null}

      {tabContent}

      <SecretDialog
        open={adminCode !== null}
        title="Code admin"
        subtitle={`Accès administrateur à « ${guild.name} »`}
        value={adminCode ?? ''}
        hint="Valable 24 h, une seule utilisation. Utile quand une guilde n’a plus d’admin."
        onClose={() => setAdminCode(null)}
      />
    </Layout>
  );
}
