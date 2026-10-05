import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { lstatSync, mkdtempSync, rmSync } from 'node:fs';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { promisify } from 'node:util';

const runFile = promisify(execFile);

export function assertExactUIRegistration(response, uiPid, servicePid) {
  assert.ok(Number.isSafeInteger(uiPid) && uiPid > 0, 'Invalid owned UI PID');
  assert.ok(Number.isSafeInteger(servicePid) && servicePid > 0, 'Invalid owned service PID');
  assert.notEqual(uiPid, servicePid, 'UI must be a separate owned process');
  assert.equal(response?.type, 'ShellStatus');
  assert.equal(response.status?.service_pid, servicePid, 'Shell status belongs to another service');
  assert.equal(response.status?.ui_pid, uiPid, 'The bundled UI did not register its own PID');
}

export function assertVisibleUIWindow(report, uiPid) {
  assert.ok(Number.isSafeInteger(uiPid) && uiPid > 0, 'Invalid owned UI PID');
  assert.equal(report?.pid, uiPid, 'Window report belongs to another process');
  assert.ok(
    Number.isSafeInteger(report.visibleLayerZeroWindows) && report.visibleLayerZeroWindows > 0,
    'The bundled UI has no visible layer-zero window with nonzero bounds',
  );
}

export function assertOwnedChildAlive(child, label) {
  assert.ok(Number.isSafeInteger(child.pid) && child.pid > 0, `${label} did not spawn`);
  assert.equal(child.exitCode, null, `${label} exited before readiness`);
  assert.equal(child.signalCode, null, `${label} terminated before readiness`);
}

export function bundledUIContents(executable) {
  executable = resolve(executable);
  const serviceBundle = resolve(dirname(executable), '../..');
  const bundle = resolve(serviceBundle, '../../..');
  assert.equal(basename(serviceBundle), 'MrdService.app', 'The service must come from the real app bundle');
  assert.equal(basename(bundle), 'Rdesk.app', 'The outer Rdesk app bundle is required');
  assert.equal(
    executable,
    join(bundle, 'Contents', 'Resources', 'MrdService.app', 'Contents', 'MacOS', 'mrd-service'),
    'Unexpected embedded service location',
  );
  return join(bundle, 'Contents');
}

