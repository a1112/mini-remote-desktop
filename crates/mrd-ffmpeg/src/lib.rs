use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

const WINDOWS_RELEASE_ESSENTIALS_ARCHIVE_URL: &str =
    "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
const WINDOWS_RELEASE_ESSENTIALS_SHA256_URL: &str =
    "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfmpegPlatform {
    Windows,
    Macos,
    Linux,
    Unknown,
}

impl FfmpegPlatform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FfmpegDownloadSettings {
    #[serde(default)]
    pub archive_url: String,
    #[serde(default)]
    pub sha256_url: Option<String>,
    #[serde(default)]
    pub require_sha256: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FfmpegSettings {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_channel")]
    pub channel: String,
    #[serde(default)]
    pub install_dir: Option<PathBuf>,
    #[serde(default)]
    pub ffmpeg_path: Option<PathBuf>,
    #[serde(default)]
    pub ffprobe_path: Option<PathBuf>,
    #[serde(default = "default_download_settings")]
    pub download: FfmpegDownloadSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FfmpegProbeResult {
    pub available: bool,
    pub ffmpeg_path: Option<PathBuf>,
    pub ffprobe_path: Option<PathBuf>,
    pub ffmpeg_version: Option<String>,
    pub ffprobe_version: Option<String>,
    pub reason: Option<String>,
}

impl FfmpegSettings {
    pub fn golden_for_platform(platform: FfmpegPlatform) -> Self {
        match platform {
            FfmpegPlatform::Windows => Self {
                enabled: true,
                channel: default_channel(),
                install_dir: Some(default_managed_install_dir_for_channel(&default_channel())),
                ffmpeg_path: None,
                ffprobe_path: None,
                download: FfmpegDownloadSettings {
                    archive_url: WINDOWS_RELEASE_ESSENTIALS_ARCHIVE_URL.to_string(),
                    sha256_url: Some(WINDOWS_RELEASE_ESSENTIALS_SHA256_URL.to_string()),
                    require_sha256: true,
                },
            },
            _ => Self {
                enabled: true,
                channel: "system".to_string(),
                install_dir: None,
                ffmpeg_path: None,
                ffprobe_path: None,
                download: FfmpegDownloadSettings {
                    archive_url: String::new(),
                    sha256_url: None,
                    require_sha256: false,
                },
            },
        }
    }
}

impl Default for FfmpegSettings {
    fn default() -> Self {
        golden_settings()
    }
}

pub fn golden_settings() -> FfmpegSettings {
    FfmpegSettings::golden_for_platform(FfmpegPlatform::current())
}

pub fn probe_ffmpeg(settings: &FfmpegSettings) -> FfmpegProbeResult {
    if !settings.enabled {
        return unavailable("FFmpeg optional tooling is disabled in settings.");
    }

    let ffmpeg = resolve_tool("ffmpeg", settings.ffmpeg_path.as_deref(), settings.install_dir.as_deref());
    let ffprobe = resolve_tool(
        "ffprobe",
        settings.ffprobe_path.as_deref(),
        settings.install_dir.as_deref(),
    );

    let Some(ffmpeg_path) = ffmpeg else {
        return unavailable("ffmpeg executable was not found in configured paths or PATH.");
    };
    let Some(ffprobe_path) = ffprobe else {
        return unavailable("ffprobe executable was not found in configured paths or PATH.");
    };

    let ffmpeg_version = match probe_tool_version(&ffmpeg_path) {
        Ok(version) => version,
        Err(error) => return unavailable(format!("ffmpeg probe failed: {error}")),
    };
    let ffprobe_version = match probe_tool_version(&ffprobe_path) {
        Ok(version) => version,
        Err(error) => return unavailable(format!("ffprobe probe failed: {error}")),
    };

    FfmpegProbeResult {
        available: true,
        ffmpeg_path: Some(ffmpeg_path),
        ffprobe_path: Some(ffprobe_path),
        ffmpeg_version: Some(ffmpeg_version),
        ffprobe_version: Some(ffprobe_version),
        reason: None,
    }
}

pub fn default_managed_install_dir_for_channel(channel: &str) -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata)
            .join("mini-remote-desktop")
            .join("tools")
            .join("ffmpeg")
            .join(channel);
    }

    std::env::temp_dir()
        .join("mini-remote-desktop")
        .join("tools")
        .join("ffmpeg")
        .join(channel)
}

fn default_enabled() -> bool {
    true
}

fn default_channel() -> String {
    "release-essentials".to_string()
}

