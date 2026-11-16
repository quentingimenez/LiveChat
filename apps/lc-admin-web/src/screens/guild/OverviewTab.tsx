import { useMemo } from 'preact/hooks';
import { getAudit, isUnavailable, stopGuild } from '../../api/client';
import { ActivityTable } from '../../components/ActivityTable';
import { Icon } from '../../components/icons';
import {
  Card,
  CardHead,
  Empty,
  ErrorNotice,
  Loading,
  Meter,
  Progress,
  Unavailable,
} from '../../components/ui';
import { describeError } from '../../errors';
import { bytes, dateTime, plural } from '../../format';
import { useAsync } from '../../use-async';
import { GuildKindBadge } from './shared';
import type { GuildTabProps } from './types';

function jobProgress(startedAt: string | null, durationSec: number): number {
  if (!startedAt || !durationSec) {
    return 0;
  }
  const elapsed = (Date.now() - Date.parse(startedAt)) / 1000;
  return Math.max(0, Math.min(100, Math.round((elapsed / durationSec) * 100)));
}

export function OverviewTab({ guild, detail, detailUnavailable, refresh, notify }: GuildTabProps) {
  const audit = useAsync((signal) => getAudit({ limit: 100 }, signal), []);
  const recent = useMemo(
    () => (audit.data?.items ?? []).filter((entry) => entry.guildId === guild.id).slice(0, 4),
    [audit.data, guild.id],
  );
  const members = detail?.members;
  const admins = members?.items.filter((member) => member.role === 'admin').length ?? 0;
  const guests = members?.items.filter((member) => member.role === 'guest').length ?? 0;
  const playing = detail?.queue.playing ?? null;
  const pending = detail?.queue.pending ?? [];

  const stop = async () => {
    try {
      await stopGuild(guild.id);
      notify('Lecture arrêtée.');
      await refresh();
    } catch (error) {
      notify(describeError(error), 'error');
    }
  };

  return (
    <div class="stack">
      <div class="grid is-tiles">
        <div class="card tile">
          <div class="label">
            <Icon name="guilds" />
            Membres
          </div>
          <div class="value">{members ? members.count : '—'}</div>
          <div class="foot">
            {members ? (
              <>
                {plural(admins, 'admin', 'admins')} · {plural(guests, 'invité', 'invités')}
              </>
            ) : (
              'Détail indisponible'
            )}
          </div>
        </div>
        <div class="card tile">
          <div class="label">
            <Icon name="device" />
            Overlays en ligne
          </div>
          <div class="value">
            {guild.overlays.connectedCount} <small>/ {guild.overlays.total}</small>
          </div>
          <div class="foot">
            <a href={`#/guilds/${encodeURIComponent(guild.id)}/devices`}>Voir les appareils</a>
          </div>
        </div>
        <div class="card tile">
          <div class="label">
            <Icon name="disk" />
            Board média
          </div>
          <div class="value">
            {guild.mediaBoard.itemsCount} <small>médias</small>
          </div>
          <div class="foot has-meter">
            <Meter
              used={guild.mediaBoard.usedBytes}
              max={guild.mediaBoard.maxBytes || 1}
              label="Board"
            />
            <span class="num">
              {bytes(guild.mediaBoard.usedBytes)} / {bytes(guild.mediaBoard.maxBytes)}
            </span>
          </div>
        </div>
        <div class="card tile">
          <div class="label">
            <Icon name="play" />
            Lectures
          </div>
          <div class="value">{guild.playback.done}</div>
          <div class="foot">
            {guild.playback.pending} en attente · {guild.playback.failed} en échec
          </div>
        </div>
      </div>

      <div class="grid is-2">
        <Card>
          <CardHead title="À l’antenne">
            <a href={`#/guilds/${encodeURIComponent(guild.id)}/queue`}>File d’attente</a>
          </CardHead>
          {playing ? (
            <div class="air">
              <span class="badge is-air">En lecture</span>
              <div>
                <div class="title">{playing.title || playing.text || 'Média en cours'}</div>
                <div class="soft">
                  {playing.kind ?? 'média'} · {playing.durationSec} s
                  {playing.authorName ? ` · ${playing.authorName}` : ''}
                </div>
                <Progress value={jobProgress(playing.startedAt, playing.durationSec)} />
              </div>
              <button class="btn is-small" type="button" onClick={() => void stop()}>
                <Icon name="stop" />
                Arrêter
              </button>
            </div>
          ) : detailUnavailable ? (
            <div class="card-body">
              <Unavailable detail="La file d’attente (GET /admin/api/guilds/{id}) n’est pas exposée par ce serveur." />
            </div>
          ) : detail === null ? (
            <Loading label="Chargement de la file…" />
          ) : (
            <Empty>Rien à l’antenne.</Empty>
          )}
          {pending.length ? (
            <p class="table-foot">
              {plural(pending.length, 'média en attente', 'médias en attente')} dans la file.
            </p>
          ) : null}
        </Card>

        <Card>
          <CardHead title="Fiche" />
          <div class="card-body">
            <dl class="kv">
              <dt>Type</dt>
              <dd>
                <GuildKindBadge kind={guild.discord.kind} />
              </dd>
              <dt>Serveur Discord</dt>
              <dd>
                {guild.discord.serverId ? (
                  <span class="mono">{guild.discord.serverId}</span>
                ) : (
                  <span class="faint">Aucun</span>
                )}
              </dd>
              <dt>Board</dt>
              <dd>
                {bytes(guild.mediaBoard.usedBytes)} sur {bytes(guild.mediaBoard.maxBytes)}
                {detail?.boardQuotaBytes
                  ? ' · quota propre à la guilde'
                  : ' · quota du serveur'}
              </dd>
              <dt>Invités</dt>
              <dd>
                {detail
                  ? detail.guestsCanViewBoard
                    ? 'Peuvent consulter le board'
                    : 'Ne peuvent pas consulter le board'
                  : '—'}
              </dd>
              <dt>Durée par défaut</dt>
              <dd>
                {guild.settings.defaultMediaTime} s
                {guild.settings.overrides?.defaultMediaTime == null ? (
                  <span class="soft"> (valeur du serveur)</span>
                ) : null}
              </dd>
              <dt>Clients d’ingestion</dt>
              <dd>{guild.ingest.total}</dd>
              <dt>Lecture occupée jusqu’à</dt>
              <dd>{guild.busyUntil ? dateTime(guild.busyUntil) : 'Libre'}</dd>
            </dl>
          </div>
        </Card>
      </div>

      <Card>
        <CardHead title="Dernières actions d’administration" />
        {isUnavailable(audit.error) ? (
          <div class="card-body">
            <Unavailable detail="Le journal (GET /admin/api/audit) n’est pas exposé par ce serveur." />
          </div>
        ) : audit.error ? (
          <div class="card-body">
            <ErrorNotice error={audit.error} onRetry={audit.reload} />
          </div>
        ) : audit.loading && !audit.data ? (
          <Loading label="Chargement du journal…" />
        ) : recent.length ? (
          <ActivityTable entries={recent} guilds={[guild]} />
        ) : (
          <Empty>Aucune action récente sur cette guilde.</Empty>
        )}
      </Card>
    </div>
  );
}
