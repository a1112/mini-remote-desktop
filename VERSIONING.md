# mini-remote-desktop version management

## Current authority

The existing source/package values below are preserved. They are not a claim that a product is stable or that these source commits were released. Independent applications, libraries, build tools and protocol/schema numbers are not reset to one repository-wide version.

- `rust-workspace`: `0.1.0` from `Cargo.toml` (workspace.package.version); explicit local bump supported. Rust workspace package version; inherited crate declarations remain source bindings.
- `crate-app`: `0.1.1` from `apps/Rdesk/src-tauri/Cargo.toml` (package.version); explicit local bump supported. Independent Rust package; no repository-wide release version is inferred Tauri application packaging version is synchronized with its own Cargo package.
- `crate-common-control-proto`: `0.1.0` from `common-control-proto/Cargo.toml` (package.version); explicit local bump supported. Independent Rust package; no repository-wide release version is inferred.
- `crate-mrd-buffer-pool`: `0.1.0` from `crates/mrd-buffer-pool/Cargo.toml` (package.version); explicit local bump supported. Independent Rust package; no repository-wide release version is inferred.
- `crate-mrd-hardware`: `0.1.0` from `crates/mrd-hardware/Cargo.toml` (package.version); explicit local bump supported. Independent Rust package; no repository-wide release version is inferred.
- `crate-mrd-render-d3d11`: `0.1.0` from `crates/mrd-render-d3d11/Cargo.toml` (package.version); explicit local bump supported. Independent Rust package; no repository-wide release version is inferred.
- `crate-mrd-render`: `0.1.0` from `crates/mrd-render/Cargo.toml` (package.version); explicit local bump supported. Independent Rust package; no repository-wide release version is inferred.
- `crate-mrd-test-telemetry`: `0.1.0` from `crates/mrd-test-telemetry/Cargo.toml` (package.version); explicit local bump supported. Independent Rust package; no repository-wide release version is inferred.
- `crate-heartbeat-server`: `0.1.0` from `heartbeat-rs/Cargo.toml` (package.version); explicit local bump supported. Independent Rust package; no repository-wide release version is inferred.
- `crate-cap-common`: `0.1.0` from `labs/capture-perf/rust/common/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `crate-desktop-duplication`: `0.1.0` from `labs/capture-perf/rust/desktop-duplication/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `crate-graphics-capture`: `0.1.0` from `labs/capture-perf/rust/graphics-capture/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `crate-shared-memory-dd`: `0.1.0` from `labs/capture-perf/rust/shared-memory-dd/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `crate-test-gc-api`: `0.1.0` from `labs/capture-perf/rust/test-gc-api/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `crate-viewer`: `0.1.0` from `labs/capture-perf/rust/viewer/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `crate-gpu-test`: `0.1.0` from `subprojects/GPUTest/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `crate-gpu-test-2`: `0.1.0` from `subprojects/GPU_Test_2/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `crate-mrd-capture-render-demo`: `0.1.0` from `tests/capture-render-demo/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `crate-integration-tests`: `0.1.0` from `tests/integration/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `crate-mrd-macos-native-surface-proof`: `0.1.0` from `tests/macos-native-surface-proof/Cargo.toml` (package.version); check-only; native release process owns updates. Independent Rust package; no repository-wide release version is inferred. Experimental/test-only independent unit; not a shipping application release.
- `node-prototypes-rdesk`: `0.0.1` from `prototypes/rdesk/package.json` (/version); check-only; native release process owns updates. Node package version; independent unless explicitly coupled by the application package below. Experimental/test-only independent unit; not a shipping application release.
- `cmake-labs-capture-perf-cpp`: `1.0.0` from `labs/capture-perf/cpp/CMakeLists.txt` (exact project version line); check-only; native release process owns updates. Qt/CMake numeric project version; Qt API/module, minimum CMake and SDK versions are excluded. Experimental/test-only independent unit; not a shipping application release.

The machine-readable inventory is `versioning.json`. Cargo workspace inheritance and local lock entries are bound explicitly. Node lock root fields are distinct from registry dependency versions. Tauri versions are bound to their own package; different frontend numbers remain independent.

## Check and change

Python 3.11 or newer; standard library only. Run from the repository root.

```sh
python3 scripts/version_manager.py show
python3 scripts/version_manager.py check
python3 scripts/test_versioning.py -v
```

Choose a single unit. Preview is non-mutating; apply is explicit and should always include the observed current value:

```sh
python3 scripts/version_manager.py bump patch --unit rust-workspace
python3 scripts/version_manager.py bump patch --unit rust-workspace --expect 0.1.0 --apply
python3 scripts/version_manager.py check
```

`set NEW --unit NAME` also previews; add `--expect CURRENT --apply` only after review. For prereleases use `bump prerelease`; stable promotion requires explicit `--allow-stable`. The helper does not run Git, tags, package managers, release hooks, signing, installers or network requests. All source edits are validated before any write.

Check-only units intentionally refuse bump/set because native dependency resolution, existing tooling or historical/test-only ownership must be respected. Incoming local Cargo version requirements and Python workspace locks are not approximated by search-and-replace. Use the project’s native packaging process, update constraints/locks together, then rerun these checks.

## Release history and channels

- No release entry was returned in the repository release inventory reviewed on 2026-10-12. Absence here does not prove the project was never distributed.

This change creates no tag, release, publication, deployment or release date. Record future changes under Unreleased first; use the existing release process and tag namespace only after platform validation and explicit release approval.

## Scope and validation

The added CI checks metadata consistency and dependency-free version-manager safety only, with `contents: read`. Apply/idempotency/stale-source/mismatch checks use temporary fixtures. They do not establish that a native app builds, signs, installs or works on macOS/Windows/Linux. No native packaging or device acceptance was run for this adoption.

Generated Apple projects, vendored/upstream material, historical evidence, protocol/schema/database numbers and dependency versions remain outside product bumps. Existing runtime version mechanisms are retained; there is no UI redesign.

## Adoption base

Source snapshot: [fix/macos-service-permission-recovery-20261012 @ 8be3028d222a](https://github.com/a1112/mini-remote-desktop/commit/8be3028d222aca2bea14acd7ad7ef3c4ffa02a8f). Default branch `main` and all existing repair/tag refs are retained.

Divergent repair chains were not merged or cherry-picked: `codex/macos-host-autoreg-20261008`.
