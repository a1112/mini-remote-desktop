import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import type { ControlInputEvent } from '../adapters/tauri/types';
import fixture from '../../../../realtime-server/tests/fixtures/browser_protocol_v3.json';
import { BrowserRemoteControl, type BrowserControlBinding } from './browserRemoteControl';
import { createBrowserSigningIdentity, exactBuffer, signContext, verifyContext, type BrowserSigningIdentity } from './browserRemoteProtocol';
import * as protocol from './browserRemoteProtocol';

const encoder = new TextEncoder();
const keyDown: ControlInputEvent = { kind: 'key', key: { kind: 'virtual_key', code: 65 }, pressed: true };
const keyUp: ControlInputEvent = { ...keyDown, pressed: false };
const buttonDown: ControlInputEvent = { kind: 'mouse_button', button: 'left', pressed: true };
let browser: BrowserSigningIdentity;
let target: BrowserSigningIdentity;
const instances: BrowserRemoteControl[] = [];

class TestChannel extends EventTarget {
  readyState: RTCDataChannelState = 'open';
  bufferedAmount = 0;
  bufferedAmountLowThreshold = 0;
  binaryType: BinaryType = 'arraybuffer';
  sent: Uint8Array[] = [];
  send(value: ArrayBuffer) { this.sent.push(new Uint8Array(value).slice()); }
  deliver(value: ArrayBuffer | Blob) { this.dispatchEvent(new MessageEvent('message', { data: value })); }
  drain() { this.bufferedAmount = 0; this.dispatchEvent(new Event('bufferedamountlow')); }
  disconnect() { this.readyState = 'closed'; this.dispatchEvent(new Event('close')); }
}

function frame(payload: unknown, lane: number, sequence: number, sessionId: string, rawJson?: string): Uint8Array {
  const session = encoder.encode(sessionId);
  const bytes = encoder.encode(rawJson ?? JSON.stringify(payload));
  const mux = new Uint8Array(38 + session.length + bytes.length);
  const view = new DataView(mux.buffer);
  mux.set(encoder.encode('MRMX')); mux[4] = 1; mux[5] = lane;
  view.setBigUint64(6, BigInt(sequence), true); view.setUint16(14, session.length, true);
  view.setUint32(34, bytes.length, true); mux.set(session, 38); mux.set(bytes, 38 + session.length);
  const result = new Uint8Array(21 + mux.length);
  const fragment = new DataView(result.buffer);
  result.set(encoder.encode('MRDF')); result[4] = 1;
  fragment.setBigUint64(5, BigInt(sequence), true); fragment.setUint16(15, 1, true);
  fragment.setUint32(17, mux.length, true); result.set(mux, 21);
  return result;
}
function parse(bytes: Uint8Array) {
  expect(new TextDecoder().decode(bytes.slice(0, 4))).toBe('MRDF');
  const f = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  expect(bytes[4]).toBe(1); expect(f.getUint16(13, true)).toBe(0); expect(f.getUint16(15, true)).toBe(1);
  expect(f.getUint32(17, true)).toBe(bytes.length - 21);
  const mux = bytes.slice(21); const v = new DataView(mux.buffer);
  expect(new TextDecoder().decode(mux.slice(0, 4))).toBe('MRMX'); expect(mux[4]).toBe(1);
  expect(mux[16]).toBe(0); expect(v.getBigUint64(17, true)).toBe(0n); expect(mux[25]).toBe(0);
  expect(v.getUint32(26, true)).toBe(0); expect(v.getUint32(30, true)).toBe(0);
  const sessionSize = v.getUint16(14, true);
  expect(mux.length).toBe(38 + sessionSize + v.getUint32(34, true));
  const sequence = Number(v.getBigUint64(6, true));
  expect(f.getBigUint64(5, true)).toBe(BigInt(sequence));
  return { sequence, lane: mux[5]!, sessionId: new TextDecoder().decode(mux.slice(38, 38 + sessionSize)), signed: JSON.parse(new TextDecoder().decode(mux.slice(38 + sessionSize))) };
}
async function commitment(signed: ReturnType<typeof parse>['signed']) {
  const signedBytes = encoder.encode(JSON.stringify({ schema_version: 2, kind: 'control_envelope', payload: signed.payload }));
  const bytes = new Uint8Array(signedBytes.length + 96);
  bytes.set(signedBytes); bytes.set(signed.public_key, signedBytes.length); bytes.set(signed.signature, signedBytes.length + 32);
  return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', exactBuffer(bytes))));
}
function setup(overrides: Partial<BrowserControlBinding> = {}) {
  const reliable = new TestChannel(); const realtime = new TestChannel(); const onError = vi.fn();
  const binding: BrowserControlBinding = {
    identity: browser, sourceDeviceId: 'browser_0123456789abcdef0123456789abcdef', sessionId: 'session-control',
    grantCommitment: 'ab'.repeat(32), targetDeviceId: '123456789012', targetKeyId: target.keyId,
    scopes: ['input.keyboard', 'input.pointer', 'screen.view'], policyRevision: 7,
    expiresAtMs: Date.now() + 30_000, isActive: () => true, ...overrides,
  };
  const control = new BrowserRemoteControl(binding, { reliable: reliable as unknown as RTCDataChannel, realtime: realtime as unknown as RTCDataChannel }, onError);
  instances.push(control);
  async function ack(channel: TestChannel, index = channel.sent.length - 1, patch: Record<string, unknown> = {}, signer = target, asBlob = false, transform?: (text: string) => string) {
    const request = parse(channel.sent[index]!);
    const p = request.signed.payload;
    const issuedAtMs = Date.now();
    const payload = {
      protocol_version: 2, session_id: p.session_id, grant_id: p.grant_id, source_key_id: p.source_key_id,
      target_key_id: p.target_key_id, sequence: p.sequence, event_id: p.event_id,
      request_commitment: await commitment(request.signed), accepted: true, reason: null,
      lane: p.authenticated_event_bytes[1] === 15 ? 'cleanup' : request.lane === 1 ? 'reliable' : 'realtime',
      event_count: 1, issued_at_ms: issuedAtMs, expires_at_ms: issuedAtMs + 2000, ...patch,
    };
    const signature = await signContext(signer, 'MRD_WAN_CONTROL_ACK_V1', encoder.encode(JSON.stringify({ schema_version: 2, kind: 'wan_control_ack', payload })));
    const signed = { payload, public_key: signer.publicKey, signature };
    const wire = frame(signed, request.lane, request.sequence, request.sessionId, transform?.(JSON.stringify(signed)));
    channel.deliver(asBlob ? new Blob([exactBuffer(wire)]) : exactBuffer(wire));
  }
  return { control, binding, reliable, realtime, onError, ack };
}
async function sent(channel: TestChannel, count: number) { await vi.waitFor(() => expect(channel.sent).toHaveLength(count)); }

