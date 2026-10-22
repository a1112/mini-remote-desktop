import type { ControlInputEvent } from '../adapters/tauri/types';
import {
  authenticatedInputBytes, boundedText, byteArray, exactBuffer, parseBoundedJson, safeInteger, sha256Hex,
  signContext, strictObject, verifyContext, type BrowserSigningIdentity,
} from './browserRemoteProtocol';

export interface BrowserControlBinding {
  identity: BrowserSigningIdentity;
  sourceDeviceId: string;
  sessionId: string;
  grantCommitment: string;
  targetDeviceId: string;
  targetKeyId: string;
  scopes: string[];
  policyRevision: number;
  expiresAtMs: number;
  isActive: () => boolean;
}

type Lane = 'reliable' | 'realtime';
type InputScope = 'input.keyboard' | 'input.pointer';
type SemanticLane = Lane | 'cleanup';
type QueuedInput = {
  bytes: Uint8Array;
  scope: InputScope;
  lane: SemanticLane;
  kind: ControlInputEvent['kind'];
  resolve: () => void;
  reject: (error: Error) => void;
  cancellation?: Error;
  cancelAwait?: (error: Error) => void;
  transmitted?: boolean;
};
type PointerWork = {
  position?: Uint8Array;
  input: Omit<QueuedInput, 'resolve' | 'reject'>;
  lane: Lane;
  resolve: () => void;
  reject: (error: Error) => void;
  cancellation?: Error;
};
type PendingAck = {
  sequence: number;
  eventId: number;
  commitment: number[];
  issuedAtMs: number;
  lane: SemanticLane;
  resolve: () => void;
  reject: (error: Error) => void;
  timer?: ReturnType<typeof setTimeout>;
  received: number;
};
type IncomingAck = { data: ArrayBuffer | Blob; size: number; counted: boolean };
type LaneState = {
  channel: RTCDataChannel;
  sequence: number;
  queue: QueuedInput[];
  running: boolean;
  current?: QueuedInput;
  pending?: PendingAck;
  inbox: IncomingAck[];
  receiving: boolean;
  activeMessage?: IncomingAck;
  idle: Set<() => void>;
};

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });
const MAX_PAYLOAD = 4096;
const MAX_FRAME_BYTES = 21 + 38 + 256 + MAX_PAYLOAD;
const MAX_INCOMING_ACKS = 16;
const MAX_QUEUE = 64;
const ACK_TIMEOUT = 750;
const HIGH_WATER = 64 * 1024;
const LOW_WATER = 16 * 1024;
const ACK_FIELDS = [
  'protocol_version', 'session_id', 'grant_id', 'source_key_id', 'target_key_id', 'sequence',
  'event_id', 'request_commitment', 'accepted', 'reason', 'lane', 'event_count', 'issued_at_ms', 'expires_at_ms',
] as const;
function protocolError(): Error { return new Error('远端控制确认的协议、身份或会话绑定无效'); }
function bytesEqual(a: number[], b: number[]): boolean { return a.length === b.length && a.every((value, index) => value === b[index]); }
function commitmentBytes(value: string): number[] {
  if (!/^[0-9a-f]{64}$/.test(value)) throw protocolError();
  return Array.from({ length: 32 }, (_, index) => Number.parseInt(value.slice(index * 2, index * 2 + 2), 16));
}
function encodeFrame(sessionId: string, lane: Lane, sequence: number, payload: Uint8Array): ArrayBuffer {
  if (!payload.length || payload.length > MAX_PAYLOAD) throw protocolError();
  const session = encoder.encode(sessionId);
  const frame = new Uint8Array(21 + 38 + session.length + payload.length);
  const fragment = new DataView(frame.buffer);
  frame.set(encoder.encode('MRDF')); frame[4] = 1;
  fragment.setBigUint64(5, BigInt(sequence), true); fragment.setUint16(15, 1, true);
  fragment.setUint32(17, frame.length - 21, true);
  const mux = frame.subarray(21); const view = new DataView(frame.buffer, 21);
  mux.set(encoder.encode('MRMX')); mux[4] = 1; mux[5] = lane === 'reliable' ? 1 : 2;
  view.setBigUint64(6, BigInt(sequence), true); view.setUint16(14, session.length, true);
  view.setUint32(34, payload.length, true); mux.set(session, 38); mux.set(payload, 38 + session.length);
  return frame.buffer;
}
function decodeFrame(bytes: Uint8Array, expectedSession: string, lane: Lane): { sequence: number; payload: unknown } {
  if (bytes.length < 59 || bytes.length > 21 + 38 + 256 + MAX_PAYLOAD) throw protocolError();
  const outer = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (decoder.decode(bytes.subarray(0, 4)) !== 'MRDF' || bytes[4] !== 1 || outer.getUint16(13, true) !== 0 || outer.getUint16(15, true) !== 1 || outer.getUint32(17, true) !== bytes.length - 21) throw protocolError();
  const mux = bytes.subarray(21); const view = new DataView(mux.buffer, mux.byteOffset, mux.byteLength);
  const sequence = safeInteger(Number(view.getBigUint64(6, true)), 1);
  const sessionLength = view.getUint16(14, true); const payloadLength = view.getUint32(34, true);
  if (decoder.decode(mux.subarray(0, 4)) !== 'MRMX' || mux[4] !== 1 || mux[5] !== (lane === 'reliable' ? 1 : 2) || outer.getBigUint64(5, true) !== BigInt(sequence) || mux[16] !== 0 || view.getBigUint64(17, true) !== 0n || mux[25] !== 0 || view.getUint32(26, true) !== 0 || view.getUint32(30, true) !== 0 || !sessionLength || sessionLength > 256 || !payloadLength || payloadLength > MAX_PAYLOAD || mux.length !== 38 + sessionLength + payloadLength || decoder.decode(mux.subarray(38, 38 + sessionLength)) !== expectedSession) throw protocolError();
  return { sequence, payload: parseBoundedJson(decoder.decode(mux.subarray(38 + sessionLength)), MAX_PAYLOAD) };
}

