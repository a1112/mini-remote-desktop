import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import test from 'node:test';

const smoke = await import('./smoke-macos-service.mjs').catch((error) => error);

function contract(name) {
  assert.equal(typeof smoke[name], 'function', `Missing importable native smoke contract: ${name}`);
  return smoke[name];
}

test('UI readiness requires its own exact PID and the verified service PID', () => {
  const verify = contract('assertExactUIRegistration');
  const status = { type: 'ShellStatus', status: { ui_pid: 122, service_pid: 121 } };
  assert.doesNotThrow(() => verify(status, 122, 121));
  for (const changed of [
    { type: 'ShellStatus', status: { ui_pid: 123, service_pid: 121 } },
    { type: 'ShellStatus', status: { ui_pid: null, service_pid: 121 } },
    { type: 'ShellStatus', status: { ui_pid: 122, service_pid: 120 } },
    { type: 'Ack', status: { ui_pid: 122, service_pid: 121 } },
  ]) assert.throws(() => verify(changed, 122, 121));
  assert.throws(() => verify(status, 0, 121));
  assert.throws(() => verify(status, 122, 122));
});

test('a visible window belonging to another process cannot prove UI readiness', () => {
  const verify = contract('assertVisibleUIWindow');
  assert.doesNotThrow(() => verify({ pid: 122, visibleLayerZeroWindows: 1 }, 122));
  assert.throws(() => verify({ pid: 123, visibleLayerZeroWindows: 1 }, 122));
  for (const count of [0, -1, 1.5, null, '1', Infinity]) {
    assert.throws(() => verify({ pid: 122, visibleLayerZeroWindows: count }, 122));
  }
  assert.throws(() => verify({ pid: 0, visibleLayerZeroWindows: 1 }, 0));
});

test('an exited or unspawned child cannot satisfy UI readiness', () => {
  const verify = contract('assertOwnedChildAlive');
  const child = { pid: 122, exitCode: null, signalCode: null };
  assert.doesNotThrow(() => verify(child, 'UI'));
  assert.throws(() => verify({ ...child, exitCode: 0 }, 'UI'));
  assert.throws(() => verify({ ...child, signalCode: 'SIGTERM' }, 'UI'));
  assert.throws(() => verify({ ...child, pid: undefined }, 'UI'));
});

test('UI contents are derived from the outer Rdesk bundle, with standalone services rejected', () => {
  const derive = contract('bundledUIContents');
  const bundle = resolve('test-fixtures', 'Rdesk.app');
  const service = join(bundle, 'Contents', 'Resources', 'MrdService.app', 'Contents', 'MacOS', 'mrd-service');
  assert.equal(derive(service), join(bundle, 'Contents'));
  assert.throws(() => derive(join(bundle, 'Contents', 'MacOS', 'mrd-service')));
  assert.throws(() => derive(join(bundle, 'Contents', 'Resources', 'MrdService.app', 'Contents', 'MacOS', 'app')));
});

test('prepared window helper requires an owner-only directory and regular executable owned by the runner', () => {
  const verify = contract('assertPreparedWindowHelperMetadata');
  const directory = { uid: 501, mode: 0o40700, isDirectory: () => true };
  const executable = { uid: 501, mode: 0o100700, isFile: () => true };
  assert.doesNotThrow(() => verify(executable, directory, 501));
  for (const changed of [
    { ...directory, uid: 502 },
    { ...directory, mode: 0o40755 },
    { ...directory, isDirectory: () => false },
  ]) assert.throws(() => verify(executable, changed, 501));
  for (const changed of [
    { ...executable, uid: 502 },
    { ...executable, mode: 0o100755 },
    { ...executable, mode: 0o100600 },
    { ...executable, isFile: () => false },
  ]) assert.throws(() => verify(changed, directory, 501));
});

function privateFixture(testContext) {
  const root = mkdtempSync(join(tmpdir(), 'mrd-prepared-helper-test-'));
  testContext.after(() => {
    assert.equal(dirname(root), resolve(tmpdir()));
    assert.match(basename(root), /^mrd-prepared-helper-test-/);
    rmSync(root, { recursive: true, force: true });
  });
  return root;
}

test('missing or implicit prepared helper paths are rejected before the service can start', (context) => {
  const verify = contract('validatePreparedWindowHelper');
  const root = privateFixture(context);
  assert.throws(() => verify(undefined, 501));
  assert.throws(() => verify('', 501));
  assert.throws(() => verify('window-check', 501));
  assert.throws(() => verify(join(root, 'window-check'), 501), { code: 'ENOENT' });
});

test('a symlinked prepared helper directory is rejected', (context) => {
  const verify = contract('validatePreparedWindowHelper');
  const root = privateFixture(context);
  const realDirectory = join(root, 'real');
  mkdirSync(realDirectory, { mode: 0o700 });
  writeFileSync(join(realDirectory, 'window-check'), 'fixture', { mode: 0o700 });
  const linkDirectory = join(root, 'linked');
  symlinkSync(realDirectory, linkDirectory, 'junction');
  assert.throws(() => verify(join(linkDirectory, 'window-check'), 501), /regular directory/);
});
