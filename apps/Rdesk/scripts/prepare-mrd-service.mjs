import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(scriptDir, '..');
const workspaceRoot = resolve(appDir, '..', '..');
const release = process.argv.includes('--release') || process.argv.includes('--profile=release');
const profile = release ? 'release' : 'debug';
const exeName = process.platform === 'win32' ? 'mrd-service.exe' : 'mrd-service';

function cargoCommand() {
  if (process.env.CARGO) {
    return process.env.CARGO;
  }

  if (process.platform === 'win32') {
    const home = process.env.USERPROFILE;
    if (home) {
      const localCargo = join(home, '.cargo', 'bin', 'cargo.exe');
      if (existsSync(localCargo)) {
        return localCargo;
      }
    }
  }

  return 'cargo';
}

const args = ['build', '-p', 'mrd-service'];
if (release) {
  args.push('--release');
}

const result = spawnSync(cargoCommand(), args, {
  cwd: workspaceRoot,
  env: process.env,
  stdio: 'inherit',
});

if (result.error) {
  throw result.error;
}

if (result.status !== 0) {
  process.exit(result.status ?? 1);
}

const targetRoot = process.env.CARGO_TARGET_DIR
  ? resolve(process.env.CARGO_TARGET_DIR)
  : join(workspaceRoot, 'target');
const cargoTarget = process.env.CARGO_BUILD_TARGET || process.env.TARGET;
const profileDir = cargoTarget ? join(targetRoot, cargoTarget, profile) : join(targetRoot, profile);
const serviceExe = join(profileDir, exeName);

if (!existsSync(serviceExe)) {
  throw new Error(`mrd-service build completed, but ${serviceExe} was not found`);
}

console.log(`mrd-service ready: ${serviceExe}`);
