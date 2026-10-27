import { isRecord } from '../../shared/guards';
import type { OverlayRuntimeConfig } from './config-service';

interface CreateMemeBoardIpcServiceOptions {
  loadConfig: () => OverlayRuntimeConfig;
  normalizeMemeBindings: (candidate: unknown) => Record<string, string>;
  applyMemeBindings: (
    nextBindings: Record<string, string>,
    options: { strict?: boolean; persist?: boolean }
  ) => {
    ok: boolean;
    appliedBindings: Record<string, string>;
    failedAccelerators: string[];
  };
  emitMemeTriggerSignal: (itemId: unknown, trigger: unknown) => { ok: boolean; reason?: string };
  emitManualStopSignal: () => { ok: boolean; reason?: string };
}

export interface MemeBoardIpcService {
  getBindings(): { bindings: Record<string, string> };
  setBindings(payload: unknown): { ok: boolean; bindings: Record<string, string>; failedAccelerators: string[] };
  trigger(payload: unknown): { ok: boolean; reason?: string };
  stop(): { ok: boolean; reason?: string };
}

export function createMemeBoardIpcService(options: CreateMemeBoardIpcServiceOptions): MemeBoardIpcService {
  const { loadConfig, normalizeMemeBindings, applyMemeBindings, emitMemeTriggerSignal, emitManualStopSignal } = options;

  function getBindings(): { bindings: Record<string, string> } {
    const cfg = loadConfig();
    return {
      bindings: normalizeMemeBindings(cfg.memeBindings)
    };
  }

  function setBindings(payload: unknown): {
    ok: boolean;
    bindings: Record<string, string>;
    failedAccelerators: string[];
  } {
    const payloadRecord = isRecord(payload) ? payload : {};
    const nextBindings = normalizeMemeBindings(payloadRecord.bindings);
    const applyResult = applyMemeBindings(nextBindings, {
      strict: true,
      persist: true
    });

    return {
      ok: applyResult.ok,
      bindings: applyResult.appliedBindings,
      failedAccelerators: applyResult.failedAccelerators
    };
  }

  function trigger(payload: unknown): { ok: boolean; reason?: string } {
    const payloadRecord = isRecord(payload) ? payload : {};
    return emitMemeTriggerSignal(payloadRecord.itemId, payloadRecord.trigger);
  }

  function stop(): { ok: boolean; reason?: string } {
    return emitManualStopSignal();
  }

  return {
    getBindings,
    setBindings,
    trigger,
    stop
  };
}
