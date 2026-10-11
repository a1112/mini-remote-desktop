import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { BrowserRemotePeer } from './browserRemotePeer';
import type { BrowserRemoteContext, BrowserRemoteState } from './browserRemoteSessionService';
import * as protocol from './browserRemoteProtocol';
import fixture from '../../../../realtime-server/tests/fixtures/browser_protocol_v3.json';
import relayFixture from './__fixtures__/browser-relay-directory.json';
const cryptoModuleName = 'node:crypto';
const { webcrypto } = await import(/* @vite-ignore */ cryptoModuleName) as { webcrypto: Crypto };

class Socket extends EventTarget {
  static sockets: Socket[] = [];
  readyState = 1;
  sent: string[] = [];
  constructor(public url: string) { super(); Socket.sockets.push(this); }
  send(value: string) { this.sent.push(value); }
  close() { this.readyState = 3; this.dispatchEvent(new Event('close')); }
  receive(message: unknown) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(message) })); }
}
class PeerConnection extends EventTarget {
  static peers: PeerConnection[] = [];
  static initialIceState = 'complete';
  connectionState = 'new';
  iceGatheringState = 'complete';
  localDescription?: RTCSessionDescriptionInit;
  remoteDescription?: RTCSessionDescriptionInit;
  channels: DataChannel[] = [];
  constructor(public config?: RTCConfiguration) { super(); this.iceGatheringState = PeerConnection.initialIceState; PeerConnection.peers.push(this); }
  addTransceiver() { return { setCodecPreferences: vi.fn() }; }
  createDataChannel(label: string) { const channel = new DataChannel(label); this.channels.push(channel); return channel; }
  async createOffer() { return { type: 'offer' as const, sdp: 'v=0\r\na=ice-ufrag:browser-ufrag\r\n' }; }
  async setLocalDescription(value: RTCSessionDescriptionInit) {
    this.localDescription = value;
    this.dispatchEvent(Object.assign(new Event('icecandidate'), { candidate: {
      candidate: 'candidate:1 1 UDP 2130706431 192.0.2.1 5000 typ host', sdpMid: '0', sdpMLineIndex: 0, usernameFragment: 'browser-ufrag',
    } }));
  }
  async setRemoteDescription(value: RTCSessionDescriptionInit) { this.remoteDescription = value; }
  async addIceCandidate() {}
  async getStats() {
    return new Map<string, Record<string, unknown>>([
      ['transport', { type: 'transport', selectedCandidatePairId: 'pair' }],
      ['pair', { type: 'candidate-pair', nominated: true, state: 'succeeded', localCandidateId: 'local', remoteCandidateId: 'remote' }],
      ['local', { type: 'local-candidate', candidateType: 'host' }],
      ['remote', { type: 'remote-candidate', candidateType: 'host' }],
    ]);
  }
  close() { this.connectionState = 'closed'; }
}
class DataChannel extends EventTarget {
  readyState = 'connecting';
  bufferedAmount = 0;
  bufferedAmountLowThreshold = 0;
  binaryType = 'arraybuffer';
  sent: ArrayBuffer[] = [];
  constructor(public label: string) { super(); }
  send(value: ArrayBuffer) { this.sent.push(value); }
  open() { this.readyState = 'open'; this.dispatchEvent(new Event('open')); }
}
let context: BrowserRemoteContext;
let onState = vi.fn<(state: BrowserRemoteState) => void>();
let onVideoStream = vi.fn<(stream: MediaStream) => void>();

