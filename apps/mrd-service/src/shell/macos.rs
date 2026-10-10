use super::{AutostartPort, TrayModel, TrayPort, UiLaunchRequest, UiLaunchResult, UiLauncherPort};
use anyhow::{anyhow, Context};
use security_framework::os::macos::code_signing::{
    Flags as CodeSigningFlags, GuestAttributes, SecCode, SecRequirement,
};
use std::{
    ffi::{c_void, CString},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const DEFAULT_LABEL_PREFIX: &str = "com.mini-remote-desktop";
const UI_IDENTIFIER: &str = "com.a1112.rdesk";
const SERVICE_IDENTIFIER: &str = "com.a1112.rdesk.service";

#[derive(Clone)]
struct PinnedUiIdentity {
    executable_path: PathBuf,
    requirement: String,
}

impl PinnedUiIdentity {
    fn from_bundle(path: &Path) -> anyhow::Result<Self> {
        let executable_path = configured_ui_executable_path(path)?;
        let bundle = executable_bundle(&executable_path)
            .ok_or_else(|| anyhow!("Rdesk must be an app bundle"))?;
        let bundle = std::fs::canonicalize(bundle)?;
        let expected_executable = configured_ui_executable_path(&bundle)?;
        if !std::fs::symlink_metadata(&expected_executable)?.is_file()
            || std::fs::canonicalize(&executable_path)? != expected_executable
        {
            return Err(anyhow!(
                "Rdesk executable must remain inside its app bundle"
            ));
        }
        let requirement = validated_cdhash_requirement(&bundle, UI_IDENTIFIER)?;
        Ok(Self {
            executable_path: expected_executable,
            requirement,
        })
    }
}

pub struct MacosUiLauncher {
    app_name: String,
    ui_path: Arc<Mutex<Option<PathBuf>>>,
    // Captured before any IPC attachment. Updating the launch path cannot
    // authorize a different ad-hoc executable.
    pinned_ui: Option<PinnedUiIdentity>,
}

impl MacosUiLauncher {
    pub fn new(app_name: impl Into<String>) -> Self {
        let configured_path = ["MRD_UI_APP_PATH", "RDESK_APP_PATH", "MRD_UI_PATH"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        let current_exe = std::env::current_exe().ok();
        Self::from_startup_paths(
            app_name.into(),
            configured_path,
            current_exe.as_deref(),
            cfg!(debug_assertions),
        )
    }

    fn from_startup_paths(
        app_name: String,
        configured_path: Option<PathBuf>,
        current_exe: Option<&Path>,
        _development: bool,
    ) -> Self {
        let embedded_bundle = current_exe.and_then(|path| embedded_ui_bundle(path).ok());
        // Configuration may choose what to launch, but cannot create a trust
        // anchor. Debug and release peers require the same verified sealed
        // outer UI bundle containing this running service.
        let pinned_ui = embedded_bundle
            .as_deref()
            .and_then(|path| PinnedUiIdentity::from_bundle(path).ok());
        Self {
            app_name,
            ui_path: Arc::new(Mutex::new(configured_path.or(embedded_bundle))),
            pinned_ui,
        }
    }

    fn configured_ui_path(&self) -> Option<PathBuf> {
        self.ui_path.lock().unwrap().clone()
    }

    fn activate(&self) -> anyhow::Result<()> {
        if let Some(path) = self.configured_ui_path() {
            if is_app_bundle(&path) {
                run_open_for_path(&path)?;
                return Ok(());
            }
        }

        run_open_for_app(&self.app_name)
    }

    fn launch(&self) -> anyhow::Result<Option<u32>> {
        if let Some(path) = self.configured_ui_path() {
            if is_app_bundle(&path) {
                run_open_for_path(&path)?;
                return Ok(wait_for_pid(|| self.get_ui_pid(), Duration::from_secs(2)));
            }

            if path.exists() {
                let child = Command::new(&path)
                    .spawn()
                    .with_context(|| format!("spawn UI executable {}", path.display()))?;
                return Ok(Some(child.id()));
            }

            return Err(anyhow!(
                "configured UI path does not exist: {}",
                path.display()
            ));
        }

        run_open_for_app(&self.app_name)?;
        Ok(wait_for_pid(|| self.get_ui_pid(), Duration::from_secs(2)))
    }
}

impl Default for MacosUiLauncher {
    fn default() -> Self {
        Self::new("Rdesk")
    }
}

impl UiLauncherPort for MacosUiLauncher {
    fn is_ui_running(&self) -> anyhow::Result<bool> {
        Ok(self.get_ui_pid()?.is_some())
    }

    fn get_ui_pid(&self) -> anyhow::Result<Option<u32>> {
        let mut candidates = pids_from_command("pgrep", &["-x", self.app_name.as_str()])?;
        if let Some(path) = self.configured_ui_path() {
            candidates.extend(pids_from_command("pgrep", &["-f", path_to_str(&path)?])?);
        }
        if let Some(pin) = &self.pinned_ui {
            candidates.extend(pids_from_command(
                "pgrep",
                &[
                    "-x",
                    path_to_str(Path::new(pin.executable_path.file_name().unwrap()))?,
                ],
            )?);
        }
        candidates.sort_unstable();
        candidates.dedup();
        for pid in candidates {
            let Some(path) = live_code(pid)
                .and_then(|code| code.path(CodeSigningFlags::NONE).ok())
                .and_then(|url| url.to_path())
                .and_then(|path| configured_ui_executable_path(&path).ok())
            else {
                continue;
            };
            if self.is_trusted_ui_peer(pid, Some(&path))? {
                return Ok(Some(pid));
            }
        }
        Ok(None)
    }

    fn launch_or_focus(&self, _request: UiLaunchRequest) -> anyhow::Result<UiLaunchResult> {
        if let Some(pid) = self.get_ui_pid()? {
            self.activate()?;
            return Ok(UiLaunchResult::FocusedExisting { pid });
        }

        match self.launch() {
            Ok(Some(pid)) => Ok(UiLaunchResult::SpawnedNew { pid }),
            Ok(None) => Ok(UiLaunchResult::Failed {
                error: format!("launched {} but could not resolve app pid", self.app_name),
            }),
            Err(error) => Ok(UiLaunchResult::Failed {
                error: error.to_string(),
            }),
        }
    }

    fn set_ui_path(&self, path: PathBuf) -> anyhow::Result<()> {
        *self.ui_path.lock().unwrap() = Some(path);
        Ok(())
    }

    fn get_ui_path(&self) -> anyhow::Result<Option<PathBuf>> {
        Ok(self.configured_ui_path())
    }

    fn is_trusted_ui_peer(
        &self,
        peer_pid: u32,
        peer_executable_path: Option<&Path>,
    ) -> anyhow::Result<bool> {
        // The PID and image path come from the kernel-bound socket peer. The
        // request's declared PID/path is intentionally never used here.
        let Some(peer_executable_path) = peer_executable_path else {
            return Ok(false);
        };

        // Resolve the live process through Security.framework using its PID,
        // then require the signed Rdesk bundle identity. This binds consent
        // to macOS's code identity instead of a forgeable same-UID path.
        let Some(guest) = live_code(peer_pid) else {
            return Ok(false);
        };
        // Every signed peer, including Apple-anchored code, must match the
        // immutable startup pin. A mutable launch path cannot grant access.
        // Release startup verifies the sealed outer UI and embedded service.
        let Some(pin) = &self.pinned_ui else {
            return Ok(false);
        };
        if !paths_refer_to_same_file(&pin.executable_path, peer_executable_path) {
            return Ok(false);
        }
        let Ok(current) = PinnedUiIdentity::from_bundle(&pin.executable_path) else {
            return Ok(false);
        };
        if current.requirement != pin.requirement {
            return Ok(false);
        }
        let requirement: SecRequirement = pin.requirement.parse()?;
        Ok(guest
            .check_validity(CodeSigningFlags::NONE, &requirement)
            .is_ok())
    }
}

fn live_code(pid: u32) -> Option<SecCode> {
    let pid = libc::pid_t::try_from(pid).ok().filter(|pid| *pid > 0)?;
    let mut attributes = GuestAttributes::new();
    attributes.set_pid(pid);
    SecCode::copy_guest_with_attribues(None, &attributes, CodeSigningFlags::NONE).ok()
}

fn configured_ui_executable_path(path: &Path) -> anyhow::Result<PathBuf> {
    if is_app_bundle(path) {
        let output = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", "Print :CFBundleExecutable"])
            .arg(path.join("Contents/Info.plist"))
            .output()
            .context("read Rdesk CFBundleExecutable")?;
        if !output.status.success() {
            return Err(anyhow!("Rdesk CFBundleExecutable is unavailable"));
        }
        let executable = String::from_utf8(output.stdout)?;
        bundle_executable_path(path, executable.trim())
    } else {
        Ok(path.to_path_buf())
    }
}

fn bundle_executable_path(bundle: &Path, executable: &str) -> anyhow::Result<PathBuf> {
    if executable.is_empty()
        || matches!(executable, "." | "..")
        || !executable
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err(anyhow!("Rdesk CFBundleExecutable is invalid"));
    }
    Ok(bundle.join("Contents/MacOS").join(executable))
}

fn executable_bundle(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && is_app_bundle(bundle))
        .then(|| bundle.to_path_buf())
}

