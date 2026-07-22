# FFmpeg Optional Tooling Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add optional FFmpeg detection, managed Windows download, golden settings, Tauri commands, and service capability reporting without changing the active decode pipeline.

**Architecture:** Introduce `crates/mrd-ffmpeg` as the single owner of FFmpeg settings, probing, checksum validation, archive extraction, and managed installation. `apps/Rdesk/src-tauri` persists settings and exposes commands, while `apps/mrd-service` maps probe results into capability snapshots. Existing decoder factories and profiles remain unchanged.

**Tech Stack:** Rust workspace, serde, reqwest, sha2, zip, tempfile, Tauri commands, mrd-service capability snapshots, Vitest capability matrix tests.

---

### Task 1: Add `mrd-ffmpeg` Workspace Crate and Golden Settings Tests

**Files:**
- Modify: `G:\Project\mini-remote-desktop\Cargo.toml`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-ffmpeg\Cargo.toml`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-ffmpeg\src\lib.rs`

**Step 1: Write failing tests**

Add tests in `crates/mrd-ffmpeg/src/lib.rs`:

```rust
#[test]
fn golden_settings_use_windows_release_essentials_source() {
    let settings = FfmpegSettings::golden_for_platform(FfmpegPlatform::Windows);

    assert!(settings.enabled);
    assert_eq!(settings.channel, "release-essentials");
    assert!(settings.download.archive_url.ends_with("/ffmpeg-release-essentials.zip"));
    assert!(settings.download.sha256_url.as_deref().unwrap().ends_with(".zip.sha256"));
    assert!(settings.download.require_sha256);
}

#[test]
fn non_windows_golden_settings_probe_without_managed_download() {
    let settings = FfmpegSettings::golden_for_platform(FfmpegPlatform::Linux);

    assert!(settings.enabled);
    assert!(settings.download.archive_url.is_empty());
    assert!(settings.download.sha256_url.is_none());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-ffmpeg golden_settings`

Expected: FAIL because the package and types do not exist yet.

**Step 3: Create minimal crate and settings model**

Add `crates/mrd-ffmpeg/Cargo.toml`:

```toml
[package]
name = "mrd-ffmpeg"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
serde.workspace = true
```

Add workspace member `"crates/mrd-ffmpeg"` to the root `Cargo.toml`.

Implement:

- `FfmpegPlatform`
- `FfmpegDownloadSettings`
- `FfmpegSettings`
- `FfmpegSettings::golden_for_platform`
- `golden_settings()`

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-ffmpeg golden_settings`

Expected: PASS.

**Step 5: Commit**

```powershell
git add Cargo.toml crates/mrd-ffmpeg
git commit -m "feat: add ffmpeg tooling settings"
```

### Task 2: Add FFmpeg Probe Logic

**Files:**
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-ffmpeg\Cargo.toml`
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-ffmpeg\src\lib.rs`

**Step 1: Write failing tests**

Add tests:

```rust
#[test]
fn probe_succeeds_with_fake_tools_in_configured_directory() {
    let dir = unique_temp_dir("mrd-ffmpeg-probe-ok");
    write_fake_tool(&dir, "ffmpeg");
    write_fake_tool(&dir, "ffprobe");

    let mut settings = FfmpegSettings::golden_for_platform(FfmpegPlatform::Windows);
    settings.install_dir = Some(dir.clone());

    let result = probe_ffmpeg(&settings);

    assert!(result.available, "{result:?}");
    assert_eq!(result.ffmpeg_path.as_deref(), Some(dir.join(exe_name("ffmpeg")).as_path()));
    assert_eq!(result.ffprobe_path.as_deref(), Some(dir.join(exe_name("ffprobe")).as_path()));
}

