import { useEffect, useState } from 'preact/hooks';
import {
  createDeviceCode,
  isUnavailable,
  revokeOverlayClient,
} from '../../api/client';
import type { GuildEntry, OverlayClient } from '../../api/types';
import { ConfirmDialog, Dialog, SecretDialog } from '../../components/Dialog';
import { Icon } from '../../components/icons';
import {
  Badge,
  Dot,
  Empty,
  ErrorNotice,
  Loading,
  Table,
  Unavailable,
  Who,
} from '../../components/ui';
import { MIN_OVERLAY_VERSION_FALLBACK } from '../../constants';
import { describeError } from '../../errors';
import { ago, plural, versionBelow } from '../../format';
import { MODE_LABELS, roleLabel } from '../../rights';
import { DeviceDialog } from '../DevicesScreen';
import type { GuildTabProps } from './types';

export function DevicesTab({ guild, detail, detailUnavailable, refresh, notify }: GuildTabProps) {
  const [selected, setSelected] = useState<OverlayClient | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<OverlayClient | null>(null);
  const [revokeBusy, setRevokeBusy] = useState(false);
  const [codeOpen, setCodeOpen] = useState(false);
  const [memberId, setMemberId] = useState('');
  const [codeBusy, setCodeBusy] = useState(false);
  const [codeError, setCodeError] = useState<unknown>(null);
  const [issuedCode, setIssuedCode] = useState<string | null>(null);
  const [actionError, setActionError] = useState<unknown>(null);

  const members = detail?.members?.items ?? [];
  const clients = detail?.overlays?.clients ?? guild.overlays.clients;

  useEffect(() => {
    if (!memberId && members.length) {
      setMemberId(members[0].id);
    }
  }, [memberId, members]);

  const revoke = async () => {
    if (!revokeTarget || revokeBusy) {
      return;
    }
    setRevokeBusy(true);
    setActionError(null);
    try {
      await revokeOverlayClient(revokeTarget.id);
      notify(`${revokeTarget.label} révoqué.`);
      setRevokeTarget(null);
      await refresh();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    } finally {
      setRevokeBusy(false);
    }
  };

  const issueCode = async () => {
    if (!memberId || codeBusy) {
      return;
    }
    setCodeBusy(true);
    setCodeError(null);
    try {
      const response = await createDeviceCode(guild.id, { memberId });
      setCodeOpen(false);
      setIssuedCode(typeof response.code === 'string' ? response.code : JSON.stringify(response));
      notify('Code d’appairage généré.');
    } catch (error) {
      setCodeError(error);
    } finally {
      setCodeBusy(false);
    }
  };

  const memberFor = (client: OverlayClient) => {
    if (!client.memberId) {
      return null;
    }
    return members.find((member) => member.id === client.memberId) ?? null;
  };

  return (
    <>
      <div class="toolbar">
        <p class="soft">
          Chaque overlay appairé à cette guilde. « Droits » règle le mode de session de l’appareil.
        </p>
        <span class="spacer" />
        <button class="btn" type="button" onClick={() => setCodeOpen(true)}>
          <Icon name="plus" />
          Code d’appairage
        </button>
      </div>

      {detailUnavailable ? (
        <Unavailable detail="Le détail enrichi des overlays (version, membre, réglages) n’est pas exposé par ce serveur. Les appareils de l’aperçu restent listés." />
      ) : null}
      {isUnavailable(actionError) ? (
        <Unavailable detail="La révocation de cet overlay n’est pas exposée par ce serveur." />
      ) : actionError ? (
        <ErrorNotice error={actionError} />
      ) : null}

      {clients.length ? (
        <Table>
          <thead>
            <tr>
              <th>Appareil</th>
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
            {clients.map((client) => {
              const member = memberFor(client);
              const old =
                Boolean(client.overlayVersion) &&
                versionBelow(client.overlayVersion, MIN_OVERLAY_VERSION_FALLBACK);
              return (
                <tr key={client.id} class={client.connected ? '' : 'is-off'}>
                  <td>
                    <span class="primary">{client.label}</span>
                    <span class="under mono">{client.id.slice(0, 13)}…</span>
                  </td>
                  <td>
                    {member ? (
                      <Who name={member.displayName} />
                    ) : client.authorName ? (
                      <Who name={client.authorName} />
                    ) : (
                      '—'
                    )}
                  </td>
                  <td>
                    {member?.role || client.role ? (
                      <Badge tone={(member?.role ?? client.role) === 'admin' ? 'admin' : undefined}>
                        {roleLabel(member?.role ?? client.role ?? null)}
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
                  <td>
                    {client.connected ? (
                      <Badge tone="ok">
                        <Dot tone="ok" /> En ligne
                      </Badge>
                    ) : client.guildSettings?.paused ? (
                      <Badge>Coupe locale</Badge>
                    ) : (
                      <span class="soft">{ago(client.lastSeenAt)}</span>
                    )}
                  </td>
                  <td>
                    <span class="mono">{client.overlayVersion ?? '—'}</span>
                    {old ? (
                      <>
                        {' '}
                        <Badge tone="err">Obsolète</Badge>
                      </>
                    ) : null}
                    <span class="under">{client.platform ?? ''}</span>
                  </td>
                  <td class="is-actions">
                    <button class="btn is-small" type="button" onClick={() => setSelected(client)}>
                      <Icon name="shield" />
                      Droits
                    </button>
                    <button
                      class="btn is-small is-quiet is-danger"
                      type="button"
                      onClick={() => setRevokeTarget(client)}
                    >
                      Révoquer
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </Table>
      ) : detail === null && !detailUnavailable ? (
        <Loading label="Chargement des overlays…" />
      ) : (
        <div class="card">
          <Empty>Aucun overlay appairé.</Empty>
        </div>
      )}

      {selected ? (
        <DeviceDialog
          guild={guild as GuildEntry}
          client={selected}
          onClose={() => setSelected(null)}
          onSaved={() => void refresh()}
          onRevoke={(target) => {
            setSelected(null);
            setRevokeTarget(target.client);
          }}
          notify={notify}
        />
      ) : null}

      <Dialog
        open={codeOpen}
        title="Code d’appairage"
        subtitle={`Nouvel overlay pour « ${guild.name} »`}
        onClose={() => setCodeOpen(false)}
        footer={
          <>
            <button type="button" class="btn" onClick={() => setCodeOpen(false)} disabled={codeBusy}>
              Annuler
            </button>
            <button
              type="button"
              class="btn is-primary"
              onClick={() => void issueCode()}
              disabled={codeBusy || !memberId}
            >
              {codeBusy ? 'Génération…' : 'Générer le code'}
            </button>
          </>
        }
      >
        {members.length ? (
          <>
            <label for="device-member">Membre qui rattache l’appareil</label>
            <select
              class="field is-after-label"
              id="device-member"
              value={memberId}
              onChange={(event) => setMemberId(event.currentTarget.value)}
            >
              {members.map((member) => (
                <option value={member.id} key={member.id}>
                  {member.displayName} ({roleLabel(member.role)})
                </option>
              ))}
            </select>
            <p class="hint">
              Le code est à usage unique, valable 10 minutes. L’appareil sera rattaché à ce membre.
            </p>
          </>
        ) : (
          <Unavailable detail="La liste des membres nécessaire pour rattacher l’appareil (GET /admin/api/guilds/{id}) n’est pas disponible." />
        )}
        {codeError ? (
          isUnavailable(codeError) ? (
            <Unavailable detail="La création d’un code d’appairage n’est pas exposée par ce serveur." />
          ) : (
            <div class="inline-error">{describeError(codeError)}</div>
          )
        ) : null}
      </Dialog>

      <SecretDialog
        open={issuedCode !== null}
        title="Code d’appairage"
        subtitle={`Nouvel overlay pour « ${guild.name} »`}
        value={issuedCode ?? ''}
        hint="À saisir dans l’overlay. Valable 10 minutes, une seule utilisation."
        onClose={() => setIssuedCode(null)}
      />

      <ConfirmDialog
        open={revokeTarget !== null}
        title={revokeTarget ? `Révoquer ${revokeTarget.label} ?` : 'Révoquer cet overlay ?'}
        body="L’overlay est déconnecté et son jeton invalidé. Le membre reste dans la guilde et peut appairer un nouvel appareil."
        confirmLabel="Révoquer l’appareil"
        danger
        busy={revokeBusy}
        onCancel={() => setRevokeTarget(null)}
        onConfirm={() => void revoke()}
      />

      <p class="table-foot">
        {plural(clients.length, 'overlay', 'overlays')} appairé
        {clients.length > 1 ? 's' : ''}
      </p>
    </>
  );
}
