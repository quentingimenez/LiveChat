import { useMemo, useState } from 'preact/hooks';
import { createGuild, isUnavailable } from '../api/client';
import type { AdminInvite, DiscordKind } from '../api/types';
import { Dialog, SecretDialog } from '../components/Dialog';
import { Layout } from '../components/Layout';
import { Icon } from '../components/icons';
import {
  Badge,
  Empty,
  ErrorNotice,
  GuildIcon,
  Loading,
  Meter,
  PageHead,
  SearchInput,
  Segmented,
  Table,
  Unavailable,
} from '../components/ui';
import { BOT_INVITE_URL } from '../constants';
import { useApp } from '../context';
import { describeError } from '../errors';
import { bytes, plural } from '../format';
import { navigate } from '../router';

type GuildFilter = 'all' | 'discord' | 'standalone' | 'air';

const FILTERS: ReadonlyArray<{ value: GuildFilter; label: string }> = [
  { value: 'all', label: 'Toutes' },
  { value: 'discord', label: 'Discord' },
  { value: 'standalone', label: 'Indépendantes' },
  { value: 'air', label: 'À l’antenne' },
];

const KIND_BADGE: Record<DiscordKind, { label: string; tone: 'info' | 'outline' }> = {
  native: { label: 'Serveur Discord', tone: 'info' },
  linked: { label: 'Indépendante, liée', tone: 'info' },
  none: { label: 'Indépendante', tone: 'outline' },
};

