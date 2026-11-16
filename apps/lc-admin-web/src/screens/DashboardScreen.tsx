import { useEffect, useMemo, useState } from 'preact/hooks';
import {
  createGuild,
  getAudit,
  getGuild,
  getIngestions,
  isUnavailable,
  stopGuild,
} from '../api/client';
import type {
  AdminInvite,
  AuditEntry,
  GuildDetail,
  Health,
  HealthCheck,
  Ingestion,
  Overview,
  QueueJob,
} from '../api/types';
import { ActivityTable } from '../components/ActivityTable';
import { Dialog, SecretDialog } from '../components/Dialog';
import { Layout } from '../components/Layout';
import { Icon } from '../components/icons';
import {
  Badge,
  Card,
  CardHead,
  Dot,
  Empty,
  ErrorNotice,
  Loading,
  Meter,
  PageHead,
  Progress,
  Unavailable,
} from '../components/ui';
import { BOT_INVITE_URL } from '../constants';
import { useApp } from '../context';
import { describeError } from '../errors';
import { ago, bytes, plural, uptime } from '../format';
import { navigate } from '../router';
import { useAsync } from '../use-async';

const HEALTH_LABELS: Record<string, string> = {
  database: 'Base SQLite',
  discord: 'Bot Discord',
  ffmpeg: 'ffmpeg',
  ytdlp: 'yt-dlp',
  piper: 'Synthèse vocale',
  ingestions: 'Ingestion',
};

const STATE_LABELS: Record<string, Record<string, string>> = {
  database: { ok: 'Accessible', warn: 'Non configurée', error: 'Inaccessible' },
  discord: { ok: 'Connecté', warn: 'Non configuré', error: 'Déconnecté' },
};

/**
 * Short French value of a health check, as in the mockup. The server detail
 * (English, technical, possibly a whole version banner) stays in the tooltip.
 */
export function healthValue(check: HealthCheck): string {
  if (check.id === 'ingestions' && check.count != null) {
    return check.count === 0 ? 'Aucun échec sur 1 h' : `${plural(check.count, 'échec', 'échecs')} sur 1 h`;
  }
  const labels = STATE_LABELS[check.id];
  if (labels) {
    return labels[check.status] ?? check.detail;
  }
  if (check.id === 'ffmpeg' || check.id === 'ytdlp' || check.id === 'piper') {
    if (check.status !== 'ok') {
      return 'Introuvable';
    }
    return check.version ? check.version.split(/[-_+]/)[0] : 'Disponible';
  }
  return check.detail;
}

function healthDotClass(status: string): 'ok' | 'warn' | 'err' {
  if (status === 'ok') {
    return 'ok';
  }
  if (status === 'warn') {
    return 'warn';
  }
  return 'err';
}

function jobProgress(job: QueueJob | null | undefined): number {
  if (!job?.startedAt || !job.durationSec) {
    return 0;
  }
  const started = Date.parse(job.startedAt);
  if (!Number.isFinite(started)) {
    return 0;
  }
  const elapsed = (Date.now() - started) / 1000;
  return Math.max(0, Math.min(100, Math.round((elapsed / job.durationSec) * 100)));
}

export interface DashboardViewProps {
  overview: Overview;
  health?: Health | null;
  healthUnavailable?: boolean;
  ingestions?: Ingestion[] | null;
  audit?: AuditEntry[] | null;
  auditError?: unknown;
  auditUnavailable?: boolean;
  airDetails?: Record<string, GuildDetail | null>;
  onCreateGuild?: () => void;
  onStop?: (guildId: string) => void;
  onReloadAudit?: () => void;
}