/** Signed input on the exact granted peer connection; never falls back to local IPC. */
export class BrowserRemoteControl {
  private readonly lanes: Record<Lane, LaneState>;
  private readonly grantId: number[];
  private nextEventId = 1;
  private closed = false;
  private fatal?: Error;
  private releasing?: Promise<void>;
  private readonly cancellations = new Set<(error: Error) => void>();
  private readonly listeners: Array<{ channel: RTCDataChannel; message: EventListener; close: EventListener }> = [];
  private incomingCount = 0;
  private incomingBytes = 0;
  private readonly pointerQueue: PointerWork[] = [];
  private pointerRunning = false;
  private currentPointer?: PointerWork;
  private readonly pointerIdle = new Set<() => void>();

  constructor(private readonly binding: BrowserControlBinding, channels: { reliable: RTCDataChannel; realtime: RTCDataChannel }, private readonly onError?: (error: Error) => void) {
    boundedText(binding.sessionId); boundedText(binding.sourceDeviceId); boundedText(binding.targetDeviceId);
    commitmentBytes(binding.targetKeyId); commitmentBytes(binding.identity.keyId);
    byteArray(binding.identity.publicKey, 32); safeInteger(binding.policyRevision, 1); safeInteger(binding.expiresAtMs, 1);
    this.grantId = commitmentBytes(binding.grantCommitment);
    const state = (channel: RTCDataChannel): LaneState => ({ channel, sequence: 1, queue: [], running: false, inbox: [], receiving: false, idle: new Set() });
    this.lanes = { reliable: state(channels.reliable), realtime: state(channels.realtime) };
    for (const lane of ['reliable', 'realtime'] as const) {
      const current = this.lanes[lane]; current.channel.binaryType = 'arraybuffer'; current.channel.bufferedAmountLowThreshold = LOW_WATER;
      const message: EventListener = event => this.acceptAck(lane, (event as MessageEvent<unknown>).data);
      const close: EventListener = () => this.fail(new Error('远程控制数据通道已断开'));
      current.channel.addEventListener('message', message); current.channel.addEventListener('close', close); current.channel.addEventListener('error', close);
      this.listeners.push({ channel: current.channel, message, close });
    }
  }

  async sendInput(input: ControlInputEvent): Promise<void> {
    if (input.kind === 'release_all') return this.releaseAll();
    this.assertActive();
    if (this.releasing) throw new Error('正在释放远端输入，请稍后再控制');
    const scope: InputScope = input.kind === 'key' ? 'input.keyboard' : 'input.pointer';
    this.assertActive(scope);
    const bytes = authenticatedInputBytes(input);
    const lane: Lane = input.kind === 'key' || input.kind === 'mouse_button' ? 'reliable' : 'realtime';
    const queued = { bytes, scope, lane, kind: input.kind };
    return scope === 'input.keyboard' ? this.enqueue(lane, queued) : this.enqueuePointer({ lane, input: queued });
  }

