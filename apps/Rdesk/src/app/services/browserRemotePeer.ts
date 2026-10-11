import type { ControlInputEvent } from '../adapters/tauri/types';
import type { BrowserRemoteContext, BrowserRemoteHandle, BrowserRemoteObserver, BrowserRemoteState } from './browserRemoteSessionService';
import { BrowserRemoteControl } from './browserRemoteControl';
import { verifyBrowserRelayAccess, waitUntilRelayDirectoryIssued, BrowserRelayDirectoryValidityError } from './browserRelayDirectory';
import {
  byteArray, canonicalProfile, canonicalWanRequest, candidateFingerprint, randomBytes,
  safeInteger, signalEnvelope, signSignal, signedSignalCommitment, strictObject, verifySignedSignal, parseBoundedJson,
  waitUntilSignalIssued, BrowserSignalValidityError,
  type SignedSignal, type SignalType, type WireObject,
} from './browserRemoteProtocol';

const encoder = new TextEncoder();
const terminalPhases = new Set(['failed', 'denied', 'closed']);
const MAX_MESSAGE_BYTES = 512 * 1024;
const MAX_QUEUED_BYTES = 2 * 1024 * 1024;
const MAX_CANDIDATES = 256;

export class BrowserRemotePeer implements BrowserRemoteHandle {
  private state: BrowserRemoteState = { phase: 'waiting_consent', grantedScopes: [] };
  private socket?: WebSocket;
  private pc?: RTCPeerConnection;
  private control?: BrowserRemoteControl;
  private controlChannels?: { reliable: RTCDataChannel; realtime: RTCDataChannel };
  private stream?: MediaStream;
  private registered = false;
  private closing = false;
  private started = false;
  private closePromise?: Promise<void>;
  private outgoing: Promise<unknown> = Promise.resolve();
  private incoming: Promise<void> = Promise.resolve();
  private queuedBytes = 0;
  private counter = 1;
  private connectionId?: number[];
  private intent?: SignedSignal;
  private grant?: SignedSignal;
  private grantCommitment?: string;
  private answer?: WireObject;
  private candidates = new Map<string, WireObject>();
  private seenMessages = new Set<string>();
  private timers = new Set<ReturnType<typeof setTimeout>>();
  private cancelWaits = new Set<() => void>();
  private heartbeat?: ReturnType<typeof setInterval>;
  private poll?: ReturnType<typeof setInterval>;
  private polling = false;
  private routeVerified = false;
  private frame?: { width: number; height: number };
  private negotiationDeadline?: ReturnType<typeof setTimeout>;
  private signalWaitAbort = new AbortController();

  constructor(private context: BrowserRemoteContext, private observer: BrowserRemoteObserver) {}
  private get sessionId(): string { return this.context.bootstrap.session.session_id; }
  private get controllerId(): string { return this.context.bootstrap.controller_device_id; }
  private get targetId(): string { return this.context.bootstrap.session.request.target_device_id; }
  private publish(change: Partial<BrowserRemoteState>): void {
    this.state = { ...this.state, ...change };
    this.observer.onState({ ...this.state, grantedScopes: [...this.state.grantedScopes] });
  }
  private deadline(callback: () => void, ms: number): ReturnType<typeof setTimeout> {
    const timer = setTimeout(() => { this.timers.delete(timer); callback(); }, Math.max(1, ms));
    this.timers.add(timer);
    return timer;
  }
  private fail(error: unknown, phase: 'failed' | 'denied' = 'failed'): void {
    if (this.closing) return;
    if (error instanceof BrowserSignalValidityError) console.warn('[rdesk] signed signal rejected', error.diagnostics);
    if (error instanceof BrowserRelayDirectoryValidityError) console.warn('[rdesk] relay directory rejected', error.diagnostics);
    this.publish({ phase, grantedScopes: [], error: error instanceof Error ? error.message : '网页远程连接失败' });
    void this.close('connection_failed').catch(() => undefined);
  }