fn embedded_ui_bundle(service_executable: &Path) -> anyhow::Result<PathBuf> {
    let service_executable = std::fs::canonicalize(service_executable)?;
    let outer = service_executable
        .ancestors()
        .nth(6)
        .filter(|path| path.file_name().is_some_and(|name| name == "Rdesk.app"))
        .ok_or_else(|| anyhow!("service is not embedded in Rdesk.app"))?;
    let expected_service =
        outer.join("Contents/Resources/MrdService.app/Contents/MacOS/mrd-service");
    if service_executable != expected_service {
        return Err(anyhow!("unexpected embedded service location"));
    }
    // Checking the outer resource envelope before trusting its UI also seals
    // the relationship to the embedded service. Bind the running service to
    // that verified image, rather than merely inspecting a nearby app.
    validated_cdhash_requirement(outer, UI_IDENTIFIER)?;
    let service_bundle = executable_bundle(&service_executable).unwrap();
    let requirement = validated_cdhash_requirement(&service_bundle, SERVICE_IDENTIFIER)?;
    SecCode::for_self(CodeSigningFlags::NONE)?
        .check_validity(CodeSigningFlags::NONE, &requirement.parse()?)?;
    Ok(outer.to_path_buf())
}

fn paths_refer_to_same_file(expected: &Path, actual: &Path) -> bool {
    match (
        std::fs::canonicalize(expected),
        std::fs::canonicalize(actual),
    ) {
        (Ok(expected), Ok(actual)) => expected == actual,
        _ => false,
    }
}

