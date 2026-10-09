import type { ControlInputEvent, MediaProfile, RemoteRoutePreference } from '../adapters/tauri/types';
import { SERVER_API_URL } from './serverConfig';
import {
  byteArray, canonicalWanRequest, createBrowserSigningIdentity, normalizedH264Profile,
  randomBytes, safeInteger, wanRequestCommitment, type BrowserSigningIdentity, type WireObject,
} from './browserRemoteProtocol';

export type BrowserRemoteState = {
  phase: 'waiting_consent' | 'negotiating' | 'streaming' | 'denied' | 'failed' | 'closed';
  grantedScopes: string[];
  frameWidth?: number;
  frameHeight?: number;
  error?: string;
  route?: 'direct' | 'relay' | null;
};
export type BrowserRemoteObserver = {
  onState: (state: BrowserRemoteState) => void;
  onVideoStream: (stream: MediaStream) => void;
};
export type BrowserRemoteHandle = {
  sendInput: (input: ControlInputEvent) => Promise<void>;
  sendPointerAction: (position: { x: number; y: number }, input: ControlInputEvent) => Promise<void>;
  releaseInputs: (reason?: string) => Promise<void>;
  markVideoReady: (width: number, height: number) => void;
  close: (reason?: string) => Promise<void>;
};
export type BrowserRemoteCreateOptions = {
  sessionId?: string;
  targetDeviceName?: string;
  targetOs?: string;
  targetIp?: string;
  requestedProfile?: MediaProfile;
  routePreference?: RemoteRoutePreference;
};

export type BrowserSessionBootstrap = {
  controller_device_id: string;
  controller_key_id: string;
  expires_at_ms: number;
  session: WireObject;
  credential: { token: string; expires_at_ms: number; device_id: string; device_key_id: string; role: string };
  signaling_url: string;
  signaling_server_device_id: string;
  signaling_server_key_id: string;
  target_key_id: string;
  relay_directory_key_id: string;
  relay_directory_public_key: number[];
};
export type BrowserRemoteContext = {
  identity: BrowserSigningIdentity;
  bootstrap: BrowserSessionBootstrap;
  getBootstrap: () => Promise<BrowserSessionBootstrap>;
  getRelayAccess: () => Promise<WireObject>;
  closeBackend: () => Promise<void>;
};
type Entry = { context: BrowserRemoteContext; handle?: BrowserRemoteHandle; closed: boolean };
const ownedSessions = new Map<string, Entry>();
const pendingCreates = new Set<string>();
let creationEpoch = 0;
const CLOSED_STATES = new Set(['rejected', 'expired', 'closed', 'revoked']);

