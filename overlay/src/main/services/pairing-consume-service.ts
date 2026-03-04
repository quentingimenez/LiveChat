import { isRecord, toTrimmedString } from '../../shared/guards';
import type { OverlayRuntimeConfig } from './config-service';

export interface PairingHttpResponse {
  statusCode: number;
  body: string;
}

interface CreatePairingConsumeServiceOptions {
  saveConfig: (nextValues: Partial<OverlayRuntimeConfig>) => OverlayRuntimeConfig;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  normalizeServerUrl: (serverUrl: string) => string;
  httpRequestJson: (
    url: string,
    payload: { code: string; deviceName?: string },
    options?: { rejectUnauthorized?: boolean; timeoutMs?: number }
  ) => Promise<PairingHttpResponse>;
  isLikelyTlsError: (error: unknown) => boolean;
  formatNetworkError: (error: unknown, endpoint: string) => string;
  setGuestMode: (checked: boolean) => OverlayRuntimeConfig;
  sendOverlaySettingsToRenderer: () => void;
  createOverlayWindow: () => void;
  connectOverlaySocket: () => void;
  closePairingWindow: () => void;
  updateTrayMenu: () => void;
}

export interface PairingConsumeService {
  consumePairing(payload: unknown): Promise<{ ok: true }>;
}

export function parsePairingConsumeResponse(payloadText: string): Record<string, unknown> {
  let parsed: unknown;

  try {
    parsed = JSON.parse(payloadText);
  } catch {
    throw new Error('invalid_pairing_response');
  }

  if (!isRecord(parsed)) {
    throw new Error('invalid_pairing_response');
  }

  return parsed;
}

export function createPairingConsumeService(options: CreatePairingConsumeServiceOptions): PairingConsumeService {
  const {
    saveConfig,
    hasPairingConfig,
    normalizeServerUrl,
    httpRequestJson,
    isLikelyTlsError,
    formatNetworkError,
    setGuestMode,
    sendOverlaySettingsToRenderer,
    createOverlayWindow,
    connectOverlaySocket,
    closePairingWindow,
    updateTrayMenu
  } = options;

  async function consumePairing(payload: unknown): Promise<{ ok: true }> {
    const payloadRecord = isRecord(payload) ? payload : {};
    const serverUrl = normalizeServerUrl(`${payloadRecord.serverUrl || ''}`);
    const code = toTrimmedString(payloadRecord.code).toUpperCase();
    const requestedDeviceName = toTrimmedString(payloadRecord.deviceName);

    if (!serverUrl || !code) {
      throw new Error('missing_required_fields');
    }

    const endpoint = `${serverUrl}/overlay/pair/consume`;
    const requestPayload = {
      code,
      deviceName: requestedDeviceName || undefined
    };
    let pairingResponse: PairingHttpResponse;

    try {
      pairingResponse = await httpRequestJson(endpoint, requestPayload, {
        rejectUnauthorized: true
      });
    } catch (error) {
      if (endpoint.startsWith('https://') && isLikelyTlsError(error)) {
        pairingResponse = await httpRequestJson(endpoint, requestPayload, {
          rejectUnauthorized: false
        });
      } else {
        throw new Error(formatNetworkError(error, endpoint));
      }
    }

    const payloadText = pairingResponse.body || '';
    if (pairingResponse.statusCode < 200 || pairingResponse.statusCode >= 300) {
      throw new Error(payloadText || `pairing_failed_${pairingResponse.statusCode}`);
    }

    const parsed = parsePairingConsumeResponse(payloadText);
    const resolvedDeviceName = toTrimmedString(parsed.deviceName || requestedDeviceName) || null;
    const resolvedAuthorName = toTrimmedString(parsed.authorName) || null;
    const sessionMode = toTrimmedString(parsed.sessionMode || 'normal').toLowerCase();
    const isInviteReadOnlySession = sessionMode === 'invite_read_only';

    saveConfig({
      serverUrl: normalizeServerUrl(`${parsed.apiBaseUrl || serverUrl}`),
      clientToken: toTrimmedString(parsed.clientToken) || null,
      clientId: toTrimmedString(parsed.clientId) || null,
      guildId: toTrimmedString(parsed.guildId) || null,
      authorName: resolvedAuthorName,
      deviceName: resolvedDeviceName,
      guestMode: isInviteReadOnlySession
    });

    const cfg = setGuestMode(isInviteReadOnlySession);
    sendOverlaySettingsToRenderer();

    if (cfg.enabled && hasPairingConfig(cfg)) {
      createOverlayWindow();
      connectOverlaySocket();
    }

    closePairingWindow();
    updateTrayMenu();

    return {
      ok: true
    };
  }

  return {
    consumePairing
  };
}
