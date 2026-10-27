interface CreateConnectionStateServiceOptions {
  onStateChanged: () => void;
}

export interface ConnectionStateService {
  getConnectionStateLabel(): string;
  getConnectionReason(): string;
  setConnectionState(nextState: string, reason?: string): void;
}

const CONNECTION_STATE_LABELS: Record<string, string> = {
  disabled: 'Désactivé',
  not_paired: 'Non appairé',
  connecting: 'Connexion...',
  reconnecting: 'Reconnexion...',
  connected: 'Connecté',
  disconnected: 'Déconnecté',
  error: 'Erreur'
};

export function createConnectionStateService(options: CreateConnectionStateServiceOptions): ConnectionStateService {
  const { onStateChanged } = options;

  let connectionState = 'disconnected';
  let connectionReason = '';

  function getConnectionStateLabel(): string {
    return CONNECTION_STATE_LABELS[connectionState] ?? connectionState;
  }

  function getConnectionReason(): string {
    return connectionReason;
  }

  function setConnectionState(nextState: string, reason = ''): void {
    connectionState = nextState;
    connectionReason = typeof reason === 'string' ? reason.trim() : '';
    onStateChanged();
  }

  return {
    getConnectionStateLabel,
    getConnectionReason,
    setConnectionState
  };
}