// These two signing-information APIs are not wrapped by security-framework
// 3.7. Keep the FFI local and store only Rust-owned paths and hashes in the
// launcher; the CoreFoundation objects never cross threads.
type CfRef = *const c_void;

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecStaticCodeCreateWithPath(path: CfRef, flags: u32, code: *mut CfRef) -> i32;
    fn SecStaticCodeCheckValidity(code: CfRef, flags: u32, requirement: CfRef) -> i32;
    fn SecRequirementCreateWithString(text: CfRef, flags: u32, requirement: *mut CfRef) -> i32;
    fn SecCodeCopySigningInformation(code: CfRef, flags: u32, information: *mut CfRef) -> i32;
    static kSecCodeInfoUnique: CfRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(object: CfRef);
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: CfRef,
        bytes: *const u8,
        length: isize,
        is_directory: u8,
    ) -> CfRef;
    fn CFStringCreateWithCString(
        allocator: CfRef,
        text: *const libc::c_char,
        encoding: u32,
    ) -> CfRef;
    fn CFDictionaryGetValue(dictionary: CfRef, key: CfRef) -> CfRef;
    fn CFGetTypeID(object: CfRef) -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFDataGetLength(data: CfRef) -> isize;
    fn CFDataGetBytePtr(data: CfRef) -> *const u8;
}

struct OwnedCf(CfRef);

