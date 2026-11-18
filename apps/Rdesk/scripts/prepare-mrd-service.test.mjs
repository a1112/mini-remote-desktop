import { spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
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

test('passes the configured identity when packaging the macOS service', { skip: process.platform !== 'darwin' }, () => {
  const tempDir = mkdtempSync(join(tmpdir(), 'mrd-service-sign-'));
  try {
    const bin = join(tempDir, 'bin');
    const target = join(tempDir, 'target');
    const log = join(tempDir, 'codesign-args.json');
    mkdirSync(bin);
    mkdirSync(join(target, 'debug'), { recursive: true });
    writeFileSync(join(target, 'debug', 'mrd-service'), 'fixture');
    const cargo = join(bin, 'cargo');
    writeFileSync(cargo, '#!/bin/sh\nexit 0\n');
    chmodSync(cargo, 0o755);
    const codesign = join(bin, 'codesign');
    writeFileSync(codesign, '#!/usr/bin/env node\nrequire("fs").writeFileSync(process.env.MRD_TEST_SIGN_LOG, JSON.stringify(process.argv.slice(2)))\n');
    chmodSync(codesign, 0o755);
    const env = { ...process.env, CARGO: cargo, CARGO_TARGET_DIR: target, PATH: `${bin}:${process.env.PATH}`, MRD_TEST_SIGN_LOG: log, MRD_MACOS_SIGNING_IDENTITY: 'b'.repeat(40) };
    delete env.MRD_SERVICE_PREBUILT_EXE;
    delete env.CARGO_BUILD_TARGET;
    delete env.TARGET;
    const result = spawnSync(process.execPath, [fileURLToPath(new URL('./prepare-mrd-service.mjs', import.meta.url)), '--profile=debug'], { env, encoding: 'utf8' });
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    const args = JSON.parse(readFileSync(log, 'utf8'));
    assert.deepEqual(args.slice(args.indexOf('--sign'), args.indexOf('--sign') + 2), ['--sign', 'b'.repeat(40)]);
    assert.deepEqual(args.slice(args.indexOf('--identifier'), args.indexOf('--identifier') + 2), ['--identifier', 'com.a1112.rdesk.service']);
    assert.equal(args.at(-1), join(target, 'debug', 'MrdService.app'));
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
});
