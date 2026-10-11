import type { ControlInputEvent } from '../adapters/tauri/types';

export type WireObject = Record<string, any>;
export type SignedSignal = { payload: WireObject; signer_public_key: number[]; signature: number[] };
export type BrowserSigningIdentity = { publicKey: number[]; keyId: string; privateKey: CryptoKey };
export type SignalType = keyof typeof CONTEXTS;
const encoder = new TextEncoder();
const CONTEXTS = {
  register: 'MRD_SIGNAL_REGISTER_V2',
  registered: 'MRD_SIGNAL_REGISTERED_V2',
  presence_heartbeat: 'MRD_SIGNAL_PRESENCE_V2',
  session_intent_v3: 'MRD_SIGNAL_SESSION_INTENT_V3',
  session_grant_v3: 'MRD_SIGNAL_SESSION_GRANT_V3',
  webrtc_offer_v3: 'MRD_SIGNAL_WEBRTC_OFFER_V3',
  webrtc_answer_v3: 'MRD_SIGNAL_WEBRTC_ANSWER_V3',
  webrtc_candidate_v3: 'MRD_SIGNAL_WEBRTC_CANDIDATE_V3',
  session_deny: 'MRD_SIGNAL_SESSION_DENY_V2',
  session_close: 'MRD_SIGNAL_SESSION_CLOSE_V2',
} as const;
const CLAIM_FIELDS = ['issuer_device_id', 'issuer_key_id', 'intended_peer_device_id', 'issued_at_ms', 'expires_at_ms', 'counter', 'nonce'];
const PROFILE_FIELDS = ['width', 'height', 'fps', 'bitrate_mbps', 'codec', 'codec_profile', 'bit_depth', 'chroma_subsampling', 'pixel_format', 'hdr_enabled', 'color_mode', 'color_pipeline'];
const REQUEST_FIELDS = ['session_id', 'idempotency_key', 'controller_device_id', 'target_device_id', 'access_mode', 'requested_scopes', 'requested_profile', 'route_policy'];
const DESCRIPTION_FIELDS = ['claims', 'session_id', 'controller_device_id', 'target_device_id', 'grant_commitment', 'sdp', 'candidate_fingerprints'];
const PAYLOAD_FIELDS: Record<SignalType, string[]> = {
  register: ['claims', 'role', 'device_name', 'backend_device_token', 'challenge_id', 'challenge_nonce'],
  registered: ['claims', 'registered_device_id', 'connection_id', 'heartbeat_interval_ms'],
  presence_heartbeat: ['claims', 'connection_id', 'observed_at_ms'],
  session_intent_v3: ['claims', 'request', 'request_commitment'],
  session_grant_v3: ['claims', 'session_id', 'controller_device_id', 'target_device_id', 'intent_commitment', 'approved_scopes', 'approved_profile', 'backend_policy_revision', 'policy_expires_at_ms', 'relay_generation', 'relay_directory_id', 'primary_relay_node_id', 'route_policy'],
  webrtc_offer_v3: DESCRIPTION_FIELDS,
  webrtc_answer_v3: DESCRIPTION_FIELDS,
  webrtc_candidate_v3: ['claims', 'session_id', 'controller_device_id', 'target_device_id', 'grant_commitment', 'description_role', 'candidate', 'sdp_mid', 'sdp_mline_index', 'username_fragment', 'candidate_fingerprint'],
  session_deny: ['claims', 'session_id', 'controller_device_id', 'reason'],
  session_close: ['claims', 'session_id', 'reason'],
};

