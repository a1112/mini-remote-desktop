import type { AdapterResult } from '../tauri/types';

export type ServiceBridgeIpcRequest = {
  type: string;
  [key: string]: unknown;
};

export type ServiceBridgeIpcResponse = {
  type: string;
  code?: string;
  message?: string;
  [key: string]: unknown;
};

export interface ServiceBridgeHealth {
  status: string;
  service: string;
  bridge_enabled: boolean;
  bind: string;
}

const DEFAULT_ENDPOINT = 'http://127.0.0.1:9532';
const ENDPOINT_STORAGE_KEY = 'mrd.serviceBridge.endpoint';
const TOKEN_STORAGE_KEY = 'mrd.serviceBridge.token';
const ENV_ENDPOINT = envString('VITE_MRD_SERVICE_BRIDGE_ENDPOINT');
const ENV_TOKEN = envString('VITE_MRD_SERVICE_BRIDGE_TOKEN');

let endpointOverrideForTest: string | null = null;

export function serviceBridgeEndpoint(): string {
  if (endpointOverrideForTest) return endpointOverrideForTest;
  if (typeof window === 'undefined') return DEFAULT_ENDPOINT;
  const configured = window.localStorage?.getItem(ENDPOINT_STORAGE_KEY)?.trim();
  return configured || ENV_ENDPOINT || DEFAULT_ENDPOINT;
}

export function hasConfiguredServiceBridgeEndpoint(): boolean {
  if (endpointOverrideForTest) return true;
  if (typeof window !== 'undefined') {
    const configured = window.localStorage?.getItem(ENDPOINT_STORAGE_KEY)?.trim();
    if (configured) return true;
  }
  return Boolean(ENV_ENDPOINT);
}

export function serviceBridgeWebSocketUrl(path: string): string {
  const url = new URL(serviceBridgeEndpoint());
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  url.pathname = path;
  url.search = '';
  if (typeof window !== 'undefined') {
    const token = serviceBridgeToken();
    if (token) url.searchParams.set('token', token);
  }
  return url.toString();
}

export function setServiceBridgeEndpointForTest(endpoint: string): void {
  endpointOverrideForTest = endpoint;
}

export function resetServiceBridgeConfigForTest(): void {
  endpointOverrideForTest = null;
  if (typeof window !== 'undefined') {
    window.localStorage?.removeItem(ENDPOINT_STORAGE_KEY);
    window.localStorage?.removeItem(TOKEN_STORAGE_KEY);
  }
}

export async function serviceBridgeHealth(): Promise<AdapterResult<ServiceBridgeHealth>> {
  try {
    const response = await fetch(`${serviceBridgeEndpoint()}/health`, {
      method: 'GET',
    });
    if (!response.ok) {
      return adapterError(`mrd-service web bridge returned HTTP ${response.status}`);
    }
    return { ok: true, value: (await response.json()) as ServiceBridgeHealth };
  } catch (error) {
    return adapterError(errorMessage(error));
  }
}

export async function invokeServiceBridgeIpc<T = ServiceBridgeIpcResponse>(
  request: ServiceBridgeIpcRequest,
  unwrap?: (response: ServiceBridgeIpcResponse) => T
): Promise<AdapterResult<T>> {
  try {
    const response = await fetch(`${serviceBridgeEndpoint()}/ipc`, {
      method: 'POST',
      headers: requestHeaders(),
      body: JSON.stringify({ request }),
    });
    if (!response.ok) {
      return adapterError(`mrd-service web bridge returned HTTP ${response.status}`);
    }

    const envelope = (await response.json()) as {
      response?: ServiceBridgeIpcResponse;
    };
    const ipcResponse = envelope.response;
    if (!ipcResponse) {
      return adapterError('mrd-service web bridge returned an empty IPC envelope');
    }
    if (ipcResponse.type === 'Error') {
      const code = ipcResponse.code ? `${ipcResponse.code}: ` : '';
      return adapterError(`${code}${ipcResponse.message ?? 'web bridge IPC failed'}`);
    }

    return {
      ok: true,
      value: unwrap ? unwrap(ipcResponse) : (ipcResponse as T),
    };
  } catch (error) {
    return adapterError(errorMessage(error));
  }
}

export async function postServiceBridgeJson<T>(
  path: string,
  body: unknown
): Promise<AdapterResult<T>> {
  try {
    const response = await fetch(`${serviceBridgeEndpoint()}${path}`, {
      method: 'POST',
      headers: requestHeaders(),
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      let detail = `mrd-service web bridge returned HTTP ${response.status}`;
      try {
        const payload = (await response.json()) as { message?: string };
        if (payload.message) detail = payload.message;
      } catch {
        // Keep the HTTP status when the bridge did not return JSON.
      }
      return adapterError(detail);
    }
    return { ok: true, value: (await response.json()) as T };
  } catch (error) {
    return adapterError(errorMessage(error));
  }
}

function requestHeaders(): Record<string, string> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };
  const token = serviceBridgeToken();
  if (token) headers['X-MRD-Bridge-Token'] = token;
  return headers;
}

function serviceBridgeToken(): string {
  if (typeof window === 'undefined') return ENV_TOKEN;
  return window.localStorage?.getItem(TOKEN_STORAGE_KEY)?.trim() || ENV_TOKEN;
}

function adapterError<T = never>(message: string): AdapterResult<T> {
  return { ok: false, error: { message } };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function envString(key: string): string {
  const value = ((import.meta as unknown as { env?: Record<string, string | undefined> }).env?.[
    key
  ] ?? '').trim();
  return value;
}
