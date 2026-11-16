import { createContext } from 'preact';
import { useContext } from 'preact/hooks';
import type { AuditEntry, Health, Ingestion, Overview, Session } from './api/types';
import type { Route } from './router';

export type ToastKind = 'ok' | 'error';
export type Notify = (message: string, kind?: ToastKind) => void;

export interface AppContextValue {
  session: Session;
  route: Route;
  overview: Overview | null;
  overviewError: unknown;
  overviewLoading: boolean;
  refreshOverview: () => Promise<void>;
  health: Health | null;
  healthUnavailable: boolean;
  liveAudit: AuditEntry[];
  liveIngestions: Ingestion[];
  notify: Notify;
  signOut: () => Promise<void>;
}

export const AppContext = createContext<AppContextValue | null>(null);

export function useApp(): AppContextValue {
  const value = useContext(AppContext);
  if (!value) {
    throw new Error('useApp must be used inside <AppContext.Provider>');
  }
  return value;
}
