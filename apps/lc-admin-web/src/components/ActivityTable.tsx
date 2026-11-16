import type { AuditEntry, GuildEntry } from '../api/types';
import { ago } from '../format';
import { Badge, Empty, Table } from './ui';

export function ActivityTable({
  entries,
  guilds = [],
  empty = 'Aucune entrée.',
}: {
  entries: AuditEntry[];
  guilds?: GuildEntry[];
  empty?: string;
}) {
  if (!entries.length) {
    return (
      <div class="table-wrap">
        <Empty>{empty}</Empty>
      </div>
    );
  }

  const guildName = (id: string | null): string | null => {
    if (!id) {
      return null;
    }
    return guilds.find((guild) => guild.id === id)?.name ?? id;
  };

  return (
    <Table>
      <thead>
        <tr>
          <th>Quand</th>
          <th>Qui</th>
          <th>Action</th>
          <th>Guilde</th>
          <th>Route</th>
          <th class="is-num">Statut</th>
        </tr>
      </thead>
      <tbody>
        {entries.map((entry) => {
          const name = guildName(entry.guildId);
          return (
            <tr key={entry.id}>
              <td class="soft">{ago(entry.at)}</td>
              <td title={entry.actor.method}>{entry.actor.name || entry.actor.method}</td>
              <td class="primary">{entry.action}</td>
              <td>
                {entry.guildId && name ? (
                  <a href={`#/guilds/${encodeURIComponent(entry.guildId)}`}>{name}</a>
                ) : (
                  <span class="faint">—</span>
                )}
              </td>
              <td class="mono soft">
                {entry.method} {entry.path}
              </td>
              <td class="is-num">
                <Badge tone={entry.status < 300 ? 'ok' : 'err'}>{entry.status}</Badge>
              </td>
            </tr>
          );
        })}
      </tbody>
    </Table>
  );
}
