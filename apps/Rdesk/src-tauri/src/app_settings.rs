use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const SETTINGS_FILE_NAME: &str = "rdesk-app-settings.json";
const SETTINGS_ENV_VAR: &str = "RDESK_APP_SETTINGS_PATH";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DecodePolicy {
    #[default]
    Auto,
    Software,
    D3d11va,
    Nvdec,
}

impl DecodePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Software => "software",
            Self::D3d11va => "d3d11va",
            Self::Nvdec => "nvdec",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AppSettings {
    #[serde(default)]
    pub decode_policy: DecodePolicy,
}

pub fn default_settings_path() -> PathBuf {
    if let Ok(path) = std::env::var(SETTINGS_ENV_VAR) {
        return PathBuf::from(path);
    }

    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata)
            .join("mini-remote-desktop")
            .join(SETTINGS_FILE_NAME);
    }

    std::env::temp_dir()
        .join("mini-remote-desktop")
        .join(SETTINGS_FILE_NAME)
}

pub fn load_settings(path: &Path) -> Result<AppSettings, String> {
    if !path.exists() {
        return Ok(AppSettings::default());
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| format!("读取应用设置失败 ({}): {error}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("解析应用设置失败 ({}): {error}", path.display()))
}

pub fn save_settings(path: &Path, settings: &AppSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建应用设置目录失败 ({}): {error}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("序列化应用设置失败: {error}"))?;
    fs::write(path, raw).map_err(|error| format!("写入应用设置失败 ({}): {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{default_settings_path, load_settings, save_settings, AppSettings, DecodePolicy};

    #[test]
    fn load_settings_defaults_to_auto_when_file_is_missing() {
        let unique = format!(
            "rdesk-settings-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        if path.exists() {
            std::fs::remove_file(&path).expect("remove stale temp file");
        }

        let settings = load_settings(&path).expect("load default settings");
        assert_eq!(settings.decode_policy, DecodePolicy::Auto);
    }

    #[test]
    fn save_and_load_settings_roundtrip_decode_policy() {
        let unique = format!(
            "rdesk-settings-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        let path = std::env::temp_dir()
            .join("mini-remote-desktop-tests")
            .join(unique);

        save_settings(
            &path,
            &AppSettings {
                decode_policy: DecodePolicy::Nvdec,
            },
        )
        .expect("save settings");
        let nvdec = load_settings(&path).expect("reload nvdec settings");
        assert_eq!(nvdec.decode_policy, DecodePolicy::Nvdec);

        save_settings(
            &path,
            &AppSettings {
                decode_policy: DecodePolicy::Software,
            },
        )
        .expect("save software settings");
        let software = load_settings(&path).expect("reload software settings");
        assert_eq!(software.decode_policy, DecodePolicy::Software);

        std::fs::remove_file(&path).expect("cleanup temp settings");
    }

    #[test]
    fn default_settings_path_uses_env_override_when_present() {
        let override_path = std::env::temp_dir().join("override-settings.json");
        std::env::set_var("RDESK_APP_SETTINGS_PATH", &override_path);
        let path = default_settings_path();
        std::env::remove_var("RDESK_APP_SETTINGS_PATH");

        assert_eq!(path, override_path);
    }
}