  async start(): Promise<void> {
    if (this.started || this.closing) return;
    if (typeof RTCPeerConnection !== 'function' || typeof WebSocket !== 'function') {
      const error = new Error('当前浏览器不支持 WebRTC 远程连接');
      this.publish({ phase: 'failed', grantedScopes: [], error: error.message });
      throw error;
    }
    this.started = true;
    this.publish({ phase: 'waiting_consent', grantedScopes: [] });
    this.socket = new WebSocket(this.context.bootstrap.signaling_url);
    this.socket.addEventListener('message', event => {
      if (this.closing) return;
      if (typeof event.data !== 'string') { this.fail(new Error('信令数据格式无效')); return; }
      const size = encoder.encode(event.data).length;
      if (size > MAX_MESSAGE_BYTES || this.queuedBytes + size > MAX_QUEUED_BYTES) { this.fail(new Error('信令接收队列超出限制')); return; }
      this.queuedBytes += size;
      this.incoming = this.incoming.then(async () => { if (!this.closing) await this.receive(event.data); })
        .catch(error => this.fail(error)).finally(() => { this.queuedBytes -= size; });
    });
    this.socket.addEventListener('error', () => this.fail(new Error('无法连接远程信令服务')));
    this.socket.addEventListener('close', () => { if (!this.closing) this.fail(new Error('远程信令连接已断开')); });
    this.deadline(() => { if (!this.registered) this.fail(new Error('网页会话身份注册超时')); }, 15_000);
    this.deadline(() => this.fail(new Error('网页会话身份已到期')), this.context.bootstrap.expires_at_ms - Date.now());
    this.poll = setInterval(() => { void this.refreshAuthorization(); }, 2_000);
  }