function malformed(): never { throw new Error('远程协议数据无效'); }
/** Reject ambiguous JSON before any authenticated schema is interpreted. */
export function parseBoundedJson(text: string, maxBytes = 512 * 1024): unknown {
  if (typeof text !== 'string' || encoder.encode(text).length > maxBytes) return malformed();
  const parsed: unknown = JSON.parse(text);
  let offset = 0;
  const skipWhitespace = () => { while (offset < text.length && /\s/.test(text[offset]!)) offset++; };
  const stringValue = (): string => {
    const start = offset++;
    while (offset < text.length) {
      const char = text[offset++]!;
      if (char === '\\') offset++;
      else if (char === '"') return JSON.parse(text.slice(start, offset)) as string;
    }
    return malformed();
  };
  const value = (depth: number): void => {
    if (depth > 64) return malformed();
    skipWhitespace();
    const char = text[offset];
    if (char === '{') {
      offset++; skipWhitespace();
      const keys = new Set<string>();
      if (text[offset] === '}') { offset++; return; }
      while (offset < text.length) {
        if (text[offset] !== '"') return malformed();
        const key = stringValue();
        if (keys.has(key)) return malformed();
        keys.add(key); skipWhitespace();
        if (text[offset++] !== ':') return malformed();
        value(depth + 1); skipWhitespace();
        if (text[offset] === '}') { offset++; return; }
        if (text[offset++] !== ',') return malformed();
        skipWhitespace();
      }
      return malformed();
    }
    if (char === '[') {
      offset++; skipWhitespace();
      if (text[offset] === ']') { offset++; return; }
      while (offset < text.length) {
        value(depth + 1); skipWhitespace();
        if (text[offset] === ']') { offset++; return; }
        if (text[offset++] !== ',') return malformed();
      }
      return malformed();
    }
    if (char === '"') { stringValue(); return; }
    const start = offset;
    while (offset < text.length && !/[\s,}\]]/.test(text[offset]!)) offset++;
    if (start === offset) return malformed();
  };
  value(0); skipWhitespace();
  if (offset !== text.length) return malformed();
  return parsed;
}
export function strictObject(value: unknown, fields: readonly string[]): WireObject {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return malformed();
  const input = value as WireObject;
  if (Object.keys(input).some(key => !fields.includes(key)) || fields.some(key => !Object.prototype.hasOwnProperty.call(input, key))) return malformed();
  return Object.fromEntries(fields.map(key => [key, input[key]]));
}
export function safeInteger(value: unknown, minimum = 0, maximum = Number.MAX_SAFE_INTEGER): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < minimum || value > maximum) return malformed();
  return value;
}
export function boundedText(value: unknown, maximum = 256): string {
  if (typeof value !== 'string' || !value.trim() || value.includes('\0') || encoder.encode(value).length > maximum) return malformed();
  return value;
}
export function byteArray(value: unknown, length: number): number[] {
  if (!Array.isArray(value) || value.length !== length) return malformed();
  return value.map(byte => safeInteger(byte, 0, 255));
}
function hexPin(value: unknown): string {
  if (typeof value !== 'string' || !/^[0-9a-f]{64}$/.test(value)) return malformed();
  return value;
}
function sortedStrings(value: unknown, maximum = 32): string[] {
  if (!Array.isArray(value) || value.length === 0 || value.length > maximum) return malformed();
  const list = value.map(item => boundedText(item, 128));
  if (list.some((item, index) => index > 0 && list[index - 1]! >= item)) return malformed();
  return list;
}
export function canonicalProfile(value: unknown): WireObject | null {
  if (value === null) return null;
  const profile = strictObject(value, PROFILE_FIELDS);
  for (const field of ['width', 'height', 'fps', 'bitrate_mbps']) safeInteger(profile[field], 1, field === 'fps' ? 240 : field === 'bitrate_mbps' ? 1000 : 16384);
  boundedText(profile.codec, 32);
  for (const field of ['codec_profile', 'chroma_subsampling', 'pixel_format', 'color_mode', 'color_pipeline']) if (profile[field] !== null) boundedText(profile[field], 64);
  if (profile.bit_depth !== null && profile.bit_depth !== 8 && profile.bit_depth !== 10) return malformed();
  if (profile.hdr_enabled !== null && typeof profile.hdr_enabled !== 'boolean') return malformed();
  return profile;
}
export function normalizedH264Profile(value?: WireObject): WireObject {
  const source = value ?? { width: 1920, height: 1080, fps: 30, bitrate_mbps: 10, codec: 'h264' };
  if (source.codec !== 'h264' || source.hdr_enabled === true || (source.bit_depth !== undefined && source.bit_depth !== null && source.bit_depth !== 8)) throw new Error('网页远控首期仅支持 H.264 SDR 视频');
  return canonicalProfile(Object.fromEntries(PROFILE_FIELDS.map(field => [field, source[field] ?? null])))!;
}
function requestObject(value: unknown): WireObject {
  const request = strictObject(value, REQUEST_FIELDS);
  for (const field of ['session_id', 'controller_device_id', 'target_device_id']) boundedText(request[field]);
  request.idempotency_key = byteArray(request.idempotency_key, 16);
  if (!request.idempotency_key.some((v: number) => v !== 0) || request.controller_device_id === request.target_device_id || request.access_mode !== 'attended' || !['direct_first', 'relay_only'].includes(request.route_policy)) return malformed();
  request.requested_scopes = sortedStrings(request.requested_scopes);
  request.requested_profile = canonicalProfile(request.requested_profile);
  return request;
}
export function canonicalWanRequest(value: unknown): string { return JSON.stringify(requestObject(value)); }
function canonicalClaims(value: unknown): WireObject {
  const claims = strictObject(value, CLAIM_FIELDS);
  boundedText(claims.issuer_device_id);
  boundedText(claims.intended_peer_device_id);
  hexPin(claims.issuer_key_id);
  safeInteger(claims.issued_at_ms);
  safeInteger(claims.expires_at_ms, claims.issued_at_ms + 1, claims.issued_at_ms + 300_000);
  safeInteger(claims.counter, 1);
  claims.nonce = byteArray(claims.nonce, 16);
  if (!claims.nonce.some((v: number) => v !== 0)) return malformed();
  return claims;
}
export function canonicalSignalPayload(type: SignalType, value: unknown): WireObject {
  if (!Object.prototype.hasOwnProperty.call(PAYLOAD_FIELDS, type)) return malformed();
  const payload = strictObject(value, PAYLOAD_FIELDS[type]);
  payload.claims = canonicalClaims(payload.claims);
  if (type === 'session_intent_v3') { payload.request = requestObject(payload.request); hexPin(payload.request_commitment); }
  if (type === 'session_grant_v3') {
    payload.approved_profile = canonicalProfile(payload.approved_profile);
    payload.approved_scopes = sortedStrings(payload.approved_scopes);
    safeInteger(payload.backend_policy_revision, 1);
    safeInteger(payload.policy_expires_at_ms, payload.claims.issued_at_ms + 1);
    if (payload.relay_generation !== 0) return malformed();
    for (const field of ['session_id', 'controller_device_id', 'target_device_id', 'relay_directory_id', 'primary_relay_node_id']) boundedText(payload[field]);
    hexPin(payload.intent_commitment);
  }
  if (type === 'webrtc_offer_v3' || type === 'webrtc_answer_v3') {
    boundedText(payload.sdp, 256 * 1024);
    payload.candidate_fingerprints = sortedStrings(payload.candidate_fingerprints, 256).map(hexPin);
  }
  if (type === 'webrtc_candidate_v3') {
    boundedText(payload.candidate, 8192);
    if (!['offer', 'answer'].includes(payload.description_role)) return malformed();
    if (payload.sdp_mid !== null) boundedText(payload.sdp_mid, 128);
    if (payload.sdp_mline_index !== null) safeInteger(payload.sdp_mline_index, 0, 65535);
    if (payload.username_fragment !== null) boundedText(payload.username_fragment, 256);
    hexPin(payload.candidate_fingerprint);
  }
  if (type.startsWith('webrtc_')) {
    for (const field of ['session_id', 'controller_device_id', 'target_device_id']) boundedText(payload[field]);
    hexPin(payload.grant_commitment);
  }
  if (type === 'register') {
    if (payload.role !== 'Controller') return malformed();
    boundedText(payload.device_name, 128);
    boundedText(payload.backend_device_token, 4096);
    payload.challenge_id = byteArray(payload.challenge_id, 16);
    payload.challenge_nonce = byteArray(payload.challenge_nonce, 32);
  }
  if (type === 'registered' || type === 'presence_heartbeat') payload.connection_id = byteArray(payload.connection_id, 16);
  if (type === 'registered') { boundedText(payload.registered_device_id); safeInteger(payload.heartbeat_interval_ms, 1, 300_000); }
  if (type === 'presence_heartbeat') safeInteger(payload.observed_at_ms, 1);
  if (type === 'session_deny' || type === 'session_close') { boundedText(payload.session_id); boundedText(payload.reason, 128); }
  return payload;
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const result = new Uint8Array(parts.reduce((size, part) => size + part.byteLength, 0));
  let offset = 0;
  for (const part of parts) { result.set(part, offset); offset += part.byteLength; }
  return result;
}
function lengthBytes(value: number, size: 2 | 8, littleEndian = false): Uint8Array {
  safeInteger(value, 0, size === 2 ? 65535 : Number.MAX_SAFE_INTEGER);
  const bytes = new Uint8Array(size);
  const view = new DataView(bytes.buffer);
  if (size === 2) view.setUint16(0, value, littleEndian);
  else view.setBigUint64(0, BigInt(value), littleEndian);
  return bytes;
}
export function exactBuffer(value: Uint8Array): ArrayBuffer { return new Uint8Array(value).buffer; }
export function contextSignatureBytes(context: string, payload: Uint8Array): Uint8Array {
  const contextBytes = encoder.encode(context);
  if (!contextBytes.byteLength) return malformed();
  return concat(encoder.encode('MRD_CONTEXT_SIGNATURE_V1'), lengthBytes(contextBytes.byteLength, 2), contextBytes, lengthBytes(payload.byteLength, 8), payload);
}
export async function sha256Hex(value: Uint8Array): Promise<string> {
  const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', exactBuffer(value)));
  return Array.from(hash, byte => byte.toString(16).padStart(2, '0')).join('');
}
async function framedDigest(parts: Uint8Array[]): Promise<string> {
  return sha256Hex(concat(...parts.flatMap(part => [lengthBytes(part.byteLength, 8), part])));
}
export async function wanRequestCommitment(value: unknown): Promise<string> {
  return sha256Hex(concat(encoder.encode('MRD_WAN_SESSION_REQUEST_V3\0'), encoder.encode(canonicalWanRequest(value))));
}
function signedObject(type: SignalType, value: unknown): SignedSignal {
  const signed = strictObject(value, ['payload', 'signer_public_key', 'signature']);
  return { payload: canonicalSignalPayload(type, signed.payload), signer_public_key: byteArray(signed.signer_public_key, 32), signature: byteArray(signed.signature, 64) };
}
export async function signedSignalCommitment(kind: 'intent' | 'grant', value: unknown): Promise<string> {
  const signed = signedObject(kind === 'intent' ? 'session_intent_v3' : 'session_grant_v3', value);
  return framedDigest([encoder.encode(`MRD_SIGNAL_SESSION_${kind.toUpperCase()}_COMMITMENT_V3\0`), encoder.encode(JSON.stringify(signed))]);
}
export async function candidateFingerprint(value: WireObject): Promise<string> {
  const optional = (item: unknown, content: (v: any) => Uint8Array) => item === null ? new Uint8Array([0]) : concat(new Uint8Array([1]), content(item));
  return framedDigest([
    encoder.encode('MRD_WEBRTC_CANDIDATE_V3\0'), encoder.encode(boundedText(value.session_id)), encoder.encode(hexPin(value.grant_commitment)),
    encoder.encode(boundedText(value.description_role)), encoder.encode(boundedText(value.candidate, 8192)),
    optional(value.sdp_mid, v => encoder.encode(boundedText(v, 128))), optional(value.sdp_mline_index, v => lengthBytes(v, 2)),
    optional(value.username_fragment, v => encoder.encode(boundedText(v, 256))),
  ]);
}
export async function createBrowserSigningIdentity(): Promise<BrowserSigningIdentity> {
  try {
    const pair = await crypto.subtle.generateKey({ name: 'Ed25519' }, false, ['sign', 'verify']) as CryptoKeyPair;
    const raw = new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey));
    return { publicKey: Array.from(raw), keyId: await sha256Hex(raw), privateKey: pair.privateKey };
  } catch { throw new Error('当前浏览器不支持安全的 Ed25519 会话身份，请使用支持 WebCrypto 的新版浏览器'); }
}
export async function signContext(identity: BrowserSigningIdentity, context: string, payload: Uint8Array): Promise<number[]> {
  return Array.from(new Uint8Array(await crypto.subtle.sign('Ed25519', identity.privateKey, exactBuffer(contextSignatureBytes(context, payload)))));
}
export async function signSignal(identity: BrowserSigningIdentity, type: SignalType, value: unknown): Promise<SignedSignal> {
  const payload = canonicalSignalPayload(type, value);
  if (payload.claims.issuer_key_id !== identity.keyId) return malformed();
  return { payload, signer_public_key: [...identity.publicKey], signature: await signContext(identity, CONTEXTS[type], encoder.encode(JSON.stringify(payload))) };
}
export async function verifyContext(publicKey: number[], signature: number[], pin: string, context: string, payload: Uint8Array): Promise<void> {
  const raw = new Uint8Array(byteArray(publicKey, 32));
  if (await sha256Hex(raw) !== hexPin(pin)) throw new Error('远端身份公钥不匹配');
  const key = await crypto.subtle.importKey('raw', exactBuffer(raw), 'Ed25519', false, ['verify']);
  if (!await crypto.subtle.verify('Ed25519', key, exactBuffer(new Uint8Array(byteArray(signature, 64))), exactBuffer(contextSignatureBytes(context, payload)))) throw new Error('远端消息签名无效');
}
export const MAX_FUTURE_MESSAGE_WAIT_MS = 2000;

