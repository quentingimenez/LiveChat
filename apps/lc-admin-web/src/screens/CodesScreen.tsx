import { useMemo, useState } from 'preact/hooks';
import {
  deleteExpiredPairingCodes,
  getGuild,
  getPairingCodes,
  isUnavailable,
  revokeInvite,
  revokePairingCode,
} from '../api/client';
import type { GuildInvite, PairingCode } from '../api/types';
import { Layout } from '../components/Layout';
import { Icon } from '../components/icons';
import {
  Badge,
  CopyButton,
  Empty,
  ErrorNotice,
  Loading,
  PageHead,
  Segmented,
  Table,
  Unavailable,
} from '../components/ui';
import { useApp } from '../context';
import { describeError } from '../errors';
import { ago, codeDisplay, expiresIn, plural } from '../format';
import { useAsync } from '../use-async';

type StatusFilter = 'all' | 'active' | 'inactive';

const STATUS_FILTERS: ReadonlyArray<{ value: StatusFilter; label: string }> = [
  { value: 'all', label: 'Tous' },
  { value: 'active', label: 'Actifs' },
  { value: 'inactive', label: 'Expirés ou utilisés' },
];

interface InviteRow {
  invite: GuildInvite;
  guildId: string;
  guildName: string;
}

interface InvitesData {
  rows: InviteRow[];
  unavailable: boolean;
  error: unknown;
}

function inviteStatus(invite: GuildInvite): { label: string; tone: 'ok' | 'err' | 'neutral' } {
  if (invite.isActive) {
    return { label: 'Active', tone: 'ok' };
  }
  if (invite.revokedAt) {
    return { label: 'Révoquée', tone: 'err' };
  }
  if (invite.maxUses != null && invite.useCount >= invite.maxUses) {
    return { label: 'Épuisée', tone: 'neutral' };
  }
  return { label: 'Expirée', tone: 'neutral' };
}

