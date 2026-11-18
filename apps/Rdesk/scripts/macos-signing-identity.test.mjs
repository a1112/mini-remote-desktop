import { mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import assert from 'node:assert/strict';
import { resolveMacOSSigningIdentity } from './macos-signing-identity.mjs';

test('selects an explicit identity before local configuration and preserves ad-hoc default', () => {
  const dir = mkdtempSync(join(tmpdir(), 'rdesk-sign-identity-'));
  try {
    assert.equal(resolveMacOSSigningIdentity(dir, {}), '-');
    const fingerprint = 'a'.repeat(40);
    writeFileSync(join(dir, '.rdesk-macos-signing-identity'), `${fingerprint}\n`);
    assert.equal(resolveMacOSSigningIdentity(dir, {}), fingerprint);
    assert.equal(resolveMacOSSigningIdentity(dir, { APPLE_SIGNING_IDENTITY: 'Development: Example' }), 'Development: Example');
    assert.equal(resolveMacOSSigningIdentity(dir, { APPLE_SIGNING_IDENTITY: 'ignored', MRD_MACOS_SIGNING_IDENTITY: '-' }), '-');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('rejects malformed or symbolic-link identity files and invalid overrides', () => {
  const dir = mkdtempSync(join(tmpdir(), 'rdesk-sign-invalid-'));
  const config = join(dir, '.rdesk-macos-signing-identity');
  try {
    for (const content of ['a'.repeat(40) + '\nextra', 'a'.repeat(39) + '\0', '$(unsafe)']) {
      writeFileSync(config, content);
      assert.throws(() => resolveMacOSSigningIdentity(dir, {}), /signing/);
    }
    rmSync(config);
    symlinkSync(join(dir, 'missing'), config);
    assert.throws(() => resolveMacOSSigningIdentity(dir, {}), /regular file/);
    assert.throws(() => resolveMacOSSigningIdentity(dir, { MRD_MACOS_SIGNING_IDENTITY: '--bad' }), /Invalid/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