impl OwnedCf {
    fn new(object: CfRef) -> anyhow::Result<Self> {
        if object.is_null() {
            Err(anyhow!("macOS code-signing object is unavailable"))
        } else {
            Ok(Self(object))
        }
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

fn validated_cdhash_requirement(path: &Path, identifier: &str) -> anyhow::Result<String> {
    let bytes = path.as_os_str().as_bytes();
    let identity = CString::new(format!(r#"identifier "{identifier}""#))?;
    unsafe {
        let url = OwnedCf::new(CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            bytes.as_ptr(),
            bytes.len().try_into()?,
            u8::from(path.is_dir()),
        ))?;
        let mut code = std::ptr::null();
        let result = SecStaticCodeCreateWithPath(url.0, 0, &mut code);
        if result != 0 {
            return Err(anyhow!("macOS static code lookup failed: {result}"));
        }
        let code = OwnedCf::new(code)?;
        let text = OwnedCf::new(CFStringCreateWithCString(
            std::ptr::null(),
            identity.as_ptr(),
            0x0800_0100, // kCFStringEncodingUTF8
        ))?;
        let mut requirement = std::ptr::null();
        let result = SecRequirementCreateWithString(text.0, 0, &mut requirement);
        if result != 0 {
            return Err(anyhow!(
                "macOS static identity requirement failed: {result}"
            ));
        }
        let requirement = OwnedCf::new(requirement)?;
        let flags = CodeSigningFlags::STRICT_VALIDATE | CodeSigningFlags::CHECK_NESTED_CODE;
        let result = SecStaticCodeCheckValidity(code.0, flags.bits(), requirement.0);
        if result != 0 {
            return Err(anyhow!(
                "macOS static signature validation failed: {result}"
            ));
        }
        let mut information = std::ptr::null();
        let result = SecCodeCopySigningInformation(code.0, 0, &mut information);
        if result != 0 {
            return Err(anyhow!("macOS signing information lookup failed: {result}"));
        }
        let information = OwnedCf::new(information)?;
        let hash = CFDictionaryGetValue(information.0, kSecCodeInfoUnique);
        if hash.is_null() || CFGetTypeID(hash) != CFDataGetTypeID() || CFDataGetLength(hash) != 20 {
            return Err(anyhow!("macOS code directory hash is unavailable"));
        }
        let bytes = CFDataGetBytePtr(hash);
        if bytes.is_null() {
            return Err(anyhow!("macOS code directory hash is unavailable"));
        }
        let hash: String = std::slice::from_raw_parts(bytes, 20)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(format!(r#"identifier "{identifier}" and cdhash H"{hash}""#))
    }
}

pub struct MacosTray {
    model: Mutex<Option<TrayModel>>,
}

impl MacosTray {
    pub fn new() -> Self {
        Self {
            model: Mutex::new(None),
        }
    }
}

impl Default for MacosTray {
    fn default() -> Self {
        Self::new()
    }
}

impl TrayPort for MacosTray {
    fn install(&self, model: TrayModel) -> anyhow::Result<()> {
        tracing::info!("MacosTray::install called; native NSStatusItem adapter is not wired yet");
        *self.model.lock().unwrap() = Some(model);
        Ok(())
    }

    fn update(&self, model: TrayModel) -> anyhow::Result<()> {
        *self.model.lock().unwrap() = Some(model);
        Ok(())
    }

    fn show_notification(&self, title: &str, message: &str) -> anyhow::Result<()> {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            escape_applescript_string(message),
            escape_applescript_string(title)
        );
        run_command_status("osascript", &["-e", script.as_str()])
    }

    fn shutdown(&self) -> anyhow::Result<()> {
        *self.model.lock().unwrap() = None;
        Ok(())
    }

    fn is_available(&self) -> bool {
        false
    }
}

pub struct MacosAutostart {
    entry_name: String,
    label: String,
    executable_path: PathBuf,
}

impl MacosAutostart {
    pub fn for_current_exe(entry_name: impl Into<String>) -> Self {
        let entry_name = entry_name.into();
        let executable_path =
            std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/mrd-service"));
        Self::with_path(entry_name, executable_path)
    }

    pub fn with_path(entry_name: impl Into<String>, executable_path: PathBuf) -> Self {
        let entry_name = entry_name.into();
        let label = launch_agent_label(&entry_name);
        Self {
            entry_name,
            label,
            executable_path,
        }
    }

    fn launch_agent_path(&self) -> anyhow::Result<PathBuf> {
        let home = std::env::var("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{}.plist", self.label)))
    }

    fn plist(&self) -> String {
        let log_dir = macos_log_dir();
        let stdout = log_dir.join("mrd-service.launchd.stdout.log");
        let stderr = log_dir.join("mrd-service.launchd.stderr.log");

        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
  <key>StandardOutPath</key>
  <string>{}</string>
  <key>StandardErrorPath</key>
  <string>{}</string>
</dict>
</plist>
"#,
            escape_xml(&self.label),
            escape_xml(path_to_str(&self.executable_path).unwrap_or("mrd-service")),
            escape_xml(path_to_str(&stdout).unwrap_or("mrd-service.launchd.stdout.log")),
            escape_xml(path_to_str(&stderr).unwrap_or("mrd-service.launchd.stderr.log")),
        )
    }

    fn bootstrap(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(uid) = current_uid() {
            let domain = format!("gui/{uid}");
            let path_string = path_to_str(path)?;
            let status = Command::new("launchctl")
                .args(["bootstrap", domain.as_str(), path_string])
                .status()
                .context("launchctl bootstrap failed to start")?;
            if status.success() {
                return Ok(());
            }
        }

        run_command_status("launchctl", &["load", path_to_str(path)?])
    }

    fn bootout(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(uid) = current_uid() {
            let service = format!("gui/{uid}/{}", self.label);
            let _ = Command::new("launchctl")
                .args(["bootout", service.as_str()])
                .status();
        }

        if path.exists() {
            let _ = Command::new("launchctl")
                .args(["unload", path_to_str(path)?])
                .status();
        }

        Ok(())
    }
}

impl AutostartPort for MacosAutostart {
    fn is_enabled(&self) -> anyhow::Result<bool> {
        Ok(self.launch_agent_path()?.exists())
    }

