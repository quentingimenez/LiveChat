import { useCallback, useEffect, useMemo, useState } from 'preact/hooks';
import {
  getEventsUrl,
  getHealth,
  getOverview,
  getSession,
  isApiError,
  loginWithSecret,
  logout,
  setUnauthorizedHandler,
} from './api/client';
import { subscribeLive } from './api/live';
import type { AuditEntry, AuthMethods, Health, Ingestion, Overview, Session } from './api/types';
import { ToastProvider, useToast } from './components/Toasts';
import { AppContext } from './context';
import type { AppContextValue, Notify } from './context';
import { describeError } from './errors';
import { navigate, useHashRoute } from './router';
import type { Route } from './router';
import { ActivityScreen } from './screens/ActivityScreen';
import { CodesScreen } from './screens/CodesScreen';
import { DashboardScreen } from './screens/DashboardScreen';
import { DevicesScreen } from './screens/DevicesScreen';
import { GuildDetailScreen } from './screens/GuildDetailScreen';
import { GuildsScreen } from './screens/GuildsScreen';
import { IngestScreen } from './screens/IngestScreen';
import { LoginScreen } from './screens/LoginScreen';
import { MediaScreen } from './screens/MediaScreen';
import { RolesScreen } from './screens/RolesScreen';
import { SettingsScreen } from './screens/SettingsScreen';

type AuthState =
  | { status: 'loading' }
  | { status: 'anonymous'; methods: AuthMethods | null; error: string | null }
  | { status: 'authenticated'; session: Session };

function Splash() {
  return (
    <div class="splash">
      <main class="splash-box" id="content" tabIndex={-1}>
        <div class="brand">
          <svg aria-hidden="true" focusable="false">
            <use href="#i-logo" />
          </svg>
          <span>
            LiveChat
            <small>Administration</small>
          </span>
        </div>
        <h2>Console LiveChat</h2>
        <p>Vérification de la session…</p>
      </main>
    </div>
  );
}

function authenticatedScreen(route: Route) {
  switch (route.name) {
    case 'guilds':
      return <GuildsScreen />;
    case 'guild':
      return <GuildDetailScreen />;
    case 'devices':
      return <DevicesScreen />;
    case 'codes':
      return <CodesScreen />;
    case 'ingest':
      return <IngestScreen />;
    case 'roles':
      return <RolesScreen />;
    case 'media':
      return <MediaScreen />;
    case 'activity':
      return <ActivityScreen />;
    case 'settings':
      return <SettingsScreen />;
    case 'dashboard':
    default:
      return <DashboardScreen />;
  }
}

