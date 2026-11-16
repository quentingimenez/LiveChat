import { useState } from 'preact/hooks';
import {
  createDiscordLinkCode,
  discordLeave,
  discordUnlink,
  isUnavailable,
} from '../../api/client';
import { ConfirmDialog, SecretDialog } from '../../components/Dialog';
import { Icon } from '../../components/icons';
import { Badge, Card, CardHead, ErrorNotice, Unavailable } from '../../components/ui';
import { BOT_INVITE_URL, DISCORD_COMMANDS } from '../../constants';
import { describeError } from '../../errors';
import type { GuildTabProps } from './types';

export function DiscordTab({ guild, detail, detailUnavailable, refresh, notify }: GuildTabProps) {
  const [confirmLeave, setConfirmLeave] = useState(false);
  const [confirmUnlink, setConfirmUnlink] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [linkCode, setLinkCode] = useState<string | null>(null);

  const kind = guild.discord.kind;
  const serverId = guild.discord.serverId;
  const botPresent = detail?.discord.botPresent ?? guild.discord.botPresent ?? null;

  const leave = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await discordLeave(guild.id);
      setConfirmLeave(false);
      notify('Le bot a quitté le serveur Discord.');
      await refresh();
    } catch (caught) {
      setConfirmLeave(false);
      setError(caught);
      notify(describeError(caught), 'error');
    } finally {
      setBusy(false);
    }
  };

  const unlink = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await discordUnlink(guild.id);
      setConfirmUnlink(false);
      notify('Serveur Discord délié.');
      await refresh();
    } catch (caught) {
      setConfirmUnlink(false);
      setError(caught);
      notify(describeError(caught), 'error');
    } finally {
      setBusy(false);
    }
  };

  const generateLinkCode = async () => {
    setBusy(true);
    setError(null);
    try {
      const response = await createDiscordLinkCode(guild.id);
      const code =
        typeof response.command === 'string'
          ? response.command
          : typeof response.code === 'string'
            ? `/lier code:${response.code}`
            : JSON.stringify(response);
      setLinkCode(code);
    } catch (caught) {
      setError(caught);
      notify(describeError(caught), 'error');
    } finally {
      setBusy(false);
    }
  };

  let body;
  if (kind === 'native') {
    body = (
      <>
        <dl class="kv">
          <dt>Rattachement</dt>
          <dd>
            Cette guilde <strong>est</strong> un serveur Discord.
          </dd>
          <dt>Serveur</dt>
          <dd class="mono">{serverId}</dd>
          <dt>Bot</dt>
          <dd>
            {botPresent === null ? (
              <Badge tone="warn">Statut inconnu</Badge>
            ) : botPresent ? (
              <Badge tone="ok">Présent</Badge>
            ) : (
              <Badge tone="err">Absent du serveur</Badge>
            )}
          </dd>
          <dt>Commandes</dt>
          <dd>
            {DISCORD_COMMANDS.map((name, index) => (
              <span key={name}>
                {index ? ', ' : ''}
                <span class="mono">/{name}</span>
              </span>
            ))}{' '}
            · {DISCORD_COMMANDS.length} synchronisées
          </dd>
        </dl>
        <div class="row is-wrap is-spaced">
          {botPresent === false ? (
            <a
              class="btn"
              href={`${BOT_INVITE_URL}&guild_id=${encodeURIComponent(guild.id)}`}
              target="_blank"
              rel="noopener noreferrer"
            >
              <Icon name="external" />
              Réinviter le bot
            </a>
          ) : (
            <button
              class="btn is-danger"
              type="button"
              onClick={() => setConfirmLeave(true)}
              disabled={busy}
            >
              Faire quitter le bot
            </button>
          )}
        </div>
      </>
    );
  } else if (kind === 'linked') {
    body = (
      <>
        <dl class="kv">
          <dt>Rattachement</dt>
          <dd>Guilde indépendante liée à un serveur Discord.</dd>
          <dt>Serveur</dt>
          <dd>
            <span class="mono soft">{serverId}</span>
          </dd>
          <dt>Bot</dt>
          <dd>
            {botPresent === null ? (
              <Badge tone="warn">Statut inconnu</Badge>
            ) : botPresent ? (
              <Badge tone="ok">Présent</Badge>
            ) : (
              <Badge tone="err">Absent</Badge>
            )}
          </dd>
        </dl>
        <div class="row is-wrap is-spaced">
          <button
            class="btn is-danger"
            type="button"
            onClick={() => setConfirmUnlink(true)}
            disabled={busy}
          >
            Délier le serveur Discord
          </button>
          <button
            class="btn"
            type="button"
            onClick={() => setConfirmLeave(true)}
            disabled={busy}
          >
            Faire quitter le bot
          </button>
        </div>
        <p class="hint">Délier ne supprime aucune donnée ; le bot quitte aussi le serveur.</p>
      </>
    );
  } else {
    body = (
      <>
        <p class="soft">
          Cette guilde indépendante n’est liée à aucun serveur Discord. Pour la lier : inviter le bot
          sur le serveur, générer un code, puis taper la commande dans Discord.
        </p>
        <div class="row is-wrap is-spaced">
          <a
            class="btn"
            href={BOT_INVITE_URL}
            target="_blank"
            rel="noopener noreferrer"
          >
            <Icon name="external" />
            Inviter le bot
          </a>
          <button
            class="btn is-primary"
            type="button"
            onClick={() => void generateLinkCode()}
            disabled={busy}
          >
            Générer un code de liaison
          </button>
        </div>
      </>
    );
  }

  return (
    <>
      <Card>
        <CardHead title="Liaison Discord" />
        <div class="card-body">
          {detailUnavailable && botPresent === null ? (
            <Unavailable detail="La présence du bot (GET /admin/api/guilds/{id}) n’est pas exposée par ce serveur." />
          ) : null}
          {body}
          {isUnavailable(error) ? (
            <Unavailable detail="Cette action Discord n’est pas exposée par ce serveur." />
          ) : error ? (
            <ErrorNotice error={error} title="Action impossible" />
          ) : null}
        </div>
      </Card>

      <ConfirmDialog
        open={confirmLeave}
        title="Faire quitter le bot ?"
        body={`Le bot quitte le serveur Discord de « ${guild.name} ». Les commandes n’y fonctionneront plus ; les données restent sur le serveur LiveChat.`}
        confirmLabel="Faire quitter le bot"
        danger
        busy={busy}
        onCancel={() => setConfirmLeave(false)}
        onConfirm={() => void leave()}
      />

      <ConfirmDialog
        open={confirmUnlink}
        title="Délier le serveur Discord ?"
        body="Les commandes de ce serveur Discord ne viseront plus cette guilde. Aucune donnée n’est supprimée."
        confirmLabel="Délier"
        danger
        busy={busy}
        onCancel={() => setConfirmUnlink(false)}
        onConfirm={() => void unlink()}
      />

      <SecretDialog
        open={linkCode !== null}
        title="Code de liaison Discord"
        subtitle={guild.name}
        value={linkCode ?? ''}
        hint="Un administrateur du serveur Discord tape cette commande. Valable 10 minutes."
        onClose={() => setLinkCode(null)}
      />
    </>
  );
}