  async sendPointerAction(position: { x: number; y: number }, input: ControlInputEvent): Promise<void> {
    this.assertActive('input.pointer');
    if (this.releasing) throw new Error('正在释放远端输入，请稍后再控制');
    if (input.kind !== 'mouse_button' && input.kind !== 'mouse_wheel' && input.kind !== 'mouse_horizontal_wheel') throw new Error('指针定位事务仅支持按键或滚轮动作');
    const positionBytes = authenticatedInputBytes({ kind: 'mouse_move', x: position.x, y: position.y });
    const lane: Lane = input.kind === 'mouse_button' ? 'reliable' : 'realtime';
    return this.enqueuePointer({
      position: positionBytes, lane,
      input: { bytes: authenticatedInputBytes(input), scope: 'input.pointer', lane, kind: input.kind },
    });
  }

  releaseAll(): Promise<void> {
    if (this.releasing) return this.releasing;
    try { this.assertActive(); } catch (error) { return Promise.reject(error); }
    const operation = this.performRelease();
    this.releasing = operation.finally(() => { this.releasing = undefined; });
    return this.releasing;
  }

  /** The peer owns channel closure. It should releaseAll before calling close. */
  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.cancel(new Error('远程控制通道已关闭'));
    for (const { channel, message, close } of this.listeners) {
      channel.removeEventListener('message', message); channel.removeEventListener('close', close); channel.removeEventListener('error', close);
    }
    this.listeners.length = 0;
  }

  private assertActive(scope?: InputScope): void {
    if (this.closed) throw new Error('远程控制通道已关闭');
    if (this.fatal) throw this.fatal;
    if (!this.binding.isActive() || Date.now() >= this.binding.expiresAtMs) throw new Error('远程控制授权已失效');
    if (scope && !this.binding.scopes.includes(scope)) throw new Error('远端未授权此输入权限');
  }

  private async performRelease(): Promise<void> {
    // Cancel unsent actions, but retain the ACK barrier for any position/action already transmitted.
    this.cancelPointerWork(new Error('待发送指针输入已取消'), false);
    await this.waitPointerIdle();
    for (const scope of ['input.pointer', 'input.keyboard'] as const) {
      if (!this.binding.scopes.includes(scope)) continue;
      this.assertActive(scope);
      await this.enqueue('reliable', { bytes: authenticatedInputBytes({ kind: 'release_all' }, scope), scope, lane: 'cleanup', kind: 'release_all' });
    }
  }

  private enqueue(lane: Lane, input: Omit<QueuedInput, 'resolve' | 'reject'>): Promise<void> {
    const state = this.lanes[lane];
    return new Promise((resolve, reject) => {
      if (state.queue.length >= MAX_QUEUE) {
        const error = new Error('远程输入队列已满，控制已停止'); reject(error); this.fail(error); return;
      }
      state.queue.push({ ...input, resolve, reject });
      if (!state.running) void this.pump(lane);
    });
  }

  private enqueuePointer(work: Omit<PointerWork, 'resolve' | 'reject' | 'cancellation'>): Promise<void> {
    return new Promise((resolve, reject) => {
      const last = this.pointerQueue[this.pointerQueue.length - 1];
      // Only ordinary motion can be discarded. A transaction position always retains its real ACK.
      if (!work.position && work.input.kind === 'mouse_move' && last && !last.position && last.input.kind === 'mouse_move') {
        this.pointerQueue.pop(); last.resolve();
      }
      if (this.pointerQueue.length >= MAX_QUEUE) {
        const error = new Error('远程指针事务队列已满，控制已停止'); reject(error); this.fail(error); return;
      }
      this.pointerQueue.push({ ...work, resolve, reject });
      if (!this.pointerRunning) void this.pumpPointer();
    });
  }

  private async pumpPointer(): Promise<void> {
    this.pointerRunning = true;
    try {
      while (this.pointerQueue.length) {
        const work = this.pointerQueue.shift()!; this.currentPointer = work;
        try {
          this.assertPointerWork(work);
          if (work.position) {
            await this.enqueue('realtime', { bytes: work.position, scope: 'input.pointer', lane: 'realtime', kind: 'mouse_move' });
            this.assertPointerWork(work);
          }
          await this.enqueue(work.lane, work.input);
          this.assertPointerWork(work); work.resolve();
        } catch (error) {
          const failure = error instanceof Error ? error : new Error(String(error)); work.reject(failure);
          if (!work.cancellation) this.fail(failure);
        } finally { this.currentPointer = undefined; }
      }
    } finally { this.pointerRunning = false; this.pointerIdle.forEach(resolve => resolve()); this.pointerIdle.clear(); }
  }

  private assertPointerWork(work: PointerWork): void {
    if (work.cancellation) throw work.cancellation;
    this.assertActive('input.pointer');
  }

  private cancelPointerWork(error: Error, includeActiveOrdinaryInput = true): void {
    if (this.currentPointer && (includeActiveOrdinaryInput || this.currentPointer.position)) {
      const work = this.currentPointer;
      work.cancellation = error; work.reject(error);
      // A transaction can be awaiting a reliable keyboard barrier or asynchronous signing. Remove
      // only its unsent lane work; anything already transmitted retains its authentic ACK barrier.
      const belongsToWork = (input: QueuedInput) => input.bytes === work.input.bytes || input.bytes === work.position;
      for (const state of Object.values(this.lanes)) {
        for (let index = state.queue.length - 1; index >= 0; index--) {
          const input = state.queue[index]!;
          if (belongsToWork(input)) { state.queue.splice(index, 1); input.cancellation = error; input.reject(error); }
        }
        const current = state.current;
        if (current && belongsToWork(current) && !current.transmitted) {
          current.cancellation = error; current.cancelAwait?.(error); current.reject(error);
        }
      }
    }
    this.pointerQueue.splice(0).forEach(work => { work.cancellation = error; work.reject(error); });
  }

  private waitPointerIdle(): Promise<void> {
    if (!this.pointerRunning) return Promise.resolve();
    return new Promise(resolve => this.pointerIdle.add(resolve));
  }

  private async pump(lane: Lane): Promise<void> {
    const state = this.lanes[lane]; state.running = true;
    try {
      while (state.queue.length) {
        const input = state.queue.shift()!; state.current = input;
        try { await this.transmit(lane, input); input.resolve(); }
        catch (error) {
          const failure = error instanceof Error ? error : new Error(String(error)); input.reject(failure);
          if (!input.cancellation) { this.fail(failure); break; }
        }
        finally { state.current = undefined; }
      }
    } finally { state.running = false; state.idle.forEach(resolve => resolve()); state.idle.clear(); }
  }

  private async transmit(lane: Lane, input: QueuedInput): Promise<void> {
    const state = this.lanes[lane]; this.assertQueuedInput(input);
    await this.waitWritable(state.channel, input); this.assertQueuedInput(input);
    const issuedAtMs = Date.now();
    const payload = {
      protocol_version: 2, session_id: this.binding.sessionId, grant_id: this.grantId,
      source_device_id: this.binding.sourceDeviceId, target_device_id: this.binding.targetDeviceId,
      source_key_id: this.binding.identity.keyId, target_key_id: this.binding.targetKeyId,
      scope: input.scope === 'input.keyboard' ? 'InputKeyboard' : 'InputPointer', sequence: safeInteger(state.sequence, 1),
      event_id: safeInteger(this.nextEventId++, 1), issued_at_ms: issuedAtMs,
      expires_at_ms: Math.min(issuedAtMs + 2000, this.binding.expiresAtMs), policy_revision: this.binding.policyRevision,
      authenticated_event_bytes: Array.from(input.bytes),
    };
    const signingBytes = encoder.encode(JSON.stringify({ schema_version: 2, kind: 'control_envelope', payload }));
    const signature = await this.awaitCancellable(input, signContext(this.binding.identity, 'MRD_LAN_CONTROL_ENVELOPE_V2', signingBytes));
    this.assertQueuedInput(input);
    const publicKey = byteArray(this.binding.identity.publicKey, 32);
    const commitmentInput = new Uint8Array(signingBytes.length + 96);
    commitmentInput.set(signingBytes); commitmentInput.set(publicKey, signingBytes.length); commitmentInput.set(signature, signingBytes.length + 32);
    const commitment = commitmentBytes(await this.awaitCancellable(input, sha256Hex(commitmentInput)));
    this.assertQueuedInput(input);
    const frame = encodeFrame(this.binding.sessionId, lane, payload.sequence, encoder.encode(JSON.stringify({ payload, public_key: publicKey, signature })));
    await new Promise<void>((resolve, reject) => {
      const pending: PendingAck = { sequence: payload.sequence, eventId: payload.event_id, issuedAtMs, commitment, lane: input.lane, resolve, reject, received: 0 };
      state.pending = pending;
      let attempts = 0;
      const send = () => {
        try {
          this.assertQueuedInput(input);
          if (Date.now() >= payload.expires_at_ms || state.channel.readyState !== 'open' || state.channel.bufferedAmount > HIGH_WATER) throw new Error('远程控制通道不可写或输入已过期');
          attempts++;
          pending.timer = setTimeout(() => {
            if (state.pending !== pending || this.closed || this.fatal) return;
            if (attempts < 2) send(); else this.rejectPending(state, new Error('远端控制确认超时，控制已停止'));
          }, ACK_TIMEOUT);
          // Retry preserves signature, event ID and sequence exactly; no later event skips this barrier.
          input.transmitted = true; state.channel.send(frame);
        } catch (error) { this.rejectPending(state, error instanceof Error ? error : new Error(String(error))); }
      };
      send();
    });
    this.assertActive(input.scope); state.sequence++;
  }

  private assertQueuedInput(input: QueuedInput): void {
    if (input.cancellation) throw input.cancellation;
    this.assertActive(input.scope);
  }

  private awaitCancellable<T>(input: QueuedInput, operation: Promise<T>): Promise<T> {
    return new Promise((resolve, reject) => {
      const cancel = (error: Error) => { if (input.cancelAwait === cancel) input.cancelAwait = undefined; reject(error); };
      input.cancelAwait = cancel;
      operation.then(value => {
        if (input.cancelAwait === cancel) input.cancelAwait = undefined;
        resolve(value);
      }, error => {
        if (input.cancelAwait === cancel) input.cancelAwait = undefined;
        reject(error);
      });
      if (input.cancellation) cancel(input.cancellation);
    });
  }

  private async receive(lane: Lane, data: unknown): Promise<void> {
    const state = this.lanes[lane]; const pending = state.pending;
    if (this.closed || this.fatal || !pending) return;
    if (++pending.received > 16) throw protocolError();
    let bytes: Uint8Array;
    if (data instanceof ArrayBuffer) bytes = new Uint8Array(data);
    else if (data instanceof Blob) {
      if (data.size > MAX_FRAME_BYTES) throw protocolError();
      bytes = new Uint8Array(await data.arrayBuffer());
    } else throw protocolError();
    if (state.pending !== pending || this.closed || this.fatal) return;
    const frame = decodeFrame(bytes, this.binding.sessionId, lane);
    // A duplicate ACK from an exact retry cannot satisfy a later event.
    if (frame.sequence < pending.sequence) return;
    if (frame.sequence !== pending.sequence) throw protocolError();
    const signed = strictObject(frame.payload, ['payload', 'public_key', 'signature']);
    const payload = strictObject(signed.payload, ACK_FIELDS);
    const now = Date.now(); this.assertActive();
    const issued = safeInteger(payload.issued_at_ms); const expires = safeInteger(payload.expires_at_ms, issued + 1, issued + 2000);
    if (payload.protocol_version !== 2 || payload.session_id !== this.binding.sessionId || !bytesEqual(byteArray(payload.grant_id, 32), this.grantId) || payload.source_key_id !== this.binding.identity.keyId || payload.target_key_id !== this.binding.targetKeyId || payload.sequence !== pending.sequence || payload.event_id !== pending.eventId || !bytesEqual(byteArray(payload.request_commitment, 32), pending.commitment) || issued > now + 2000 || issued < pending.issuedAtMs - 2000 || now > expires || typeof payload.accepted !== 'boolean') throw protocolError();
    await verifyContext(byteArray(signed.public_key, 32), byteArray(signed.signature, 64), this.binding.targetKeyId, 'MRD_WAN_CONTROL_ACK_V1', encoder.encode(JSON.stringify({ schema_version: 2, kind: 'wan_control_ack', payload })));
    if (state.pending !== pending || this.closed || this.fatal) return;
    this.assertActive();
    if (!payload.accepted) {
      if (payload.lane !== null || payload.event_count !== 0 || typeof payload.reason !== 'string') throw protocolError();
      throw new Error(`远端拒绝控制输入：${boundedText(payload.reason, 128)}`);
    }
    if (payload.reason !== null || payload.lane !== pending.lane) throw protocolError();
    // Rust reports physical injections, rather than acknowledged envelope count. Idempotent
    // transitions and shared ownership inject zero; scoped cleanup can release multiple held inputs.
    safeInteger(payload.event_count, 0, pending.lane === 'cleanup' ? 65536 : 1);
    clearTimeout(pending.timer); state.pending = undefined; pending.resolve();
  }

  private acceptAck(lane: Lane, data: unknown): void {
    if (this.closed || this.fatal) return;
    const state = this.lanes[lane];
    // Account at the event boundary, before any asynchronous Blob read or signature verification.
    // The budget includes the currently verifying frame and is shared across both control lanes.
    const size = data instanceof ArrayBuffer ? data.byteLength : data instanceof Blob ? data.size : -1;
    if (size < 0 || size > MAX_FRAME_BYTES) { this.fail(protocolError()); return; }
    if (!state.pending) return;
    if (this.incomingCount >= MAX_INCOMING_ACKS || this.incomingBytes + size > MAX_INCOMING_ACKS * MAX_FRAME_BYTES) {
      this.fail(new Error('远端控制确认队列超过容量，控制已停止')); return;
    }
    const incoming: IncomingAck = { data: data as ArrayBuffer | Blob, size, counted: true };
    this.incomingCount++; this.incomingBytes += size; state.inbox.push(incoming);
    if (!state.receiving) void this.consumeAcks(lane);
  }

  private async consumeAcks(lane: Lane): Promise<void> {
    const state = this.lanes[lane]; state.receiving = true;
    try {
      while (!this.closed && !this.fatal && state.inbox.length) {
        const incoming = state.inbox.shift()!; state.activeMessage = incoming;
        try { await this.receive(lane, incoming.data); }
        catch (error) { this.fail(error); }
        finally { this.releaseAck(incoming); state.activeMessage = undefined; }
      }
    } finally { state.receiving = false; }
  }

  private releaseAck(incoming: IncomingAck): void {
    // cancel() can release a frame while WebCrypto is still in flight; finally must release it once.
    if (!incoming.counted) return;
    incoming.counted = false; this.incomingCount--; this.incomingBytes -= incoming.size;
  }

  private waitWritable(channel: RTCDataChannel, input: QueuedInput): Promise<void> {
    if (channel.readyState !== 'open') return Promise.reject(new Error('远程控制数据通道尚未就绪'));
    if (channel.bufferedAmount <= HIGH_WATER) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const finish = (error?: Error) => {
        clearTimeout(timer); channel.removeEventListener('bufferedamountlow', drained); this.cancellations.delete(cancel);
        if (input.cancelAwait === cancel) input.cancelAwait = undefined;
        if (error) reject(error); else resolve();
      };
      const cancel = (error: Error) => finish(error);
      const drained = () => { if (channel.bufferedAmount <= HIGH_WATER) finish(); };
      const timer = setTimeout(() => finish(new Error('远程控制通道持续拥塞，控制已停止')), ACK_TIMEOUT);
      channel.addEventListener('bufferedamountlow', drained); this.cancellations.add(cancel); input.cancelAwait = cancel;
      drained();
    });
  }

  private rejectPending(state: LaneState, error: Error): void {
    const pending = state.pending; if (!pending) return;
    clearTimeout(pending.timer); state.pending = undefined; pending.reject(error);
  }

  private cancel(error: Error): void {
    this.cancelPointerWork(error);
    // WebCrypto cannot be cancelled; let cleanup observe closed/fatal without awaiting a late result.
    this.pointerIdle.forEach(resolve => resolve()); this.pointerIdle.clear();
    for (const state of Object.values(this.lanes)) {
      this.rejectPending(state, error); state.current?.reject(error);
      state.queue.splice(0).forEach(input => input.reject(error));
      if (state.activeMessage) this.releaseAck(state.activeMessage);
      state.inbox.splice(0).forEach(incoming => this.releaseAck(incoming));
      // WebCrypto itself is not cancellable; cleanup must not wait for a late signature to finish.
      state.idle.forEach(resolve => resolve()); state.idle.clear();
    }
    this.cancellations.forEach(cancel => cancel(error)); this.cancellations.clear();
  }

  private fail(error: unknown): void {
    if (this.closed || this.fatal) return;
    this.fatal = error instanceof Error ? error : new Error(String(error)); this.cancel(this.fatal);
    // Final ACK loss leaves reliable acceptance unknown. The peer must close so the target releases
    // its owned input; sending a different cleanup at the unresolved sequence would violate replay rules.
    try { this.onError?.(this.fatal); } catch { /* A consumer callback cannot leave internal work pending. */ }
  }
}
