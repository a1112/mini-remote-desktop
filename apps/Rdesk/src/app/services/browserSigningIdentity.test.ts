import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { IDBFactory } from 'fake-indexeddb';
import type { BrowserSigningIdentity } from './browserRemoteProtocol';

const cryptoModuleName = 'node:crypto';
const { webcrypto } = await import(/* @vite-ignore */ cryptoModuleName) as { webcrypto: Crypto };

beforeEach(() => {
  vi.resetModules();
  vi.stubGlobal('crypto', webcrypto);
  vi.stubGlobal('indexedDB', new IDBFactory());
});
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });

async function loader() {
  const protocol = await import('./browserRemoteProtocol') as unknown as {
    loadBrowserSigningIdentity?: () => Promise<BrowserSigningIdentity>;
  };
  expect(typeof protocol.loadBrowserSigningIdentity).toBe('function');
  return protocol.loadBrowserSigningIdentity!;
}
async function reserver() {
  const protocol = await import('./browserRemoteProtocol') as unknown as {
    reserveBrowserSignalCounter?: (keyId: string) => Promise<number>;
  };
  expect(typeof protocol.reserveBrowserSignalCounter).toBe('function');
  return protocol.reserveBrowserSignalCounter!;
}
async function mutateStoredIdentity(change: (record: Record<string, unknown>) => unknown) {
  const databases = await indexedDB.databases();
  expect(databases).toHaveLength(1);
  const database = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open(databases[0]!.name!);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  try {
    await new Promise<void>((resolve, reject) => {
      const transaction = database.transaction(database.objectStoreNames[0]!, 'readwrite');
      const store = transaction.objectStore(database.objectStoreNames[0]!);
      const request = store.openCursor();
      request.onsuccess = () => {
        const cursor = request.result!;
        const next = change(cursor.value);
        if (next === undefined) cursor.delete();
        else cursor.update(next);
      };
      transaction.oncomplete = () => resolve();
      transaction.onabort = () => reject(transaction.error);
    });
  } finally { database.close(); }
}

