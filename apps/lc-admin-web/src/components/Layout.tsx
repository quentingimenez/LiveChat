import type { ComponentChildren } from 'preact';
import { useEffect } from 'preact/hooks';
import { useApp } from '../context';
import type { Route } from '../router';
import { Avatar, Dot } from './ui';
import { Sidebar } from './Sidebar';
import { Icon } from './icons';

export interface Crumb {
  label: string;
  href?: string;
}

export function Layout({
  route,
  title,
  crumbs,
  children,
}: {
  route: Route;
  title: string;
  crumbs: Crumb[];
  children: ComponentChildren;
}) {
  const { session, overview, health, signOut, notify } = useApp();

  useEffect(() => {
    document.title = `${title} — LiveChat Administration`;
  }, [title]);

  useEffect(() => {
    window.scrollTo(0, 0);
    const main = document.getElementById('content');
    if (main) {
      main.focus({ preventScroll: true });
    }
  }, [route.name, route.guildId, route.tab]);

  const connected = overview?.totals.overlayConnectedCount;
  let pillLabel = 'État inconnu';
  let pillTone: 'ok' | 'err' | 'warn' = 'err';
  if (health) {
    if (health.status === 'ok') {
      pillLabel = 'Opérationnel';
      pillTone = 'ok';
    } else if (health.status === 'degraded') {
      pillLabel = 'Dégradé';
      pillTone = 'warn';
    } else {
      pillLabel = 'Incident';
      pillTone = 'err';
    }
  } else if (overview) {
    pillLabel = 'En ligne';
    pillTone = 'ok';
  }

  const userName = session.user?.name || 'Administrateur';
  const methodTitle =
    session.method === 'discord'
      ? 'Connecté avec Discord'
      : 'Connecté avec le secret d’administration';

  const logout = async () => {
    try {
      await signOut();
    } catch {
      notify('La déconnexion a échoué. La session locale a été fermée.', 'error');
    }
  };

  return (
    <div class="shell">
      <Sidebar route={route} />
      <div class="main">
        <header class="top">
          <div class="crumbs">
            {crumbs.map((crumb, index) => {
              const last = index === crumbs.length - 1;
              if (last || !crumb.href) {
                return <strong key={`${crumb.label}-${index}`}>{crumb.label}</strong>;
              }
              return (
                <span key={`${crumb.label}-${index}`} class="row">
                  <a href={crumb.href}>{crumb.label}</a>
                  <Icon name="chevron" />
                </span>
              );
            })}
          </div>
          <span class="spacer" />
          <span class="health-pill" title={health ? `Santé : ${health.status}` : 'Santé indisponible'}>
            <Dot tone={pillTone} />
            {pillLabel}
            {connected != null ? (
              <span class="long"> · {connected} overlays en ligne</span>
            ) : null}
          </span>
          <span class="user" title={methodTitle}>
            <Avatar name={userName} />
            <span class="long">{userName}</span>
          </span>
          <button
            class="btn is-quiet is-icon"
            type="button"
            data-act="logout"
            aria-label="Se déconnecter"
            title="Se déconnecter"
            onClick={logout}
          >
            <Icon name="logout" />
          </button>
        </header>
        <main class="page" id="content" tabIndex={-1}>
          {children}
        </main>
      </div>
    </div>
  );
}
