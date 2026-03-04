import { registerOverlayIpcHandlers } from './register-overlay-ipc-handlers';
import { registerPairingIpcHandler } from './register-pairing-ipc-handler';
import type { RegisterIpcHandlersOptions } from './ipc-handler-types';

export function registerIpcHandlers(options: RegisterIpcHandlersOptions): void {
  registerOverlayIpcHandlers(options);
  registerPairingIpcHandler(options);
}
