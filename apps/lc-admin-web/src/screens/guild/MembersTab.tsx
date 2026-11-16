import { useState } from 'preact/hooks';
import { isUnavailable, patchMember, removeMember } from '../../api/client';
import type { GuildInvite, GuildMember, GuildRole } from '../../api/types';
import { ConfirmDialog, SecretDialog } from '../../components/Dialog';
import { Icon } from '../../components/icons';
import {
  Avatar,
  Badge,
  Empty,
  ErrorNotice,
  Loading,
  Table,
  Unavailable,
  Who,
} from '../../components/ui';
import { describeError } from '../../errors';
import { ago, plural } from '../../format';
import { ROLE_LABELS, roleLabel } from '../../rights';
import { CreateInviteDialog, ROLE_OPTIONS } from './shared';
import type { GuildTabProps } from './types';

export function MembersTab({ guild, detail, detailUnavailable, refresh, notify }: GuildTabProps) {
  const [busyMember, setBusyMember] = useState<string | null>(null);
  const [kickTarget, setKickTarget] = useState<GuildMember | null>(null);
  const [kickBusy, setKickBusy] = useState(false);
  const [inviteOpen, setInviteOpen] = useState(false);
  const [createdInvite, setCreatedInvite] = useState<GuildInvite | null>(null);
  const [actionError, setActionError] = useState<unknown>(null);

  const members = detail?.members?.items ?? [];

  const changeRole = async (member: GuildMember, role: GuildRole) => {
    if (busyMember || role === member.role) {
      return;
    }
    setBusyMember(member.id);
    setActionError(null);
    try {
      await patchMember(guild.id, member.id, { role });
      notify(`${member.displayName} est maintenant ${roleLabel(role).toLowerCase()}.`);
      await refresh();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
      await refresh();
    } finally {
      setBusyMember(null);
    }
  };

  const kick = async () => {
    if (!kickTarget || kickBusy) {
      return;
    }
    setKickBusy(true);
    setActionError(null);
    try {
      await removeMember(guild.id, kickTarget.id);
      notify(`${kickTarget.displayName} a été exclu.`);
      setKickTarget(null);
      await refresh();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
      await refresh();
    } finally {
      setKickBusy(false);
    }
  };

  const onInviteCreated = (invite: GuildInvite) => {
    setInviteOpen(false);
    setCreatedInvite(invite);
    notify('Invitation créée.');
    void refresh();
  };

  let body;
  if (detailUnavailable) {
    body = (
      <Unavailable detail="La liste des membres (GET /admin/api/guilds/{id}) n’est pas exposée par ce serveur." />
    );
  } else if (!detail) {
    body = <Loading label="Chargement des membres…" />;
  } else if (!members.length) {
    body = <Empty>Aucun membre dans cette guilde.</Empty>;
  } else {
    body = (
      <Table>
        <thead>
          <tr>
            <th>Membre</th>
            <th>Compte Discord</th>
            <th>Rôle</th>
            <th class="is-num">Appareils</th>
            <th>Présence</th>
            <th>Arrivée</th>
            <th>
              <span class="sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {members.map((member) => {
            const online = member.connected === true;
            return (
              <tr key={member.id}>
                <td class="primary">
                  <span class="who">
                    <Avatar name={member.displayName} />
                    {member.displayName}
                    {member.isSelf ? (
                      <>
                        {' '}
                        <Badge tone="info">Vous</Badge>
                      </>
                    ) : null}
                  </span>
                </td>
                <td>
                  {member.discordLinked ? (
                    <>
                      <div class="discord-cell">
                        <Badge tone="info">Lié</Badge>
                        {member.discordAdmin ? (
                          <Badge tone="admin" title="Administrateur du serveur Discord">
                            Admin Discord
                          </Badge>
                        ) : null}
                      </div>
                      {member.discordUserId ? (
                        <span class="under mono">{member.discordUserId}</span>
                      ) : null}
                    </>
                  ) : (
                    <span class="faint">Non lié</span>
                  )}
                </td>
                <td>
                  <select
                    class="field is-inline"
                    aria-label={`Rôle de ${member.displayName}`}
                    value={member.role}
                    disabled={busyMember === member.id}
                    onChange={(event) =>
                      void changeRole(member, event.currentTarget.value as GuildRole)
                    }
                  >
                    {ROLE_OPTIONS.map((option) => (
                      <option value={option.value} key={option.value}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                </td>
                <td class="is-num">{member.deviceCount ?? 0}</td>
                <td>
                  {online ? (
                    <Badge tone="ok">
                      <span class="dot is-ok" /> En ligne
                    </Badge>
                  ) : (
                    <span class="soft">
                      {member.deviceCount
                        ? ago(member.lastSeenAt)
                        : 'Aucun appareil'}
                    </span>
                  )}
                </td>
                <td class="soft">{ago(member.createdAt)}</td>
                <td class="is-actions">
                  <button
                    class="btn is-small is-quiet is-danger"
                    type="button"
                    onClick={() => setKickTarget(member)}
                  >
                    Exclure
                  </button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </Table>
    );
  }

  return (
    <>
      <div class="toolbar">
        <p class="soft">
          Le rôle fixe les droits de tous les appareils du membre. Un administrateur du serveur
          Discord devient admin en s’appairant avec <span class="mono">/pair-code</span>.
        </p>
        <span class="spacer" />
        <button class="btn" type="button" onClick={() => setInviteOpen(true)}>
          <Icon name="plus" />
          Inviter un membre
        </button>
      </div>

      {isUnavailable(actionError) ? (
        <Unavailable detail="Cette action sur les membres n’est pas exposée par ce serveur." />
      ) : actionError ? (
        <ErrorNotice error={actionError} title="Action impossible" />
      ) : null}

      {body}

      <ConfirmDialog
        open={kickTarget !== null}
        title={kickTarget ? `Exclure ${kickTarget.displayName} ?` : 'Exclure ce membre ?'}
        body={
          kickTarget
            ? `Ses overlays sont déconnectés tout de suite et ses extensions révoquées. Il lui faudra un nouveau code pour revenir.`
            : ''
        }
        confirmLabel="Exclure"
        danger
        busy={kickBusy}
        onCancel={() => setKickTarget(null)}
        onConfirm={() => void kick()}
      />

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
        subtitle={
          createdInvite
            ? `${ROLE_LABELS[createdInvite.role]} · ${guild.name}`
            : undefined
        }
        value={createdInvite?.code ?? ''}
        hint="À saisir dans l’overlay : Rejoindre une guilde."
        onClose={() => setCreatedInvite(null)}
      />

      {detail ? (
        <p class="table-foot">
          {plural(members.length, 'membre', 'membres')} ·{' '}
          {plural(
            members.reduce((sum, member) => sum + (member.deviceCount ?? 0), 0),
            'appareil',
            'appareils',
          )}
        </p>
      ) : null}
    </>
  );
}