export function DashboardView({
  overview,
  health = null,
  healthUnavailable = false,
  ingestions = null,
  audit = null,
  auditError = null,
  auditUnavailable = false,
  airDetails = {},
  onCreateGuild = () => {},
  onStop = () => {},
  onReloadAudit = () => {},
}: DashboardViewProps) {
  const guilds = overview.guilds;
  const connected = overview.totals.overlayConnectedCount;
  const totalOverlays = overview.totals.overlayClientCount;
  const activeCodes = overview.pairingCodes.active + overview.totals.activeInviteCount;
  const ingestClients = overview.totals.ingestClientCount;
  const nativeCount = guilds.filter((guild) => guild.discord.kind === 'native').length;
  const standaloneCount = guilds.length - nativeCount;
  const totalDone = guilds.reduce((sum, guild) => sum + guild.playback.done, 0);
  const totalFailed = guilds.reduce((sum, guild) => sum + guild.playback.failed, 0);
  const failedIngestions = ingestions?.filter((item) => item.status === 'failed').length ?? 0;
  const onAir = guilds.filter((guild) => guild.playback.playing > 0);
  const boardUsed = guilds.reduce((sum, guild) => sum + guild.mediaBoard.usedBytes, 0);
  const fullest = [...guilds]
    .sort((left, right) => right.mediaBoard.usedBytes - left.mediaBoard.usedBytes)
    .slice(0, 4);

  return (
    <>
      <PageHead
        title="Tableau de bord"
        sub="État de l’instance, mis à jour en direct."
        actions={
          <>
            <a class="btn" href={BOT_INVITE_URL} target="_blank" rel="noopener noreferrer">
              <Icon name="external" />
              Inviter le bot sur Discord
            </a>
            <button class="btn is-primary" type="button" onClick={onCreateGuild}>
              <Icon name="plus" />
              Créer une guilde
            </button>
          </>
        }
      />

      <div class="stack">
        {health ? (
          <section class="health" aria-label="Santé du serveur">
            <div>
              <div class="k">
                <Dot tone={healthDotClass(health.status)} />
                Serveur
              </div>
              <div class="v">
                v{health.serverVersion} · {uptime(health.uptimeSec)}
              </div>
            </div>
            {health.checks.map((check) => (
              <div key={check.id}>
                <div class="k">
                  <Dot tone={healthDotClass(check.status)} />
                  {HEALTH_LABELS[check.id] ?? check.id}
                </div>
                <div class="v" title={check.detail}>
                  {healthValue(check)}
                </div>
              </div>
            ))}
          </section>
        ) : healthUnavailable ? (
          <Unavailable detail="La santé détaillée (GET /admin/api/health) n’est pas exposée par ce serveur. Les chiffres ci-dessous viennent de l’aperçu." />
        ) : null}

        {failedIngestions > 0 ? (
          <div class="notice is-err">
            <Icon name="alert" />
            <div>
              <strong>{plural(failedIngestions, 'ingestion a échoué', 'ingestions ont échoué')} récemment.</strong>
              <div class="soft">Les détails et les erreurs sont sur la page Médias et stockage.</div>
            </div>
            <a class="btn is-small" href="#/media">
              Voir les ingestions
            </a>
          </div>
        ) : null}

        <section class="grid is-tiles" aria-label="Chiffres clés">
          <div class="card tile">
            <div class="label">
              <Icon name="guilds" />
              Guildes
            </div>
            <div class="value">{guilds.length}</div>
            <div class="foot">
              {nativeCount} Discord · {standaloneCount} indépendantes
            </div>
          </div>
          <div class="card tile">
            <div class="label">
              <Icon name="device" />
              Overlays en ligne
            </div>
            <div class="value">
              {connected} <small>/ {totalOverlays}</small>
            </div>
            <div class="foot">{plural(totalOverlays, 'appareil appairé', 'appareils appairés')}</div>
          </div>
          <div class="card tile">
            <div class="label">
              <Icon name="key" />
              Codes actifs
            </div>
            <div class="value">{activeCodes}</div>
            <div class="foot">invitations et appairages</div>
          </div>
          <div class="card tile">
            <div class="label">
              <Icon name="plug" />
              Clients d’ingestion
            </div>
            <div class="value">{ingestClients}</div>
            <div class="foot">extensions navigateur</div>
          </div>
          <div class="card tile">
            <div class="label">
              <Icon name="play" />
              Lectures terminées
            </div>
            <div class="value">{totalDone}</div>
            <div class="foot">{plural(totalFailed, 'échec', 'échecs')}</div>
          </div>
        </section>

        <div class="grid is-2">
          <Card>
            <CardHead title="À l’antenne" meta={plural(onAir.length, 'guilde', 'guildes')} />
            {onAir.length ? (
              onAir.map((guild) => {
                const queue = airDetails[guild.id]?.queue;
                const job = queue?.playing ?? null;
                const pending = queue?.pending.length ?? Math.max(0, guild.playback.pending);
                return (
                  <div class="air" key={guild.id}>
                    <span class="badge is-air">À l’antenne</span>
                    <div>
                      <div class="title">{job?.title || job?.text || 'Lecture en cours'}</div>
                      <div class="soft">
                        <a href={`#/guilds/${encodeURIComponent(guild.id)}/queue`}>{guild.name}</a>
                        {job?.authorName ? ` · ${job.authorName}` : ''} ·{' '}
                        {plural(pending, 'média en attente', 'médias en attente')}
                      </div>
                      <Progress value={jobProgress(job)} />
                    </div>
                    <button
                      class="btn is-small"
                      type="button"
                      onClick={() => onStop(guild.id)}
                      title={`Arrêter la lecture sur ${guild.name}`}
                    >
                      <Icon name="stop" />
                      Arrêter
                    </button>
                  </div>
                );
              })
            ) : (
              <Empty>Rien à l’antenne pour le moment.</Empty>
            )}
          </Card>

          <Card>
            <CardHead title="Stockage">
              <a href="#/media">Détail</a>
            </CardHead>
            <div class="card-body stack">
              <div class="meter-row">
                <span>
                  Cache média <span class="faint">(médias hors board)</span>
                </span>
                <span class="num soft">
                  {bytes(overview.cache.usedBytes)} / {bytes(overview.cache.maxBytes)}
                </span>
                <Meter used={overview.cache.usedBytes} max={overview.cache.maxBytes} label="Cache média" />
              </div>
              <div class="meter-row">
                <span>Boards, toutes guildes</span>
                <span class="num soft">{bytes(boardUsed)}</span>
              </div>
              {fullest.map((guild) => (
                <div class="meter-row" key={guild.id}>
                  <a href={`#/guilds/${encodeURIComponent(guild.id)}/board`}>{guild.name}</a>
                  <span class="num soft">
                    {bytes(guild.mediaBoard.usedBytes)} / {bytes(guild.mediaBoard.maxBytes)}
                  </span>
                  <Meter
                    used={guild.mediaBoard.usedBytes}
                    max={guild.mediaBoard.maxBytes || 1}
                    label={guild.name}
                  />
                </div>
              ))}
            </div>
          </Card>
        </div>

        <Card>
          <CardHead title="Activité récente">
            <a href="#/activity">Tout le journal</a>
          </CardHead>
          {auditUnavailable ? (
            <div class="card-body">
              <Unavailable detail="Le journal (GET /admin/api/audit) n’est pas encore exposé par ce serveur." />
            </div>
          ) : auditError ? (
            <div class="card-body">
              <ErrorNotice error={auditError} onRetry={onReloadAudit} title="Journal indisponible" />
            </div>
          ) : audit ? (
            <ActivityTable entries={audit.slice(0, 5)} guilds={guilds} />
          ) : (
            <Loading label="Chargement du journal…" />
          )}
        </Card>
      </div>
    </>
  );
}