beforeAll(async () => {
  const moduleName = 'node:crypto'; const { webcrypto } = await import(moduleName) as { webcrypto: Crypto };
  vi.stubGlobal('crypto', webcrypto);
  browser = await createBrowserSigningIdentity(); target = await createBrowserSigningIdentity();
});
afterEach(() => { instances.splice(0).forEach(control => control.close()); vi.restoreAllMocks(); });

describe('BrowserRemoteControl authenticated WebRTC control', () => {
  it('matches the independently generated Rust MRDF/MRMX golden header', async () => {
    const ctx = setup({ sessionId: fixture.transport.session_id }); const operation = ctx.control.sendInput(keyDown);
    await sent(ctx.reliable, 1);
    const actual = ctx.reliable.sent[0]!;
    const golden = new Uint8Array(fixture.transport.fragment_hex.match(/../g)!.map(value => Number.parseInt(value, 16)));
    const header = golden.slice(0, 59); const view = new DataView(header.buffer);
    view.setBigUint64(5, 1n, true); view.setUint32(17, actual.length - 21, true);
    view.setBigUint64(27, 1n, true); view.setUint32(55, actual.length - 59 - encoder.encode(ctx.binding.sessionId).length, true);
    expect(actual.slice(0, 59)).toEqual(header);
    await ctx.ack(ctx.reliable); await operation;
  });
  it('sends Rust-ordered signed envelopes and waits for the authentic target ACK', async () => {
    const ctx = setup(); const done = vi.fn(); const promise = ctx.control.sendInput(keyDown).then(done);
    await sent(ctx.reliable, 1); expect(done).not.toHaveBeenCalled();
    const { signed, lane, sequence, sessionId } = parse(ctx.reliable.sent[0]!);
    expect({ lane, sequence, sessionId }).toEqual({ lane: 1, sequence: 1, sessionId: ctx.binding.sessionId });
    expect(Object.keys(signed.payload)).toEqual(['protocol_version', 'session_id', 'grant_id', 'source_device_id', 'target_device_id', 'source_key_id', 'target_key_id', 'scope', 'sequence', 'event_id', 'issued_at_ms', 'expires_at_ms', 'policy_revision', 'authenticated_event_bytes']);
    expect(signed.payload).toMatchObject({ scope: 'InputKeyboard', source_device_id: ctx.binding.sourceDeviceId, grant_id: Array(32).fill(171), authenticated_event_bytes: [2, 4, 0, 5, 0, 0, 0, 65, 1] });
    expect(signed.payload.expires_at_ms - signed.payload.issued_at_ms).toBeLessThanOrEqual(2000);
    await verifyContext(signed.public_key, signed.signature, browser.keyId, 'MRD_LAN_CONTROL_ENVELOPE_V2', encoder.encode(JSON.stringify({ schema_version: 2, kind: 'control_envelope', payload: signed.payload })));
    await ctx.ack(ctx.reliable); await promise; expect(done).toHaveBeenCalledOnce();
  });
  it('serializes key and button transitions without advancing before ACK', async () => {
    const ctx = setup(); const down = ctx.control.sendInput(keyDown); const up = ctx.control.sendInput(keyUp);
    await sent(ctx.reliable, 1); await ctx.ack(ctx.reliable); await down;
    await sent(ctx.reliable, 2); expect(parse(ctx.reliable.sent[1]!).sequence).toBe(2);
    await ctx.ack(ctx.reliable); await up;
  });
  it('uses independent lane sequences and globally unique event IDs', async () => {
    const ctx = setup(); const key = ctx.control.sendInput(keyDown); const move = ctx.control.sendInput({ kind: 'mouse_move', x: -100, y: 200 });
    await sent(ctx.reliable, 1); await sent(ctx.realtime, 1);
    const a = parse(ctx.reliable.sent[0]!); const b = parse(ctx.realtime.sent[0]!);
    expect([a.sequence, b.sequence]).toEqual([1, 1]); expect(a.signed.payload.event_id).not.toBe(b.signed.payload.event_id);
    expect(b.signed.payload.authenticated_event_bytes).toEqual([2, 1, 0, 8, 255, 255, 255, 156, 0, 0, 0, 200]);
    await ctx.ack(ctx.reliable); await ctx.ack(ctx.realtime); await Promise.all([key, move]);
  });
  it('coalesces stale movement while preserving every wheel delta', async () => {
    const ctx = setup(); const operations: Promise<void>[] = [ctx.control.sendInput({ kind: 'mouse_move', x: 1, y: 1 })];
    await sent(ctx.realtime, 1);
    for (let x = 2; x <= 40; x++) operations.push(ctx.control.sendInput({ kind: 'mouse_move', x, y: x }));
    operations.push(ctx.control.sendInput({ kind: 'mouse_wheel', delta: 120 }));
    operations.push(ctx.control.sendInput({ kind: 'mouse_wheel', delta: -20 }));
    await ctx.ack(ctx.realtime); await sent(ctx.realtime, 2);
    expect(parse(ctx.realtime.sent[1]!).signed.payload.authenticated_event_bytes).toEqual([2, 1, 0, 8, 0, 0, 0, 40, 0, 0, 0, 40]);
    await ctx.ack(ctx.realtime); await sent(ctx.realtime, 3);
    const wheel1 = parse(ctx.realtime.sent[2]!).signed.payload.authenticated_event_bytes;
    await ctx.ack(ctx.realtime); await sent(ctx.realtime, 4);
    const wheel2 = parse(ctx.realtime.sent[3]!).signed.payload.authenticated_event_bytes;
    expect(new DataView(new Uint8Array(wheel1).buffer).getInt32(4) + new DataView(new Uint8Array(wheel2).buffer).getInt32(4)).toBe(100);
    await ctx.ack(ctx.realtime); await Promise.all(operations);
  });
  it('preserves atomic location ACK then click ACK ahead of later ordinary motion', async () => {
    const ctx = setup();
    const a = ctx.control.sendInput({ kind: 'mouse_move', x: 10, y: 10 }).catch(error => error);
    await sent(ctx.realtime, 1);
    const b = ctx.control.sendPointerAction({ x: 20, y: 20 }, buttonDown).catch(error => error);
    const c = ctx.control.sendInput({ kind: 'mouse_move', x: 30, y: 30 }).catch(error => error);
    expect(ctx.reliable.sent).toHaveLength(0);
    await ctx.ack(ctx.realtime); await sent(ctx.realtime, 2);
    expect(parse(ctx.realtime.sent[1]!).signed.payload.authenticated_event_bytes).toEqual([2, 1, 0, 8, 0, 0, 0, 20, 0, 0, 0, 20]);
    expect(ctx.reliable.sent).toHaveLength(0);
    await ctx.ack(ctx.realtime); await sent(ctx.reliable, 1);
    expect(parse(ctx.reliable.sent[0]!).signed.payload.authenticated_event_bytes).toEqual([2, 2, 0, 2, 0, 1]);
    expect(ctx.realtime.sent).toHaveLength(2);
    await ctx.ack(ctx.reliable); await sent(ctx.realtime, 3);
    expect(parse(ctx.realtime.sent[2]!).signed.payload.authenticated_event_bytes).toEqual([2, 1, 0, 8, 0, 0, 0, 30, 0, 0, 0, 30]);
    await ctx.ack(ctx.realtime); expect(await Promise.all([a, b, c])).toEqual([undefined, undefined, undefined]);
  });
  it('keeps wheel increments in atomic FIFO transactions and gates later motion', async () => {
    const ctx = setup();
    const a = ctx.control.sendPointerAction({ x: 20, y: 30 }, { kind: 'mouse_wheel', delta: 120 }).catch(error => error);
    const b = ctx.control.sendPointerAction({ x: 40, y: 50 }, { kind: 'mouse_horizontal_wheel', delta: -20 }).catch(error => error);
    const c = ctx.control.sendInput({ kind: 'mouse_move', x: 60, y: 70 }).catch(error => error);
    const types: number[] = []; const deltas: number[] = [];
    for (let index = 0; index < 5; index++) {
      await sent(ctx.realtime, index + 1);
      const bytes = parse(ctx.realtime.sent[index]!).signed.payload.authenticated_event_bytes;
      types.push(bytes[1]); if (bytes[1] === 3 || bytes[1] === 14) deltas.push(new DataView(new Uint8Array(bytes).buffer).getInt32(4));
      expect(ctx.realtime.sent).toHaveLength(index + 1); await ctx.ack(ctx.realtime);
    }
    expect(types).toEqual([1, 3, 1, 14, 1]); expect(deltas).toEqual([120, -20]);
    expect(await Promise.all([a, b, c])).toEqual([undefined, undefined, undefined]);
  });
  it('keeps keyboard input independent of a pending pointer transaction', async () => {
    const ctx = setup();
    const click = ctx.control.sendPointerAction({ x: 20, y: 30 }, buttonDown).catch(error => error);
    await sent(ctx.realtime, 1);
    const key = ctx.control.sendInput(keyDown).catch(error => error);
    await sent(ctx.reliable, 1); expect(parse(ctx.reliable.sent[0]!).signed.payload.authenticated_event_bytes[1]).toBe(4);
    await ctx.ack(ctx.reliable); expect(await key).toBeUndefined();
    await ctx.ack(ctx.realtime); await sent(ctx.reliable, 2); await ctx.ack(ctx.reliable);
    expect(await click).toBeUndefined();
  });
  it('releaseAll cancels pointer transactions after location without sending their action', async () => {
    const ctx = setup();
    const click = ctx.control.sendPointerAction({ x: 20, y: 30 }, buttonDown).catch(error => error);
    await sent(ctx.realtime, 1);
    const next = ctx.control.sendPointerAction({ x: 40, y: 50 }, { kind: 'mouse_wheel', delta: 120 }).catch(error => error);
    const release = ctx.control.releaseAll();
    expect(await click).toBeInstanceOf(Error); expect(await next).toBeInstanceOf(Error);
    await ctx.ack(ctx.realtime); await sent(ctx.reliable, 1);
    expect(parse(ctx.reliable.sent[0]!).signed.payload.authenticated_event_bytes[1]).toBe(15);
    await ctx.ack(ctx.reliable); await sent(ctx.reliable, 2); await ctx.ack(ctx.reliable); await release;
    expect(ctx.realtime.sent).toHaveLength(1); expect(ctx.onError).not.toHaveBeenCalled();
  });
  it('releaseAll removes a pointer action queued behind an outstanding keyboard ACK', async () => {
    const ctx = setup(); const key = ctx.control.sendInput(keyDown).catch(error => error);
    await sent(ctx.reliable, 1);
    const click = ctx.control.sendPointerAction({ x: 20, y: 30 }, buttonDown).catch(error => error);
    await sent(ctx.realtime, 1); await ctx.ack(ctx.realtime);
    await new Promise(resolve => setTimeout(resolve, 30));
    const release = ctx.control.releaseAll().catch(error => error);
    expect(await click).toBeInstanceOf(Error);
    await ctx.ack(ctx.reliable); expect(await key).toBeUndefined(); await sent(ctx.reliable, 2);
    expect(parse(ctx.reliable.sent[1]!).signed.payload.authenticated_event_bytes[1]).toBe(15);
    await ctx.ack(ctx.reliable); await sent(ctx.reliable, 3); await ctx.ack(ctx.reliable);
    expect(await release).toBeUndefined(); expect(ctx.onError).not.toHaveBeenCalled();
  });
  it('releaseAll cancels a pointer action whose signature is pending before it reaches the wire', async () => {
    let unblock!: () => void; const gate = new Promise<void>(resolve => { unblock = resolve; });
    const actualSign = protocol.signContext; const entered = vi.fn();
    vi.spyOn(protocol, 'signContext').mockImplementation(async (...args) => {
      if (args[1] === 'MRD_LAN_CONTROL_ENVELOPE_V2' && JSON.parse(new TextDecoder().decode(args[2])).payload.authenticated_event_bytes[1] === 2) { entered(); await gate; }
      return actualSign(...args);
    });
    const ctx = setup({ scopes: ['input.pointer'] });
    const click = ctx.control.sendPointerAction({ x: 20, y: 30 }, buttonDown).catch(error => error);
    await sent(ctx.realtime, 1); await ctx.ack(ctx.realtime); await vi.waitFor(() => expect(entered).toHaveBeenCalledOnce());
    const release = ctx.control.releaseAll().catch(error => error);
    try {
      expect(await click).toBeInstanceOf(Error);
      await sent(ctx.reliable, 1); expect(parse(ctx.reliable.sent[0]!).signed.payload.authenticated_event_bytes[1]).toBe(15);
      await ctx.ack(ctx.reliable); expect(await release).toBeUndefined(); expect(ctx.onError).not.toHaveBeenCalled();
    } finally { unblock(); }
  });
  it('close cancels active and queued pointer transactions without late action', async () => {
    const ctx = setup();
    const click = ctx.control.sendPointerAction({ x: 20, y: 30 }, buttonDown).catch(error => error);
    await sent(ctx.realtime, 1);
    const next = ctx.control.sendPointerAction({ x: 40, y: 50 }, buttonDown).catch(error => error);
    ctx.control.close();
    expect(await click).toBeInstanceOf(Error); expect(await next).toBeInstanceOf(Error);
    expect(ctx.reliable.sent).toHaveLength(0); expect(ctx.realtime.sent).toHaveLength(1); expect(ctx.onError).not.toHaveBeenCalled();
  });
  it('fails closed and cancels queued pointer actions on a denied position ACK', async () => {
    const ctx = setup();
    const click = ctx.control.sendPointerAction({ x: 20, y: 30 }, buttonDown).catch(error => error);
    await sent(ctx.realtime, 1);
    const next = ctx.control.sendPointerAction({ x: 40, y: 50 }, buttonDown).catch(error => error);
    await ctx.ack(ctx.realtime, 0, { accepted: false, reason: 'grant_revoked', lane: null, event_count: 0 });
    expect(await click).toBeInstanceOf(Error); expect(await next).toBeInstanceOf(Error);
    expect(ctx.reliable.sent).toHaveLength(0); expect(ctx.onError).toHaveBeenCalledOnce();
  });
  it('bounds queued atomic pointer actions instead of dropping clicks or wheel increments', async () => {
    const ctx = setup(); const pending = [ctx.control.sendPointerAction({ x: 20, y: 30 }, buttonDown).catch(error => error)];
    await sent(ctx.realtime, 1);
    for (let index = 0; index < 65; index++) pending.push(ctx.control.sendPointerAction({ x: index, y: index }, { kind: 'mouse_wheel', delta: 1 }).catch(error => error));
    expect(await Promise.all(pending)).toEqual(expect.arrayContaining([expect.any(Error)]));
    expect(ctx.onError).toHaveBeenCalledOnce(); expect(ctx.realtime.sent).toHaveLength(1); expect(ctx.reliable.sent).toHaveLength(0);
  });
  it('rejects non-pointer discrete actions and invalid location coordinates', async () => {
    const ctx = setup();
    await expect(ctx.control.sendPointerAction({ x: 1, y: 2 }, keyDown)).rejects.toThrow();
    await expect(ctx.control.sendPointerAction({ x: NaN, y: 2 }, buttonDown)).rejects.toThrow();
    expect(ctx.reliable.sent).toHaveLength(0); expect(ctx.realtime.sent).toHaveLength(0);
  });
  it('releases only granted scopes through the reliable cleanup lane', async () => {
    const ctx = setup({ scopes: ['input.keyboard', 'screen.view'] }); const release = ctx.control.releaseAll();
    await sent(ctx.reliable, 1);
    expect(parse(ctx.reliable.sent[0]!).signed.payload.authenticated_event_bytes).toEqual([2, 15, 0, 1, 2]);
    await ctx.ack(ctx.reliable); await release; expect(ctx.realtime.sent).toHaveLength(0); expect(ctx.reliable.sent).toHaveLength(1);
  });
  it('accepts signed zero-injection ACKs for idempotent or shared-owner key transitions', async () => {
    const ctx = setup(); const operation = ctx.control.sendInput(keyDown);
    await sent(ctx.reliable, 1); await ctx.ack(ctx.reliable, 0, { event_count: 0 }); await operation;
    expect(ctx.onError).not.toHaveBeenCalled();
  });
  it.each([0, 3])('accepts actual Rust cleanup injection count %d', async count => {
    const ctx = setup({ scopes: ['input.keyboard'] }); const release = ctx.control.releaseAll();
    await sent(ctx.reliable, 1); await ctx.ack(ctx.reliable, 0, { event_count: count }); await release;
    expect(ctx.onError).not.toHaveBeenCalled();
  });
  it('rejects an oversized cleanup injection count even with a matched target signature', async () => {
    const ctx = setup({ scopes: ['input.keyboard'] }); const release = ctx.control.releaseAll().catch(error => error);
    await sent(ctx.reliable, 1); await ctx.ack(ctx.reliable, 0, { event_count: 65537 });
    expect(await release).toBeInstanceOf(Error); expect(ctx.onError).toHaveBeenCalledOnce();
  });
  it('releaseAll cancels stale movement and follows outstanding key transitions', async () => {
    const ctx = setup(); const first = ctx.control.sendInput(keyDown); const move = ctx.control.sendInput({ kind: 'mouse_move', x: 1, y: 1 });
    await sent(ctx.reliable, 1); await sent(ctx.realtime, 1);
    const stale = ctx.control.sendInput({ kind: 'mouse_move', x: 2, y: 2 }).catch(error => error); const release = ctx.control.releaseAll();
    await ctx.ack(ctx.reliable); await first; await ctx.ack(ctx.realtime); await move;
    await sent(ctx.reliable, 2); expect(parse(ctx.reliable.sent[1]!).signed.payload.authenticated_event_bytes[1]).toBe(15);
    await ctx.ack(ctx.reliable); await sent(ctx.reliable, 3); await ctx.ack(ctx.reliable); await release;
    expect(await stale).toBeInstanceOf(Error); expect(ctx.realtime.sent).toHaveLength(1);
  });
  it.each([
    { scopes: ['screen.view'] }, { isActive: () => false }, { expiresAtMs: 1 },
  ])('checks grants, expiry and active binding before any send (%j)', async overrides => {
    const ctx = setup(overrides); await expect(ctx.control.sendInput(keyDown)).rejects.toThrow();
    expect(ctx.reliable.sent).toHaveLength(0); expect(ctx.realtime.sent).toHaveLength(0);
  });
  it('rechecks authorization for queued events', async () => {
    let active = true; const ctx = setup({ isActive: () => active }); const first = ctx.control.sendInput(keyDown); const next = ctx.control.sendInput(keyUp).catch(error => error);
    await sent(ctx.reliable, 1); active = false; await ctx.ack(ctx.reliable); await expect(first).rejects.toThrow();
    expect(await next).toBeInstanceOf(Error); expect(ctx.reliable.sent).toHaveLength(1);
  });
  it('accepts a signed ACK delivered as Blob', async () => {
    const ctx = setup(); const operation = ctx.control.sendInput(keyDown); await sent(ctx.reliable, 1);
    await ctx.ack(ctx.reliable, 0, {}, target, true); await operation;
  });
  it('uses one ACK timestamp snapshot even when the clock advances on every read', async () => {
    const baseTime = Date.now(); let ticks = 0;
    vi.spyOn(Date, 'now').mockImplementation(() => baseTime + ticks++);
    const ctx = setup(); const lifetimes: number[] = [];
    ctx.reliable.addEventListener('message', event => {
      const payload = parse(new Uint8Array((event as MessageEvent<ArrayBuffer>).data)).signed.payload;
      lifetimes.push(payload.expires_at_ms - payload.issued_at_ms);
    });
    for (let index = 0; index < 8; index++) {
      const operation = ctx.control.sendInput(index % 2 ? keyUp : keyDown).catch(error => error);
      await sent(ctx.reliable, index + 1); await ctx.ack(ctx.reliable);
      expect(await operation).toBeUndefined();
    }
    expect(lifetimes).toEqual(Array(8).fill(2000)); expect(ctx.onError).not.toHaveBeenCalled();
  });
  it.each([
    ['wrong commitment', { request_commitment: Array(32).fill(0) }],
    ['wrong grant', { grant_id: Array(32).fill(0) }],
    ['wrong event', { event_id: 99 }], ['wrong lane', { lane: 'realtime' }],
    ['wrong count', { event_count: 2 }], ['expired ACK', { issued_at_ms: 1, expires_at_ms: 2 }],
    ['negative ACK', { accepted: false, reason: 'grant_revoked', lane: null, event_count: 0 }],
  ])('rejects %s and freezes subsequent input', async (_label, patch) => {
    const ctx = setup(); const operation = ctx.control.sendInput(keyDown).catch(error => error);
    await sent(ctx.reliable, 1); await ctx.ack(ctx.reliable, 0, patch);
    expect(await operation).toBeInstanceOf(Error); expect(ctx.onError).toHaveBeenCalledOnce();
    await expect(ctx.control.sendInput(keyUp)).rejects.toThrow(); expect(ctx.reliable.sent).toHaveLength(1);
  });
  it('rejects an ACK signed by another key', async () => {
    const ctx = setup(); const operation = ctx.control.sendInput(keyDown).catch(error => error);
    await sent(ctx.reliable, 1); await ctx.ack(ctx.reliable, 0, {}, browser);
    expect(await operation).toBeInstanceOf(Error); expect(ctx.onError).toHaveBeenCalledOnce();
  });
  it.each([
    ['nested duplicate accepted', (text: string) => text.replace('"accepted":true', '"accepted":false,"accepted":true')],
    ['nested duplicate version', (text: string) => text.replace('"protocol_version":2', '"protocol_version":1,"protocol_version":2')],
    ['Unicode key alias', (text: string) => text.replace('"accepted":true', '"accep\\u0074ed":false,"accepted":true')],
    ['duplicate outer public key', (text: string) => text.replace('"public_key":', '"public_key":[],"public_key":')],
  ])('rejects ambiguous signed ACK JSON before canonical signature validation: %s', async (_label, transform) => {
    const ctx = setup(); const operation = ctx.control.sendInput(keyDown).catch(error => error);
    await sent(ctx.reliable, 1);
    // JSON.parse would retain the valid final values and the genuine target signature would verify.
    await ctx.ack(ctx.reliable, 0, {}, target, false, transform);
    expect(await operation).toBeInstanceOf(Error); expect(ctx.onError).toHaveBeenCalledOnce();
    await expect(ctx.control.sendInput(keyUp)).rejects.toThrow();
  });
  it('bounds ACK work synchronously while genuine target signature verification is pending', async () => {
    let unblock!: () => void;
    const gate = new Promise<void>(resolve => { unblock = resolve; });
    const actualVerify = protocol.verifyContext;
    const verification = vi.spyOn(protocol, 'verifyContext').mockImplementationOnce(async (...args) => { await gate; return actualVerify(...args); });
    const ctx = setup(); const operation = ctx.control.sendInput(keyDown).catch(error => error);
    let validAck!: ArrayBuffer;
    ctx.reliable.addEventListener('message', event => { validAck = (event as MessageEvent<ArrayBuffer>).data; }, { once: true });
    await sent(ctx.reliable, 1); await ctx.ack(ctx.reliable);
    await vi.waitFor(() => expect(verification).toHaveBeenCalledOnce());
    try {
      for (let index = 0; index < 16; index++) ctx.reliable.deliver(validAck.slice(0));
      // One verifying frame plus sixteen waiting frames crosses the sixteen-frame budget now.
      expect(ctx.onError).toHaveBeenCalledOnce();
      expect(await operation).toBeInstanceOf(Error);
      ctx.control.close();
    } finally { unblock(); }
    await new Promise(resolve => setTimeout(resolve, 30));
    expect(verification).toHaveBeenCalledOnce(); expect(ctx.onError).toHaveBeenCalledOnce();
  });
  it.each(['unsupported', 'oversized ArrayBuffer', 'oversized Blob'])('rejects invalid incoming ACK data immediately: %s', async variant => {
    const ctx = setup(); const operation = ctx.control.sendInput(keyDown).catch(error => error);
    await sent(ctx.reliable, 1);
    const data = variant === 'unsupported' ? 'not binary' : variant === 'oversized Blob' ? new Blob([new Uint8Array(5000)]) : new Uint8Array(5000).buffer;
    ctx.reliable.dispatchEvent(new MessageEvent('message', { data }));
    expect(ctx.onError).toHaveBeenCalledOnce(); expect(await operation).toBeInstanceOf(Error);
  });
  it('retries reliable input once with the exact signed bytes', async () => {
    const ctx = setup(); const operation = ctx.control.sendInput(keyDown); await sent(ctx.reliable, 1);
    await vi.waitFor(() => expect(ctx.reliable.sent).toHaveLength(2), { timeout: 1200 });
    expect(ctx.reliable.sent[1]).toEqual(ctx.reliable.sent[0]); await ctx.ack(ctx.reliable); await operation;
  });
  it('freezes reliable sequence after final ACK timeout instead of sending later keys', async () => {
    const ctx = setup(); const first = ctx.control.sendInput(keyDown).catch(error => error); const queued = ctx.control.sendInput(keyUp).catch(error => error);
    expect(await first).toBeInstanceOf(Error); expect(await queued).toBeInstanceOf(Error);
    expect(ctx.reliable.sent).toHaveLength(2); expect(ctx.reliable.sent[1]).toEqual(ctx.reliable.sent[0]); expect(ctx.onError).toHaveBeenCalledOnce();
  });
  it('waits for bounded channel backpressure without allocating a sequence', async () => {
    const ctx = setup(); ctx.reliable.bufferedAmount = 1_000_000; const operation = ctx.control.sendInput(keyDown);
    await new Promise(resolve => setTimeout(resolve, 30)); expect(ctx.reliable.sent).toHaveLength(0);
    ctx.reliable.drain(); await sent(ctx.reliable, 1); expect(parse(ctx.reliable.sent[0]!).sequence).toBe(1);
    await ctx.ack(ctx.reliable); await operation;
  });
  it('fails continued backpressure without sending input', async () => {
    const ctx = setup(); ctx.reliable.bufferedAmount = 1_000_000;
    await expect(ctx.control.sendInput(keyDown)).rejects.toThrow('拥塞');
    expect(ctx.reliable.sent).toHaveLength(0); expect(ctx.onError).toHaveBeenCalledOnce();
  });
  it('bounds reliable work and never drops a key transition silently', async () => {
    const ctx = setup(); const pending = [ctx.control.sendInput(keyDown).catch(error => error)];
    await sent(ctx.reliable, 1);
    for (let index = 0; index < 65; index++) pending.push(ctx.control.sendInput(index % 2 ? keyDown : keyUp).catch(error => error));
    expect(await Promise.all(pending)).toEqual(expect.arrayContaining([expect.any(Error)]));
    expect(ctx.onError).toHaveBeenCalledOnce(); expect(ctx.reliable.sent).toHaveLength(1);
  });
  it('ignores an authentic duplicate ACK without acknowledging the next key', async () => {
    const ctx = setup(); const down = ctx.control.sendInput(keyDown); const upDone = vi.fn();
    const up = ctx.control.sendInput(keyUp).then(upDone);
    await sent(ctx.reliable, 1); await ctx.ack(ctx.reliable); await down; await sent(ctx.reliable, 2);
    await ctx.ack(ctx.reliable, 0); await new Promise(resolve => setTimeout(resolve, 30));
    expect(upDone).not.toHaveBeenCalled(); await ctx.ack(ctx.reliable, 1); await up;
  });
  it('rejects new input during cleanup and enables it after confirmed release', async () => {
    const ctx = setup({ scopes: ['input.keyboard'] }); const release = ctx.control.releaseAll();
    await sent(ctx.reliable, 1); await expect(ctx.control.sendInput(keyDown)).rejects.toThrow('释放');
    await ctx.ack(ctx.reliable); await release;
    const operation = ctx.control.sendInput(keyDown); await sent(ctx.reliable, 2); await ctx.ack(ctx.reliable); await operation;
  });
  it('cancels signing, pending ACK and queued work when closed', async () => {
    const ctx = setup(); const first = ctx.control.sendInput(keyDown).catch(error => error); const second = ctx.control.sendInput(keyUp).catch(error => error);
    await sent(ctx.reliable, 1); ctx.control.close(); expect(await first).toBeInstanceOf(Error); expect(await second).toBeInstanceOf(Error);
    await new Promise(resolve => setTimeout(resolve, 30)); expect(ctx.reliable.sent).toHaveLength(1); expect(ctx.onError).not.toHaveBeenCalled();
  });
  it('cancels cleanup promptly even while an old signing operation remains unresolved', async () => {
    let unblock!: () => void;
    const gate = new Promise<void>(resolve => { unblock = resolve; });
    const actualSign = protocol.signContext;
    const signing = vi.spyOn(protocol, 'signContext').mockImplementationOnce(async (...args) => { await gate; return actualSign(...args); });
    const ctx = setup(); const move = ctx.control.sendInput({ kind: 'mouse_move', x: 1, y: 2 }).catch(error => error);
    await vi.waitFor(() => expect(signing).toHaveBeenCalledOnce());
    const finished = vi.fn(); const release = ctx.control.releaseAll().catch(error => { finished(error); return error; });
    ctx.control.close();
    try {
      await vi.waitFor(() => expect(finished).toHaveBeenCalledOnce(), { timeout: 100 });
      expect(await move).toBeInstanceOf(Error); expect(await release).toBeInstanceOf(Error);
    } finally { unblock(); }
    await new Promise(resolve => setTimeout(resolve, 30)); expect(ctx.realtime.sent).toHaveLength(0); expect(ctx.reliable.sent).toHaveLength(0);
  });
  it('fails pending work if the channel closes', async () => {
    const ctx = setup(); const operation = ctx.control.sendInput(keyDown).catch(error => error);
    await sent(ctx.reliable, 1); ctx.reliable.disconnect(); expect(await operation).toBeInstanceOf(Error); expect(ctx.onError).toHaveBeenCalledOnce();
  });
  it.each(['oversize', 'fragment', 'trailing bytes', 'session mismatch', 'lane mismatch'])('rejects malformed pending ACK frames: %s', async variant => {
    const ctx = setup(); const operation = ctx.control.sendInput(keyDown).catch(error => error); await sent(ctx.reliable, 1);
    let bytes = frame({}, 1, 1, ctx.binding.sessionId);
    if (variant === 'oversize') bytes = new Uint8Array(5000);
    if (variant === 'fragment') new DataView(bytes.buffer).setUint16(15, 2, true);
    if (variant === 'trailing bytes') { const longer = new Uint8Array(bytes.length + 1); longer.set(bytes); bytes = longer; }
    if (variant === 'session mismatch') bytes = frame({}, 1, 1, 'different-session');
    if (variant === 'lane mismatch') bytes = frame({}, 2, 1, ctx.binding.sessionId);
    ctx.reliable.deliver(exactBuffer(bytes)); expect(await operation).toBeInstanceOf(Error); expect(ctx.onError).toHaveBeenCalledOnce();
  });
});
