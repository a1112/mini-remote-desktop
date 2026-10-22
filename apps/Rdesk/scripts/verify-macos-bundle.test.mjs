import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { verifyMacOSBundleLayout } from './verify-macos-bundle.mjs';

async function bundleFixture(t, executable = 'app') {
  const parent = resolve(tmpdir());
  const root = await mkdtemp(join(parent, 'mrd-bundle-contract-'));
  t.after(async () => {
    assert.equal(dirname(resolve(root)), parent);
    assert.ok(basename(root).startsWith('mrd-bundle-contract-'));
    await rm(root, { recursive: true, force: true });
  });
  const bundle = join(root, 'Rdesk.app');
  const clientDirectory = join(bundle, 'Contents', 'MacOS');
  const serviceDirectory = join(bundle, 'Contents', 'Resources', 'MrdService.app', 'Contents', 'MacOS');
  await mkdir(clientDirectory, { recursive: true });
  await mkdir(serviceDirectory, { recursive: true });
  await writeFile(join(clientDirectory, executable), 'client');
  await writeFile(join(serviceDirectory, 'mrd-service'), 'service');
  return { bundle, clientDirectory, serviceDirectory, executable };
}

test('accepts only the declared client and resident service executable', async (t) => {
  const fixture = await bundleFixture(t, 'Rdesk-client');
  await verifyMacOSBundleLayout(fixture.bundle, fixture.executable);
});

test('rejects the accidentally bundled macos_metal_present_probe', async (t) => {
  const fixture = await bundleFixture(t);
  await writeFile(join(fixture.clientDirectory, 'macos_metal_present_probe'), 'diagnostic');
  await assert.rejects(verifyMacOSBundleLayout(fixture.bundle, fixture.executable), /unexpected|only/i);
});

test('rejects an unknown extra client executable', async (t) => {
  const fixture = await bundleFixture(t);
  await writeFile(join(fixture.clientDirectory, 'another-helper'), 'unknown');
  await assert.rejects(verifyMacOSBundleLayout(fixture.bundle, fixture.executable), /unexpected|only/i);
});

test('rejects an extra resident service executable', async (t) => {
  const fixture = await bundleFixture(t);
  await writeFile(join(fixture.serviceDirectory, 'service-diagnostic'), 'diagnostic');
  await assert.rejects(verifyMacOSBundleLayout(fixture.bundle, fixture.executable), /unexpected|only/i);
});

test('rejects extra directories in the client and service executable directories', async (t) => {
  for (const side of ['clientDirectory', 'serviceDirectory']) {
    const fixture = await bundleFixture(t);
    await mkdir(join(fixture[side], 'extra-directory'));
    await assert.rejects(verifyMacOSBundleLayout(fixture.bundle, fixture.executable), /unexpected|only/i);
  }
});

test('rejects a directory in place of either declared executable', async (t) => {
  for (const side of ['clientDirectory', 'serviceDirectory']) {
    const fixture = await bundleFixture(t);
    const executable = side === 'clientDirectory' ? fixture.executable : 'mrd-service';
    await rm(join(fixture[side], executable));
    await mkdir(join(fixture[side], executable));
    await assert.rejects(verifyMacOSBundleLayout(fixture.bundle, fixture.executable), /regular file/i);
  }
});

test('rejects missing executables', async (t) => {
  const fixture = await bundleFixture(t);
  await rm(join(fixture.serviceDirectory, 'mrd-service'));
  await assert.rejects(verifyMacOSBundleLayout(fixture.bundle, fixture.executable), /only|missing|unexpected/i);
});

test('rejects unsafe or empty executable metadata', async (t) => {
  const fixture = await bundleFixture(t);
  for (const executable of ['', '..', '../app', '..\\app', '/app']) {
    await assert.rejects(verifyMacOSBundleLayout(fixture.bundle, executable), /metadata/i);
  }
});
