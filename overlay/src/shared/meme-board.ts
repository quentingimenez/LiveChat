import type { Dictionary } from './types';

export interface MemeBoardBindingsResponse {
  bindings: Dictionary<string>;
}

export interface MemeBoardSetBindingsRequest {
  bindings: Dictionary<string>;
}

export interface MemeBoardSetBindingsResponse {
  ok: boolean;
  bindings: Dictionary<string>;
  failedAccelerators: string[];
}

export interface MemeBoardTriggerRequest {
  itemId?: string;
  trigger?: 'ui' | 'shortcut';
}

export interface MemeBoardActionResponse {
  ok: boolean;
  reason?: string;
}
