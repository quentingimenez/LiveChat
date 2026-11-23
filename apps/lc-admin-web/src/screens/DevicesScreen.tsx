import { useEffect, useMemo, useState } from 'preact/hooks';
import {
  getGuild,
  getRights,
  getRuntimeSettings,
  isUnavailable,
  revokeOverlayClient,
  updateOverlayClient,
  updateOverlayGuildSettings,
} from '../api/client';
import type {
  GuildDetail,
  GuildEntry,
  GuildMember,
  GuildRole,
  OverlayClient,
  OverlayGuildSettings,
  OverlaySessionMode,
  RightsResponse,
} from '../api/types';
import { ConfirmDialog, Dialog } from '../components/Dialog';
import { Layout } from '../components/Layout';
import { Icon } from '../components/icons';
import {
  Badge,
  Dot,
  Empty,
  ErrorNotice,
  Loading,
  PageHead,
  SearchInput,
  Segmented,
  Switch,
  Table,
  Unavailable,
  Who,
} from '../components/ui';
import { MIN_OVERLAY_VERSION_FALLBACK } from '../constants';
import { useApp } from '../context';
import { describeError } from '../errors';
import { ago, plural, versionBelow } from '../format';
import { effectivePermissions, MODE_LABELS, roleLabel } from '../rights';
import { useAsync } from '../use-async';

type DeviceFilter = 'all' | 'online' | 'readonly' | 'outdated';

const FILTERS: ReadonlyArray<{ value: DeviceFilter; label: string }> = [
  { value: 'all', label: 'Tous' },
  { value: 'online', label: 'En ligne' },
  { value: 'readonly', label: 'Lecture seule' },
  { value: 'outdated', label: 'Obsolètes' },
];

interface DeviceEnrichment {
  guildId: string;
  memberId: string | null;
  role: GuildRole | null;
  overlayVersion: string | null;
  platform: string | null;
  guildSettings: OverlayGuildSettings | null;
}

interface DevicesDetails {
  byClient: Record<string, DeviceEnrichment>;
  byGuild: Record<string, GuildDetail | null>;
  unavailable: boolean;
  error: unknown;
}

interface DeviceRow {
  client: OverlayClient;
  guild: GuildEntry;
  enriched: DeviceEnrichment | null;
  member: GuildMember | null;
}

function deviceState(client: OverlayClient, guildSettings: OverlayGuildSettings | null) {
  if (client.connected) {
    return (
      <Badge tone="ok">
        <Dot tone="ok" /> En ligne
      </Badge>
    );
  }
  if (guildSettings?.paused) {
    return <Badge>Coupe locale</Badge>;
  }
  return <span class="soft">{ago(client.lastSeenAt)}</span>;
}

