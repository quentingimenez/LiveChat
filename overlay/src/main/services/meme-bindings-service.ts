import { OVERLAY_SOCKET_EVENTS } from '../../shared/socket-events';
import type { Logger } from '../../shared/types';

interface SocketLike {
  connected: boolean;
  emit(event: string, payload: unknown): void;
}

interface ShortcutRegistrar {
  register(accelerator: string, callback: () => void): boolean;
  unregister(accelerator: string): void;
}

interface ApplyMemeBindingsOptions {
  strict?: boolean;
  persist?: boolean;
}

interface ApplyMemeBindingsResult {
  ok: boolean;
  appliedBindings: Record<string, string>;
  failedAccelerators: string[];
}

interface MemeTriggerResult {
  ok: boolean;
  reason?: string;
}

interface CreateMemeBindingsServiceOptions {
  logger: Logger;
  normalizeMemeBindings: (candidate: unknown) => Record<string, string>;
  isGuestModeEnabled: () => boolean;
  saveConfig: (nextValues: { memeBindings: Record<string, string> }) => unknown;
  getSocket: () => SocketLike | null;
  shortcutRegistrar: ShortcutRegistrar;
}

export interface MemeBindingsService {
  emitManualStopSignal(): MemeTriggerResult;
  emitMemeTriggerSignal(itemId: unknown, trigger?: unknown): MemeTriggerResult;
  unregisterMemeShortcuts(): void;
  applyMemeBindings(nextBindings: unknown, options?: ApplyMemeBindingsOptions): ApplyMemeBindingsResult;
}

export function createMemeBindingsService(options: CreateMemeBindingsServiceOptions): MemeBindingsService {
  const { logger, normalizeMemeBindings, isGuestModeEnabled, saveConfig, getSocket, shortcutRegistrar } = options;

  let activeMemeBindings: Record<string, string> = {};

  function emitManualStopSignal(): MemeTriggerResult {
    if (isGuestModeEnabled()) {
      return {
        ok: false,
        reason: 'guest_mode'
      };
    }

    const stopPayload = {
      jobId: 'manual-stop'
    };

    const overlaySocket = getSocket();
    if (!overlaySocket || !overlaySocket.connected) {
      return {
        ok: false,
        reason: 'socket_offline'
      };
    }

    overlaySocket.emit(OVERLAY_SOCKET_EVENTS.STOP, stopPayload);

    return {
      ok: true
    };
  }

  function emitMemeTriggerSignal(itemId: unknown, trigger: unknown = 'shortcut'): MemeTriggerResult {
    const normalizedItemId = `${itemId || ''}`.trim();
    const normalizedTrigger = trigger === 'ui' ? 'ui' : 'shortcut';

    if (isGuestModeEnabled()) {
      return {
        ok: false,
        reason: 'guest_mode'
      };
    }

    if (!normalizedItemId) {
      return {
        ok: false,
        reason: 'invalid_item_id'
      };
    }

    const overlaySocket = getSocket();
    if (!overlaySocket || !overlaySocket.connected) {
      return {
        ok: false,
        reason: 'socket_offline'
      };
    }

    overlaySocket.emit(OVERLAY_SOCKET_EVENTS.MEME_TRIGGER, {
      itemId: normalizedItemId,
      trigger: normalizedTrigger
    });

    logger.info(
      `[OVERLAY] Forwarded meme trigger to bot (itemId: ${normalizedItemId}, trigger: ${normalizedTrigger})`
    );

    return {
      ok: true
    };
  }

  function unregisterMemeShortcuts(): void {
    for (const accelerator of Object.keys(activeMemeBindings)) {
      shortcutRegistrar.unregister(accelerator);
    }

    activeMemeBindings = {};
  }

  function tryRegisterMemeBindings(bindings: unknown): {
    registered: Record<string, string>;
    failures: string[];
  } {
    const normalizedBindings = normalizeMemeBindings(bindings);
    const failures: string[] = [];
    const registered: Record<string, string> = {};

    for (const [accelerator, itemId] of Object.entries(normalizedBindings)) {
      const registeredOk = shortcutRegistrar.register(accelerator, () => {
        emitMemeTriggerSignal(itemId, 'shortcut');
      });

      if (!registeredOk) {
        failures.push(accelerator);
        continue;
      }

      registered[accelerator] = itemId;
    }

    return {
      registered,
      failures
    };
  }

  function applyMemeBindings(nextBindings: unknown, options: ApplyMemeBindingsOptions = {}): ApplyMemeBindingsResult {
    const strict = options.strict !== false;
    const persist = options.persist !== false;
    const normalizedBindings = normalizeMemeBindings(nextBindings);

    if (isGuestModeEnabled()) {
      unregisterMemeShortcuts();

      if (persist) {
        saveConfig({
          memeBindings: normalizedBindings
        });
      }

      return {
        ok: true,
        appliedBindings: {},
        failedAccelerators: []
      };
    }

    const previousBindings = { ...activeMemeBindings };

    unregisterMemeShortcuts();
    const registrationResult = tryRegisterMemeBindings(normalizedBindings);

    if (strict && registrationResult.failures.length > 0) {
      unregisterMemeShortcuts();
      const rollbackResult = tryRegisterMemeBindings(previousBindings);
      activeMemeBindings = rollbackResult.registered;

      return {
        ok: false,
        appliedBindings: { ...activeMemeBindings },
        failedAccelerators: registrationResult.failures
      };
    }

    activeMemeBindings = registrationResult.registered;

    if (persist) {
      saveConfig({
        memeBindings: activeMemeBindings
      });
    }

    return {
      ok: registrationResult.failures.length === 0,
      appliedBindings: { ...activeMemeBindings },
      failedAccelerators: registrationResult.failures
    };
  }

  return {
    emitManualStopSignal,
    emitMemeTriggerSignal,
    unregisterMemeShortcuts,
    applyMemeBindings
  };
}