export function DashboardScreen() {
  const {
    route,
    overview,
    overviewError,
    overviewLoading,
    refreshOverview,
    health,
    healthUnavailable,
    liveAudit,
    liveIngestions,
    notify,
  } = useApp();
  const ingestions = useAsync((signal) => getIngestions(50, signal), []);
  const audit = useAsync((signal) => getAudit({ limit: 8 }, signal), []);
  const [airDetails, setAirDetails] = useState<Record<string, GuildDetail | null>>({});
  const [airVersion, setAirVersion] = useState(0);
  const [createOpen, setCreateOpen] = useState(false);
  const [guildName, setGuildName] = useState('');
  const [createBusy, setCreateBusy] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [created, setCreated] = useState<{ guildId: string; name: string; invite: AdminInvite } | null>(
    null,
  );

  const guilds = overview?.guilds ?? [];
  const airKey = guilds
    .filter((guild) => guild.playback.playing > 0)
    .map((guild) => guild.id)
    .join('|');

  useEffect(() => {
    if (!airKey) {
      setAirDetails({});
      return;
    }
    const controller = new AbortController();
    const ids = airKey.split('|');
    Promise.allSettled(
      ids.map((id) =>
        getGuild(id, controller.signal)
          .then((detail) => ({ id, detail }))
          .catch(() => ({ id, detail: null })),
      ),
    ).then((results) => {
      if (controller.signal.aborted) {
        return;
      }
      const next: Record<string, GuildDetail | null> = {};
      for (const result of results) {
        if (result.status === 'fulfilled') {
          next[result.value.id] = result.value.detail;
        }
      }
      setAirDetails(next);
    });
    return () => controller.abort();
  }, [airKey, airVersion]);

  const auditEntries = useMemo(() => {
    const base = audit.data?.items ?? [];
    const known = new Set(base.map((entry) => entry.id));
    const live = liveAudit.filter((entry) => !known.has(entry.id));
    return [...live, ...base].slice(0, 8);
  }, [audit.data, liveAudit]);

  const ingestionItems = useMemo(() => {
    const base = ingestions.data?.items ?? [];
    const known = new Set(base.map((item) => item.id));
    return [...liveIngestions.filter((item) => !known.has(item.id)), ...base];
  }, [ingestions.data, liveIngestions]);

  const openCreate = () => {
    setGuildName('');
    setCreateError(null);
    setCreateOpen(true);
  };

  const submitCreate = async () => {
    const name = guildName.trim();
    if (!name || createBusy) {
      return;
    }
    setCreateBusy(true);
    setCreateError(null);
    try {
      const response = await createGuild({ name });
      setCreateOpen(false);
      setCreated({ guildId: response.guild.id, name: response.guild.name, invite: response.adminInvite });
      notify('Guilde créée.');
      await refreshOverview();
    } catch (error) {
      setCreateError(describeError(error));
    } finally {
      setCreateBusy(false);
    }
  };

  const stop = async (guildId: string) => {
    try {
      await stopGuild(guildId);
      notify('Lecture arrêtée.');
      await refreshOverview();
      setAirVersion((version) => version + 1);
    } catch (error) {
      notify(describeError(error), 'error');
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
    <DashboardView
      overview={overview}
      health={health}
      healthUnavailable={healthUnavailable}
      ingestions={ingestionItems.length ? ingestionItems : ingestions.data ? [] : null}
      audit={auditEntries.length ? auditEntries : audit.data ? [] : null}
      auditError={audit.error}
      auditUnavailable={isUnavailable(audit.error)}
      airDetails={airDetails}
      onCreateGuild={openCreate}
      onStop={stop}
      onReloadAudit={audit.reload}
    />
  ) : overviewLoading ? (
    <Loading label="Chargement du tableau de bord…" />
  ) : isUnavailable(overviewError) ? (
    <Unavailable detail="L’aperçu (GET /admin/api/overview) n’est pas exposé par ce serveur." />
  ) : (
    <ErrorNotice error={overviewError} onRetry={() => void refreshOverview()} />
  );

  return (
    <Layout route={route} title="Tableau de bord" crumbs={[{ label: 'Tableau de bord' }]}>
      {content}

      <Dialog
        open={createOpen}
        title="Créer une guilde indépendante"
        subtitle="Un groupe sans serveur Discord. Pour un serveur Discord, il suffit d’y inviter le bot."
        onClose={() => setCreateOpen(false)}
        footer={
          <>
            <button type="button" class="btn" onClick={() => setCreateOpen(false)} disabled={createBusy}>
              Annuler
            </button>
            <button
              type="button"
              class="btn is-primary"
              onClick={() => void submitCreate()}
              disabled={createBusy || !guildName.trim()}
            >
              {createBusy ? 'Création…' : 'Créer la guilde'}
            </button>
          </>
        }
      >
        <label for="ng-name">Nom de la guilde</label>
        <input
          class="field is-after-label"
          id="ng-name"
          maxlength={60}
          placeholder="Soirée jeux du vendredi"
          value={guildName}
          onInput={(event) => setGuildName(event.currentTarget.value)}
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

      {created ? (
        <p class="sr-only" aria-live="polite">
          Guilde {created.name} créée. Code admin affiché.
        </p>
      ) : null}

      {overview ? (
        <p class="table-foot">
          Aperçu généré {ago(overview.generatedAt)} · {plural(guilds.length, 'guilde', 'guildes')}
          {health ? ` · v${health.serverVersion}` : ''}
        </p>
      ) : null}
    </Layout>
  );
}