export function CodesScreen() {
  const { route, overview, notify } = useApp();
  const [status, setStatus] = useState<StatusFilter>('all');
  const [actionError, setActionError] = useState<unknown>(null);
  const [purgeBusy, setPurgeBusy] = useState(false);
  const guilds = overview?.guilds ?? [];
  const guildKey = guilds.map((guild) => guild.id).join('|');

  const pairing = useAsync((signal) => getPairingCodes({}, signal), []);

  const invites = useAsync<InvitesData>(
    async (signal) => {
      if (!guilds.length) {
        return { rows: [], unavailable: false, error: null };
      }
      const results = await Promise.allSettled(
        guilds.map((guild) =>
          getGuild(guild.id, signal).then((detail) => ({ guild, detail })),
        ),
      );
      const rows: InviteRow[] = [];
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
        for (const invite of result.value.detail.invites?.items ?? []) {
          rows.push({
            invite,
            guildId: result.value.guild.id,
            guildName: result.value.guild.name,
          });
        }
      }
      rows.sort((left, right) => Date.parse(right.invite.createdAt) - Date.parse(left.invite.createdAt));
      return { rows, unavailable, error };
    },
    [guildKey],
  );

  const rows = useMemo(() => {
    const pairingRows = (pairing.data?.items ?? []).map((item: PairingCode) => ({
      kind: 'pairing' as const,
      code: item.code,
      guildId: item.guildId,
      guildName: guilds.find((guild) => guild.id === item.guildId)?.name ?? item.guildId,
      active: item.isActive,
      role: null,
      createdAt: item.createdAt,
      expiresAt: item.expiresAt,
      author: item.authorName ?? item.createdByDiscordUserId ?? '—',
      usedAt: item.usedAt,
      invite: null as GuildInvite | null,
    }));
    const inviteRows = (invites.data?.rows ?? []).map((row) => ({
      kind: 'invite' as const,
      code: row.invite.code,
      guildId: row.guildId,
      guildName: row.guildName,
      active: row.invite.isActive,
      role: row.invite.role,
      createdAt: row.invite.createdAt,
      expiresAt: row.invite.expiresAt,
      author: row.invite.createdByMemberId ?? '—',
      usedAt: null,
      invite: row.invite,
    }));
    const combined = [...inviteRows, ...pairingRows]
      .sort((left, right) => Date.parse(right.createdAt) - Date.parse(left.createdAt))
      .filter((row) =>
        status === 'all' ? true : status === 'active' ? row.active : !row.active,
      );
    return combined;
  }, [guilds, invites.data, pairing.data, status]);

  const reloadAll = () => {
    pairing.reload();
    invites.reload();
  };

  const purge = async () => {
    if (purgeBusy) {
      return;
    }
    setPurgeBusy(true);
    setActionError(null);
    try {
      const result = await deleteExpiredPairingCodes();
      notify(`${plural(result.deletedCount, 'code d’appairage supprimé', 'codes d’appairage supprimés')}.`);
      pairing.reload();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    } finally {
      setPurgeBusy(false);
    }
  };

  const revokePairing = async (code: string) => {
    try {
      const result = await revokePairingCode(code);
      notify(result.revoked ? 'Code d’appairage révoqué.' : 'Ce code n’était déjà plus actif.');
      pairing.reload();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    }
  };

  const revokeInvitation = async (row: InviteRow) => {
    try {
      await revokeInvite(row.guildId, row.invite.code);
      notify('Invitation révoquée.');
      invites.reload();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    }
  };

  const pairingBlocked = isUnavailable(pairing.error);
  const invitesUnavailable = invites.data?.unavailable ?? isUnavailable(invites.error);
  const everythingUnavailable =
    pairingBlocked && invitesUnavailable && !pairing.data && !invites.data;

  let content;
  if (everythingUnavailable) {
    content = (
      <Unavailable detail="Ni les codes d’appairage ni les invitations ne sont exposés par ce serveur." />
    );
  } else if (pairing.loading && !pairing.data && invites.loading && !invites.data) {
    content = <Loading label="Chargement des codes…" />;
  } else {
    content = (
      <>
        {pairingBlocked ? (
          <Unavailable detail="Les codes d’appairage (GET /admin/api/pairing-codes) ne sont pas exposés par ce serveur." />
        ) : null}
        {invitesUnavailable ? (
          <Unavailable detail="Les invitations d’overlay (GET /admin/api/guilds/{id}) ne sont pas exposées par ce serveur." />
        ) : null}
        {isUnavailable(actionError) ? (
          <Unavailable detail="Cette action de révocation n’est pas exposée par ce serveur." />
        ) : null}
        {rows.length ? (
          <Table>
            <thead>
              <tr>
                <th>Code</th>
                <th>Guilde</th>
                <th>Nature</th>
                <th>Accorde</th>
                <th>État</th>
                <th>Expire</th>
                <th class="is-num">Utilisations</th>
                <th>Créé par</th>
                <th>
                  <span class="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => {
                const invitationStatus = row.invite ? inviteStatus(row.invite) : null;
                return (
                  <tr class={row.active ? '' : 'is-off'} key={`${row.kind}-${row.code}`}>
                    <td>
                      <span class="code">
                        {row.kind === 'invite' ? codeDisplay(row.code) : row.code}
                      </span>
                    </td>
                    <td>
                      <a href={`#/guilds/${encodeURIComponent(row.guildId)}/access`}>
                        {row.guildName}
                      </a>
                    </td>
                    <td>
                      <Badge>
                        {row.kind === 'invite' ? 'Invitation' : 'Appairage Discord'}
                      </Badge>
                    </td>
                    <td>
                      {row.role ? (
                        <Badge tone={row.role === 'admin' ? 'admin' : undefined}>
                          {row.role === 'admin' ? 'Admin' : row.role === 'guest' ? 'Invité' : 'Membre'}
                        </Badge>
                      ) : (
                        <span class="faint">—</span>
                      )}
                    </td>
                    <td>
                      <Badge
                        tone={
                          row.active
                            ? 'ok'
                            : invitationStatus?.tone === 'err'
                              ? 'err'
                              : undefined
                        }
                      >
                        {row.active ? 'Actif' : invitationStatus?.label ?? 'Expiré'}
                      </Badge>
                    </td>
                    <td class="soft">{row.active ? expiresIn(row.expiresAt) : '—'}</td>
                    <td class="is-num">
                      {row.invite
                        ? `${row.invite.useCount}${row.invite.maxUses ? ` / ${row.invite.maxUses}` : ''}`
                        : row.usedAt
                          ? '1 / 1'
                          : '0 / 1'}
                    </td>
                    <td class="soft">
                      {row.author}, {ago(row.createdAt)}
                    </td>
                    <td class="is-actions">
                      {row.active ? (
                        <div class="code-actions">
                          <CopyButton text={row.code} label={`Copier le code ${row.code}`} iconOnly />
                          {row.kind === 'invite' && row.invite ? (
                            <button
                              class="btn is-small is-quiet is-danger"
                              type="button"
                              onClick={() =>
                                void revokeInvitation({
                                  invite: row.invite as GuildInvite,
                                  guildId: row.guildId,
                                  guildName: row.guildName,
                                })
                              }
                            >
                              Révoquer
                            </button>
                          ) : (
                            <button
                              class="btn is-small is-quiet is-danger"
                              type="button"
                              onClick={() => void revokePairing(row.code)}
                            >
                              Révoquer
                            </button>
                          )}
                        </div>
                      ) : null}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </Table>
        ) : (
          <div class="table-wrap">
            <Empty>Aucun code ne correspond.</Empty>
          </div>
        )}
        <p class="table-foot">
          Les invitations sont créées depuis l’onglet « Invitations et codes » d’une guilde ; la purge
          ci-dessus ne concerne que les codes d’appairage expirés.
        </p>
      </>
    );
  }

  return (
    <Layout route={route} title="Codes et invitations" crumbs={[{ label: 'Codes et invitations' }]}>
      <PageHead
        title="Codes et invitations"
        sub="Tout ce qui permet d’entrer dans une guilde : invitations d’overlay et codes d’appairage émis depuis Discord."
        actions={
          <button class="btn" type="button" onClick={() => void purge()} disabled={purgeBusy}>
            <Icon name="trash" />
            {purgeBusy ? 'Purge…' : 'Purger les expirés'}
          </button>
        }
      />
      <div class="toolbar">
        <Segmented
          value={status}
          options={STATUS_FILTERS}
          onChange={setStatus}
          label="Filtrer par état"
        />
        <span class="spacer" />
        <button class="btn is-quiet is-small" type="button" onClick={reloadAll}>
          <Icon name="refresh" />
          Actualiser
        </button>
      </div>
      {invites.error && !invitesUnavailable ? (
        <ErrorNotice error={invites.error} title="Invitations indisponibles" onRetry={invites.reload} />
      ) : null}
      {content}
    </Layout>
  );
}