    fn set_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let path = self.launch_agent_path()?;
        if enabled {
            if !self.executable_path.exists() {
                return Err(anyhow!(
                    "mrd-service executable does not exist: {}",
                    self.executable_path.display()
                ));
            }

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            std::fs::create_dir_all(macos_log_dir()).context("create macOS service log dir")?;
            std::fs::write(&path, self.plist())
                .with_context(|| format!("write {}", path.display()))?;
            self.bootstrap(&path)
        } else {
            self.bootout(&path)?;
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("remove {}", path.display()))?;
            }
            Ok(())
        }
    }

    fn is_supported(&self) -> bool {
        true
    }

    fn get_entry_name(&self) -> &str {
        &self.entry_name
    }
}

fn is_app_bundle(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("app"))
        .unwrap_or(false)
}

fn run_open_for_app(app_name: &str) -> anyhow::Result<()> {
    run_command_status("open", &["-a", app_name])
}

fn run_open_for_path(path: &Path) -> anyhow::Result<()> {
    run_command_status("open", &[path_to_str(path)?])
}

fn run_command_status(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {program}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    Err(anyhow!(
        "{program} exited with status {}: {}",
        output.status,
        detail
    ))
}

fn pids_from_command(program: &str, args: &[&str]) -> anyhow::Result<Vec<u32>> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {program}"))?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect())
}

fn wait_for_pid<F>(mut get_pid: F, timeout: Duration) -> Option<u32>
where
    F: FnMut() -> anyhow::Result<Option<u32>>,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(Some(pid)) = get_pid() {
            return Some(pid);
        }
        thread::sleep(Duration::from_millis(100));
    }
    None
}

fn path_to_str(path: &Path) -> anyhow::Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))
}

fn current_uid() -> Option<String> {
    let output = Command::new("id").arg("-u").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if uid.is_empty() {
        None
    } else {
        Some(uid)
    }
}

fn launch_agent_label(entry_name: &str) -> String {
    if entry_name.contains('.') {
        entry_name.to_string()
    } else {
        format!("{DEFAULT_LABEL_PREFIX}.{entry_name}")
    }
}