// Like mrd-signal-client/issued_time.rs, this only delays an early message.
// It never authenticates it or changes its signed timestamps. The caller must
// reread Date.now() and perform all strict checks after the bounded wait.
export async function waitUntilSignalIssued(type: SignalType, value: unknown, signal: AbortSignal): Promise<void> {
  const issuedAtMs = signedObject(type, value).payload.claims.issued_at_ms as number;
  const deadline = performance.now() + MAX_FUTURE_MESSAGE_WAIT_MS;
  while (true) {
    if (signal.aborted) throw new Error('网页连接已取消');
    const futureMs = issuedAtMs - Date.now();
    const remainingMs = deadline - performance.now();
    if (futureMs <= 0 || futureMs > MAX_FUTURE_MESSAGE_WAIT_MS || remainingMs <= 0) return;
    await new Promise<void>((resolve, reject) => {
      const finish = () => { signal.removeEventListener('abort', cancel); resolve(); };
      const timer = setTimeout(finish, Math.min(futureMs, remainingMs));
      const cancel = () => {
        clearTimeout(timer); signal.removeEventListener('abort', cancel);
        reject(new Error('网页连接已取消'));
      };
      signal.addEventListener('abort', cancel, { once: true });
    });
  }
}

type SignalValidityDiagnostics = Readonly<{
  message_type: SignalType; issuer_matches: boolean; intended_peer_matches: boolean; key_pin_matches: boolean;
  issued_delta_ms: number; expiry_remaining_ms: number;
}>;
export class BrowserSignalValidityError extends Error {
  constructor(readonly diagnostics: SignalValidityDiagnostics) {
    super('远端消息身份或有效期不匹配');
    this.name = 'BrowserSignalValidityError';
    Object.freeze(diagnostics);
  }
}
export async function verifySignedSignal(type: SignalType, value: unknown, expected: { peerDeviceId: string; signerDeviceId: string; signerKeyId: string; nowMs: number }): Promise<WireObject> {
  const signed = signedObject(type, value);
  const claims = signed.payload.claims;
  if (claims.issuer_device_id !== expected.signerDeviceId || claims.intended_peer_device_id !== expected.peerDeviceId || claims.issuer_key_id !== expected.signerKeyId || claims.issued_at_ms > expected.nowMs || claims.expires_at_ms <= expected.nowMs) {
    throw new BrowserSignalValidityError({
      message_type: type, issuer_matches: claims.issuer_device_id === expected.signerDeviceId,
      intended_peer_matches: claims.intended_peer_device_id === expected.peerDeviceId,
      key_pin_matches: claims.issuer_key_id === expected.signerKeyId,
      issued_delta_ms: claims.issued_at_ms - expected.nowMs, expiry_remaining_ms: claims.expires_at_ms - expected.nowMs,
    });
  }
  await verifyContext(signed.signer_public_key, signed.signature, expected.signerKeyId, CONTEXTS[type], encoder.encode(JSON.stringify(signed.payload)));
  return signed.payload;
}
export function randomBytes(length: number): number[] {
  const bytes = crypto.getRandomValues(new Uint8Array(length));
  if (!bytes.some(byte => byte !== 0)) bytes[0] = 1;
  return Array.from(bytes);
}
export function signalEnvelope(type: SignalType, signed: SignedSignal): string {
  return JSON.stringify({ version: type.endsWith('_v3') ? 3 : 2, message: { type, payload: signed } });
}
export function authenticatedInputBytes(event: ControlInputEvent, releaseScope?: 'input.keyboard' | 'input.pointer'): Uint8Array {
  let type: number;
  let payload: Uint8Array;
  const int32 = (value: number) => { safeInteger(value, -2147483648, 2147483647); const bytes = new Uint8Array(4); new DataView(bytes.buffer).setInt32(0, value); return bytes; };
  switch (event.kind) {
    case 'mouse_move': type = 1; payload = concat(int32(event.x), int32(event.y)); break;
    case 'mouse_button': {
      type = 2;
      const button = { left: 0, right: 1, middle: 2, x1: 3, x2: 4 }[event.button];
      if (button === undefined || typeof event.pressed !== 'boolean') return malformed();
      payload = new Uint8Array([button, event.pressed ? 1 : 0]); break;
    }
    case 'mouse_wheel': type = 3; payload = int32(event.delta); break;
    case 'mouse_horizontal_wheel': type = 14; payload = int32(event.delta); break;
    case 'key': {
      type = 4;
      safeInteger(event.key.code, 0, 4294967295);
      if (event.key.kind !== 'virtual_key' || typeof event.pressed !== 'boolean') return malformed();
      const key = new Uint8Array(4); new DataView(key.buffer).setUint32(0, event.key.code);
      payload = concat(key, new Uint8Array([event.pressed ? 1 : 0])); break;
    }
    case 'release_all': {
      if (!releaseScope) return malformed();
      type = 15; payload = new Uint8Array([releaseScope === 'input.keyboard' ? 2 : 1]); break;
    }
    default: return malformed();
  }
  return concat(new Uint8Array([2, type]), lengthBytes(payload.byteLength, 2), payload);
}
