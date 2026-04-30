use super::{AutostartPort, TrayModel, TrayPort, UiLaunchRequest, UiLaunchResult, UiLauncherPort};
use anyhow::{anyhow, Context};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const DEFAULT_LABEL_PREFIX: &str = "com.mini-remote-desktop";

pub struct MacosUiLauncher {
    app_name: String,
    ui_path: Arc<Mutex<Option<PathBuf>>>,
}

impl MacosUiLauncher {
    pub fn new(app_name: impl Into<String>) -> Self {
        Self {
            app_name: app_name.into(),
            ui_path: Arc::new(Mutex::new(None)),
        }
    }

    fn configured_ui_path(&self) -> Option<PathBuf> {
        self.ui_path.lock().unwrap().clone().or_else(|| {
            ["MRD_UI_APP_PATH", "RDESK_APP_PATH", "MRD_UI_PATH"]
                .into_iter()
                .filter_map(|key| std::env::var(key).ok())
                .find(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        })
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
        if let Some(pid) = first_pid_from_command("pgrep", &["-x", self.app_name.as_str()])? {
            return Ok(Some(pid));
        }

        if let Some(path) = self.configured_ui_path() {
            if let Some(pid) = first_pid_from_command("pgrep", &["-f", path_to_str(&path)?])? {
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

fn first_pid_from_command(program: &str, args: &[&str]) -> anyhow::Result<Option<u32>> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {program}"))?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<u32>().ok()))
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