async function verifyBundledUI(executable, servicePid, management) {
  const uiContents = bundledUIContents(executable);
  const { stdout } = await runFile('/usr/libexec/PlistBuddy', [
    '-c', 'Print :CFBundleExecutable', join(uiContents, 'Info.plist'),
  ], { timeout: 5_000, maxBuffer: 64 * 1024 });
  const executableName = stdout.trim();
  assert.ok(executableName && !['.', '..'].includes(executableName) && !/[\\/]/.test(executableName));
  const uiExecutable = join(uiContents, 'MacOS', executableName);
  assert.equal(lstatSync(uiExecutable).isFile(), true);

  const helperDirectory = mkdtempSync(join(tmpdir(), 'mrd-ui-window-smoke-'));
  const helperMetadata = lstatSync(helperDirectory);
  assert.equal(helperMetadata.uid, process.geteuid());
  assert.equal(helperMetadata.mode & 0o777, 0o700);
  const windowHelper = join(helperDirectory, 'window-check');
  let child;
  let exited;
  const logs = [];
  let logBytes = 0;
  try {
    // Reads window metadata only. It does not request screen capture, inject
    // input, automate another app, or change any macOS privacy permission.
    await runFile('/usr/bin/xcrun', [
      'swiftc', join(dirname(fileURLToPath(import.meta.url)), 'smoke-macos-ui-window.swift'),
      '-o', windowHelper,
    ], { timeout: 30_000, maxBuffer: 64 * 1024 });
    child = spawn(uiExecutable, [], { stdio: ['ignore', 'pipe', 'pipe'] });
    exited = new Promise((resolveExit) => {
      child.once('error', (error) => resolveExit({ error }));
      child.once('exit', (code, signal) => resolveExit({ code, signal }));
    });
    for (const stream of [child.stdout, child.stderr]) stream.on('data', (part) => {
      if (logBytes < 64 * 1024) {
        const retained = part.subarray(0, 64 * 1024 - logBytes);
        logs.push(retained.toString());
        logBytes += retained.length;
      }
    });

    const deadline = Date.now() + 30_000;
    let verified = false;
    while (Date.now() < deadline) {
      assertOwnedChildAlive(child, 'Bundled UI');
      const shell = await request(management, { type: 'GetShellStatus' });
      assert.equal(shell.type, 'ShellStatus');
      assert.equal(shell.status?.service_pid, servicePid);
      if (shell.status.ui_pid !== null && shell.status.ui_pid !== undefined) {
        assertExactUIRegistration(shell, child.pid, servicePid);
        const result = await runFile(windowHelper, [String(child.pid)], { timeout: 5_000, maxBuffer: 64 * 1024 });
        const report = JSON.parse(result.stdout);
        assert.equal(report.pid, child.pid);
        assert.ok(Number.isSafeInteger(report.visibleLayerZeroWindows) && report.visibleLayerZeroWindows >= 0);
        assertOwnedChildAlive(child, 'Bundled UI');
        if (report.visibleLayerZeroWindows > 0) {
          assertVisibleUIWindow(report, child.pid);
          verified = true;
          break;
        }
      }
      await sleep(100);
    }
    assert.equal(verified, true, 'Bundled UI registration/window readiness deadline exceeded');
    await sleep(5_000);
    assertOwnedChildAlive(child, 'Bundled UI');
    assertExactUIRegistration(await request(management, { type: 'GetShellStatus' }), child.pid, servicePid);
    const finalWindow = await runFile(windowHelper, [String(child.pid)], { timeout: 5_000, maxBuffer: 64 * 1024 });
    assertVisibleUIWindow(JSON.parse(finalWindow.stdout), child.pid);
    assertOwnedChildAlive(child, 'Bundled UI');
    assert.doesNotMatch(logs.join(''), /Class NSVisualEffectViewTagged is implemented in both/i,
      'The native vibrancy class has conflicting runtime implementations');
  } catch (error) {
    process.stderr.write(logs.join(''));
    throw error;
  } finally {
    // Only signal the ChildProcess created here. UI teardown is test cleanup;
    // the service is stopped separately through its graceful management IPC.
    if (child?.pid && child.exitCode === null && child.signalCode === null) {
      child.kill('SIGTERM');
      await Promise.race([exited, sleep(5_000, undefined, { ref: false })]);
      if (child.exitCode === null && child.signalCode === null) {
        child.kill('SIGKILL');
        await Promise.race([exited, sleep(5_000, undefined, { ref: false })]);
      }
      assert.ok(child.exitCode !== null || child.signalCode !== null, 'Owned UI process did not exit during cleanup');
    }
    rmSync(helperDirectory, { recursive: true, force: true });
  }
  console.log('Native macOS bundled UI: exact PID registration and visible layer-zero window passed.');
}

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

async function verifyOneStart(executable, verifyUI = false) {
  const endpoint = `/tmp/mrd-service-${process.geteuid()}/service.sock`;
  const management = endpoint.replace(/\.sock$/, '-management.sock');
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
    if (verifyUI) await verifyBundledUI(executable, child.pid, management);
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

async function runSmoke() {
  if (process.platform !== 'darwin') throw new Error('Native macOS runtime verification is required.');
  const executable = resolve(process.argv[2] ?? '');
  const first = await verifyOneStart(executable);
  const second = await verifyOneStart(executable, true);
  assert.ok(second === first, 'Protected machine identity changed after service restart');
  console.log('Native macOS service: healthy IPC, owner-only sockets, protected identity persistence, and graceful restart passed.');
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await runSmoke();
