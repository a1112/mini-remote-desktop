use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}
