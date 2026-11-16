import { useState } from 'preact/hooks';
import {
  clearMediaBoard,
  discordLeave,
  discordUnlink,
  isUnavailable,
  purgeGuild,
  revokeGuildClients,
} from '../../api/client';
import { ConfirmDialog } from '../../components/Dialog';
import { Card, CardHead, ErrorNotice, Unavailable } from '../../components/ui';
import { describeError } from '../../errors';
import { bytes, plural } from '../../format';
import { navigate } from '../../router';
import type { GuildTabProps } from './types';

type DangerAction = 'revoke' | 'clear' | 'unlink' | 'leave' | 'purge';

export function DangerTab({ guild, detail, refresh, notify }: GuildTabProps) {
  const [action, setAction] = useState<DangerAction | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const botPresent = detail?.discord.botPresent ?? guild.discord.botPresent ?? false;

  const run = async () => {
    if (!action || busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      if (action === 'revoke') {
        await revokeGuildClients(guild.id, guild.id);
        notify('Tous les accès ont été révoqués.');
        setAction(null);
        await refresh();
      } else if (action === 'clear') {
        await clearMediaBoard(guild.id, guild.id);
        notify('Board vidé.');
        setAction(null);
        await refresh();
      } else if (action === 'unlink') {
        await discordUnlink(guild.id);
        notify('Serveur Discord délié.');
        setAction(null);
        await refresh();
      } else if (action === 'leave') {
        await discordLeave(guild.id);
        notify('Le bot a quitté le serveur Discord.');
        setAction(null);
        await refresh();
      } else {
        await purgeGuild(guild.id, guild.id);
        setAction(null);
        notify(`Guilde « ${guild.name} » supprimée.`);
        navigate({ name: 'guilds', query: {} });
      }
    } catch (caught) {
      setAction(null);
      setError(caught);
      notify(describeError(caught), 'error');
    } finally {
      setBusy(false);
    }
  };

  const dialog = (() => {
    switch (action) {
      case 'revoke':
        return {
          title: 'Révoquer tous les accès ?',
          body: `${plural(guild.overlays.total, 'overlay', 'overlays')}, ${plural(
            guild.ingest.total,
            'client d’ingestion',
            'clients d’ingestion',
          )} et les membres perdent l’accès à « ${guild.name} ». Un code admin permettra de repartir.`,
          label: 'Révoquer les accès',
          expect: guild.id,
        };
      case 'clear':
        return {
          title: 'Vider le board média ?',
          body: `Les ${guild.mediaBoard.itemsCount} médias de « ${guild.name} » et leurs fichiers (${bytes(
            guild.mediaBoard.usedBytes,
          )}) sont supprimés. Irréversible.`,
          label: 'Vider le board',
          expect: guild.id,
        };
      case 'unlink':
        return {
          title: 'Délier le serveur Discord ?',
          body: 'Les commandes de ce serveur Discord ne viseront plus cette guilde.',
          label: 'Délier',
          expect: undefined,
        };
      case 'leave':
        return {
          title: 'Faire quitter le bot ?',
          body: `Le bot quitte le serveur Discord de « ${guild.name} ». Les commandes n’y fonctionneront plus ; les données restent sur le serveur LiveChat.`,
          label: 'Faire quitter le bot',
          expect: undefined,
        };
      case 'purge':
        return {
          title: `Supprimer « ${guild.name} » ?`,
          body: 'Membres, overlays, codes, file d’attente, board et réglages sont supprimés définitivement.',
          label: 'Supprimer la guilde',
          expect: guild.id,
        };
      default:
        return null;
    }
  })();

  const rows: Array<{ title: string; text: string; label: string; action: DangerAction }> = [
    {
      title: 'Révoquer tous les accès',
      text: 'Arrête la lecture, révoque overlays et clients d’ingestion, retire les membres, supprime codes et invitations. La guilde, son board et ses réglages sont conservés.',
      label: 'Révoquer les accès',
      action: 'revoke',
    },
    {
      title: 'Vider le board média',
      text: `Supprime les ${guild.mediaBoard.itemsCount} médias du board et les fichiers devenus orphelins (${bytes(
        guild.mediaBoard.usedBytes,
      )}).`,
      label: 'Vider le board',
      action: 'clear',
    },
  ];
  if (guild.discord.kind === 'linked') {
    rows.push({
      title: 'Délier le serveur Discord',
      text: 'Les commandes de ce serveur Discord ne viseront plus cette guilde. Aucune donnée n’est supprimée.',
      label: 'Délier',
      action: 'unlink',
    });
  }
  if (guild.discord.kind !== 'none' && botPresent) {
    rows.push({
      title: 'Faire quitter le bot',
      text: 'Le bot quitte le serveur Discord. Les données de la guilde restent sur le serveur LiveChat.',
      label: 'Faire quitter le bot',
      action: 'leave',
    });
  }
  rows.push({
    title: 'Supprimer la guilde',
    text: 'Supprime définitivement membres, overlays, codes, file, board et réglages. Irréversible.',
    label: 'Supprimer la guilde',
    action: 'purge',
  });

  return (
    <>
      <Card className="is-danger">
        <CardHead title="Zone de danger" meta="chaque action demande une confirmation" />
        {isUnavailable(error) ? (
          <div class="card-body">
            <Unavailable detail="Cette action irréversible n’est pas exposée par ce serveur." />
          </div>
        ) : error ? (
          <div class="card-body">
            <ErrorNotice error={error} title="Action impossible" />
          </div>
        ) : null}
        {rows.map((row) => (
          <div class="danger-row" key={row.action}>
            <div>
              <h3>{row.title}</h3>
              <p>{row.text}</p>
            </div>
            <button
              class={`btn ${row.action === 'purge' ? 'is-danger-solid' : 'is-danger'}`}
              type="button"
              onClick={() => setAction(row.action)}
              disabled={busy}
            >
              {row.label}
            </button>
          </div>
        ))}
      </Card>

      <ConfirmDialog
        open={dialog !== null}
        title={dialog?.title ?? ''}
        body={dialog?.body ?? ''}
        confirmLabel={dialog?.label ?? 'Confirmer'}
        danger
        expect={dialog?.expect}
        busy={busy}
        onCancel={() => setAction(null)}
        onConfirm={() => void run()}
      />
    </>
  );
}