export function GuildsScreen() {
  const { route, overview, overviewError, overviewLoading, refreshOverview, notify } = useApp();
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<GuildFilter>('all');
  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [created, setCreated] = useState<{ guildId: string; name: string; invite: AdminInvite } | null>(
    null,
  );

  const guilds = overview?.guilds ?? [];
  const rows = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return guilds.filter((guild) => {
      const kindOk =
        filter === 'all' ||
        (filter === 'air'
          ? guild.playback.playing > 0
          : filter === 'discord'
            ? guild.discord.kind === 'native'
            : guild.discord.kind !== 'native');
      const queryOk = !needle || guild.name.toLowerCase().includes(needle) || guild.id.includes(needle);
      return kindOk && queryOk;
    });
  }, [filter, guilds, query]);

  const openCreate = () => {
    setName('');
    setCreateError(null);
    setCreateOpen(true);
  };

  const submitCreate = async () => {
    const guildName = name.trim();
    if (!guildName || busy) {
      return;
    }
    setBusy(true);
    setCreateError(null);
    try {
      const response = await createGuild({ name: guildName });
      setCreateOpen(false);
      setCreated({ guildId: response.guild.id, name: response.guild.name, invite: response.adminInvite });
      notify('Guilde créée.');
      await refreshOverview();
    } catch (error) {
      setCreateError(describeError(error));
    } finally {
      setBusy(false);
    }
  };

  const closeCreated = () => {
    const guildId = created?.guildId;
    setCreated(null);
    if (guildId) {
      navigate({ name: 'guild', guildId, tab: 'overview', query: {} });
    }
  };

  const content = overview ? (
    <>
      <div class="toolbar">
        <SearchInput
          value={query}
          onInput={setQuery}
          placeholder="Nom ou identifiant"
          label="Rechercher une guilde"
        />
        <Segmented value={filter} options={FILTERS} onChange={setFilter} label="Filtrer par type" />
      </div>
      {rows.length ? (
        <>
          <Table>
            <thead>
              <tr>
                <th>Guilde</th>
                <th>Type</th>
                <th class="is-num">Overlays</th>
                <th>Board</th>
                <th>Lecture</th>
                <th class="is-num">Ingestions</th>
                <th>
                  <span class="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((guild) => {
                const kind = KIND_BADGE[guild.discord.kind];
                const online = guild.overlays.connectedCount;
                return (
                  <tr key={guild.id} class={guild.playback.playing > 0 ? 'is-air' : ''}>
                    <td>
                      <div class="guild-cell">
                        <GuildIcon name={guild.name} kind={guild.discord.kind} />
                        <div>
                          <a class="primary" href={`#/guilds/${encodeURIComponent(guild.id)}`}>
                            {guild.name}
                          </a>
                          <span class="under mono">{guild.id}</span>
                        </div>
                      </div>
                    </td>
                    <td>
                      <Badge tone={kind.tone}>{kind.label}</Badge>
                    </td>
                    <td class="is-num">
                      <span class={`dot is-inline${online ? ' is-ok' : ''}`} aria-hidden="true" />
                      {online} / {guild.overlays.total}
                    </td>
                    <td>
                      <span class="mini-meter">
                        <Meter
                          used={guild.mediaBoard.usedBytes}
                          max={guild.mediaBoard.maxBytes || 1}
                          label={`Board de ${guild.name}`}
                        />
                      </span>
                      <span class="num soft">{bytes(guild.mediaBoard.usedBytes)}</span>
                    </td>
                    <td>
                      {guild.playback.playing > 0 ? (
                        <>
                          <Badge tone="air">À l’antenne</Badge>{' '}
                          <span class="soft">+{Math.max(0, guild.playback.pending)}</span>
                        </>
                      ) : (
                        <span class="faint">Au repos</span>
                      )}
                    </td>
                    <td class="is-num">{guild.ingest.total}</td>
                    <td class="is-actions">
                      <a class="btn is-small" href={`#/guilds/${encodeURIComponent(guild.id)}`}>
                        Ouvrir
                      </a>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </Table>
          <p class="table-foot">
            {plural(rows.length, 'guilde', 'guildes')} sur {guilds.length}
          </p>
        </>
      ) : (
        <div class="table-wrap">
          <Empty>Aucune guilde ne correspond à cette recherche.</Empty>
        </div>
      )}
    </>
  ) : overviewLoading ? (
    <Loading label="Chargement des guildes…" />
  ) : isUnavailable(overviewError) ? (
    <Unavailable detail="La liste des guildes vient de GET /admin/api/overview, non exposé par ce serveur." />
  ) : (
    <ErrorNotice error={overviewError} onRetry={() => void refreshOverview()} />
  );

  return (
    <Layout route={route} title="Guildes" crumbs={[{ label: 'Guildes' }]}>
      <PageHead
        title="Guildes"
        sub="Serveurs Discord où le bot est installé et groupes indépendants."
        actions={
          <>
            <a class="btn" href={BOT_INVITE_URL} target="_blank" rel="noopener noreferrer">
              <Icon name="external" />
              Inviter le bot sur Discord
            </a>
            <button class="btn is-primary" type="button" onClick={openCreate}>
              <Icon name="plus" />
              Créer une guilde
            </button>
          </>
        }
      />
      {content}

      <Dialog
        open={createOpen}
        title="Créer une guilde indépendante"
        subtitle="Un groupe sans serveur Discord. Pour un serveur Discord, il suffit d’y inviter le bot."
        onClose={() => setCreateOpen(false)}
        footer={
          <>
            <button type="button" class="btn" onClick={() => setCreateOpen(false)} disabled={busy}>
              Annuler
            </button>
            <button
              type="button"
              class="btn is-primary"
              onClick={() => void submitCreate()}
              disabled={busy || !name.trim()}
            >
              {busy ? 'Création…' : 'Créer la guilde'}
            </button>
          </>
        }
      >
        <label for="guild-name">Nom de la guilde</label>
        <input
          class="field is-after-label"
          id="guild-name"
          maxlength={60}
          placeholder="Soirée jeux du vendredi"
          value={name}
          onInput={(event) => setName(event.currentTarget.value)}
        />
        <p class="hint">
          Un code d’invitation admin, valable 24 h, est créé en même temps : transmets-le à la
          personne qui gérera la guilde.
        </p>
        {createError ? <div class="inline-error">{createError}</div> : null}
      </Dialog>

      <SecretDialog
        open={created !== null}
        title="Guilde créée"
        subtitle={created ? `Code d’invitation admin pour « ${created.name} »` : undefined}
        value={created?.invite.code ?? ''}
        hint="À saisir dans l’overlay : Rejoindre une guilde. Valable 24 h, une seule utilisation."
        onClose={closeCreated}
      />
    </Layout>
  );
}
