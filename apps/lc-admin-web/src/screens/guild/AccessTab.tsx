import { useMemo, useState } from 'preact/hooks';
import {
  deleteExpiredPairingCodes,
  getPairingCodes,
  isUnavailable,
  revokeInvite,
  revokePairingCode,
} from '../../api/client';
import type { GuildInvite, PairingCode } from '../../api/types';
import { SecretDialog } from '../../components/Dialog';
import { Icon } from '../../components/icons';
import {
  Badge,
  CopyButton,
  Empty,
  ErrorNotice,
  Loading,
  Table,
  Unavailable,
} from '../../components/ui';
import { describeError } from '../../errors';
import { ago, codeDisplay, expiresIn, plural } from '../../format';
import { ROLE_LABELS } from '../../rights';
import { useAsync } from '../../use-async';
import { CreateInviteDialog, inviteState } from './shared';
import type { GuildTabProps } from './types';

export function AccessTab({ guild, detail, detailUnavailable, refresh, notify }: GuildTabProps) {
  const pairing = useAsync((signal) => getPairingCodes({ guildId: guild.id }, signal), [guild.id]);
  const [inviteOpen, setInviteOpen] = useState(false);
  const [createdInvite, setCreatedInvite] = useState<GuildInvite | null>(null);
  const [actionError, setActionError] = useState<unknown>(null);
  const [purgeBusy, setPurgeBusy] = useState(false);

  const invites = detail?.invites?.items ?? [];
  const rows = useMemo(() => {
    const inviteRows = invites.map((invite) => ({
      kind: 'invite' as const,
      invite,
      code: invite.code,
      active: invite.isActive,
      role: invite.role,
      createdAt: invite.createdAt,
      expiresAt: invite.expiresAt,
      author: invite.createdByMemberId ?? '—',
      uses: `${invite.useCount}${invite.maxUses ? ` / ${invite.maxUses}` : ''}`,
    }));
    const pairingRows = (pairing.data?.items ?? []).map((item: PairingCode) => ({
      kind: 'pairing' as const,
      invite: null as GuildInvite | null,
      code: item.code,
      active: item.isActive,
      role: null as GuildInvite['role'] | null,
      createdAt: item.createdAt,
      expiresAt: item.expiresAt,
      author: item.authorName ?? item.createdByDiscordUserId ?? '—',
      uses: item.usedAt ? '1 / 1' : '0 / 1',
    }));
    return [...inviteRows, ...pairingRows].sort(
      (left, right) => Date.parse(right.createdAt) - Date.parse(left.createdAt),
    );
  }, [invites, pairing.data]);

  const purge = async () => {
    if (purgeBusy) {
      return;
    }
    setPurgeBusy(true);
    setActionError(null);
    try {
      const result = await deleteExpiredPairingCodes(guild.id);
      notify(
        `${plural(result.deletedCount, 'code d’appairage supprimé', 'codes d’appairage supprimés')}.`,
      );
      pairing.reload();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    } finally {
      setPurgeBusy(false);
    }
  };

  const revokeInvitation = async (invite: GuildInvite) => {
    try {
      await revokeInvite(guild.id, invite.code);
      notify('Invitation révoquée.');
      await refresh();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
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

  const onInviteCreated = (invite: GuildInvite) => {
    setInviteOpen(false);
    setCreatedInvite(invite);
    notify('Invitation créée.');
    void refresh();
  };

  return (
    <>
      <div class="toolbar">
        <p class="soft">
          Invitations créées dans l’overlay et codes d’appairage émis depuis Discord (
          <span class="mono">/pair-code</span>).
        </p>
        <span class="spacer" />
        <button class="btn" type="button" onClick={() => void purge()} disabled={purgeBusy}>
          <Icon name="trash" />
          {purgeBusy ? 'Purge…' : 'Purger les expirés'}
        </button>
        <button class="btn is-primary" type="button" onClick={() => setInviteOpen(true)}>
          <Icon name="plus" />
          Créer une invitation
        </button>
      </div>

      {detailUnavailable ? (
        <Unavailable detail="Les invitations d’overlay (GET /admin/api/guilds/{id}) ne sont pas exposées par ce serveur ; les codes d’appairage restent disponibles." />
      ) : null}
      {isUnavailable(pairing.error) ? (
        <Unavailable detail="Les codes d’appairage (GET /admin/api/pairing-codes) ne sont pas exposés par ce serveur." />
      ) : isUnavailable(actionError) ? (
        <Unavailable detail="Cette action sur les codes n’est pas exposée par ce serveur." />
      ) : actionError ? (
        <ErrorNotice error={actionError} />
      ) : null}

      {rows.length ? (
        <Table>
          <thead>
            <tr>
              <th>Code</th>
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
              const status = row.invite ? inviteState(row.invite) : null;
              return (
                <tr class={row.active ? '' : 'is-off'} key={`${row.kind}-${row.code}`}>
                  <td>
                    <span class="code">
                      {row.kind === 'invite' ? codeDisplay(row.code) : row.code}
                    </span>
                  </td>
                  <td>
                    <Badge>{row.kind === 'invite' ? 'Invitation' : 'Appairage Discord'}</Badge>
                  </td>
                  <td>
                    {row.role ? (
                      <Badge tone={row.role === 'admin' ? 'admin' : undefined}>
                        {ROLE_LABELS[row.role]}
                      </Badge>
                    ) : (
                      <span class="faint">—</span>
                    )}
                  </td>
                  <td>
                    <Badge
                      tone={row.active ? 'ok' : status?.tone === 'err' ? 'err' : undefined}
                    >
                      {row.active ? (row.kind === 'invite' ? 'Active' : 'Actif') : status?.label ?? 'Expiré'}
                    </Badge>
                  </td>
                  <td class="soft">{row.active ? expiresIn(row.expiresAt) : '—'}</td>
                  <td class="is-num">{row.uses}</td>
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
                            onClick={() => void revokeInvitation(row.invite as GuildInvite)}
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
      ) : pairing.loading && !pairing.data && !detailUnavailable ? (
        <Loading label="Chargement des codes…" />
      ) : (
        <div class="card">
          <Empty>Aucun code pour cette guilde.</Empty>
        </div>
      )}

      <CreateInviteDialog
        open={inviteOpen}
        guildId={guild.id}
        guildName={guild.name}
        onClose={() => setInviteOpen(false)}
        onCreated={onInviteCreated}
      />

      <SecretDialog
        open={createdInvite !== null}
        title="Invitation créée"
        subtitle={createdInvite ? `${ROLE_LABELS[createdInvite.role]} · ${guild.name}` : undefined}
        value={createdInvite?.code ?? ''}
        hint="À saisir dans l’overlay : Rejoindre une guilde."
        onClose={() => setCreatedInvite(null)}
      />
    </>
  );
}