#[test]
fn probe_fails_when_ffprobe_is_missing() {
    let dir = unique_temp_dir("mrd-ffmpeg-probe-missing");
    write_fake_tool(&dir, "ffmpeg");

    let mut settings = FfmpegSettings::golden_for_platform(FfmpegPlatform::Windows);
    settings.install_dir = Some(dir);

    let result = probe_ffmpeg(&settings);

    assert!(!result.available);
    assert!(result.reason.unwrap().contains("ffprobe"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-ffmpeg probe_`

Expected: FAIL because probing APIs do not exist.

**Step 3: Implement probe model and logic**

Add:

- `FfmpegProbeResult`
- `FfmpegToolVersion`
- `probe_ffmpeg(settings: &FfmpegSettings) -> FfmpegProbeResult`

Probe order:

1. explicit paths from settings;
2. `install_dir/bin/<tool>.exe`, then `install_dir/<tool>.exe`;
3. PATH lookup.

Each candidate must run `-version` and return success from the child process.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-ffmpeg probe_`

Expected: PASS.

**Step 5: Commit**

```powershell
git add crates/mrd-ffmpeg
git commit -m "feat: probe optional ffmpeg tools"
```

### Task 3: Add Checksum and Archive Install Support

**Files:**
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-ffmpeg\Cargo.toml`
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-ffmpeg\src\lib.rs`

**Step 1: Write failing tests**

Add tests:

```rust
#[test]
fn parses_plain_and_filename_sha256_formats() {
    let plain = parse_sha256("a".repeat(64).as_str()).expect("plain hash");
    let named = parse_sha256(format!("{}  ffmpeg-release-essentials.zip", "b".repeat(64)).as_str())
        .expect("named hash");

    assert_eq!(plain, "a".repeat(64));
    assert_eq!(named, "b".repeat(64));
}

#[test]
fn checksum_mismatch_is_reported() {
    let path = unique_temp_dir("mrd-ffmpeg-hash")
        .join("archive.zip");
    std::fs::write(&path, b"not the expected content").expect("write archive");

    let error = verify_sha256(&path, &"0".repeat(64)).unwrap_err();

    assert!(error.to_string().contains("checksum mismatch"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-ffmpeg sha256`

Expected: FAIL because checksum helpers do not exist.

**Step 3: Implement checksum helpers and install primitive**

Add dependencies:

```toml
sha2 = "0.10"
zip = "2"
thiserror.workspace = true
reqwest.workspace = true
tokio.workspace = true
```

Implement:

- `FfmpegError`
- `parse_sha256`
- `verify_sha256`
- `download_ffmpeg(settings: &FfmpegSettings) -> impl Future<Output = Result<FfmpegInstallResult, FfmpegError>>`
- `install_ffmpeg_archive(archive_path, install_dir, expected_sha256)`

Use a temporary sibling directory and only replace the managed install after checksum, extraction, executable discovery, and post-install probe succeed.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-ffmpeg sha256`

Expected: PASS.

**Step 5: Commit**

```powershell
git add crates/mrd-ffmpeg
git commit -m "feat: verify ffmpeg downloads"
```

### Task 4: Persist FFmpeg Settings in Rdesk

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\Cargo.toml`
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\app_settings.rs`

**Step 1: Write failing tests**

Extend settings tests:

```rust
#[test]
fn load_settings_defaults_ffmpeg_to_golden_values() {
    let path = unique_settings_path("ffmpeg-defaults");

    let settings = load_settings(&path).expect("load defaults");

    assert!(settings.ffmpeg.enabled);
    assert_eq!(settings.ffmpeg.channel, "release-essentials");
}

#[test]
fn save_and_load_settings_roundtrip_ffmpeg_overrides() {
    let path = unique_settings_path("ffmpeg-roundtrip");
    let mut settings = AppSettings::default();
    settings.ffmpeg.enabled = false;
    settings.ffmpeg.channel = "custom".to_string();

    save_settings(&path, &settings).expect("save settings");
    let loaded = load_settings(&path).expect("load settings");

    assert!(!loaded.ffmpeg.enabled);
    assert_eq!(loaded.ffmpeg.channel, "custom");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p app app_settings::tests::load_settings_defaults_ffmpeg_to_golden_values`

Expected: FAIL because `AppSettings::ffmpeg` does not exist.

**Step 3: Add dependency and settings field**

Add `mrd-ffmpeg = { path = "../../../crates/mrd-ffmpeg" }` to `apps/Rdesk/src-tauri/Cargo.toml`.

Update `AppSettings`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    #[serde(default)]
    pub decode_policy: DecodePolicy,
    #[serde(default = "mrd_ffmpeg::golden_settings")]
    pub ffmpeg: mrd_ffmpeg::FfmpegSettings,
}
```

Implement `Default` manually to use golden FFmpeg settings.

**Step 4: Run test to verify it passes**

Run: `cargo test -p app app_settings::tests::load_settings_defaults_ffmpeg_to_golden_values app_settings::tests::save_and_load_settings_roundtrip_ffmpeg_overrides`

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/Rdesk/src-tauri/Cargo.toml apps/Rdesk/src-tauri/src/app_settings.rs
git commit -m "feat: persist ffmpeg settings"
```

### Task 5: Add Tauri FFmpeg Commands

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`

**Step 1: Write failing command tests**

Add unit tests for command helper functions:

```rust
#[test]
fn reset_ffmpeg_settings_uses_golden_defaults() {
    let path = unique_settings_path("ffmpeg-reset");
    save_settings(&path, &AppSettings {
        decode_policy: DecodePolicy::Auto,
        ffmpeg: mrd_ffmpeg::FfmpegSettings {
            enabled: false,
            channel: "custom".to_string(),
            ..mrd_ffmpeg::golden_settings()
        },
    })
    .expect("save custom settings");

    let settings = reset_ffmpeg_settings_at_path(&path).expect("reset ffmpeg");

    assert!(settings.ffmpeg.enabled);
    assert_eq!(settings.ffmpeg.channel, "release-essentials");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p app reset_ffmpeg_settings_uses_golden_defaults`

Expected: FAIL because helper does not exist.

**Step 3: Implement command helpers and Tauri commands**

Add serializable wrappers only if direct `mrd-ffmpeg` types are not enough.

Implement:

- `ffmpeg_probe(state: tauri::State<'_, AppState>)`
- `ffmpeg_download(state: tauri::State<'_, AppState>)`
- `ffmpeg_reset_golden_settings(state: tauri::State<'_, AppState>)`
- `reset_ffmpeg_settings_at_path(path: &Path)`

Register the commands in `tauri::generate_handler!`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p app reset_ffmpeg_settings_uses_golden_defaults`

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/Rdesk/src-tauri/src/main.rs
git commit -m "feat: expose ffmpeg tooling commands"
```

### Task 6: Add FFmpeg Capabilities to mrd-service

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\mrd-service\Cargo.toml`
- Modify: `G:\Project\mini-remote-desktop\apps\mrd-service\src\capabilities.rs`

**Step 1: Write failing tests**

Add service capability tests:

```rust
#[test]
fn static_snapshot_includes_optional_ffmpeg_capability() {
    let snapshot = local_capability_snapshot_static();

    let ffmpeg = snapshot
        .capabilities
        .iter()
        .find(|item| item.id == "service.ffmpeg")
        .expect("service.ffmpeg capability");

    assert!(matches!(ffmpeg.status, CapabilityStatus::Supported | CapabilityStatus::Degraded));
}

#[test]
fn default_profiles_do_not_require_ffmpeg() {
    let snapshot = local_capability_snapshot_static();

    assert!(snapshot
        .profiles
        .iter()
        .all(|profile| profile.required_capabilities.iter().all(|id| !id.contains("ffmpeg"))));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-service ffmpeg`

Expected: FAIL because no FFmpeg capabilities exist.

**Step 3: Add service capability mapping**

Add `mrd-ffmpeg = { path = "../../crates/mrd-ffmpeg" }`.

In `add_service_capabilities`, add `service.ffmpeg`.

In `add_decode_capabilities`, add optional `decode.ffmpeg_h264` and `decode.ffmpeg_hevc`.

Runtime mapping:

- probe available: `CapabilityStatus::Available`;
- missing tools: `CapabilityStatus::DriverMissing`;
- static snapshot: `CapabilityStatus::Supported`.

Existing profiles must not depend on FFmpeg.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-service ffmpeg`

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/Cargo.toml apps/mrd-service/src/capabilities.rs
git commit -m "feat: report optional ffmpeg capability"
```

### Task 7: Update Frontend Capability Normalization

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src\app\services\capabilityMatrix.ts`
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src\app\services\capabilityMatrix.test.ts`

**Step 1: Write failing tests**

Add tests:

```ts
it("normalizes optional ffmpeg service and decoder capabilities", () => {
  const snapshot = capabilitySnapshotFromService({
    schema_version: 1,
    platform: "windows",
    service_version: "0.1.0",
    updated_at_ms: 1,
    constraints: [],
    profiles: [],
    capabilities: [
      capability("service.ffmpeg", "service", "available"),
      capability("decode.ffmpeg_h264", "decode", "available"),
      capability("decode.ffmpeg_hevc", "decode", "driver_missing"),
    ],
  });

  expect(statusOf(snapshot, "service.ffmpeg")).toBe("available");
  expect(statusOf(snapshot, "decode.ffmpeg_h264")).toBe("available");
  expect(statusOf(snapshot, "decode.ffmpeg_hevc")).toBe("driver_missing");
});
```

**Step 2: Run test to verify it fails**

Run: `pnpm --dir apps/Rdesk test -- capabilityMatrix`

Expected: FAIL until known status mapping includes FFmpeg IDs if the local fallback path needs it.

**Step 3: Update known capability IDs**

Add:

- `service.ffmpeg`
- `decode.ffmpeg_h264`
- `decode.ffmpeg_hevc`

Keep FFmpeg out of required profile defaults.

**Step 4: Run test to verify it passes**

Run: `pnpm --dir apps/Rdesk test -- capabilityMatrix`

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/Rdesk/src/app/services/capabilityMatrix.ts apps/Rdesk/src/app/services/capabilityMatrix.test.ts
git commit -m "feat: surface optional ffmpeg capabilities"
```

### Task 8: Final Verification

**Files:**
- Inspect all touched files.

**Step 1: Run targeted Rust tests**

Run:

```powershell
cargo test -p mrd-ffmpeg
cargo test -p app app_settings ffmpeg
cargo test -p mrd-service ffmpeg
```

Expected: PASS.

**Step 2: Run targeted frontend tests**

Run:

```powershell
pnpm --dir apps/Rdesk test -- capabilityMatrix
```

Expected: PASS.

**Step 3: Run compile checks**

Run:

```powershell
cargo check -p mrd-ffmpeg
cargo check -p app
cargo check -p mrd-service
```

Expected: PASS.

**Step 4: Review final diff**

Run:

```powershell
git status --short --branch
git diff --stat HEAD
```

Expected: only unrelated pre-existing local changes remain outside the committed FFmpeg work.

**Step 5: Commit any remaining implementation changes**

If previous task commits already captured all changes, no commit is needed. Otherwise commit only FFmpeg-related files:

```powershell
git add Cargo.toml crates/mrd-ffmpeg apps/Rdesk/src-tauri apps/mrd-service apps/Rdesk/src/app/services
git commit -m "feat: integrate optional ffmpeg tooling"
```