async function fixtureIdentity(value: typeof fixture.browser): Promise<protocol.BrowserSigningIdentity> {
  const keyBytes = Uint8Array.from(('302e020100300506032b657004220420' + value.seed_hex).match(/../g)!, pair => Number.parseInt(pair, 16));
  return {
    publicKey: value.public_key, keyId: value.key_id,
    privateKey: await crypto.subtle.importKey('pkcs8', protocol.exactBuffer(keyBytes), 'Ed25519', false, ['sign']),
  };
}
const timerModuleName = 'node:timers';
const { setTimeout: realTimeout } = await import(/* @vite-ignore */ timerModuleName) as { setTimeout: (fn: () => void, ms: number) => unknown };
const pause = (ms = 5) => new Promise<void>(resolve => realTimeout(resolve, ms));
async function until(check: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 200; attempt++) { if (check()) return; await pause(); }
  throw new Error('Expected peer behavior did not occur');
}
async function serverRegistered(issuedAtMs = Date.now(), expiresAtMs = issuedAtMs + 10000, identity?: protocol.BrowserSigningIdentity): Promise<protocol.SignedSignal> {
  const signer = identity ?? await fixtureIdentity(fixture.target);
  return protocol.signSignal(signer, 'registered', {
    claims: { issuer_device_id: 'signal-server', issuer_key_id: signer.keyId,
      intended_peer_device_id: fixture.browser.device_id, issued_at_ms: issuedAtMs, expires_at_ms: expiresAtMs,
      counter: 1, nonce: Array(16).fill(4) },
    registered_device_id: fixture.browser.device_id, connection_id: Array(16).fill(8), heartbeat_interval_ms: 30000,
  });
}
function controlledClock() {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'performance'] });
  let now = fixture.now_ms;
  vi.mocked(Date.now).mockImplementation(() => now);
  context.getBootstrap = vi.fn(async () => context.bootstrap);
  return {
    set: (value: number) => { now = value; },
    advance: async (milliseconds: number) => { now += milliseconds; await vi.advanceTimersByTimeAsync(milliseconds); },
  };
}
async function beginRegistration(peer: BrowserRemotePeer): Promise<Socket> {
  await peer.start();
  const socket = Socket.sockets[0]!;
  socket.receive({ version: 2, message: { type: 'server_challenge', payload: {
    challenge_id: Array(16).fill(7), challenge_nonce: Array(32).fill(9), issued_at_ms: Date.now(), expires_at_ms: Date.now() + 10000,
  } } });
  await until(() => socket.sent.some(text => JSON.parse(text).message.type === 'register'));
  return socket;
}
async function approvedPeer(gathering = false, candidateFutureMs = 0): Promise<{ peer: BrowserRemotePeer; pc: PeerConnection; socket: Socket; candidate?: protocol.SignedSignal }> {
  if (gathering) PeerConnection.initialIceState = 'gathering';
  const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
  const socket = await beginRegistration(peer);
  socket.receive({ version: 2, message: { type: 'registered', payload: await serverRegistered() } });
  await until(() => socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3'));
  const intent = JSON.parse(socket.sent.find(text => JSON.parse(text).message.type === 'session_intent_v3')!).message.payload;
  const target = await fixtureIdentity(fixture.target);
  const grantPayload = { ...fixture.grant.payload, intent_commitment: await protocol.signedSignalCommitment('intent', intent) };
  const grant = await protocol.signSignal(target, 'session_grant_v3', grantPayload);
  context.getBootstrap = vi.fn(async () => ({ ...context.bootstrap, session: {
    ...context.bootstrap.session, status: 'approved', approved_scopes: grantPayload.approved_scopes, approved_profile: grantPayload.approved_profile,
    policy_revision: grantPayload.backend_policy_revision, active_relay_generation: 0,
    policy_expires_at: new Date(grantPayload.policy_expires_at_ms).toISOString(), grant_expires_at: new Date(grantPayload.policy_expires_at_ms).toISOString(),
  } }));
  context.getRelayAccess = vi.fn(async () => structuredClone(relayFixture.access));
  socket.receive({ version: 3, message: { type: 'session_grant_v3', payload: grant } });
  if (gathering) {
    await until(() => Boolean(PeerConnection.peers[0]?.localDescription));
    return { peer, pc: PeerConnection.peers[0]!, socket };
  }
  await until(() => socket.sent.some(text => JSON.parse(text).message.type === 'webrtc_candidate_v3'));
  const commitment = await protocol.signedSignalCommitment('grant', grant);
  const candidatePayload = { ...fixture.candidate.payload,
    claims: { ...grantPayload.claims, counter: 4, nonce: Array(16).fill(5),
      issued_at_ms: Date.now() + candidateFutureMs, expires_at_ms: Date.now() + candidateFutureMs + 10000 },
    grant_commitment: commitment, description_role: 'answer',
    candidate: 'candidate:2 1 UDP 2130706431 192.0.2.2 6000 typ host', username_fragment: 'target-ufrag' };
  candidatePayload.candidate_fingerprint = await protocol.candidateFingerprint(candidatePayload);
  const candidate = await protocol.signSignal(target, 'webrtc_candidate_v3', candidatePayload);
  const answer = await protocol.signSignal(target, 'webrtc_answer_v3', {
    claims: { ...grantPayload.claims, counter: 3, nonce: Array(16).fill(6) }, session_id: grantPayload.session_id,
    controller_device_id: grantPayload.controller_device_id, target_device_id: grantPayload.target_device_id,
    grant_commitment: commitment, sdp: 'v=0\r\na=ice-ufrag:target-ufrag\r\n', candidate_fingerprints: [candidatePayload.candidate_fingerprint],
  });
  // A genuine signed candidate is intentionally delivered before its answer.
  socket.receive({ version: 3, message: { type: 'webrtc_candidate_v3', payload: candidate } });
  socket.receive({ version: 3, message: { type: 'webrtc_answer_v3', payload: answer } });
  const pc = PeerConnection.peers[0]!;
  await until(() => Boolean(pc.remoteDescription));
  return { peer, pc, socket, candidate };
}

beforeEach(async () => {
  vi.spyOn(console, 'warn').mockImplementation(() => undefined);
  Socket.sockets = []; PeerConnection.peers = []; PeerConnection.initialIceState = 'complete';
  vi.spyOn(Date, 'now').mockReturnValue(fixture.now_ms);
  vi.stubGlobal('crypto', webcrypto);
  vi.stubGlobal('WebSocket', Socket);
  vi.stubGlobal('RTCPeerConnection', PeerConnection);
  vi.stubGlobal('RTCRtpReceiver', { getCapabilities: () => ({ codecs: [{ mimeType: 'video/H264', clockRate: 90000,
    sdpFmtpLine: 'level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f' }] }) });
  onState = vi.fn<(state: BrowserRemoteState) => void>(); onVideoStream = vi.fn<(stream: MediaStream) => void>();
  context = {
    identity: await fixtureIdentity(fixture.browser),
    bootstrap: {
      controller_device_id: fixture.browser.device_id, controller_key_id: fixture.browser.key_id,
      expires_at_ms: Date.now() + 600000, session: { session_id: fixture.request.session_id, request: fixture.request, request_commitment: fixture.request_commitment, status: 'requested' },
      credential: { token: 'browser-only-credential', expires_at_ms: Date.now() + 600000, device_id: fixture.browser.device_id, device_key_id: fixture.browser.key_id, role: 'Controller' },
      signaling_url: 'wss://signal.example.test/ws', signaling_server_device_id: 'signal-server', signaling_server_key_id: fixture.target.key_id,
      target_key_id: fixture.target.key_id, relay_directory_key_id: relayFixture.key_id, relay_directory_public_key: relayFixture.public_key,
    },
    getBootstrap: vi.fn(), getRelayAccess: vi.fn(), closeBackend: vi.fn(async () => undefined),
  };
});
afterEach(() => { vi.restoreAllMocks(); vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('independent browser peer lifecycle', () => {
  it.each([1, 1500, 2000])('waits for a registered reply issued %i ms ahead before sending the session intent', async futureMs => {
    const clock = controlledClock();
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    try {
      const socket = await beginRegistration(peer);
      const serverIdentity = await protocol.createBrowserSigningIdentity();
      context.bootstrap.signaling_server_key_id = serverIdentity.keyId;
      expect(serverIdentity.keyId).not.toBe(context.bootstrap.target_key_id);
      const registered = await serverRegistered(Date.now() + futureMs, Date.now() + futureMs + 10000, serverIdentity);
      socket.receive({ version: 2, message: { type: 'registered', payload: registered } });
      await pause(20);
      expect(onState.mock.calls.some(([state]) => state.phase === 'failed')).toBe(false);
      expect(socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3')).toBe(false);
      await clock.advance(futureMs - 1);
      expect(socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3')).toBe(false);
      await clock.advance(1);
      await until(() => socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3'));
      expect(registered.payload.claims.issued_at_ms).toBe(fixture.now_ms + futureMs);
    } finally { await peer.close(); }
  });

  it('rejects a registered reply more than 2000 ms ahead without waiting', async () => {
    controlledClock();
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    try {
      const socket = await beginRegistration(peer);
      socket.receive({ version: 2, message: { type: 'registered', payload: await serverRegistered(Date.now() + 2001) } });
      await until(() => onState.mock.calls.some(([state]) => state.phase === 'failed'));
      expect(performance.now()).toBe(0);
      expect(socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3')).toBe(false);
    } finally { await peer.close(); }
  });

  it('does not renew the 2000 ms monotonic budget when the wall clock moves backward', async () => {
    const clock = controlledClock();
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    try {
      const socket = await beginRegistration(peer);
      socket.receive({ version: 2, message: { type: 'registered', payload: await serverRegistered(Date.now() + 1500) } });
      await pause(20);
      expect(onState.mock.calls.some(([state]) => state.phase === 'failed')).toBe(false);
      await clock.advance(1000);
      clock.set(fixture.now_ms + 500);
      await vi.advanceTimersByTimeAsync(500);
      expect(onState.mock.calls.some(([state]) => state.phase === 'failed')).toBe(false);
      await vi.advanceTimersByTimeAsync(499);
      expect(onState.mock.calls.some(([state]) => state.phase === 'failed')).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      await until(() => onState.mock.calls.some(([state]) => state.phase === 'failed'));
      expect(performance.now()).toBe(2000);
      expect(socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3')).toBe(false);
    } finally { await peer.close(); }
  });

  it('still rejects a reply that expires while its issue-time wait is pending', async () => {
    const clock = controlledClock();
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    try {
      const socket = await beginRegistration(peer);
      socket.receive({ version: 2, message: { type: 'registered', payload: await serverRegistered(Date.now() + 100, Date.now() + 101) } });
      await pause(20);
      expect(onState.mock.calls.some(([state]) => state.phase === 'failed')).toBe(false);
      clock.set(fixture.now_ms + 102);
      await vi.advanceTimersByTimeAsync(100);
      await until(() => onState.mock.calls.some(([state]) => state.phase === 'failed'));
      expect(socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3')).toBe(false);
    } finally { await peer.close(); }
  });

  it('still checks the signature after waiting for the actual issue time', async () => {
    const clock = controlledClock();
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    try {
      const socket = await beginRegistration(peer);
      const registered = await serverRegistered(Date.now() + 100);
      registered.signature[0] = registered.signature[0]! ^ 1;
      socket.receive({ version: 2, message: { type: 'registered', payload: registered } });
      await pause(20);
      expect(onState.mock.calls.some(([state]) => state.phase === 'failed')).toBe(false);
      await clock.advance(100);
      await until(() => onState.mock.calls.some(([state]) => state.phase === 'failed'));
      expect(onState.mock.calls.find(([state]) => state.phase === 'failed')?.[0].error).toBe('远端消息签名无效');
      expect(socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3')).toBe(false);
    } finally { await peer.close(); }
  });

  it('still rejects a replayed registered reply after its bounded wait succeeds', async () => {
    const clock = controlledClock();
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    try {
      const socket = await beginRegistration(peer);
      const registered = await serverRegistered(Date.now() + 100);
      socket.receive({ version: 2, message: { type: 'registered', payload: registered } });
      await pause(20);
      await clock.advance(100);
      await until(() => socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3'));
      socket.receive({ version: 2, message: { type: 'registered', payload: registered } });
      await until(() => onState.mock.calls.some(([state]) => state.phase === 'failed'));
      expect(socket.sent.filter(text => JSON.parse(text).message.type === 'session_intent_v3')).toHaveLength(1);
      expect(onState.mock.calls.find(([state]) => state.phase === 'failed')?.[0].error).toBe('重复的会话身份注册');
    } finally { await peer.close(); }
  });

  it('rejects replay of the same authenticated candidate after waiting for its issue time', async () => {
    const clock = controlledClock();
    const wait = vi.spyOn(protocol, 'waitUntilSignalIssued');
    const preparing = approvedPeer(false, 100);
    await until(() => wait.mock.calls.some(([type]) => type === 'webrtc_candidate_v3'));
    expect(PeerConnection.peers[0]?.remoteDescription).toBeUndefined();
    await clock.advance(100);
    const { peer, pc, socket, candidate } = await preparing;
    try {
      expect(pc.remoteDescription).toBeDefined();
      socket.receive({ version: 3, message: { type: 'webrtc_candidate_v3', payload: candidate } });
      await until(() => onState.mock.calls.some(([state]) => state.phase === 'failed'));
      expect(onState.mock.calls.find(([state]) => state.phase === 'failed')?.[0].error).toBe('重复的远程认证消息');
      expect(PeerConnection.peers).toHaveLength(1);
    } finally { await peer.close(); }
  });

  it('still rejects the wrong trusted identity and logs no raw identity or signed message', async () => {
    const clock = controlledClock();
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    try {
      const socket = await beginRegistration(peer);
      context.bootstrap.signaling_server_device_id = 'different-trusted-server';
      socket.receive({ version: 2, message: { type: 'registered', payload: await serverRegistered(Date.now() + 100) } });
      await pause(20);
      await clock.advance(100);
      await until(() => onState.mock.calls.some(([state]) => state.phase === 'failed'));
      expect(console.warn).toHaveBeenCalledWith('[rdesk] signed signal rejected', {
        message_type: 'registered', issuer_matches: false, intended_peer_matches: true, key_pin_matches: true,
        issued_delta_ms: 0, expiry_remaining_ms: 10000,
      });
      expect(socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3')).toBe(false);
    } finally { await peer.close(); }
  });

  it('cancels the pending issue-time wait on close without sending an intent or heartbeat', async () => {
    const clock = controlledClock();
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    const socket = await beginRegistration(peer);
    socket.receive({ version: 2, message: { type: 'registered', payload: await serverRegistered(Date.now() + 1500) } });
    await pause(20);
    expect(onState.mock.calls.some(([state]) => state.phase === 'failed')).toBe(false);
    await peer.close();
    await clock.advance(31000);
    expect(socket.sent.map(text => JSON.parse(text).message.type)).toEqual(['register']);
    expect(vi.getTimerCount()).toBe(0);
    expect(context.closeBackend).toHaveBeenCalledTimes(1);
    expect(onState).toHaveBeenLastCalledWith(expect.objectContaining({ phase: 'closed' }));
  });

  it('opens the configured signaling connection without a token in its URL or a localhost bridge', async () => {
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    await peer.start();
    expect(Socket.sockets).toHaveLength(1);
    expect(Socket.sockets[0]!.url).toBe('wss://signal.example.test/ws');
    expect(Socket.sockets[0]!.sent).toEqual([]);
    expect(PeerConnection.peers).toHaveLength(0);
    await peer.close();
  });
  it('does not enable media or input when the UI reports a frame before authorization', async () => {
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    await peer.start();
    peer.markVideoReady(1920, 1080);
    await expect(peer.sendInput({ kind: 'key', key: { kind: 'virtual_key', code: 65 }, pressed: true })).rejects.toThrow('授权');
    expect(onState.mock.calls.some(([state]) => state.phase === 'streaming')).toBe(false);
    expect(context.getRelayAccess).not.toHaveBeenCalled();
    await peer.close();
  });
  it('closes only the owned backend session and ignores delayed signaling after cancellation', async () => {
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    await peer.start();
    const socket = Socket.sockets[0]!;
    await peer.close('user_cancel');
    await peer.close('again');
    socket.receive({ version: 2, message: { type: 'server_challenge', payload: { invalid: true } } });
    expect(context.closeBackend).toHaveBeenCalledTimes(1);
    expect(socket.sent).toEqual([]);
    expect(PeerConnection.peers).toHaveLength(0);
    expect(onState).toHaveBeenLastCalledWith(expect.objectContaining({ phase: 'closed', grantedScopes: [] }));
  });
  it('rejects unsupported peer APIs with an actual visible failure', async () => {
    vi.stubGlobal('RTCPeerConnection', undefined);
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    await expect(peer.start()).rejects.toThrow('WebRTC');
    expect(Socket.sockets).toHaveLength(0);
  });

  it('does not create a heartbeat after close while real registered verification was pending', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    const socket = await beginRegistration(peer);
    const registered = await serverRegistered();
    const actual = protocol.verifySignedSignal;
    let entered = false, release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    vi.spyOn(protocol, 'verifySignedSignal').mockImplementationOnce(async (...args) => { entered = true; await gate; return actual(...args); });
    socket.receive({ version: 2, message: { type: 'registered', payload: registered } });
    await until(() => entered);
    await peer.close('user_cancel');
    release();
    await pause(30);
    expect(vi.getTimerCount()).toBe(0);
    expect(socket.sent.map(text => JSON.parse(text).message.type)).toEqual(['register']);
    expect(context.closeBackend).toHaveBeenCalledTimes(1);
  });

  it('does not send a late real signed intent while final session-close waits behind it', async () => {
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    const socket = await beginRegistration(peer);
    const registered = await serverRegistered();
    const actual = protocol.signSignal;
    let entered = false, release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    vi.spyOn(protocol, 'signSignal').mockImplementation(async (...args) => {
      const signed = await actual(...args);
      if (args[1] === 'session_intent_v3') { entered = true; await gate; }
      return signed;
    });
    socket.receive({ version: 2, message: { type: 'registered', payload: registered } });
    await until(() => entered);
    const closed = peer.close('user_cancel');
    release();
    await closed;
    expect(socket.sent.map(text => JSON.parse(text).message.type)).toEqual(['register', 'session_close']);
    expect(onState).toHaveBeenLastCalledWith(expect.objectContaining({ phase: 'closed', grantedScopes: [] }));
  });

  it('disposes the peer and owned backend without waiting for an unfinished signal signature', async () => {
    const peer = new BrowserRemotePeer(context, { onState, onVideoStream });
    const socket = await beginRegistration(peer);
    const registered = await serverRegistered();
    const actual = protocol.signSignal;
    let entered = false, release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    vi.spyOn(protocol, 'signSignal').mockImplementation(async (...args) => {
      const signed = await actual(...args);
      if (args[1] === 'session_intent_v3') { entered = true; await gate; }
      return signed;
    });
    socket.receive({ version: 2, message: { type: 'registered', payload: registered } });
    await until(() => entered);
    const closed = peer.close('user_cancel');
    const settled = await Promise.race([closed.then(() => true), pause(900).then(() => false)]);
    try {
      expect(settled).toBe(true);
      expect(context.closeBackend).toHaveBeenCalledTimes(1);
      expect(socket.readyState).toBe(3);
    } finally { release(); await closed; }
    await pause(10);
    expect(socket.sent.some(text => JSON.parse(text).message.type === 'session_intent_v3')).toBe(false);
  });

  it('keeps control disabled until signed negotiation, an actual track frame and required data channels are open', async () => {
    const { peer, pc } = await approvedPeer();
    const track = Object.assign(new EventTarget(), { kind: 'video', stop: vi.fn() });
    const stream = { getTracks: () => [track] };
    pc.dispatchEvent(Object.assign(new Event('track'), { track, streams: [stream] }));
    pc.connectionState = 'connected'; pc.dispatchEvent(new Event('connectionstatechange'));
    await until(() => onState.mock.calls.some(([state]) => state.route === 'direct'));
    peer.markVideoReady(1280, 720);
    expect(onVideoStream).toHaveBeenCalledWith(stream);
    expect(onState.mock.calls.some(([state]) => state.phase === 'streaming')).toBe(false);
    await expect(peer.sendInput({ kind: 'mouse_move', x: 1, y: 1 })).rejects.toThrow('就绪');
    expect(pc.connectionState).toBe('connected');
    pc.channels.find(channel => channel.label === 'ctrl_rel')!.open();
    expect(onState.mock.calls.some(([state]) => state.phase === 'streaming')).toBe(false);
    pc.channels.find(channel => channel.label === 'ctrl_rt')!.open();
    expect(onState).toHaveBeenLastCalledWith(expect.objectContaining({ phase: 'streaming', frameWidth: 1280, frameHeight: 720 }));
    // No input has been sent, so dispose channels before best-effort release.
    pc.channels.forEach(channel => { channel.readyState = 'closed'; });
    await peer.close('test_cleanup');
  });

  it('settles the owned negotiation when closed during gathering without any further ICE events', async () => {
    const { peer, pc, socket } = await approvedPeer(true);
    expect(pc.iceGatheringState).toBe('gathering');
    const incoming = (peer as unknown as { incoming: Promise<void> }).incoming;
    await peer.close('user_cancel');
    const settled = await Promise.race([incoming.then(() => true), pause(75).then(() => false)]);
    expect(settled).toBe(true);
    expect(socket.sent.some(text => JSON.parse(text).message.type === 'webrtc_offer_v3')).toBe(false);
    expect(context.closeBackend).toHaveBeenCalledTimes(1);
  });
});
