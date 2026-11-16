import { BOT_INVITE_URL } from '../constants';
import { useApp } from '../context';
import type { Route } from '../router';
import { Icon } from './icons';
import type { IconName } from './icons';

interface NavItem {
  id: string;
  label: string;
  icon: IconName;
  count?: 'guilds' | 'devices' | 'ingest';
}

const NAV: ReadonlyArray<{ group: string; items: ReadonlyArray<NavItem> }> = [
  {
    group: 'Serveur',
    items: [
      { id: 'dashboard', label: 'Tableau de bord', icon: 'dashboard' },
      { id: 'guilds', label: 'Guildes', icon: 'guilds', count: 'guilds' },
      { id: 'devices', label: 'Overlays', icon: 'device', count: 'devices' },
    ],
  },
  {
    group: 'Accès',
    items: [
      { id: 'codes', label: 'Codes et invitations', icon: 'key' },
      { id: 'ingest', label: 'Clients d’ingestion', icon: 'plug', count: 'ingest' },
      { id: 'roles', label: 'Rôles et droits', icon: 'shield' },
    ],
  },
  {
    group: 'Exploitation',
    items: [
      { id: 'media', label: 'Médias et stockage', icon: 'disk' },
      { id: 'activity', label: 'Journal', icon: 'log' },
      { id: 'settings', label: 'Paramètres du serveur', icon: 'settings' },
    ],
  },
];

export function Sidebar({ route }: { route: Route }) {
  const { overview, health } = useApp();
  const current = route.name === 'guild' ? 'guilds' : route.name;

  const countFor = (kind: NavItem['count']): number | null => {
    if (!overview || !kind) {
      return null;
    }
    if (kind === 'guilds') {
      return overview.totals.guildCount;
    }
    if (kind === 'devices') {
      return overview.totals.overlayClientCount;
    }
    return overview.totals.ingestClientCount;
  };

  return (
    <aside class="side">
      <a class="brand" href="#/dashboard" aria-label="LiveChat, tableau de bord">
        <svg aria-hidden="true" focusable="false">
          <use href="#i-logo" />
        </svg>
        <span>
          LiveChat
          <small>Administration</small>
        </span>
      </a>
      <nav class="nav" aria-label="Navigation principale">
        {NAV.map((group) => (
          <div key={group.group}>
            <p class="nav-group">{group.group}</p>
            {group.items.map((item) => {
              const count = countFor(item.count);
              return (
                <a
                  key={item.id}
                  href={`#/${item.id}`}
                  title={item.label}
                  aria-current={current === item.id ? 'page' : undefined}
                >
                  <Icon name={item.icon} />
                  <span>{item.label}</span>
                  {count != null ? <span class="count">{count}</span> : null}
                </a>
              );
            })}
          </div>
        ))}
      </nav>
      <div class="side-foot">
        <a
          class="btn"
          href={BOT_INVITE_URL}
          target="_blank"
          rel="noopener noreferrer"
          title="Inviter le bot sur un serveur Discord"
        >
          <Icon name="plus" />
          <span>Inviter le bot</span>
        </a>
        <p>
          Serveur {health?.serverVersion ? `v${health.serverVersion}` : 'LiveChat'} ·{' '}
          {typeof window !== 'undefined' ? window.location.host : ''}
        </p>
      </div>
    </aside>
  );
}
