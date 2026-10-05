import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { lstatSync } from 'node:fs';
import { connect } from 'node:net';
import { resolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

if (process.platform !== 'darwin') throw new Error('Native macOS runtime verification is required.');
const executable = resolve(process.argv[2] ?? '');
const endpoint = `/tmp/mrd-service-${process.geteuid()}/service.sock`;
const management = endpoint.replace(/\.sock$/, '-management.sock');

function request(path, message) {
  return new Promise((resolveResponse, reject) => {
    const socket = connect(path);
    let data = Buffer.alloc(0);
    socket.setTimeout(4_000, () => socket.destroy(new Error('IPC response deadline exceeded')));
    socket.on('error', reject);
    socket.once('end', () => reject(new Error('IPC server ended before a complete response')));
    socket.on('connect', () => {
      const body = Buffer.from(JSON.stringify(message));
      const prefix = Buffer.alloc(4);
      prefix.writeUInt32LE(body.length);
      socket.write(Buffer.concat([prefix, body]));
    });
    socket.on('data', (chunk) => {
      data = Buffer.concat([data, chunk]);
      if (data.length < 4) return;
      const size = data.readUInt32LE();
      if (size === 0 || size > 64 * 1024) return socket.destroy(new Error('Invalid IPC response length'));
      if (data.length < size + 4) return;
      try {
        const response = JSON.parse(data.subarray(4, size + 4));
        socket.end();
        resolveResponse(response);
      } catch {
        socket.destroy(new Error('Invalid IPC response JSON'));
      }
    });
  });
}

async function verifyOneStart() {
  const child = spawn(executable, [], { stdio: ['ignore', 'pipe', 'pipe'] });
  const logs = [];
  for (const stream of [child.stdout, child.stderr]) stream.on('data', (part) => {
    if (logs.join('').length < 64 * 1024) logs.push(part.toString());
  });
  const exited = new Promise((resolveExit) => child.once('exit', (code, signal) => resolveExit({ code, signal })));
  try {
    let health;
    const readyDeadline = Date.now() + 20_000;
    while (Date.now() < readyDeadline) {
      if (child.exitCode !== null || child.signalCode !== null) throw new Error('Service exited before IPC readiness');
      try {
        health = await request(management, { type: 'ServiceHealth' });
        if (health.type === 'ServiceHealth' && health.status?.running && health.status?.healthy) break;
      } catch {}
      await sleep(100);
    }
    assert.equal(health?.type, 'ServiceHealth');
    assert.equal(health.status.running, true);
    assert.equal(health.status.healthy, true);
    assert.equal(health.status.pid, child.pid);
    for (const path of [endpoint, management]) {
      const metadata = lstatSync(path);
      assert.equal(metadata.isSocket(), true);
      assert.equal(metadata.uid, process.geteuid());
      assert.equal(metadata.mode & 0o777, 0o600);
    }
    const identity = await request(endpoint, { type: 'GetDeviceIdentitySnapshot' });
    assert.equal(identity.type, 'DeviceIdentitySnapshot');
    const fingerprint = identity.snapshot?.certificate_fingerprint;
    assert.equal(typeof fingerprint, 'string');
    assert.ok(fingerprint.length > 0);
    assert.equal(identity.snapshot.consent_required, true);
    const status = await request(management, { type: 'GetPublicServerStatus' });
    assert.equal(status.type, 'PublicServerStatus');
    const stopped = await request(management, { type: 'ShutdownService', mode: 'graceful' });
    assert.equal(stopped.type, 'Ack');
    const result = await Promise.race([exited, sleep(15_000, undefined, { ref: false }).then(() => { throw new Error('Service stop deadline exceeded'); })]);
    assert.equal(result.code, 0);
    assert.equal(result.signal, null);
    return fingerprint;
  } catch (error) {
    // Startup errors are sanitized by the service. Keep credentials and machine
    // identifiers out of the public workflow log on successful runs.
    process.stderr.write(logs.join(''));
    throw error;
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill('SIGTERM');
      await Promise.race([exited, sleep(5_000, undefined, { ref: false })]);
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    }
  }
}

const first = await verifyOneStart();
const second = await verifyOneStart();
assert.ok(second === first, 'Protected machine identity changed after service restart');
console.log('Native macOS service: healthy IPC, owner-only sockets, protected identity persistence, and graceful restart passed.');