describe('persistent browser controller signing identity', () => {
  it('aborts a late initial schema upgrade after opening has already timed out', async () => {
    const abort = vi.fn();
    const createObjectStore = vi.fn(() => ({ add: vi.fn() }));
    const request = { onupgradeneeded: undefined as ((event: IDBVersionChangeEvent) => void) | undefined,
      result: { objectStoreNames: { length: 0 }, createObjectStore }, transaction: { abort } };
    vi.stubGlobal('indexedDB', { open: () => request });
    vi.useFakeTimers();
    const load = await loader();
    const failure = expect(load()).rejects.toThrow('存储');
    await vi.waitFor(() => expect(typeof request.onupgradeneeded).toBe('function'));
    await vi.advanceTimersByTimeAsync(5_000);
    await failure;
    request.onupgradeneeded!({ oldVersion: 0 } as IDBVersionChangeEvent);
    expect(abort).toHaveBeenCalledOnce();
    expect(createObjectStore).not.toHaveBeenCalled();
  });
  it('reserves unique counters for concurrent sessions and retains the high-water mark after reload', async () => {
    const identity = await (await loader())();
    const reserve = await reserver();
    vi.resetModules();
    const other = await reserver();
    const counters = await Promise.all(Array.from({ length: 20 }, (_, index) => (index % 2 ? reserve : other)(identity.keyId)));
    expect(counters.sort((a, b) => a - b)).toEqual(Array.from({ length: 20 }, (_, index) => index + 1));
    vi.resetModules();
    expect(await (await reserver())(identity.keyId)).toBe(21);
  });
  it('does not reuse a committed reservation that was never sent', async () => {
    const identity = await (await loader())();
    const reserve = await reserver();
    expect(await reserve(identity.keyId)).toBe(1);
    expect(await reserve(identity.keyId)).toBe(2);
  });
  it.each([undefined, -1, 1.5, '4', Number.MAX_SAFE_INTEGER])('rejects a missing, malformed or exhausted counter: %s', async signal_counter => {
    const identity = await (await loader())();
    const reserve = await reserver();
    await mutateStoredIdentity(record => ({ ...record, signal_counter }));
    await expect(reserve(identity.keyId)).rejects.toThrow('计数');
  });
  it('rejects reservation for a different identity and does not advance the stored counter', async () => {
    const identity = await (await loader())();
    const reserve = await reserver();
    await expect(reserve('b'.repeat(64))).rejects.toThrow('计数');
    expect(await reserve(identity.keyId)).toBe(1);
  });
  it('rejects counter storage failure without any in-memory fallback', async () => {
    const identity = await (await loader())();
    const reserve = await reserver();
    vi.stubGlobal('indexedDB', undefined);
    await expect(reserve(identity.keyId)).rejects.toThrow('计数');
  });
  it('does not reserve from a malformed identity record even when its key ID and counter remain intact', async () => {
    const identity = await (await loader())();
    const reserve = await reserver();
    await mutateStoredIdentity(record => ({ ...record, privateKey: null }));
    await expect(reserve(identity.keyId)).rejects.toThrow('计数');
  });
  it('reuses the same non-exportable identity after the module is reloaded', async () => {
    const first = await (await loader())();
    expect(first.privateKey.extractable).toBe(false);
    await expect(crypto.subtle.exportKey('pkcs8', first.privateKey)).rejects.toThrow();
    vi.resetModules();
    const second = await (await loader())();
    expect(second.keyId).toBe(first.keyId);
    expect(second.publicKey).toEqual(first.publicKey);
    const challenge = new TextEncoder().encode('persisted signing key');
    const signature = await crypto.subtle.sign('Ed25519', second.privateKey, challenge);
    const publicKey = await crypto.subtle.importKey('raw', new Uint8Array(first.publicKey), 'Ed25519', false, ['verify']);
    expect(await crypto.subtle.verify('Ed25519', publicKey, signature, challenge)).toBe(true);
  });
  it('serializes competing same-origin initializations across independent module contexts', async () => {
    const firstLoader = await loader();
    vi.resetModules();
    const otherLoader = await loader();
    const identities = await Promise.all(Array.from({ length: 8 }, (_, index) => (index % 2 ? firstLoader : otherLoader)()));
    expect(new Set(identities.map(identity => identity.keyId)).size).toBe(1);
    vi.resetModules();
    expect((await (await loader())()).keyId).toBe(identities[0]!.keyId);
  });
  it('rejects unavailable storage before generating a replacement key', async () => {
    vi.stubGlobal('indexedDB', undefined);
    const generation = vi.spyOn(crypto.subtle, 'generateKey');
    await expect((await loader())()).rejects.toThrow('存储');
    expect(generation).not.toHaveBeenCalled();
  });
  it('rejects an opening failure instead of falling back to an ephemeral identity', async () => {
    vi.stubGlobal('indexedDB', { open: () => { throw new Error('blocked storage'); } });
    await expect((await loader())()).rejects.toThrow('存储');
  });
  it('rejects a deleted existing identity instead of silently changing controller keys', async () => {
    const load = await loader();
    await load();
    await mutateStoredIdentity(() => undefined);
    await expect(load()).rejects.toThrow('身份');
  });
  it('rejects a corrupted record instead of regenerating it', async () => {
    const load = await loader();
    await load();
    await mutateStoredIdentity(() => ({ malformed: true }));
    await expect(load()).rejects.toThrow('身份');
  });
  it('rejects a signing key that does not match its persisted public identity', async () => {
    const load = await loader();
    await load();
    const other = await crypto.subtle.generateKey('Ed25519', false, ['sign', 'verify']) as CryptoKeyPair;
    await mutateStoredIdentity(record => ({ ...record, privateKey: other.privateKey }));
    await expect(load()).rejects.toThrow('身份');
  });
  it('rejects extractable private keys stored by another writer', async () => {
    const load = await loader();
    await load();
    const other = await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify']) as CryptoKeyPair;
    await mutateStoredIdentity(record => ({ ...record, privateKey: other.privateKey }));
    await expect(load()).rejects.toThrow('身份');
  });
});
