import { spawnSync } from 'node:child_process';
import { lstat, readdir } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

async function verifyExecutableDirectory(directory, expectedExecutable) {
  if (!(await lstat(directory)).isDirectory()) {
    throw new Error('The bundle executable directory must be a real directory.');
  }
  const entries = await readdir(directory);
  if (entries.length !== 1 || entries[0] !== expectedExecutable) {
    throw new Error(`The executable directory must contain only ${expectedExecutable}; unexpected or missing entries found.`);
  }
  if (!(await lstat(join(directory, expectedExecutable))).isFile()) {
    throw new Error(`The declared executable ${expectedExecutable} must be a regular file.`);
  }
}

export async function verifyMacOSBundleLayout(bundlePath, clientExecutable) {
  if (typeof clientExecutable !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(clientExecutable)) {
    throw new Error('Invalid CFBundleExecutable metadata.');
  }
  await verifyExecutableDirectory(join(bundlePath, 'Contents', 'MacOS'), clientExecutable);
  await verifyExecutableDirectory(
    join(bundlePath, 'Contents', 'Resources', 'MrdService.app', 'Contents', 'MacOS'),
    'mrd-service',
  );
}

function bundleExecutable(infoPlist) {
  const result = spawnSync('/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleExecutable', infoPlist], {
    encoding: 'utf8',
    timeout: 10_000,
  });
  if (result.error || result.status !== 0) {
    throw new Error('Cannot read the actual bundle executable metadata.');
  }
  return result.stdout.trim();
}

async function main() {
  if (process.platform !== 'darwin' || process.argv.length !== 3) {
    throw new Error('Usage on native macOS: node verify-macos-bundle.mjs <Rdesk.app>');
  }
  const bundle = resolve(process.argv[2]);
  const clientExecutable = bundleExecutable(join(bundle, 'Contents', 'Info.plist'));
  const serviceExecutable = bundleExecutable(join(bundle, 'Contents', 'Resources', 'MrdService.app', 'Contents', 'Info.plist'));
  if (serviceExecutable !== 'mrd-service') {
    throw new Error('Unexpected resident service CFBundleExecutable metadata.');
  }
  await verifyMacOSBundleLayout(bundle, clientExecutable);
  console.log('Verified macOS production bundle: only declared client and resident service executables.');
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