fn macos_log_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("Library")
        .join("Logs")
        .join("mini-remote-desktop")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Child;

    struct TestApp {
        directory: PathBuf,
        bundle: PathBuf,
    }

    impl TestApp {
        fn new(identifier: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "mrd-ui-signature-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let bundle = directory.join("Rdesk.app");
            std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
            // System /bin/sleep has multiple arm64e subtypes. Make a thin
            // test image so static and dynamic validation pin the same image,
            // as they do for the native thin product build.
            let architecture = if cfg!(target_arch = "aarch64") {
                "arm64e"
            } else {
                "x86_64"
            };
            let output = Command::new("/usr/bin/lipo")
                .args(["/bin/sleep", "-thin", architecture, "-output"])
                .arg(bundle.join("Contents/MacOS/app"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            std::fs::write(
                bundle.join("Contents/Info.plist"),
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{identifier}</string>
<key>CFBundleExecutable</key><string>app</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>"#
                ),
            )
            .unwrap();
            let app = Self { directory, bundle };
            app.sign(identifier);
            app
        }

        fn sign(&self, identifier: &str) {
            let output = Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-", "--identifier", identifier])
                .arg(&self.bundle)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn executable(&self) -> PathBuf {
            self.bundle.join("Contents/MacOS/app")
        }

        // Peer-checker fixture only: startup relationship is exercised separately.
        fn pinned_peer_launcher(&self, app_name: &str) -> MacosUiLauncher {
            MacosUiLauncher {
                app_name: app_name.to_owned(),
                ui_path: Arc::new(Mutex::new(Some(self.bundle.clone()))),
                pinned_ui: Some(PinnedUiIdentity::from_bundle(&self.bundle).unwrap()),
            }
        }

        fn spawn(&self) -> TestProcess {
            let child = Command::new(self.executable()).arg("60").spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while live_code(child.id()).is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            TestProcess(child)
        }
    }

    impl Drop for TestApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    struct TestProcess(Child);

    impl Drop for TestProcess {
        fn drop(&mut self) {
            // This is only the test-owned sleep process, never the live app.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn bundle_executable_uses_plist_metadata_and_rejects_path_traversal() {
        let app = TestApp::new(UI_IDENTIFIER);
        assert_eq!(
            configured_ui_executable_path(&app.bundle).unwrap(),
            app.executable()
        );
        for invalid in [
            "",
            ".",
            "..",
            "../app",
            "/bin/sleep",
            "app\nother",
            "app;other",
        ] {
            assert!(bundle_executable_path(&app.bundle, invalid).is_err());
        }
    }

    #[test]
    fn static_pin_rejects_wrong_identity_unsigned_code_and_tampered_resources() {
        let wrong = TestApp::new("com.example.impostor");
        assert!(PinnedUiIdentity::from_bundle(&wrong.bundle).is_err());
        let app = TestApp::new(UI_IDENTIFIER);
        assert!(PinnedUiIdentity::from_bundle(&app.bundle).is_ok());
        std::fs::write(app.bundle.join("Contents/Info.plist"), b"tampered").unwrap();
        assert!(validated_cdhash_requirement(&app.bundle, UI_IDENTIFIER).is_err());
        let unsigned = TestApp::new(UI_IDENTIFIER);
        let result = Command::new("/usr/bin/codesign")
            .arg("--remove-signature")
            .arg(unsigned.executable())
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(PinnedUiIdentity::from_bundle(&unsigned.bundle).is_err());
    }

    #[test]
    fn startup_pin_accepts_live_code_without_process_name_gate_and_stays_fixed() {
        let app = TestApp::new(UI_IDENTIFIER);
        let other = TestApp::new(UI_IDENTIFIER);
        let launcher = app.pinned_peer_launcher("ANameThatCannotMatchThePeer");
        let original_pin = launcher.pinned_ui.as_ref().unwrap().requirement.clone();
        launcher.set_ui_path(other.bundle.clone()).unwrap();
        assert_eq!(
            launcher.pinned_ui.as_ref().unwrap().requirement,
            original_pin
        );
        let child = app.spawn();
        live_code(child.0.id())
            .unwrap()
            .check_validity(CodeSigningFlags::NONE, &original_pin.parse().unwrap())
            .expect("test process must match its statically verified CDHash");
        assert!(launcher
            .is_trusted_ui_peer(child.0.id(), Some(&app.executable()))
            .unwrap());
        let other_child = other.spawn();
        assert!(!launcher
            .is_trusted_ui_peer(other_child.0.id(), Some(&other.executable()))
            .unwrap());
        assert!(!launcher.is_trusted_ui_peer(child.0.id(), None).unwrap());
        assert!(!launcher
            .is_trusted_ui_peer(0, Some(&app.executable()))
            .unwrap());
        assert!(!launcher
            .is_trusted_ui_peer(std::process::id(), Some(&app.executable()))
            .unwrap());
        std::fs::write(app.bundle.join("Contents/Info.plist"), b"tampered").unwrap();
        assert!(!launcher
            .is_trusted_ui_peer(child.0.id(), Some(&app.executable()))
            .unwrap());
    }

    #[test]
    fn a_new_signature_at_the_same_path_cannot_reuse_the_startup_pin() {
        let app = TestApp::new(UI_IDENTIFIER);
        let launcher = app.pinned_peer_launcher("Rdesk");
        let original = launcher.pinned_ui.as_ref().unwrap().requirement.clone();
        std::fs::create_dir_all(app.bundle.join("Contents/Resources")).unwrap();
        std::fs::write(
            app.bundle.join("Contents/Resources/new-resource"),
            b"new build",
        )
        .unwrap();
        app.sign(UI_IDENTIFIER);
        assert_ne!(
            original,
            validated_cdhash_requirement(&app.bundle, UI_IDENTIFIER).unwrap()
        );
        let child = app.spawn();
        assert!(!launcher
            .is_trusted_ui_peer(child.0.id(), Some(&app.executable()))
            .unwrap());
    }

    #[test]
    fn configured_paths_cannot_create_startup_trust_without_containing_service() {
        let app = TestApp::new(UI_IDENTIFIER);
        for development in [false, true] {
            let launcher = MacosUiLauncher::from_startup_paths(
                "Rdesk".to_owned(),
                Some(app.bundle.clone()),
                Some(&app.executable()),
                development,
            );
            assert!(launcher.pinned_ui.is_none());
        }
    }

    #[test]
    fn release_paths_and_later_attachments_cannot_create_an_ad_hoc_pin() {
        let app = TestApp::new(UI_IDENTIFIER);
        let launcher = MacosUiLauncher::from_startup_paths(
            "Rdesk".to_owned(),
            Some(app.bundle.clone()),
            None,
            false,
        );
        assert!(launcher.pinned_ui.is_none());
        let unconfigured =
            MacosUiLauncher::from_startup_paths("Rdesk".to_owned(), None, None, true);
        unconfigured.set_ui_path(app.executable()).unwrap();
        assert!(unconfigured.pinned_ui.is_none());
        let child = app.spawn();
        assert!(!launcher
            .is_trusted_ui_peer(child.0.id(), Some(&app.executable()))
            .unwrap());
        assert!(!unconfigured
            .is_trusted_ui_peer(child.0.id(), Some(&app.executable()))
            .unwrap());
        assert!(embedded_ui_bundle(&app.executable()).is_err());
    }

    #[test]
    fn missing_files_and_executable_symlinks_are_not_trusted() {
        let app = TestApp::new(UI_IDENTIFIER);
        let missing = app.directory.join("missing");
        assert!(!paths_refer_to_same_file(&missing, &missing));
        std::fs::remove_file(app.executable()).unwrap();
        std::os::unix::fs::symlink("/bin/sleep", app.executable()).unwrap();
        assert!(PinnedUiIdentity::from_bundle(&app.bundle).is_err());
    }

    #[test]
    fn launch_agent_label_uses_reverse_dns_prefix() {
        assert_eq!(
            launch_agent_label("mrd-service"),
            "com.mini-remote-desktop.mrd-service"
        );
        assert_eq!(
            launch_agent_label("com.example.mrd-service"),
            "com.example.mrd-service"
        );
    }

    #[test]
    fn plist_escapes_xml_values() {
        let autostart = MacosAutostart::with_path(
            "mrd-service",
            PathBuf::from("/tmp/Mini & Remote/mrd-service"),
        );
        let plist = autostart.plist();
        assert!(plist.contains("com.mini-remote-desktop.mrd-service"));
        assert!(plist.contains("/tmp/Mini &amp; Remote/mrd-service"));
    }

    #[test]
    fn applescript_strings_escape_quotes() {
        assert_eq!(
            escape_applescript_string(r#"hello "Rdesk""#),
            r#"hello \"Rdesk\""#
        );
    }
}
