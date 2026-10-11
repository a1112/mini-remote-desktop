import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import fixture from '../../../../realtime-server/tests/fixtures/browser_protocol_v3.json';
import * as wireModule from './browserRemoteProtocol';

const wire = wireModule;
const bytes = (value: Uint8Array | undefined) => value && Array.from(value);
const cryptoModuleName = 'node:crypto';
const { webcrypto } = await import(/* @vite-ignore */ cryptoModuleName) as { webcrypto: Crypto };

beforeEach(() => vi.stubGlobal('crypto', webcrypto));
afterEach(() => vi.unstubAllGlobals());

describe('browser v3 interoperability', () => {
  it('parses a bounded valid wire without changing signed values', () => {
    expect(wire.parseBoundedJson?.(fixture.intent_compact)).toEqual(fixture.intent);
  });
  it.each([
    '{"version":999,"version":3}',
    '{"message":{"type":"a","type":"b"}}',
    '{"claims":{"counter":1,"\\u0063ounter":2}}',
  ])('rejects ambiguous duplicate keys before typed verification: %s', text => {
    expect(typeof wire.parseBoundedJson).toBe('function');
    if (!wire.parseBoundedJson) return;
    expect(() => wire.parseBoundedJson!(text)).toThrow();
  });
  it('bounds wire size and nesting while allowing delimiter characters inside strings', () => {
    expect(typeof wire.parseBoundedJson).toBe('function');
    if (!wire.parseBoundedJson) return;
    expect(() => wire.parseBoundedJson!('['.repeat(70) + '0' + ']'.repeat(70))).toThrow();
    expect(() => wire.parseBoundedJson!(JSON.stringify({ text: 'a'.repeat(512 * 1024) }))).toThrow();
    expect(wire.parseBoundedJson!('{"text":"[{},]\\\"\\u0061"}')).toEqual({ text: '[{},]"a' });
  });
  it('frames UTF-8 contextual signing bytes with big-endian lengths', () => {
    const context = new TextEncoder().encode('测试');
    const payload = new TextEncoder().encode('{"a":1}');
    const prefix = new TextEncoder().encode('MRD_CONTEXT_SIGNATURE_V1');
    const expected = [...prefix, 0, 6, ...context, 0, 0, 0, 0, 0, 0, 0, 7, ...payload];
    expect(bytes(wire.contextSignatureBytes?.('测试', payload))).toEqual(expected);
  });

  it('serializes a request in Rust field order including every nullable profile field', () => {
    const reversed = Object.fromEntries(Object.entries(fixture.request).reverse());
    expect(wire.canonicalWanRequest?.(reversed)).toBe(fixture.request_compact);
  });

  it('matches the independent request and signed intent commitments', async () => {
    expect(await wire.wanRequestCommitment?.(fixture.request)).toBe(fixture.request_commitment);
    expect(await wire.signedSignalCommitment?.('intent', fixture.intent)).toBe(fixture.intent_commitment);
    expect(await wire.signedSignalCommitment?.('grant', fixture.grant)).toBe(fixture.grant_commitment);
  });

  it('matches the candidate fingerprint including nullable fields and their length frames', async () => {
    expect(await wire.candidateFingerprint?.(fixture.candidate.payload)).toBe(fixture.candidate_fingerprint);
  });

  it('verifies a target signed grant against the trusted key pin', async () => {
    expect(await wire.verifySignedSignal?.('session_grant_v3', fixture.grant, {
      peerDeviceId: fixture.request.controller_device_id,
      signerDeviceId: fixture.request.target_device_id,
      signerKeyId: fixture.grant.payload.claims.issuer_key_id,
      nowMs: fixture.now_ms,
    })).toEqual(fixture.grant.payload);
  });

  it.each(['issuer', 'peer', 'pin', 'future', 'expired'])('reports only bounded identity/time diagnostics for a %s rejection', async reason => {
    const claims = fixture.grant.payload.claims;
    const expected = {
      peerDeviceId: reason === 'peer' ? 'unexpected-peer' : fixture.request.controller_device_id,
      signerDeviceId: reason === 'issuer' ? 'unexpected-issuer' : fixture.request.target_device_id,
      signerKeyId: reason === 'pin' ? '0'.repeat(64) : claims.issuer_key_id,
      nowMs: reason === 'future' ? claims.issued_at_ms - 1 : reason === 'expired' ? claims.expires_at_ms : fixture.now_ms,
    };
    try {
      await wire.verifySignedSignal!('session_grant_v3', fixture.grant, expected);
      expect.fail('Expected strict identity/time rejection');
    } catch (error) {
      expect(error).toMatchObject({
        message: '远端消息身份或有效期不匹配',
        diagnostics: {
          message_type: 'session_grant_v3', issuer_matches: reason !== 'issuer',
          intended_peer_matches: reason !== 'peer', key_pin_matches: reason !== 'pin',
          issued_delta_ms: claims.issued_at_ms - expected.nowMs,
          expiry_remaining_ms: claims.expires_at_ms - expected.nowMs,
        },
      });
      const diagnostics = (error as { diagnostics: Record<string, unknown> }).diagnostics;
      expect(Object.keys(diagnostics).sort()).toEqual([
        'expiry_remaining_ms', 'intended_peer_matches', 'issued_delta_ms', 'issuer_matches', 'key_pin_matches', 'message_type',
      ]);
    }
  });

  it('rejects a modified signed grant and an unexpected signer key', async () => {
    const modified = structuredClone(fixture.grant);
    modified.payload.backend_policy_revision += 1;
    expect(typeof wire.verifySignedSignal).toBe('function');
    if (!wire.verifySignedSignal) return;
    await expect(wire.verifySignedSignal('session_grant_v3', modified, {
      peerDeviceId: fixture.request.controller_device_id,
      signerDeviceId: fixture.request.target_device_id,
      signerKeyId: fixture.grant.payload.claims.issuer_key_id,
      nowMs: fixture.now_ms,
    })).rejects.toThrow();
    await expect(wire.verifySignedSignal('session_grant_v3', fixture.grant, {
      peerDeviceId: fixture.request.controller_device_id,
      signerDeviceId: fixture.request.target_device_id,
      signerKeyId: '0'.repeat(64),
      nowMs: fixture.now_ms,
    })).rejects.toThrow();
  });

  it('encodes authenticated pointer, keyboard and per-scope release events', () => {
    expect(bytes(wire.authenticatedInputBytes?.({ kind: 'mouse_move', x: 123, y: -456 }))).toEqual([
      2, 1, 0, 8, 0, 0, 0, 123, 255, 255, 254, 56,
    ]);
    expect(bytes(wire.authenticatedInputBytes?.({ kind: 'key', key: { kind: 'virtual_key', code: 65 }, pressed: true }))).toEqual([
      2, 4, 0, 5, 0, 0, 0, 65, 1,
    ]);
    expect(bytes(wire.authenticatedInputBytes?.({ kind: 'release_all' }, 'input.keyboard'))).toEqual([2, 15, 0, 1, 2]);
  });

  it.each(['left', 'right', 'middle', 'x1', 'x2'] as const)('encodes %s with the target service zero-based button mapping', button => {
    const expected = { left: 0, right: 1, middle: 2, x1: 3, x2: 4 }[button];
    expect(bytes(wire.authenticatedInputBytes?.({ kind: 'mouse_button', button, pressed: true }))).toEqual([2, 2, 0, 2, expected, 1]);
  });

  it('rejects unknown request fields and unsafe numeric values before signing', () => {
    expect(typeof wire.canonicalWanRequest).toBe('function');
    if (!wire.canonicalWanRequest) return;
    expect(() => wire.canonicalWanRequest!({ ...fixture.request, other_device_id: 'attacker' })).toThrow();
    expect(() => wire.canonicalWanRequest!({ ...fixture.request, requested_profile: { ...fixture.request.requested_profile, fps: NaN } })).toThrow();
  });
});