  private claims(peer: string): WireObject {
    const now = Date.now();
    return {
      issuer_device_id: this.controllerId,
      issuer_key_id: this.context.identity.keyId,
      intended_peer_device_id: peer,
      issued_at_ms: now,
      expires_at_ms: Math.min(now + 60_000, this.context.bootstrap.expires_at_ms, this.context.bootstrap.credential.expires_at_ms),
      counter: safeInteger(this.counter++, 1),
      nonce: randomBytes(16),
    };
  }
  private send(type: SignalType, body: WireObject, peer = this.targetId): Promise<SignedSignal> {
    const result = this.outgoing.then(async () => {
      if (this.closing && type !== 'session_close') throw new Error('网页连接已取消');
      if (!this.socket || this.socket.readyState !== 1) throw new Error('信令连接尚未就绪');
      const signed = await signSignal(this.context.identity, type, { claims: this.claims(peer), ...body });
      if (this.closing && type !== 'session_close') throw new Error('网页连接已取消');
      if (!this.socket || this.socket.readyState !== 1 || this.socket.bufferedAmount > MAX_QUEUED_BYTES) throw new Error('信令发送通道不可用');
      this.socket.send(signalEnvelope(type, signed));
      return signed;
    });
    this.outgoing = result.catch(() => undefined);
    return result;
  }
  private async receive(text: string): Promise<void> {
    const envelope = strictObject(parseBoundedJson(text), ['version', 'message']);
    const message = strictObject(envelope.message, ['type', 'payload']);
    const type = message.type as string;
    if (envelope.version !== (type.endsWith('_v3') ? 3 : 2)) throw new Error('信令协议版本不兼容');
    if (type === 'protocol_error') throw new Error('远程信令服务拒绝了当前操作');
    if (type === 'server_challenge') {
      if (this.registered || this.connectionId) throw new Error('重复的信令身份挑战');
      const challenge = strictObject(message.payload, ['challenge_id', 'challenge_nonce', 'issued_at_ms', 'expires_at_ms']);
      byteArray(challenge.challenge_id, 16); byteArray(challenge.challenge_nonce, 32);
      safeInteger(challenge.issued_at_ms, 0, Date.now() + 2000);
      safeInteger(challenge.expires_at_ms, Date.now() + 1, challenge.issued_at_ms + 60_000);
      await this.send('register', {
        role: 'Controller', device_name: 'Rdesk Browser',
        backend_device_token: this.context.bootstrap.credential.token,
        challenge_id: challenge.challenge_id, challenge_nonce: challenge.challenge_nonce,
      }, this.context.bootstrap.signaling_server_device_id);
      return;
    }
    if (type === 'registered') {
      if (this.registered) throw new Error('重复的会话身份注册');
      const payload = await this.verified('registered', message.payload, true);
      if (this.closing) return;
      if (payload.registered_device_id !== this.controllerId) throw new Error('注册的浏览器身份不匹配');
      this.connectionId = payload.connection_id;
      this.registered = true;
      this.heartbeat = setInterval(() => {
        if (!this.closing && this.connectionId) void this.send('presence_heartbeat', { connection_id: this.connectionId, observed_at_ms: Date.now() }, this.context.bootstrap.signaling_server_device_id).catch(error => this.fail(error));
      }, Math.max(1000, Math.min(30_000, payload.heartbeat_interval_ms)));
      const intent = await this.send('session_intent_v3', { request: this.context.bootstrap.session.request, request_commitment: this.context.bootstrap.session.request_commitment });
      if (this.closing) return;
      this.intent = intent;
      return;
    }
    if (!this.registered || !this.intent) throw new Error('远程会话身份尚未注册');
    if (type === 'session_deny') {
      const denial = await this.verified('session_deny', message.payload);
      if (this.closing) return;
      if (denial.session_id !== this.sessionId || denial.controller_device_id !== this.controllerId) throw new Error('远端拒绝消息不属于本次会话');
      this.fail(new Error('远端拒绝了本次连接请求'), 'denied');
      return;
    }
    if (type === 'session_close') {
      const close = await this.verified('session_close', message.payload);
      if (this.closing) return;
      if (close.session_id !== this.sessionId) throw new Error('远端关闭消息不属于本次会话');
      this.publish({ phase: 'closed', grantedScopes: [] });
      await this.close('remote_close');
      return;
    }
    if (type === 'session_grant_v3') {
      if (this.grant) throw new Error('远端重复签发了会话授权');
      const grant = await this.verified('session_grant_v3', message.payload);
      if (this.closing) return;
      if (grant.session_id !== this.sessionId || grant.controller_device_id !== this.controllerId || grant.target_device_id !== this.targetId
        || grant.intent_commitment !== await signedSignalCommitment('intent', this.intent)
        || grant.route_policy !== this.context.bootstrap.session.request.route_policy
        || grant.relay_generation !== 0 || !grant.approved_scopes.includes('screen.view')
        || grant.approved_scopes.some((scope: string) => !this.context.bootstrap.session.request.requested_scopes.includes(scope))
        || grant.approved_profile?.codec !== 'h264' || grant.policy_expires_at_ms <= Date.now()
        || grant.policy_expires_at_ms > this.context.bootstrap.expires_at_ms) throw new Error('远端会话授权与本次请求不匹配');
      const fresh = await this.context.getBootstrap();
      if (this.closing) return;
      this.assertApprovedPolicy(grant, fresh.session);
      this.grant = message.payload;
      this.grantCommitment = await signedSignalCommitment('grant', this.grant!);
      if (this.closing) return;
      this.publish({ phase: 'negotiating', grantedScopes: [...grant.approved_scopes] });
      this.deadline(() => this.fail(new Error('远端会话授权已到期')), grant.policy_expires_at_ms - Date.now());
      await this.negotiate(grant);
      return;
    }
    if (type === 'webrtc_answer_v3' || type === 'webrtc_candidate_v3') {
      if (!this.grant || !this.pc || !this.grantCommitment) throw new Error('远端媒体授权尚未完成');
      const payload = await this.verified(type, message.payload);
      if (this.closing) return;
      if (payload.session_id !== this.sessionId || payload.controller_device_id !== this.controllerId || payload.target_device_id !== this.targetId
        || payload.grant_commitment !== this.grantCommitment) throw new Error('远端媒体描述绑定不匹配');
      if (type === 'webrtc_answer_v3') {
        if (this.answer) throw new Error('重复的远端视频协商响应');
        this.answer = payload;
      } else {
        const fingerprint = await candidateFingerprint(payload);
        if (this.closing) return;
        if (payload.description_role !== 'answer' || payload.candidate_fingerprint !== fingerprint
          || this.candidates.size >= MAX_CANDIDATES || this.candidates.has(payload.candidate_fingerprint)) throw new Error('远端候选验证失败');
        this.candidates.set(payload.candidate_fingerprint, payload);
      }
      await this.applyAnswer();
      return;
    }
    if (type.startsWith('relay_migration') || type.startsWith('reconnect')) throw new Error('网页暂不支持会话路径迁移，请重新连接');
    throw new Error('收到不支持的远程会话消息');
  }
  private async verified(type: SignalType, signed: SignedSignal, server = false): Promise<WireObject> {
    const bootstrap = this.context.bootstrap;
    await waitUntilSignalIssued(type, signed, this.signalWaitAbort.signal);
    const payload = await verifySignedSignal(type, signed, {
      peerDeviceId: this.controllerId,
      signerDeviceId: server ? bootstrap.signaling_server_device_id : this.targetId,
      signerKeyId: server ? bootstrap.signaling_server_key_id : bootstrap.target_key_id,
      nowMs: Date.now(),
    });
    if (this.closing) throw new Error('网页连接已取消');
    const key = `${payload.claims.issuer_key_id}:${payload.claims.counter}:${payload.claims.nonce.join(',')}`;
    if (this.seenMessages.has(key)) throw new Error('重复的远程认证消息');
    this.seenMessages.add(key);
    if (this.seenMessages.size > 512) this.seenMessages.delete(this.seenMessages.values().next().value!);
    return payload;
  }
  private assertApprovedPolicy(grant: WireObject, snapshot: WireObject): void {
    if (snapshot.status !== 'approved' || canonicalWanRequest(snapshot.request) !== canonicalWanRequest(this.context.bootstrap.session.request)
      || snapshot.request_commitment !== this.context.bootstrap.session.request_commitment
      || snapshot.policy_revision !== grant.backend_policy_revision || snapshot.active_relay_generation !== 0
      || JSON.stringify(snapshot.approved_scopes) !== JSON.stringify(grant.approved_scopes)
      || JSON.stringify(canonicalProfile(snapshot.approved_profile)) !== JSON.stringify(canonicalProfile(grant.approved_profile))
      || Date.parse(snapshot.policy_expires_at) !== grant.policy_expires_at_ms || Date.parse(snapshot.grant_expires_at) <= Date.now()) throw new Error('远端授权不符合服务器当前会话策略');
  }
  private async refreshAuthorization(): Promise<void> {
    if (this.closing || this.polling) return;
    this.polling = true;
    try {
      const fresh = await this.context.getBootstrap();
      if (this.closing) return;
      if (this.grant) this.assertApprovedPolicy(this.grant.payload, fresh.session);
      else if (fresh.session.status === 'rejected') this.fail(new Error('远端拒绝了连接'), 'denied');
    } catch (error) { this.fail(error); }
    finally { this.polling = false; }
  }
  private async negotiate(grant: WireObject): Promise<void> {
    const access = await this.context.getRelayAccess();
    if (this.closing) return;
    await waitUntilRelayDirectoryIssued(access, this.signalWaitAbort.signal);
    if (this.closing) return;
    const config = await verifyBrowserRelayAccess(access, {
      grant, targetDeviceId: this.targetId, keyId: this.context.bootstrap.relay_directory_key_id,
      publicKey: this.context.bootstrap.relay_directory_public_key, nowMs: Date.now(),
    });
    if (this.closing) return;
    this.pc = new RTCPeerConnection(config);
    const pc = this.pc;
    const video = pc.addTransceiver('video', { direction: 'recvonly' });
    const capabilities = typeof RTCRtpReceiver !== 'undefined' ? RTCRtpReceiver.getCapabilities('video') : null;
    const h264 = capabilities?.codecs.filter(codec => codec.mimeType.toLowerCase() === 'video/h264' && /(?:^|;)packetization-mode=1(?:;|$)/.test(codec.sdpFmtpLine ?? ''));
    if (!h264?.length || typeof video.setCodecPreferences !== 'function') throw new Error('当前浏览器缺少可用的 H.264 WebRTC 解码能力');
    video.setCodecPreferences(h264);
    const reliable = pc.createDataChannel('ctrl_rel', { ordered: true });
    const realtime = pc.createDataChannel('ctrl_rt', { ordered: false, maxRetransmits: 0 });
    pc.createDataChannel('bulk', { ordered: true });
    this.controlChannels = { reliable, realtime };
    reliable.addEventListener('open', () => this.presentStreaming());
    realtime.addEventListener('open', () => this.presentStreaming());
    this.control = new BrowserRemoteControl({
      identity: this.context.identity, sourceDeviceId: this.controllerId, sessionId: this.sessionId,
      grantCommitment: this.grantCommitment!, targetDeviceId: this.targetId, targetKeyId: this.context.bootstrap.target_key_id,
      scopes: [...grant.approved_scopes], policyRevision: grant.backend_policy_revision,
      expiresAtMs: Math.min(grant.policy_expires_at_ms, this.context.bootstrap.expires_at_ms),
      isActive: () => !this.closing && this.routeVerified && this.controlReady() && this.state.phase === 'streaming' && this.pc?.connectionState === 'connected',
    }, { reliable, realtime }, error => this.fail(error));
    pc.addEventListener('track', event => {
      if (this.closing || event.track.kind !== 'video') return;
      this.stream = event.streams[0] ?? new MediaStream([event.track]);
      this.observer.onVideoStream(this.stream);
      event.track.addEventListener('ended', () => this.fail(new Error('远端画面已停止')));
    });
    pc.addEventListener('connectionstatechange', () => {
      if (this.closing) return;
      if (pc.connectionState === 'connected') void this.verifySelectedRoute().catch(error => this.fail(error));
      if (pc.connectionState === 'failed' || pc.connectionState === 'disconnected' || pc.connectionState === 'closed') this.fail(new Error('远端媒体连接已断开'));
    });
    this.negotiationDeadline = this.deadline(() => this.fail(new Error('远端 WebRTC 协商超时')), 25_000);
    const candidates: WireObject[] = [];
    pc.addEventListener('icecandidate', event => {
      if (!event.candidate?.candidate || this.closing) return;
      if (candidates.length >= MAX_CANDIDATES) { this.fail(new Error('本地候选数量超出限制')); return; }
      candidates.push({
        session_id: this.sessionId, controller_device_id: this.controllerId, target_device_id: this.targetId,
        grant_commitment: this.grantCommitment, description_role: 'offer', candidate: event.candidate.candidate,
        sdp_mid: event.candidate.sdpMid, sdp_mline_index: event.candidate.sdpMLineIndex,
        username_fragment: event.candidate.usernameFragment ?? null,
      });
    });
    await pc.setLocalDescription(await pc.createOffer());
    await this.waitForIce(pc);
    if (this.closing) return;
    if (!candidates.length || !pc.localDescription?.sdp) throw new Error('浏览器未收集到有效连接候选');
    const fingerprints: string[] = [];
    for (const candidate of candidates) {
      if (this.closing) return;
      candidate.candidate_fingerprint = await candidateFingerprint(candidate);
      if (this.closing) return;
      fingerprints.push(candidate.candidate_fingerprint);
    }
    if (new Set(fingerprints).size !== fingerprints.length) throw new Error('浏览器收集到重复连接候选');
    await this.send('webrtc_offer_v3', {
      session_id: this.sessionId, controller_device_id: this.controllerId, target_device_id: this.targetId,
      grant_commitment: this.grantCommitment, sdp: pc.localDescription.sdp, candidate_fingerprints: fingerprints.sort(),
    });
    for (const candidate of candidates) {
      if (this.closing) return;
      await this.send('webrtc_candidate_v3', candidate);
    }
  }
  private waitForIce(pc: RTCPeerConnection): Promise<void> {
    if (this.closing) return Promise.reject(new Error('网页连接已取消'));
    if (pc.iceGatheringState === 'complete') return Promise.resolve();
    return new Promise((resolve, reject) => {
      let finished = false;
      const cancel = () => { if (finished) return; cleanup(); reject(new Error('网页连接已取消')); };
      const done = () => {
        if (finished) return;
        if (this.closing) cancel();
        else if (pc.iceGatheringState === 'complete') { cleanup(); resolve(); }
      };
      const cleanup = () => {
        finished = true; clearTimeout(timer); this.timers.delete(timer); this.cancelWaits.delete(cancel);
        pc.removeEventListener('icegatheringstatechange', done);
      };
      const timer = this.deadline(() => { cleanup(); reject(new Error('浏览器 ICE 候选收集超时')); }, 12_000);
      this.cancelWaits.add(cancel);
      pc.addEventListener('icegatheringstatechange', done);
      done();
    });
  }
  private async applyAnswer(): Promise<void> {
    if (this.closing || !this.answer || !this.pc || this.pc.remoteDescription) return;
    const manifest = this.answer.candidate_fingerprints as string[];
    if ([...this.candidates.keys()].some(fingerprint => !manifest.includes(fingerprint))) throw new Error('远端 ICE 候选不属于签名描述');
    if (this.candidates.size !== manifest.length) return;
    await this.pc.setRemoteDescription({ type: 'answer', sdp: this.answer.sdp });
    for (const fingerprint of manifest) {
      if (this.closing) return;
      const candidate = this.candidates.get(fingerprint)!;
      await this.pc.addIceCandidate({ candidate: candidate.candidate, sdpMid: candidate.sdp_mid, sdpMLineIndex: candidate.sdp_mline_index, usernameFragment: candidate.username_fragment });
    }
  }
  private async verifySelectedRoute(): Promise<void> {
    const pc = this.pc;
    if (!pc || this.closing || !this.grant) return;
    const reports = await pc.getStats();
    if (this.closing) return;
    let pair: WireObject | undefined;
    reports.forEach(report => {
      if (report.type === 'transport' && report.selectedCandidatePairId) pair = reports.get(report.selectedCandidatePairId);
    });
    if (!pair) reports.forEach(report => { if (report.type === 'candidate-pair' && report.nominated && report.state === 'succeeded') pair = report; });
    if (!pair) throw new Error('无法验证实际 WebRTC 连接路径');
    const local = reports.get(pair.localCandidateId), remote = reports.get(pair.remoteCandidateId);
    if (!local?.candidateType || !remote?.candidateType) throw new Error('WebRTC 连接候选证据不完整');
    const relay = local.candidateType === 'relay' || remote.candidateType === 'relay';
    if (this.grant.payload.route_policy === 'relay_only' && (local.candidateType !== 'relay' || remote.candidateType !== 'relay')) throw new Error('连接路径不满足已授权的中继策略');
    this.routeVerified = true;
    this.publish({ route: relay ? 'relay' : 'direct' });
    this.presentStreaming();
  }
  private presentStreaming(): void {
    if (this.closing || !this.frame || !this.grant || !this.routeVerified || !this.controlReady() || this.pc?.connectionState !== 'connected' || this.grant.payload.policy_expires_at_ms <= Date.now()) return;
    if (this.negotiationDeadline) { clearTimeout(this.negotiationDeadline); this.timers.delete(this.negotiationDeadline); }
    this.publish({ phase: 'streaming', frameWidth: this.frame.width, frameHeight: this.frame.height, grantedScopes: [...this.grant.payload.approved_scopes] });
  }
  private controlReady(): boolean {
    if (!this.grant) return false;
    const scopes = this.grant.payload.approved_scopes as string[];
    if (scopes.includes('input.keyboard') || scopes.includes('input.pointer')) {
      if (this.controlChannels?.reliable.readyState !== 'open') return false;
    }
    if (scopes.includes('input.pointer') && this.controlChannels?.realtime.readyState !== 'open') return false;
    return true;
  }
  markVideoReady(width: number, height: number): void {
    if (!this.grant || this.closing || !this.stream || width <= 0 || height <= 0) return;
    safeInteger(width, 1, 16384); safeInteger(height, 1, 16384);
    this.frame = { width, height };
    this.presentStreaming();
  }
  async sendInput(input: ControlInputEvent): Promise<void> {
    if (this.closing || !this.control || this.state.phase !== 'streaming' || !this.routeVerified || !this.controlReady()) throw new Error('远端控制授权或媒体连接尚未就绪');
    await this.control.sendInput(input);
  }
  async sendPointerAction(position: { x: number; y: number }, input: ControlInputEvent): Promise<void> {
    if (this.closing || !this.control || this.state.phase !== 'streaming' || !this.routeVerified || !this.controlReady()) throw new Error('远端控制授权或媒体连接尚未就绪');
    await this.control.sendPointerAction(position, input);
  }
  async releaseInputs(_reason?: string): Promise<void> { if (this.control && !this.closing) await this.control.releaseAll(); }
  close(_reason?: string): Promise<void> {
    if (this.closePromise) return this.closePromise;
    this.closing = true;
    this.signalWaitAbort.abort();
    this.routeVerified = false;
    for (const cancel of [...this.cancelWaits]) cancel();
    this.cancelWaits.clear();
    if (!terminalPhases.has(this.state.phase)) this.publish({ phase: 'closed', grantedScopes: [] });
    for (const timer of this.timers) clearTimeout(timer);
    this.timers.clear();
    if (this.heartbeat) clearInterval(this.heartbeat);
    if (this.poll) clearInterval(this.poll);
    this.closePromise = (async () => {
      try {
        if (this.control) {
          let timer: ReturnType<typeof setTimeout> | undefined;
          await Promise.race([this.control.releaseAll().catch(() => undefined), new Promise<void>(resolve => { timer = setTimeout(resolve, 1750); })]);
          if (timer) clearTimeout(timer);
        }
        if (this.registered && this.socket?.readyState === 1) {
          let timer: ReturnType<typeof setTimeout> | undefined;
          // The final signal is best effort; it must not retain media or the backend
          // session while another WebCrypto operation is still in the send FIFO.
          await Promise.race([
            this.send('session_close', { session_id: this.sessionId, reason: 'unknown_session' }).catch(() => undefined),
            new Promise<void>(resolve => { timer = setTimeout(resolve, 500); }),
          ]);
          if (timer) clearTimeout(timer);
        }
      } finally {
        this.control?.close();
        this.stream?.getTracks().forEach(track => track.stop());
        this.pc?.close(); this.socket?.close();
        this.candidates.clear(); this.seenMessages.clear();
      }
      try { await this.context.closeBackend(); }
      catch (error) { this.publish({ error: '本地连接已关闭，未能确认远端会话已关闭' }); throw error; }
    })();
    return this.closePromise;
  }
}