function AppShell() {
  const route = useHashRoute();
  const notify = useToast();
  const [auth, setAuth] = useState<AuthState>({ status: 'loading' });
  const [overview, setOverview] = useState<Overview | null>(null);
  const [overviewError, setOverviewError] = useState<unknown>(null);
  const [overviewLoading, setOverviewLoading] = useState(true);
  const [health, setHealth] = useState<Health | null>(null);
  const [healthUnavailable, setHealthUnavailable] = useState(false);
  const [liveAudit, setLiveAudit] = useState<AuditEntry[]>([]);
  const [liveIngestions, setLiveIngestions] = useState<Ingestion[]>([]);

  const refreshOverview = useCallback(async () => {
    try {
      const data = await getOverview();
      setOverview(data);
      setOverviewError(null);
      setOverviewLoading(false);
    } catch (error) {
      setOverviewError(error);
      setOverviewLoading(false);
    }
  }, []);

  const loadSession = useCallback(async () => {
    try {
      const session = await getSession();
      setAuth({ status: 'authenticated', session });
    } catch (error) {
      if (isApiError(error) && error.status === 401) {
        setAuth({
          status: 'anonymous',
          methods: error.payload?.methods ?? null,
          error: null,
        });
      } else {
        setAuth({ status: 'anonymous', methods: null, error: describeError(error) });
      }
    }
  }, []);

  useEffect(() => {
    void loadSession();
  }, [loadSession]);

  useEffect(() => {
    setUnauthorizedHandler(() => {
      setAuth((current) =>
        current.status === 'anonymous'
          ? current
          : { status: 'anonymous', methods: null, error: null },
      );
      setOverview(null);
      setHealth(null);
      setLiveAudit([]);
      setLiveIngestions([]);
      if (!window.location.hash.startsWith('#/login')) {
        navigate({ name: 'login', query: { error: 'unauthorized' } });
      }
      // Refresh the available methods for the login screen.
      void getSession()
        .then((session) => setAuth({ status: 'authenticated', session }))
        .catch((error: unknown) => {
          if (isApiError(error) && error.status === 401) {
            setAuth({
              status: 'anonymous',
              methods: error.payload?.methods ?? null,
              error: null,
            });
          }
        });
    });
    return () => setUnauthorizedHandler(null);
  }, []);

  // Once authenticated, load the overview and the detailed health once.
  useEffect(() => {
    if (auth.status !== 'authenticated') {
      return;
    }
    setOverviewLoading(true);
    setHealthUnavailable(false);
    void refreshOverview();
    getHealth()
      .then((value) => {
        setHealth(value);
        setHealthUnavailable(false);
      })
      .catch(() => {
        setHealth(null);
        setHealthUnavailable(true);
      });
  }, [auth.status, refreshOverview]);

  // Live events (SSE) with the documented polling fallback.
  useEffect(() => {
    if (auth.status !== 'authenticated') {
      return;
    }
    let cleanup = () => {};
    cleanup = subscribeLive(getEventsUrl(), {
      onOverview: (data) => {
        setOverview(data);
        setOverviewError(null);
        setOverviewLoading(false);
      },
      onAudit: (entry) => {
        setLiveAudit((current) => [entry, ...current.filter((item) => item.id !== entry.id)].slice(0, 200));
      },
      onIngestion: (entry) => {
        setLiveIngestions((current) =>
          [entry, ...current.filter((item) => item.id !== entry.id)].slice(0, 200),
        );
      },
      onPoll: () => {
        void getOverview()
          .then((data) => {
            setOverview(data);
            setOverviewError(null);
            setOverviewLoading(false);
          })
          .catch((error: unknown) => {
            setOverviewError(error);
          });
      },
    });
    return () => cleanup();
  }, [auth.status]);

  // Keep the URL coherent with the auth state.
  useEffect(() => {
    if (auth.status === 'anonymous' && route.name !== 'login') {
      navigate({ name: 'login', query: {} });
    }
    if (auth.status === 'authenticated' && route.name === 'login') {
      navigate({ name: 'dashboard', query: {} });
    }
  }, [auth.status, route.name]);

  const loginSecret = async (secret: string) => {
    await loginWithSecret(secret);
    const session = await getSession();
    setAuth({ status: 'authenticated', session });
    navigate({ name: 'dashboard', query: {} });
    notify('Connexion réussie.');
  };

  const signOut = async () => {
    try {
      await logout();
    } catch {
      // The local session is cleared even if the route is not available yet.
    }
    let methods: AuthMethods | null = null;
    let sessionError: string | null = null;
    try {
      // A logout that did not clear the cookie is still treated as a local
      // sign-out; reading the session back yields the available methods.
      const session = await getSession();
      methods = {
        discord: session.method === 'discord',
        secret: session.method === 'secret',
      };
    } catch (error) {
      if (isApiError(error) && error.status === 401) {
        methods = error.payload?.methods ?? null;
      } else {
        sessionError = describeError(error);
      }
    }
    setAuth({ status: 'anonymous', methods, error: sessionError });
    setOverview(null);
    setHealth(null);
    setLiveAudit([]);
    setLiveIngestions([]);
    navigate({ name: 'login', query: {} });
    notify('Déconnecté.');
  };

  const contextValue = useMemo<AppContextValue | null>(() => {
    if (auth.status !== 'authenticated') {
      return null;
    }
    return {
      session: auth.session,
      route,
      overview,
      overviewError,
      overviewLoading,
      refreshOverview,
      health,
      healthUnavailable,
      liveAudit,
      liveIngestions,
      notify: notify as Notify,
      signOut,
    };
  }, [
    auth,
    health,
    healthUnavailable,
    liveAudit,
    liveIngestions,
    notify,
    overview,
    overviewError,
    overviewLoading,
    refreshOverview,
    route,
  ]);

  if (auth.status === 'loading') {
    return <Splash />;
  }

  if (auth.status === 'anonymous') {
    return (
      <LoginScreen
        methods={auth.methods}
        errorCode={route.query.error}
        serverError={auth.error}
        onSubmitSecret={loginSecret}
      />
    );
  }

  if (!contextValue) {
    return <Splash />;
  }

  return (
    <AppContext.Provider value={contextValue}>
      {route.name === 'login' ? <Splash /> : authenticatedScreen(route)}
    </AppContext.Provider>
  );
}

export function App() {
  return (
    <ToastProvider>
      <AppShell />
    </ToastProvider>
  );
}
