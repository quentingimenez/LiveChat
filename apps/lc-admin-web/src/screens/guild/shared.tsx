import { useState } from 'preact/hooks';
import { createInvite } from '../../api/client';
import type { DiscordKind, GuildInvite, GuildRole } from '../../api/types';
import { Dialog } from '../../components/Dialog';
import { Badge } from '../../components/ui';
import { describeError } from '../../errors';

export const KIND_LABELS: Record<DiscordKind, string> = {
  native: 'Serveur Discord',
  linked: 'Indépendante, liée',
  none: 'Indépendante',
};

export function GuildKindBadge({ kind }: { kind: DiscordKind }) {
  return <Badge tone={kind === 'none' ? 'outline' : 'info'}>{KIND_LABELS[kind]}</Badge>;
}

export function inviteState(invite: GuildInvite): {
  label: string;
  tone: 'ok' | 'err' | undefined;
} {
  if (invite.isActive) {
    return { label: 'Active', tone: 'ok' };
  }
  if (invite.revokedAt) {
    return { label: 'Révoquée', tone: 'err' };
  }
  if (invite.maxUses != null && invite.useCount >= invite.maxUses) {
    return { label: 'Épuisée', tone: undefined };
  }
  return { label: 'Expirée', tone: undefined };
}

export const ROLE_OPTIONS: ReadonlyArray<{ value: GuildRole; label: string }> = [
  { value: 'member', label: 'Membre' },
  { value: 'guest', label: 'Invité' },
  { value: 'admin', label: 'Admin' },
];

export function numberOrNull(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

export function CreateInviteDialog({
  open,
  guildId,
  guildName,
  onClose,
  onCreated,
}: {
  open: boolean;
  guildId: string;
  guildName?: string;
  onClose: () => void;
  onCreated: (invite: GuildInvite) => void;
}) {
  const [role, setRole] = useState<GuildRole>('member');
  const [expires, setExpires] = useState('168');
  const [maxUses, setMaxUses] = useState('1');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const submit = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const { invite } = await createInvite(guildId, {
        role,
        expiresInHours: numberOrNull(expires),
        maxUses: numberOrNull(maxUses),
      });
      onCreated(invite);
    } catch (caught) {
      setError(caught);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      title="Créer une invitation"
      subtitle={guildName ?? guildId}
      onClose={onClose}
      footer={
        <>
          <button type="button" class="btn" onClick={onClose} disabled={busy}>
            Annuler
          </button>
          <button type="button" class="btn is-primary" onClick={() => void submit()} disabled={busy}>
            {busy ? 'Création…' : 'Créer le code'}
          </button>
        </>
      }
    >
      <div class="stack">
        <div>
          <label for="inv-role">Rôle accordé</label>
          <select
            class="field is-after-label"
            id="inv-role"
            value={role}
            onChange={(event) => setRole(event.currentTarget.value as GuildRole)}
          >
            {ROLE_OPTIONS.map((option) => (
              <option value={option.value} key={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label for="inv-exp">Expiration</label>
          <select
            class="field is-after-label"
            id="inv-exp"
            value={expires}
            onChange={(event) => setExpires(event.currentTarget.value)}
          >
            <option value="24">Dans 24 h</option>
            <option value="168">Dans 7 jours</option>
            <option value="">Jamais</option>
          </select>
        </div>
        <div>
          <label for="inv-uses">Nombre d’utilisations</label>
          <select
            class="field is-after-label"
            id="inv-uses"
            value={maxUses}
            onChange={(event) => setMaxUses(event.currentTarget.value)}
          >
            <option value="1">1 utilisation</option>
            <option value="10">10 utilisations</option>
            <option value="">Illimité</option>
          </select>
        </div>
      </div>
      {error ? (
        <div class="inline-error">{describeError(error)}</div>
      ) : null}
    </Dialog>
  );
}