async function browserApi<T>(userToken: string, path: string, body?: unknown): Promise<T> {
  const cancel = new AbortController();
  const timer = setTimeout(() => cancel.abort(), 10_000);
  try {
    const response = await fetch(`${SERVER_API_URL}${path}`, {
      method: body === undefined ? 'GET' : 'POST',
      headers: { Authorization: `Bearer ${userToken}`, 'Content-Type': 'application/json' },
      credentials: 'omit',
      signal: cancel.signal,
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    if (!response.ok) {
      let message = `网页远程请求失败（HTTP ${response.status}）`;
      const data = await response.json().catch(() => null);
      if (typeof data?.detail?.message === 'string') message = data.detail.message;
      else if (typeof data?.detail === 'string') message = data.detail;
      throw new Error(message);
    }
    return await response.json() as T;
  } catch (error) {
    if (cancel.signal.aborted) throw new Error('网页远程请求超时');
    throw error;
  } finally { clearTimeout(timer); }
}

function requirePin(value: unknown): void {
  if (typeof value !== 'string' || !/^[a-f0-9]{64}$/.test(value)) throw new Error('网页远控缺少可信身份配置');
}
export async function validateBrowserBootstrap(
  raw: BrowserSessionBootstrap,
  identity: BrowserSigningIdentity,
  expected: { sessionId: string; targetDeviceId: string; requestBody?: WireObject; controllerId?: string },
): Promise<BrowserSessionBootstrap> {
  if (!raw || !raw.session || !raw.credential || !/^browser_[a-f0-9]{32}$/.test(raw.controller_device_id)
    || raw.controller_key_id !== identity.keyId || raw.session.session_id !== expected.sessionId
    || raw.session.request?.session_id !== expected.sessionId || raw.session.request?.target_device_id !== expected.targetDeviceId
    || raw.session.request?.controller_device_id !== raw.controller_device_id
    || (expected.controllerId !== undefined && raw.controller_device_id !== expected.controllerId)) throw new Error('网页会话身份绑定不匹配');
  safeInteger(raw.expires_at_ms, Date.now() + 1);
  if (raw.credential.device_id !== raw.controller_device_id || raw.credential.device_key_id !== identity.keyId
    || raw.credential.role !== 'Controller' || typeof raw.credential.token !== 'string' || !raw.credential.token
    || raw.credential.token.length > 4096) throw new Error('网页信令凭据无效');
  safeInteger(raw.credential.expires_at_ms, Date.now() + 1, raw.expires_at_ms);
  for (const pin of [raw.signaling_server_key_id, raw.target_key_id, raw.relay_directory_key_id]) requirePin(pin);
  byteArray(raw.relay_directory_public_key, 32);
  if (!raw.signaling_server_device_id || typeof raw.signaling_url !== 'string') throw new Error('网页远控缺少信令配置');
  const signalUrl = new URL(raw.signaling_url);
  const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(signalUrl.hostname);
  if (signalUrl.username || signalUrl.password || signalUrl.search || signalUrl.hash
    || (signalUrl.protocol !== 'wss:' && !(signalUrl.protocol === 'ws:' && loopback && location.protocol === 'http:'))) throw new Error('网页信令必须使用可信 WSS 连接');
  const request = raw.session.request;
  if (expected.requestBody) {
    const normalized: WireObject = { ...expected.requestBody, controller_device_id: raw.controller_device_id };
    delete normalized.controller_public_key;
    if (canonicalWanRequest(request) !== canonicalWanRequest(normalized)) throw new Error('网页远程请求内容绑定不匹配');
  }
  if (await wanRequestCommitment(request) !== raw.session.request_commitment) throw new Error('网页远程请求承诺不匹配');
  if (CLOSED_STATES.has(raw.session.status)) throw new Error(`网页会话已终止：${raw.session.status}`);
  return raw;
}

export async function createBrowserRemoteSession(
  targetDeviceId: string,
  options?: BrowserRemoteCreateOptions,
): Promise<{ sessionId: string }> {
  const token = localStorage.getItem('rdesk_access_token')?.trim();
  if (!token) throw new Error('请先登录后连接远端设备');
  if (!globalThis.isSecureContext || !globalThis.crypto?.subtle) throw new Error('网页远控需要 HTTPS 安全连接（开发环境可使用 localhost）');
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(targetDeviceId)) throw new Error('远端设备标识无效');
  if (ownedSessions.size + pendingCreates.size >= 4) throw new Error('请先关闭已有网页会话');
  const sessionId = options?.sessionId ?? crypto.randomUUID();
  if (ownedSessions.has(sessionId) || pendingCreates.has(sessionId)) throw new Error('网页会话已存在');
  const epoch = creationEpoch;
  const stillCurrent = () => epoch === creationEpoch && localStorage.getItem('rdesk_access_token')?.trim() === token;
  pendingCreates.add(sessionId);
  const path = `/browser-sessions/${encodeURIComponent(sessionId)}`;
  let created = false;
  try {
    const identity = await createBrowserSigningIdentity();
    if (!stillCurrent()) throw new Error('网页会话创建已取消');
    const body = {
      session_id: sessionId, idempotency_key: randomBytes(16), target_device_id: targetDeviceId,
      access_mode: 'attended', requested_scopes: ['input.keyboard', 'input.pointer', 'screen.view'],
      requested_profile: normalizedH264Profile(options?.requestedProfile),
      route_policy: options?.routePreference === 'wan_relay' ? 'relay_only' : 'direct_first',
      controller_public_key: identity.publicKey,
    };
    const raw = await browserApi<BrowserSessionBootstrap>(token, '/browser-sessions', body);
    created = true;
    if (!stillCurrent()) throw new Error('网页会话创建已取消');
    const bootstrap = await validateBrowserBootstrap(raw, identity, { sessionId, targetDeviceId, requestBody: body });
    if (!stillCurrent()) throw new Error('网页会话创建已取消');
    const context: BrowserRemoteContext = {
      identity, bootstrap,
      getBootstrap: async () => validateBrowserBootstrap(await browserApi<BrowserSessionBootstrap>(token, path), identity, { sessionId, targetDeviceId, controllerId: bootstrap.controller_device_id }),
      getRelayAccess: () => browserApi(token, `${path}/relay-access`, { generation: 0 }),
      closeBackend: async () => { await browserApi(token, `${path}/close`, {}); },
    };
    ownedSessions.set(sessionId, { context, closed: false });
    return { sessionId };
  } catch (error) {
    if (created) await browserApi(token, `${path}/close`, {}).catch(() => undefined);
    throw error;
  } finally { pendingCreates.delete(sessionId); }
}

export function attachBrowserRemoteSession(sessionId: string, observer: BrowserRemoteObserver): BrowserRemoteHandle {
  const entry = ownedSessions.get(sessionId);
  if (!entry || entry.closed) {
    observer.onState({ phase: 'failed', grantedScopes: [], error: '网页会话身份已失效，请返回设备列表重新连接' });
    return {
      sendInput: async () => { throw new Error('网页会话身份已失效'); },
      sendPointerAction: async () => { throw new Error('网页会话身份已失效'); },
      releaseInputs: async () => undefined,
      markVideoReady: () => undefined,
      close: async () => undefined,
    };
  }
  if (entry.handle) throw new Error('网页会话已在另一页面使用');
  let peer: BrowserRemoteHandle | undefined;
  let closed = false;
  let startupError: string | undefined;
  let closePromise: Promise<void> | undefined;
  const handle: BrowserRemoteHandle = {
    sendInput: input => peer ? peer.sendInput(input) : Promise.reject(new Error('远端控制通道尚未就绪')),
    sendPointerAction: (position, input) => peer ? peer.sendPointerAction(position, input) : Promise.reject(new Error('远端控制通道尚未就绪')),
    releaseInputs: reason => peer ? peer.releaseInputs(reason) : Promise.resolve(),
    markVideoReady: (width, height) => peer?.markVideoReady(width, height),
    close: reason => {
      if (closePromise) return closePromise;
      closed = true; entry.closed = true; ownedSessions.delete(sessionId);
      closePromise = Promise.resolve().then(async () => {
        if (peer) await peer.close(reason);
        else {
          observer.onState(startupError ? { phase: 'failed', grantedScopes: [], error: startupError } : { phase: 'closed', grantedScopes: [] });
          await entry.context.closeBackend();
        }
      });
      return closePromise;
    },
  };
  entry.handle = handle;
  observer.onState({ phase: 'waiting_consent', grantedScopes: [] });
  void import('./browserRemotePeer').then(({ BrowserRemotePeer }) => {
    if (closed) return;
    const startedPeer = new BrowserRemotePeer(entry.context, observer);
    peer = startedPeer;
    return startedPeer.start();
  }).catch(error => {
    if (closed) return;
    startupError = error instanceof Error ? error.message : '网页远程连接失败';
    observer.onState({ phase: 'failed', grantedScopes: [], error: startupError });
    void handle.close('connection_failed').catch(() => undefined);
  });
  return handle;
}

export async function closeAllBrowserRemoteSessions(reason = 'logout'): Promise<void> {
  creationEpoch += 1;
  const entries = [...ownedSessions.entries()];
  await Promise.all(entries.map(async ([id, entry]) => {
    if (entry.handle) await entry.handle.close(reason);
    else { entry.closed = true; ownedSessions.delete(id); await entry.context.closeBackend(); }
  }));
}
