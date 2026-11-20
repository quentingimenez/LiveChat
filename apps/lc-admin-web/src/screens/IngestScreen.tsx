import { useEffect, useMemo, useState } from 'preact/hooks';
import {
  createIngestClient,
  getIngestAuthors,
  isUnavailable,
  revokeIngestClient,
} from '../api/client';
import type { IngestClient } from '../api/types';
import { ConfirmDialog, Dialog, SecretDialog } from '../components/Dialog';
import { Layout } from '../components/Layout';
import { Icon } from '../components/icons';
import {
  Badge,
  Empty,
  ErrorNotice,
  Loading,
  PageHead,
  Table,
  Unavailable,
  Who,
} from '../components/ui';
import { useApp } from '../context';
import { describeError } from '../errors';
import { ago } from '../format';
import { useAsync } from '../use-async';

interface IngestRow {
  guildId: string;
  guildName: string;
  client: IngestClient;
}

function clientAuthor(client: IngestClient): string {
  if (client.defaultAuthorName) {
    return client.defaultAuthorName;
  }
  const separator = client.label.indexOf('-');
  return separator >= 0 ? client.label.slice(separator + 1) : '—';
}

export function IngestScreen() {
  const { route, overview, overviewError, refreshOverview, notify } = useApp();
  const authors = useAsync((signal) => getIngestAuthors(signal), []);
  const [createOpen, setCreateOpen] = useState(false);
  const [guildId, setGuildId] = useState('');
  const [authorId, setAuthorId] = useState('');
  const [label, setLabel] = useState('Chrome');
  const [busy, setBusy] = useState(false);
  const [createError, setCreateError] = useState<unknown>(null);
  const [token, setToken] = useState<string | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<IngestRow | null>(null);
  const [revokeBusy, setRevokeBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);

  const guilds = overview?.guilds ?? [];
  const rows = useMemo<IngestRow[]>(
    () =>
      guilds.flatMap((guild) =>
        guild.ingest.clients.map((client) => ({
          guildId: guild.id,
          guildName: guild.name,
          client,
        })),
      ),
    [guilds],
  );

  useEffect(() => {
    if (!guildId && guilds.length) {
      setGuildId(guilds[0].id);
    }
  }, [guildId, guilds]);

  useEffect(() => {
    const items = authors.data?.items ?? [];
    if (!authorId && items.length) {
      setAuthorId(items[0].discordUserId);
    }
  }, [authorId, authors.data]);

  const openCreate = () => {
    setCreateError(null);
    setCreateOpen(true);
  };

  const submitCreate = async () => {
    if (busy || !guildId || !authorId || !label.trim()) {
      return;
    }
    setBusy(true);
    setCreateError(null);
    try {
      const response = await createIngestClient({
        guildId,
        authorDiscordUserId: authorId,
        label: label.trim(),
      });
      setCreateOpen(false);
      setToken(response.ingestApiToken);
      notify('Client d’ingestion créé.');
      await refreshOverview();
    } catch (error) {
      setCreateError(error);
    } finally {
      setBusy(false);
    }
  };

  const revoke = async () => {
    if (!revokeTarget || revokeBusy) {
      return;
    }
    setRevokeBusy(true);
    setActionError(null);
    try {
      await revokeIngestClient(revokeTarget.client.id);
      notify(`${revokeTarget.client.label} révoqué.`);
      setRevokeTarget(null);
      await refreshOverview();
    } catch (error) {
      setActionError(error);
      notify(describeError(error), 'error');
    } finally {
      setRevokeBusy(false);
    }
  };

  let content;
  if (overview) {
    content = (
      <>
        {rows.length ? (
          <Table>
            <thead>
              <tr>
                <th>Client</th>
                <th>Guilde</th>
                <th>Auteur par défaut</th>
                <th>Rôle de l’auteur</th>
                <th>Dernière utilisation</th>
                <th>Créé</th>
                <th>
                  <span class="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map(({ guildId: rowGuildId, guildName, client }) => {
                const author = clientAuthor(client);
                return (
                  <tr key={client.id}>
                    <td>
                      <span class="primary">{client.label}</span>
                      <span class="under mono">{client.id.slice(0, 13)}…</span>
                    </td>
                    <td>
                      <a href={`#/guilds/${encodeURIComponent(rowGuildId)}`}>{guildName}</a>
                    </td>
                    <td>{author === '—' ? <span class="faint">—</span> : <Who name={author} />}</td>
                    <td>
                      <Badge tone="warn" title="Le rôle n’est vérifié qu’à l’appairage aujourd’hui">
                        À l’appairage
                      </Badge>
                    </td>
                    <td class="soft">{ago(client.lastSeenAt)}</td>
                    <td class="soft">{ago(client.createdAt)}</td>
                    <td class="is-actions">
                      <button
                        class="btn is-small is-quiet is-danger"
                        type="button"
                        onClick={() => setRevokeTarget({ guildId: rowGuildId, guildName, client })}
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
            <Empty>Aucun client d’ingestion actif.</Empty>
          </div>
        )}
        <p class="table-foot">
          Le jeton d’un client n’est affiché qu’une fois, à sa création. Depuis l’overlay, un membre
          peut aussi appairer son extension avec un code (<span class="mono">POST /overlay/ingest-code</span>).
          Pour un raccourci iOS, importe un{' '}
          <a
            href="https://github.com/quentingimenez/LiveChat/tree/main/ios"
            target="_blank"
            rel="noreferrer"
          >
            raccourci signé
          </a>{' '}
          puis colle l’adresse du serveur et le jeton dans les questions d’import.
        </p>
      </>
    );
  } else if (overviewError) {
    content = (
      <ErrorNotice error={overviewError} onRetry={() => void refreshOverview()} />
    );
  } else {
    content = <Loading label="Chargement des clients d’ingestion…" />;
  }

  return (
    <Layout
      route={route}
      title="Clients d’ingestion"
      crumbs={[{ label: 'Clients d’ingestion' }]}
    >
      <PageHead
        title="Clients d’ingestion"
        sub={
          <>
            Extensions de navigateur et scripts autorisés à envoyer des médias à une guilde (
            <span class="mono">POST /ingest</span>).
          </>
        }
        actions={
          <button class="btn is-primary" type="button" onClick={openCreate}>
            <Icon name="plus" />
            Créer un client
          </button>
        }
      />
      {isUnavailable(actionError) ? (
        <Unavailable detail="La révocation d’un client d’ingestion n’est pas exposée par ce serveur." />
      ) : actionError ? (
        <ErrorNotice error={actionError} title="Action impossible" />
      ) : null}
      {content}

      <Dialog
        open={createOpen}
        title="Créer un client d’ingestion"
        subtitle="Pour une extension de navigateur, un script ou un raccourci iOS."
        onClose={() => setCreateOpen(false)}
        footer={
          <>
            <button type="button" class="btn" onClick={() => setCreateOpen(false)} disabled={busy}>
              Annuler
            </button>
            <button
              type="button"
              class="btn is-primary"
              onClick={() => void submitCreate()}
              disabled={busy || !guildId || !authorId || !label.trim()}
            >
              {busy ? 'Création…' : 'Créer le client'}
            </button>
          </>
        }
      >
        <div class="stack">
          <div>
            <label for="ic-guild">Guilde</label>
            <select
              class="field is-after-label"
              id="ic-guild"
              value={guildId}
              onChange={(event) => setGuildId(event.currentTarget.value)}
            >
              {guilds.map((guild) => (
                <option value={guild.id} key={guild.id}>
                  {guild.name}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label for="ic-author">Auteur par défaut</label>
            {isUnavailable(authors.error) ? (
              <Unavailable detail="La liste des auteurs connus (GET /admin/api/ingest-authors) n’est pas exposée par ce serveur." />
            ) : authors.loading ? (
              <Loading label="Chargement des auteurs…" />
            ) : authors.error ? (
              <ErrorNotice error={authors.error} onRetry={authors.reload} />
            ) : (
              <>
                <select
                  class="field is-after-label"
                  id="ic-author"
                  value={authorId}
                  onChange={(event) => setAuthorId(event.currentTarget.value)}
                >
                  {(authors.data?.items ?? []).map((author) => (
                    <option value={author.discordUserId} key={author.discordUserId}>
                      {author.displayName}
                    </option>
                  ))}
                </select>
                <p class="hint">Les médias envoyés par ce client seront signés de ce nom.</p>
              </>
            )}
          </div>
          <div>
            <label for="ic-label">Libellé</label>
            <input
              class="field is-after-label"
              id="ic-label"
              maxlength={120}
              value={label}
              onInput={(event) => setLabel(event.currentTarget.value)}
            />
          </div>
        </div>
        {createError ? (
          isUnavailable(createError) ? (
            <Unavailable detail="La création de clients d’ingestion n’est pas exposée par ce serveur." />
          ) : (
            <div class="inline-error">{describeError(createError)}</div>
          )
        ) : null}
      </Dialog>

      <SecretDialog
        open={token !== null}
        title="Client créé"
        subtitle="Jeton d’ingestion"
        value={token ?? ''}
        hint="Ce jeton n’est affiché qu’une seule fois : copie-le maintenant dans l’extension ou le raccourci iOS."
        onClose={() => setToken(null)}
      />

      <ConfirmDialog
        open={revokeTarget !== null}
        title={revokeTarget ? `Révoquer ${revokeTarget.client.label} ?` : 'Révoquer ce client ?'}
        body="Son jeton cesse de fonctionner tout de suite."
        confirmLabel="Révoquer"
        danger
        busy={revokeBusy}
        onCancel={() => setRevokeTarget(null)}
        onConfirm={() => void revoke()}
      />
    </Layout>
  );
}
