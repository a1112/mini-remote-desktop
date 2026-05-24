import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import assert from 'node:assert/strict';

test('uses prebuilt mrd-service without invoking cargo', () => {
  const tempDir = mkdtempSync(join(tmpdir(), 'mrd-service-prebuilt-'));
  try {
    const serviceExe = join(
      tempDir,
      process.platform === 'win32' ? 'mrd-service.exe' : 'mrd-service',
    );
    writeFileSync(serviceExe, '');

    const result = spawnSync(
      process.execPath,
      [fileURLToPath(new URL('./prepare-mrd-service.mjs', import.meta.url)), '--profile=debug'],
      {
        cwd: new URL('..', import.meta.url),
        env: {
          ...process.env,
          CARGO: 'definitely-missing-cargo-for-prebuilt-test',
          MRD_SERVICE_PREBUILT_EXE: serviceExe,
        },
        encoding: 'utf8',
      },
    );

    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    assert.ok(result.stdout.includes(`mrd-service ready: ${serviceExe}`), result.stdout);
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
});
