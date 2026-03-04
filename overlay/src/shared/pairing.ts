export interface PairingConsumeRequest {
  serverUrl: string;
  code: string;
  deviceName?: string;
}

export interface PairingConsumeResponse {
  ok: boolean;
}
