import { describe, expect, it } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import * as adapterModule from './index';

const adapter = adapterModule as unknown as Record<string, (...args: any[]) => Promise<any>>;
const policy = { peer_key_id: 'a'.repeat(64), epoch: '2', trust_revision: '3', scopes: ['screen.view'], revision: '9007199254740993', enabled: true };
describe('formal paired access IPC contracts', () => {
  it('pairs the authenticated pending session without accepting a frontend public key', async () => {
    expect(typeof adapter.ipcPairPendingSession).toBe('function');
    const session = { session_id: 'pending-session' };
    getMockInvoke().mockResolvedValue({ type: 'ConsentRecorded', session });
    const request = { session_id: 'pending-session', approved_scopes: ['screen.view'], expected_policy_revision: '1' };
    expect(await adapter.ipcPairPendingSession!(request)).toEqual({ ok: true, value: session });
    expect(getMockInvoke()).toHaveBeenCalledWith('ipc_secure_remote', { request: { type: 'PairPendingSession', ...request } });
  });
  it('lists policies through the verified bridge and keeps revisions as decimal strings', async () => {
    expect(typeof adapter.ipcListPairedAccessPolicies).toBe('function');
    getMockInvoke().mockResolvedValue({ type: 'PairedAccessPolicies', policies: [policy] });
    expect(await adapter.ipcListPairedAccessPolicies!()).toEqual({ ok: true, value: [policy] });
    expect(getMockInvoke()).toHaveBeenCalledWith('ipc_secure_remote', { request: { type: 'ListPairedAccessPolicies' } });
  });
  it('revokes exactly the selected peer and policy revision', async () => {
    expect(typeof adapter.ipcRevokePairedAccess).toBe('function');
    getMockInvoke().mockResolvedValue({ type: 'PairedAccessPolicyUpdated', policy: { ...policy, enabled: false } });
    expect(await adapter.ipcRevokePairedAccess!(policy.peer_key_id, policy.revision)).toEqual({ ok: true, value: { ...policy, enabled: false } });
    expect(getMockInvoke()).toHaveBeenCalledWith('ipc_secure_remote', { request: { type: 'RevokePairedAccess', peer_key_id: policy.peer_key_id, expected_revision: policy.revision } });
  });
  it.each([{ policies: [policy, policy] }, { policies: [{ ...policy, owner_fingerprint: 'unexpected internal field' }] }])('rejects ambiguous or undeclared policy DTOs', async ({ policies }) => {
    getMockInvoke().mockResolvedValue({ type: 'PairedAccessPolicies', policies });
    expect(await adapter.ipcListPairedAccessPolicies!()).toMatchObject({ ok: false, error: { code: 'E_INVALID_RESPONSE' } });
  });
  it('rejects a pairing success response bound to another session', async () => {
    getMockInvoke().mockResolvedValue({ type: 'ConsentRecorded', session: { session_id: 'different-session' } });
    expect(await adapter.ipcPairPendingSession!({ session_id: 'pending', approved_scopes: ['screen.view'], expected_policy_revision: '1' })).toMatchObject({ ok: false, error: { code: 'E_INVALID_RESPONSE' } });
  });
  it('returns a formal denial and does not treat an unrelated response as approval', async () => {
    expect(typeof adapter.ipcPairPendingSession).toBe('function');
    getMockInvoke().mockResolvedValue({ type: 'RemoteAccessError', failure: { code: 'trust_required', message: '未配对' } });
    expect(await adapter.ipcPairPendingSession!({ session_id: 'pending', approved_scopes: ['screen.view'], expected_policy_revision: '1' })).toMatchObject({ ok: false, error: { message: '未配对' } });
    getMockInvoke().mockResolvedValue({ type: 'ConsentRecorded', session: {} });
    expect(await adapter.ipcListPairedAccessPolicies!()).toMatchObject({ ok: false });
  });
});
