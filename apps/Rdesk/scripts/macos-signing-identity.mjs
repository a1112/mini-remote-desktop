import { lstatSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

// The local file is data, never executable shell; its fingerprint identifies a
// certificate already in the user's Keychain and contains no private key.
export function resolveMacOSSigningIdentity(workspaceRoot, env = process.env) {
  const explicit = env.MRD_MACOS_SIGNING_IDENTITY ?? env.APPLE_SIGNING_IDENTITY;
  if (explicit !== undefined) {
    if (!explicit.trim() || explicit !== explicit.trim() || explicit.length > 512
        || /[\x00-\x1f\x7f]/.test(explicit) || (explicit.startsWith('-') && explicit !== '-')) {
      throw new Error('Invalid macOS signing identity.');
    }
    return explicit;
  }
  const path = join(workspaceRoot, '.rdesk-macos-signing-identity');
  let stat;
  try {
    stat = lstatSync(path);
  } catch (error) {
    if (error.code === 'ENOENT') return '-';
    throw error;
  }
  if (!stat.isFile() || stat.isSymbolicLink() || (stat.size !== 40 && stat.size !== 41)) {
    throw new Error('The local macOS signing identity must be a regular file containing one SHA-1 fingerprint.');
  }
  const content = readFileSync(path, 'utf8');
  if (!/^[0-9a-fA-F]{40}\n?$/.test(content) || (stat.size === 41 && !content.endsWith('\n'))) {
    throw new Error('Invalid local macOS signing fingerprint.');
  }
  return content.slice(0, 40);
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  try {
    if (process.argv.length !== 3) throw new Error('Expected the workspace root.');
    process.stdout.write(resolveMacOSSigningIdentity(resolve(process.argv[2])));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