export function DevicesScreen() {
  const { route, overview, overviewError, refreshOverview, notify } = useApp();
  const [filter, setFilter] = useState<DeviceFilter>('all');
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<{ guild: GuildEntry; client: OverlayClient } | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<{ guild: GuildEntry; client: OverlayClient } | null>(
    null,
  );
  const [revokeBusy, setRevokeBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);

  const guilds = overview?.guilds ?? [];
  const guildKey = guilds.map((guild) => guild.id).join('|');

  const runtime = useAsync((signal) => getRuntimeSettings(signal), []);
  const minVersion = useMemo(() => {
    const value = runtime.data?.settings?.minOverlayVersion?.value;
    return typeof value === 'string' && value ? value : MIN_OVERLAY_VERSION_FALLBACK;
  }, [runtime.data]);

  const details = useAsync<DevicesDetails>(
    async (signal) => {
      if (!guilds.length) {
        return { byClient: {}, byGuild: {}, unavailable: false, error: null };
      }
      const results = await Promise.allSettled(
        guilds.map((guild) => getGuild(guild.id, signal).then((detail) => ({ guild, detail }))),
      );
      const byClient: Record<string, DeviceEnrichment> = {};
      const byGuild: Record<string, GuildDetail | null> = {};
      let unavailable = false;
      let error: unknown = null;
      for (const result of results) {
        if (result.status === 'rejected') {
          if (isUnavailable(result.reason)) {
            unavailable = true;
          } else {
            error = result.reason;
          }
          continue;
        }
        const { guild, detail } = result.value;
        byGuild[guild.id] = detail;
        for (const client of detail.overlays?.clients ?? []) {
          byClient[client.id] = {
            guildId: guild.id,
            memberId: client.memberId ?? null,
            role: client.role ?? null,
            overlayVersion: client.overlayVersion ?? null,
            platform: client.platform ?? null,
            guildSettings: client.guildSettings ?? null,
          };
        }
      }
      return { byClient, byGuild, unavailable, error };
    },
    [guildKey],
  );

  const rows = useMemo<DeviceRow[]>(() => {
    const needle = query.trim().toLowerCase();
    return guilds
      .flatMap((guild) =>
        guild.overlays.clients.map((client) => {
          const enriched = details.data?.byClient[client.id] ?? null;
          const detail = details.data?.byGuild[guild.id] ?? null;
          const member = enriched?.memberId
            ? (detail?.members?.items ?? []).find((item) => item.id === enriched.memberId) ?? null
            : null;
          return { client, guild, enriched, member };
        }),
      )
      .filter((row) => {
        const stateOk =
          filter === 'all' ||
          (filter === 'online' && row.client.connected) ||
          (filter === 'readonly' && row.client.sessionMode === 'invite_read_only') ||
          (filter === 'outdated' &&
            Boolean(row.enriched?.overlayVersion) &&
            versionBelow(row.enriched?.overlayVersion, minVersion));
        const haystack = `${row.client.label} ${row.guild.name} ${row.member?.displayName ?? ''}`.toLowerCase();
        return stateOk && (!needle || haystack.includes(needle));
      });
  }, [details.data, filter, guilds, minVersion, query]);

  const revoke = async () => {
    if (!revokeTarget || revokeBusy) {
      return;
    }
    setRevokeBusy(true);
    setActionError(null);
    try {
      await revokeOverlayClient(revokeTarget.client.id);
      notify(`${revokeTarget.client.label} révoqué.`);
      setRevokeTarget(null);
      await refreshOverview();
      details.reload();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    } finally {
      setRevokeBusy(false);
    }
  };

  let content;
  if (!overview) {
    content = overviewError ? (
      <ErrorNotice error={overviewError} onRetry={() => void refreshOverview()} />
    ) : (
      <Loading label="Chargement des overlays…" />
    );
  } else {
    content = (
      <>
        {details.data?.unavailable ? (
          <Unavailable detail="Le détail des overlays (GET /admin/api/guilds/{id}) n’est pas exposé par ce serveur : version, rôle et réglages à distance sont indisponibles." />
        ) : null}
        {isUnavailable(actionError) ? (
          <Unavailable detail="La révocation d’un overlay n’est pas exposée par ce serveur." />
        ) : null}
        {details.data?.error && !details.data.unavailable ? (
          <ErrorNotice error={details.data.error} title="Détails partiellement indisponibles" />
        ) : null}
        {rows.length ? (
          <Table>
            <thead>
              <tr>
                <th>Appareil</th>
                <th>Guilde</th>
                <th>Membre</th>
                <th>Rôle</th>
                <th>Session</th>
                <th>État</th>
                <th>Version</th>
                <th>
                  <span class="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map(({ client, guild, enriched, member }) => {
                const old =
                  Boolean(enriched?.overlayVersion) &&
                  versionBelow(enriched?.overlayVersion, minVersion);
                return (
                  <tr key={client.id} class={client.connected ? '' : 'is-off'}>
                    <td>
                      <span class="primary">{client.label}</span>
                      <span class="under mono">{client.id.slice(0, 13)}…</span>
                    </td>
                    <td>
                      <a href={`#/guilds/${encodeURIComponent(guild.id)}/devices`}>{guild.name}</a>
                    </td>
                    <td>{member ? <Who name={member.displayName} /> : '—'}</td>
                    <td>
                      {member?.role || enriched?.role ? (
                        <Badge tone={(member?.role ?? enriched?.role) === 'admin' ? 'admin' : undefined}>
                          {roleLabel(member?.role ?? enriched?.role ?? null)}
                        </Badge>
                      ) : (
                        <span class="faint">—</span>
                      )}
                    </td>
                    <td>
                      <Badge tone={client.sessionMode === 'normal' ? 'info' : 'outline'}>
                        {MODE_LABELS[client.sessionMode]}
                      </Badge>
                    </td>
                    <td>{deviceState(client, enriched?.guildSettings ?? null)}</td>
                    <td>
                      <span class="mono">{enriched?.overlayVersion ?? '—'}</span>
                      {old ? (
                        <>
                          {' '}
                          <Badge tone="err">Obsolète</Badge>
                        </>
                      ) : null}
                      <span class="under">{enriched?.platform ?? ''}</span>
                    </td>
                    <td class="is-actions">
                      <button
                        class="btn is-small"
                        type="button"
                        onClick={() => setSelected({ guild, client })}
                      >
                        <Icon name="shield" />
                        Droits
                      </button>
                      <button
                        class="btn is-small is-quiet is-danger"
                        type="button"
                        onClick={() => setRevokeTarget({ guild, client })}
                      >
                        Révoquer
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </Table>
        ) : (
          <div class="table-wrap">
            <Empty>Aucun overlay ne correspond.</Empty>
          </div>
        )}
        <p class="table-foot">
          {plural(rows.length, 'overlay', 'overlays')} sur{' '}
          {guilds.reduce((sum, guild) => sum + guild.overlays.clients.length, 0)} · version minimale{' '}
          <span class="mono">{minVersion}</span>
        </p>
      </>
    );
  }

  return (
    <Layout route={route} title="Overlays" crumbs={[{ label: 'Overlays' }]}>
      <PageHead
        title="Overlays"
        sub={
          <>
            Tous les appareils appairés, toutes guildes confondues. Version minimale acceptée :{' '}
            <span class="mono">{minVersion}</span>.
          </>
        }
      />
      <div class="toolbar">
        <SearchInput
          value={query}
          onInput={setQuery}
          placeholder="Appareil, membre ou guilde"
          label="Rechercher un overlay"
        />
        <Segmented value={filter} options={FILTERS} onChange={setFilter} label="Filtrer par état" />
        <span class="spacer" />
        <button
          class="btn is-quiet is-small"
          type="button"
          onClick={() => {
            details.reload();
            void refreshOverview();
          }}
        >
          <Icon name="refresh" />
          Actualiser
        </button>
      </div>
      {content}

      {selected ? (
        <DeviceDialog
          guild={selected.guild}
          client={selected.client}
          onClose={() => setSelected(null)}
          onSaved={() => {
            details.reload();
            void refreshOverview();
          }}
          onRevoke={(target) => {
            setSelected(null);
            setRevokeTarget(target);
          }}
          notify={notify}
        />
      ) : null}

      <ConfirmDialog
        open={revokeTarget !== null}
        title={revokeTarget ? `Révoquer ${revokeTarget.client.label} ?` : 'Révoquer cet overlay ?'}
        body="L’overlay est déconnecté et son jeton invalidé. Le membre reste dans la guilde et peut appairer un nouvel appareil."
        confirmLabel="Révoquer l’appareil"
        danger
        busy={revokeBusy}
        onCancel={() => setRevokeTarget(null)}
        onConfirm={() => void revoke()}
      />
    </Layout>
  );
}

export function DeviceDialog({
  guild,
  client,
  onClose,
  onSaved,
  onRevoke,
  notify,
}: {
  guild: GuildEntry;
  client: OverlayClient;
  onClose: () => void;
  onSaved: () => void;
  onRevoke: (target: { guild: GuildEntry; client: OverlayClient }) => void;
  notify: (message: string, kind?: 'ok' | 'error') => void;
}) {
  const [detail, setDetail] = useState<GuildDetail | null>(null);
  const [detailError, setDetailError] = useState<unknown>(null);
  const [rights, setRights] = useState<RightsResponse | null>(null);
  const [rightsError, setRightsError] = useState<unknown>(null);
  const [label, setLabel] = useState(client.label);
  const [mode, setMode] = useState<OverlaySessionMode>(client.sessionMode);
  const [volume, setVolume] = useState(100);
  const [mediaMuted, setMediaMuted] = useState(false);
  const [paused, setPaused] = useState(false);
  const [busy, setBusy] = useState(false);
  const [saveError, setSaveError] = useState<unknown>(null);

  useEffect(() => {
    const controller = new AbortController();
    setDetailError(null);
    setRightsError(null);
    getGuild(guild.id, controller.signal)
      .then((value) => {
        if (!controller.signal.aborted) {
          setDetail(value);
        }
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) {
          setDetailError(error);
        }
      });
    getRights(controller.signal)
      .then((value) => {
        if (!controller.signal.aborted) {
          setRights(value);
        }
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) {
          setRightsError(error);
        }
      });
    return () => controller.abort();
  }, [guild.id, client.id]);

  useEffect(() => {
    if (!detail) {
      return;
    }
    const enriched = detail.overlays?.clients?.find((item) => item.id === client.id);
    setLabel(enriched?.label ?? client.label);
    setMode(enriched?.sessionMode ?? client.sessionMode);
    if (enriched?.guildSettings) {
      setVolume(enriched.guildSettings.volume);
      setMediaMuted(enriched.guildSettings.mediaMuted);
      setPaused(enriched.guildSettings.paused);
    }
  }, [detail, client.id, client.label, client.sessionMode]);

  const enriched = detail?.overlays?.clients?.find((item) => item.id === client.id) ?? null;
  const member =
    enriched?.memberId
      ? (detail?.members?.items ?? []).find((item) => item.id === enriched.memberId) ?? null
      : null;
  const role: GuildRole | null = member?.role ?? enriched?.role ?? null;
  const detailAvailable = detail !== null;
  const rightsAvailable = rights !== null;
  const detailLoading = !detailAvailable && detailError === null;
  const rightsLoading = !rightsAvailable && rightsError === null;
  const permissions =
    rights && role && detail
      ? effectivePermissions(rights, role, mode, detail.guestsCanViewBoard)
      : null;

  const save = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    setSaveError(null);
    try {
      const trimmed = label.trim();
      await updateOverlayClient(client.id, {
        label: trimmed && trimmed !== client.label ? trimmed : undefined,
        sessionMode: mode,
      });
      if (detailAvailable) {
        await updateOverlayGuildSettings(client.id, { volume, mediaMuted, paused });
      }
      notify(
        `${trimmed || client.label} : session « ${MODE_LABELS[mode]} ».${
          client.connected ? ' Appliqué sans reconnexion.' : ''
        }`,
      );
      onSaved();
      onClose();
    } catch (error) {
      setSaveError(error);
    } finally {
      setBusy(false);
    }
  };

  const permissionsUnavailable =
    !permissions && !detailLoading && !rightsLoading;

  return (
    <Dialog
      open
      wide
      title={label || client.label}
      subtitle="Droits et session de l’overlay"
      onClose={onClose}
      footer={
        <>
          <button
            type="button"
            class="btn is-danger left"
            onClick={() => onRevoke({ guild, client })}
            disabled={busy}
          >
            Révoquer l’appareil
          </button>
          <button type="button" class="btn" onClick={onClose} disabled={busy}>
            Annuler
          </button>
          <button type="button" class="btn is-primary" onClick={() => void save()} disabled={busy}>
            {busy ? 'Enregistrement…' : 'Enregistrer'}
          </button>
        </>
      }
    >
      <dl class="kv">
        <dt>Guilde</dt>
        <dd>{guild.name}</dd>
        <dt>Membre</dt>
        <dd>
          {member ? (
            <>
              <Who name={member.displayName} />{' '}
              <Badge tone={member.role === 'admin' ? 'admin' : undefined}>
                {roleLabel(member.role)}
              </Badge>
            </>
          ) : detailAvailable ? (
            <span class="faint">Membre introuvable</span>
          ) : (
            <span class="faint">—</span>
          )}
        </dd>
        <dt>État</dt>
        <dd>
          {client.connected ? (
            <Badge tone="ok">
              <Dot tone="ok" /> En ligne
            </Badge>
          ) : (
            <>Hors ligne, vu {ago(client.lastSeenAt)}</>
          )}
        </dd>
        <dt>Version</dt>
        <dd>
          <span class="mono">{enriched?.overlayVersion ?? '—'}</span>
          {enriched?.platform ? ` · ${enriched.platform}` : ''}
        </dd>
        <dt>Identifiant</dt>
        <dd class="mono">{client.id}</dd>
      </dl>

      {!detailAvailable && isUnavailable(detailError) ? (
        <div class="dlg-section">
          <Unavailable detail="Le détail de la guilde (membre, rôle, réglages à distance) n’est pas exposé par ce serveur. Le nom et le mode de session restent modifiables." />
        </div>
      ) : null}

      <div class="dlg-section">
        <h3>Nom de l’appareil</h3>
        <input
          class="field"
          id="dev-label"
          maxlength={120}
          aria-label="Nom de l’appareil"
          value={label}
          onInput={(event) => setLabel(event.currentTarget.value)}
        />
      </div>

      <div class="dlg-section">
        <h3>Mode de session</h3>
        <label class="choice">
          <input
            type="radio"
            name="mode"
            value="normal"
            checked={mode === 'normal'}
            onChange={() => setMode('normal')}
          />
          <strong>Contrôle complet</strong>
          <span>
            L’appareil a tous les droits du rôle de son membre
            {role ? ` (${roleLabel(role).toLowerCase()})` : ''}.
          </span>
        </label>
        <label class="choice">
          <input
            type="radio"
            name="mode"
            value="invite_read_only"
            checked={mode === 'invite_read_only'}
            onChange={() => setMode('invite_read_only')}
          />
          <strong>Lecture seule</strong>
          <span>
            L’appareil reçoit les médias mais ne peut rien lancer, arrêter ni modifier, quel que soit
            le rôle du membre.
          </span>
        </label>
      </div>

      <div class="dlg-section">
        <h3>Son et affichage sur cet appareil</h3>
        {detailAvailable ? (
          <>
            <p class="hint is-lead">
              Les réglages « Guilde » de l’overlay, modifiés à distance.{' '}
              {client.connected ? 'Appliqués tout de suite.' : 'Appliqués à sa prochaine connexion.'}
            </p>
            <div class="form is-compact">
              <label for="dev-volume">Volume de la guilde</label>
              <div class="row">
                <input
                  class="range"
                  id="dev-volume"
                  type="range"
                  min={0}
                  max={100}
                  step={5}
                  value={volume}
                  onInput={(event) => setVolume(Number(event.currentTarget.value))}
                />
                <output class="num" for="dev-volume">
                  {volume} %
                </output>
              </div>
              <span class="lbl">Son des médias</span>
              <Switch
                checked={mediaMuted}
                onChange={setMediaMuted}
                label="Couper le son"
                hint="Les médias restent affichés, sans leur son."
              />
              <span class="lbl">Coupe locale</span>
              <Switch
                checked={paused}
                onChange={setPaused}
                label="Couper cette guilde sur cet appareil"
                hint="Plus aucun média de cette guilde ne s’affiche sur cet appareil."
              />
            </div>
            <p class="hint">
              La zone d’affichage se place depuis l’appareil lui-même (« Placer sur l’écran »).
            </p>
          </>
        ) : (
          <Unavailable detail="Les réglages à distance de cet appareil ne sont pas exposés par ce serveur." />
        )}
      </div>

      <div class="dlg-section">
        <h3>Droits effectifs de cet appareil</h3>
        {permissions ? (
          <ul class="perms">
            {permissions.map((permission) => (
              <li class={permission.granted ? '' : 'is-no'} key={permission.action}>
                <Icon name={permission.granted ? 'check' : 'minus'} />
                <span>
                  {permission.label}
                  <span class="sr-only">{permission.granted ? ' : autorisé' : ' : refusé'}</span>
                </span>
              </li>
            ))}
          </ul>
        ) : permissionsUnavailable ? (
          <Unavailable detail="Les droits effectifs (GET /admin/api/rights et détail de la guilde) ne sont pas exposés par ce serveur." />
        ) : (
          <Loading label="Chargement des droits…" />
        )}
        <p class="hint">
          {member
            ? `Pour changer les droits sur tous les appareils de ${member.displayName}, modifie son rôle dans l’onglet Membres.`
            : 'Le rôle du membre fixe les droits de tous ses appareils.'}
        </p>
      </div>

      {saveError ? (
        isUnavailable(saveError) ? (
          <Unavailable detail="L’enregistrement des droits de l’appareil n’est pas exposé par ce serveur." />
        ) : (
          <div class="inline-error">{describeError(saveError)}</div>
        )
      ) : null}
    </Dialog>
  );
}
