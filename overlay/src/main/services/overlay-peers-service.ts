import type { OverlayRuntimeConfig } from './config-service';

export interface OverlayPeer {
  clientId: string;
  label: string;
}

interface BuildTrayTooltipOptions {
  config: OverlayRuntimeConfig;
  connectionStateLabel: string;
}

interface CreateOverlayPeersServiceOptions {
  maxOtherActiveOverlaysInTooltip?: number;
}

export interface OverlayPeersService {
  setConnectedOverlayPeers(peers: unknown): void;
  getConnectedOverlayPeers(): OverlayPeer[];
  buildTrayTooltip(options: BuildTrayTooltipOptions): string;
}

const DEFAULT_OTHER_ACTIVE_OVERLAYS_LIMIT = 2;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function toTrimmedString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : '';
}

function normalizeConnectedPeers(peers: unknown): OverlayPeer[] {
  if (!Array.isArray(peers)) {
    return [];
  }

  const peersByClientId = new Map<string, OverlayPeer>();

  for (const rawPeer of peers) {
    if (!isRecord(rawPeer)) {
      continue;
    }

    const clientId = toTrimmedString(rawPeer.clientId);
    const label = toTrimmedString(rawPeer.label) || 'unknown-device';

    if (!clientId || peersByClientId.has(clientId)) {
      continue;
    }

    peersByClientId.set(clientId, {
      clientId,
      label
    });
  }

  return Array.from(peersByClientId.values()).sort((leftPeer, rightPeer) => {
    const labelComparison = leftPeer.label.localeCompare(rightPeer.label, undefined, { sensitivity: 'base' });
    if (labelComparison !== 0) {
      return labelComparison;
    }

    return leftPeer.clientId.localeCompare(rightPeer.clientId, undefined, { sensitivity: 'base' });
  });
}

function getSelfOverlayPeer(peers: OverlayPeer[], config: OverlayRuntimeConfig): OverlayPeer | null {
  const selfClientId = toTrimmedString(config.clientId);
  if (!selfClientId) {
    return null;
  }

  return peers.find((peer) => peer.clientId === selfClientId) ?? null;
}

function getOtherActiveOverlays(peers: OverlayPeer[], config: OverlayRuntimeConfig): OverlayPeer[] {
  const selfClientId = toTrimmedString(config.clientId);
  return peers.filter((peer) => peer.clientId !== selfClientId);
}

function truncateTooltipSegment(value: unknown, maxLength = 18): string {
  const normalized = toTrimmedString(value);
  if (!normalized) {
    return '';
  }

  if (normalized.length <= maxLength) {
    return normalized;
  }

  return `${normalized.slice(0, Math.max(1, maxLength - 1))}…`;
}

function stripOverlayAutoPrefix(value: unknown): string {
  const normalized = toTrimmedString(value);
  if (!normalized) {
    return '';
  }

  const stripped = normalized.replace(/^overlay[-_\s]+/i, '').trim();
  return stripped || normalized;
}

function normalizeOtherActiveOverlaysLimit(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return DEFAULT_OTHER_ACTIVE_OVERLAYS_LIMIT;
  }

  return Math.max(0, Math.floor(value));
}

export function createOverlayPeersService(options: CreateOverlayPeersServiceOptions = {}): OverlayPeersService {
  const otherActiveOverlaysLimit = normalizeOtherActiveOverlaysLimit(options.maxOtherActiveOverlaysInTooltip);
  let connectedOverlayPeers: OverlayPeer[] = [];

  return {
    setConnectedOverlayPeers(peers: unknown): void {
      connectedOverlayPeers = normalizeConnectedPeers(peers);
    },

    getConnectedOverlayPeers(): OverlayPeer[] {
      return [...connectedOverlayPeers];
    },

    buildTrayTooltip({ config, connectionStateLabel }: BuildTrayTooltipOptions): string {
      const selfPeer = getSelfOverlayPeer(connectedOverlayPeers, config);
      const suffixLabel = toTrimmedString(config.authorName) || selfPeer?.label || '';
      const suffix = suffixLabel ? ` (${suffixLabel})` : '';
      const status = `Overlay ${connectionStateLabel}${suffix}`;
      const otherActiveOverlays = getOtherActiveOverlays(connectedOverlayPeers, config);
      const visibleOverlays = otherActiveOverlays.slice(0, otherActiveOverlaysLimit);
      const visibleNames = visibleOverlays
        .map((peer) => truncateTooltipSegment(stripOverlayAutoPrefix(peer.label)))
        .filter((label) => label !== '');
      const extraCount = Math.max(0, otherActiveOverlays.length - visibleOverlays.length);
      const namesPart = visibleNames.length > 0 ? `: ${visibleNames.join(', ')}` : ': aucun';
      const extraPart = extraCount > 0 ? ` +${extraCount}` : '';

      return `${status} | Autres: ${otherActiveOverlays.length}${namesPart}${extraPart}`;
    }
  };
}