fn default_download_settings() -> FfmpegDownloadSettings {
    FfmpegSettings::golden_for_platform(FfmpegPlatform::current()).download
}

fn unavailable(reason: impl Into<String>) -> FfmpegProbeResult {
    FfmpegProbeResult {
        available: false,
        ffmpeg_path: None,
        ffprobe_path: None,
        ffmpeg_version: None,
        ffprobe_version: None,
        reason: Some(reason.into()),
    }
}

fn resolve_tool(tool: &str, explicit_path: Option<&Path>, install_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit_path.filter(|path| path.is_file()) {
        return Some(path.to_path_buf());
    }

    if let Some(install_dir) = install_dir {
        for candidate in install_dir_candidates(install_dir, tool) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    path_candidates(tool).into_iter().find(|candidate| candidate.is_file())
}

fn install_dir_candidates(install_dir: &Path, tool: &str) -> Vec<PathBuf> {
    tool_file_names(tool)
        .into_iter()
        .flat_map(|file_name| [install_dir.join("bin").join(&file_name), install_dir.join(file_name)])
        .collect()
}

fn path_candidates(tool: &str) -> Vec<PathBuf> {
    let Some(path_env) = std::env::var_os("PATH") else {
        return Vec::new();
    };

    std::env::split_paths(&path_env)
        .flat_map(|dir| {
            tool_file_names(tool)
                .into_iter()
                .map(move |file_name| dir.join(file_name))
        })
        .collect()
}

fn tool_file_names(tool: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            format!("{tool}.exe"),
            format!("{tool}.cmd"),
            format!("{tool}.bat"),
            tool.to_string(),
        ]
    } else {
        vec![tool.to_string()]
    }
}

fn probe_tool_version(path: &Path) -> Result<String, String> {
    let output = Command::new(path)
        .arg("-version")
        .output()
        .map_err(|error| format!("failed to run {}: {error}", path.display()))?;

    if !output.status.success() {
        return Err(format!(
            "{} exited with status {}",
            path.display(),
            output.status
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let version = stdout
        .lines()
        .chain(stderr.lines())
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("version output was empty")
        .to_string();
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_settings_use_windows_release_essentials_source() {
        let settings = FfmpegSettings::golden_for_platform(FfmpegPlatform::Windows);

        assert!(settings.enabled);
        assert_eq!(settings.channel, "release-essentials");
        assert!(settings
            .download
            .archive_url
            .ends_with("/ffmpeg-release-essentials.zip"));
        assert!(settings
            .download
            .sha256_url
            .as_deref()
            .unwrap()
            .ends_with(".zip.sha256"));
        assert!(settings.download.require_sha256);
    }

    #[test]
    fn non_windows_golden_settings_probe_without_managed_download() {
        let settings = FfmpegSettings::golden_for_platform(FfmpegPlatform::Linux);

        assert!(settings.enabled);
        assert!(settings.download.archive_url.is_empty());
        assert!(settings.download.sha256_url.is_none());
    }

    #[test]
    fn probe_succeeds_with_fake_tools_in_configured_directory() {
        let dir = unique_temp_dir("mrd-ffmpeg-probe-ok");
        write_fake_tool(&dir, "ffmpeg");
        write_fake_tool(&dir, "ffprobe");

        let mut settings = FfmpegSettings::golden_for_platform(FfmpegPlatform::Windows);
        settings.install_dir = Some(dir.clone());

        let result = probe_ffmpeg(&settings);

        assert!(result.available, "{result:?}");
        assert_eq!(
            result.ffmpeg_path.as_deref(),
            Some(dir.join(exe_name("ffmpeg")).as_path())
        );
        assert_eq!(
            result.ffprobe_path.as_deref(),
            Some(dir.join(exe_name("ffprobe")).as_path())
        );
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

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_fake_tool(dir: &std::path::Path, name: &str) {
        let path = dir.join(exe_name(name));
        #[cfg(windows)]
        {
            std::fs::write(
                &path,
                format!("@echo off\r\necho {name} version test\r\nexit /b 0\r\n"),
            )
            .expect("write fake tool");
        }

        #[cfg(not(windows))]
        {
            std::fs::write(
                &path,
                format!("#!/bin/sh\necho \"{name} version test\"\nexit 0\n"),
            )
            .expect("write fake tool");
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path)
                .expect("fake tool metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).expect("chmod fake tool");
        }
    }

    fn exe_name(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.cmd")
        } else {
            name.to_string()
        }
    }
}
