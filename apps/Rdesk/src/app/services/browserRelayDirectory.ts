import { boundedText, byteArray, exactBuffer, safeInteger, sha256Hex, strictObject, type WireObject } from './browserRemoteProtocol';

const encoder = new TextEncoder();
const transportCode: Record<string, number> = { udp: 1, tcp: 2, tls: 3 };
function invalid(): never { throw new Error('远程中继目录无效或不属于当前会话'); }
class Writer {
  parts: Uint8Array[] = [];
  bytes(value: Uint8Array) { this.parts.push(value); }
  number(value: unknown, width: 1 | 2 | 4 | 8) {
    const number = safeInteger(value, 0, width === 8 ? Number.MAX_SAFE_INTEGER : 2 ** (width * 8) - 1);
    const bytes = new Uint8Array(width), view = new DataView(bytes.buffer);
    if (width === 1) view.setUint8(0, number);
    if (width === 2) view.setUint16(0, number);
    if (width === 4) view.setUint32(0, number);
    if (width === 8) view.setBigUint64(0, BigInt(number));
    this.bytes(bytes);
  }
  text(value: unknown) { const bytes = encoder.encode(boundedText(value)); this.number(bytes.length, 4); this.bytes(bytes); }
  finish(): Uint8Array {
    const result = new Uint8Array(this.parts.reduce((n, bytes) => n + bytes.length, 0));
    let offset = 0;
    for (const bytes of this.parts) { result.set(bytes, offset); offset += bytes.length; }
    return result;
  }
}
function utf8Compare(left: string, right: string): number {
  const a = encoder.encode(left), b = encoder.encode(right);
  for (let i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) return a[i]! - b[i]!;
  return a.length - b.length;
}
function directoryPayload(raw: unknown): WireObject {
  const payload = strictObject(raw, ['format_version', 'policy_revision', 'directory_id', 'issued_at_ms', 'expires_at_ms', 'session_id', 'intended_peer_digest', 'candidates']);
  if (payload.format_version !== 1) return invalid();
  safeInteger(payload.policy_revision, 1);
  safeInteger(payload.issued_at_ms);
  safeInteger(payload.expires_at_ms, payload.issued_at_ms + 1);
  for (const key of ['directory_id', 'session_id', 'intended_peer_digest']) boundedText(payload[key]);
  if (!Array.isArray(payload.candidates) || !payload.candidates.length || payload.candidates.length > 8) return invalid();
  const reservations = new Set<string>();
  payload.candidates = payload.candidates.map((rawCandidate: unknown, i: number) => {
    const candidate = strictObject(rawCandidate, ['node_id', 'region', 'failure_domain', 'endpoints', 'capabilities', 'load_class', 'selection_reason', 'reservation']);
    for (const field of ['node_id', 'region', 'failure_domain', 'selection_reason']) boundedText(candidate[field]);
    if (i > 0 && utf8Compare(payload.candidates[i - 1].node_id, candidate.node_id) >= 0) return invalid();
    safeInteger(candidate.capabilities, 0, 4294967295);
    safeInteger(candidate.load_class, 0, 3);
    if (!Array.isArray(candidate.endpoints) || !candidate.endpoints.length || candidate.endpoints.length > 4) return invalid();
    candidate.endpoints = candidate.endpoints.map((rawEndpoint: unknown, index: number) => {
      const endpoint = strictObject(rawEndpoint, ['transport', 'host', 'port']);
      if (!transportCode[endpoint.transport] || !/^[A-Za-z0-9_.:-]+$/.test(boundedText(endpoint.host))) return invalid();
      safeInteger(endpoint.port, 1, 65535);
      if (index > 0) {
        const prior = candidate.endpoints[index - 1];
        const order = transportCode[prior.transport]! - transportCode[endpoint.transport]!
          || utf8Compare(prior.host, endpoint.host) || prior.port - endpoint.port;
        if (order >= 0) return invalid();
      }
      return endpoint;
    });
    candidate.reservation = strictObject(candidate.reservation, ['reservation_id', 'expires_at_ms']);
    boundedText(candidate.reservation.reservation_id);
    if (reservations.has(candidate.reservation.reservation_id)) return invalid();
    reservations.add(candidate.reservation.reservation_id);
    safeInteger(candidate.reservation.expires_at_ms, payload.issued_at_ms + 1, payload.expires_at_ms);
    return candidate;
  });
  return payload;
}
export function canonicalRelayDirectoryBytes(raw: unknown): Uint8Array {
  const payload = directoryPayload(raw), writer = new Writer();
  writer.bytes(encoder.encode('MRD_RELAY_DIRECTORY_V1'));
  writer.number(payload.format_version, 2); writer.number(payload.policy_revision, 8);
  writer.text(payload.directory_id); writer.number(payload.issued_at_ms, 8); writer.number(payload.expires_at_ms, 8);
  writer.text(payload.session_id); writer.text(payload.intended_peer_digest); writer.number(payload.candidates.length, 4);
  for (const candidate of payload.candidates) {
    writer.text(candidate.node_id); writer.text(candidate.region); writer.text(candidate.failure_domain);
    writer.number(candidate.endpoints.length, 4);
    for (const endpoint of candidate.endpoints) { writer.number(transportCode[endpoint.transport], 1); writer.text(endpoint.host); writer.number(endpoint.port, 2); }
    writer.number(candidate.capabilities, 4); writer.number(candidate.load_class, 1); writer.text(candidate.selection_reason);
    writer.text(candidate.reservation.reservation_id); writer.number(candidate.reservation.expires_at_ms, 8);
  }
  const bytes = writer.finish();
  if (bytes.length > 16384) return invalid();
  return bytes;
}
function endpointUrl(endpoint: WireObject): string {
  const host = endpoint.host.includes(':') ? `[${endpoint.host}]` : endpoint.host;
  return `${endpoint.transport === 'tls' ? 'turns' : 'turn'}:${host}:${endpoint.port}?transport=${endpoint.transport === 'udp' ? 'udp' : 'tcp'}`;
}
export async function verifyBrowserRelayAccess(raw: unknown, binding: {
  grant: WireObject; targetDeviceId: string; keyId: string; publicKey: number[]; nowMs: number;
}): Promise<RTCConfiguration> {
  const access = strictObject(raw, ['generation', 'directory_id', 'relay_url_digest', 'directory', 'credentials']);
  const signed = strictObject(access.directory, ['payload', 'signing_key_id', 'signature_b64']);
  const payload = directoryPayload(signed.payload), grant = binding.grant;
  if (access.generation !== 0 || grant.relay_generation !== 0 || access.directory_id !== grant.relay_directory_id
    || payload.directory_id !== access.directory_id || payload.session_id !== grant.session_id
    || payload.policy_revision !== grant.backend_policy_revision || binding.nowMs < payload.issued_at_ms
    || binding.nowMs >= payload.expires_at_ms || signed.signing_key_id !== binding.keyId
    || !['relay_only', 'direct_first'].includes(grant.route_policy)) return invalid();
  const publicKey = new Uint8Array(byteArray(binding.publicKey, 32));
  if (await sha256Hex(publicKey) !== binding.keyId) return invalid();
  const peerBytes = encoder.encode(`MRD_RELAY_PEER_V1\0${binding.targetDeviceId}`);
  if (payload.intended_peer_digest !== `peer-sha256-${await sha256Hex(peerBytes)}`) return invalid();
  if (typeof signed.signature_b64 !== 'string') return invalid();
  let signature: Uint8Array;
  try { signature = Uint8Array.from(atob(signed.signature_b64), char => char.charCodeAt(0)); }
  catch { return invalid(); }
  if (signature.length !== 64 || btoa(String.fromCharCode(...signature)) !== signed.signature_b64) return invalid();
  const key = await crypto.subtle.importKey('raw', exactBuffer(publicKey), 'Ed25519', false, ['verify']);
  if (!await crypto.subtle.verify('Ed25519', key, exactBuffer(signature), exactBuffer(canonicalRelayDirectoryBytes(payload)))) throw new Error('中继目录签名验证失败');
  if (!Array.isArray(access.credentials) || access.credentials.length !== payload.candidates.length) return invalid();
  const credentials = new Map<string, WireObject>();
  for (const rawCredential of access.credentials) {
    const credential = strictObject(rawCredential, ['node_id', 'urls', 'username', 'credential', 'expires_at_unix_seconds']);
    if (credentials.has(credential.node_id)) return invalid();
    boundedText(credential.node_id);
    for (const field of ['username', 'credential']) {
      boundedText(credential[field], 512);
      if (/[\u0000-\u001f\u007f]/.test(credential[field])) return invalid();
    }
    safeInteger(credential.expires_at_unix_seconds, Math.floor(binding.nowMs / 1000) + 1);
    if (!Array.isArray(credential.urls) || credential.urls.length < 1 || credential.urls.length > 4
      || new Set(credential.urls).size !== credential.urls.length) return invalid();
    credentials.set(credential.node_id, credential);
  }
  for (const candidate of payload.candidates) {
    if (candidate.reservation.expires_at_ms <= binding.nowMs) return invalid();
    const credential = credentials.get(candidate.node_id);
    const endpoints = candidate.endpoints.map(endpointUrl).sort();
    if (!credential || JSON.stringify([...credential.urls].sort()) !== JSON.stringify(endpoints)) return invalid();
  }
  const primary = credentials.get(grant.primary_relay_node_id);
  if (!primary) return invalid();
  const digestWriter = new Writer();
  digestWriter.bytes(encoder.encode('MRD_RELAY_URLS_V1\0'));
  for (const url of [...primary.urls].sort()) digestWriter.text(url);
  if (await sha256Hex(digestWriter.finish()) !== access.relay_url_digest) return invalid();
  const iceServers: RTCIceServer[] = [{ urls: [...primary.urls], username: primary.username, credential: primary.credential }];
  if (grant.route_policy === 'direct_first') {
    const stun = [...new Set<string>(primary.urls.filter((url: string) => url.startsWith('turn:') && url.endsWith('?transport=udp'))
      .map((url: string) => `stun:${url.slice(5).split('?')[0]}`))].sort();
    if (stun.length) iceServers.push({ urls: stun });
  }
  return { iceServers, iceTransportPolicy: grant.route_policy === 'relay_only' ? 'relay' : 'all' };
}
