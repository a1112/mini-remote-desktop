import type { BrowserSigningIdentity } from './browserRemoteProtocol';

const DATABASE_NAME = 'mrd-browser-controller-identity-v1';
const STORE_NAME = 'identity';
const SLOT = 'controller';
const STORAGE_ERROR = '浏览器身份存储不可用，无法使用已配对连接';
const IDENTITY_ERROR = '浏览器身份记录损坏或不匹配，请在远端重新配对';
const COUNTER_ERROR = '浏览器信令计数存储不可用、损坏或已耗尽，连接已停止';
let loading: Promise<BrowserSigningIdentity> | undefined;
type IdentityRecord = BrowserSigningIdentity & { schema_version: number; signal_counter: number };

function validIdentityRecord(record: unknown): record is IdentityRecord {
  if (!record || typeof record !== 'object') return false;
  const value = record as IdentityRecord;
  return Object.keys(value).sort().join(',') === 'keyId,privateKey,publicKey,schema_version,signal_counter'
    && value.schema_version === 1 && Array.isArray(value.publicKey) && value.publicKey.length === 32
    && Number.isSafeInteger(value.signal_counter) && value.signal_counter >= 0
    && value.publicKey.every(byte => Number.isInteger(byte) && byte >= 0 && byte <= 255)
    && typeof value.keyId === 'string' && /^[a-f0-9]{64}$/.test(value.keyId)
    && value.privateKey?.type === 'private' && value.privateKey.extractable === false
    && value.privateKey.algorithm?.name === 'Ed25519'
    && value.privateKey.usages.length === 1 && value.privateKey.usages[0] === 'sign';
}

async function generateCandidate(): Promise<BrowserSigningIdentity> {
  const pair = await crypto.subtle.generateKey('Ed25519', false, ['sign', 'verify']) as CryptoKeyPair;
  const raw = new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey));
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', raw));
  return { publicKey: Array.from(raw), keyId: Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join(''), privateKey: pair.privateKey };
}

function openIdentityDatabase(candidate?: BrowserSigningIdentity): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const fail = () => { if (!settled) { settled = true; clearTimeout(timer); reject(new Error(STORAGE_ERROR)); } };
    const timer = setTimeout(fail, 5_000);
    try {
      const request = indexedDB.open(DATABASE_NAME, 1);
      request.onblocked = fail;
      request.onerror = fail;
      request.onupgradeneeded = event => {
        // Upgrade transactions serialize first creation across tabs. Generate
        // candidates beforehand: awaiting WebCrypto here would close the transaction.
        if (settled || !candidate || (event as IDBVersionChangeEvent).oldVersion !== 0 || request.result.objectStoreNames.length !== 0) {
          request.transaction?.abort(); return;
        }
        request.result.createObjectStore(STORE_NAME).add({ schema_version: 1, signal_counter: 0, ...candidate }, SLOT);
      };
      request.onsuccess = () => {
        const database = request.result;
        if (settled) { database.close(); return; }
        if (database.objectStoreNames.length !== 1 || !database.objectStoreNames.contains(STORE_NAME)) {
          database.close(); fail(); return;
        }
        settled = true; clearTimeout(timer); resolve(database);
      };
    } catch { fail(); }
  });
}

async function validateIdentity(record: unknown): Promise<BrowserSigningIdentity> {
  try {
    if (!validIdentityRecord(record)) throw new Error();
    const value = record;
    const raw = new Uint8Array(value.publicKey);
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', raw));
    if (Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('') !== value.keyId) throw new Error();
    const challenge = crypto.getRandomValues(new Uint8Array(32));
    const signature = await crypto.subtle.sign('Ed25519', value.privateKey, challenge);
    const publicKey = await crypto.subtle.importKey('raw', raw, 'Ed25519', false, ['verify']);
    if (!await crypto.subtle.verify('Ed25519', publicKey, signature, challenge)) throw new Error();
    return { publicKey: [...value.publicKey], keyId: value.keyId, privateKey: value.privateKey };
  } catch { throw new Error(IDENTITY_ERROR); }
}

async function loadIdentity(): Promise<BrowserSigningIdentity> {
  if (!globalThis.indexedDB || !globalThis.crypto?.subtle) throw new Error(STORAGE_ERROR);
  // Candidate keys are never exposed or used to authenticate unless the initial
  // schema transaction stores them. Existing databases are never repaired or overwritten.
  const candidate = await generateCandidate();
  const database = await openIdentityDatabase(candidate);
  try {
    const record = await new Promise<unknown>((resolve, reject) => {
      const transaction = database.transaction(STORE_NAME, 'readonly');
      const request = transaction.objectStore(STORE_NAME).get(SLOT);
      transaction.oncomplete = () => resolve(request.result);
      transaction.onabort = () => reject(new Error(STORAGE_ERROR));
      transaction.onerror = () => reject(new Error(STORAGE_ERROR));
    });
    return await validateIdentity(record);
  } finally { database.close(); }
}

/** Load the one stored controller identity; storage failures never create a fallback identity. */
export function loadBrowserSigningIdentity(): Promise<BrowserSigningIdentity> {
  if (!loading) {
    loading = loadIdentity().finally(() => { loading = undefined; });
  }
  return loading;
}

/** Commit a unique counter before signing; failed sends consume reservations. */
export async function reserveBrowserSignalCounter(keyId: string): Promise<number> {
  let database: IDBDatabase | undefined;
  try {
    if (!globalThis.indexedDB || !/^[a-f0-9]{64}$/.test(keyId)) throw new Error();
    database = await openIdentityDatabase();
    return await new Promise<number>((resolve, reject) => {
      const transaction = database!.transaction(STORE_NAME, 'readwrite', { durability: 'strict' });
      const store = transaction.objectStore(STORE_NAME);
      const request = store.get(SLOT);
      let counter: number | undefined;
      request.onsuccess = () => {
        const record = request.result;
        if (!validIdentityRecord(record) || record.keyId !== keyId
          || record.signal_counter >= Number.MAX_SAFE_INTEGER) { transaction.abort(); return; }
        counter = record.signal_counter + 1;
        store.put({ ...record, signal_counter: counter }, SLOT);
      };
      transaction.oncomplete = () => counter === undefined ? reject(new Error()) : resolve(counter);
      transaction.onabort = () => reject(new Error());
      transaction.onerror = () => reject(new Error());
    });
  } catch { throw new Error(COUNTER_ERROR); }
  finally { database?.close(); }
}
