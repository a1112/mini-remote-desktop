use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

const SETTINGS_FILE_NAME: &str = "rdesk-app-settings.json";
const SETTINGS_ENV_VAR: &str = "RDESK_APP_SETTINGS_PATH";
static SETTINGS_FILE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    #[default]
    HideToTray,
    ExitUi,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UiPreferences {
    pub close_behavior: CloseBehavior,
}

pub struct UiPreferencesCache {
    preferences: Mutex<UiPreferences>,
}

impl UiPreferencesCache {
    pub fn from_settings(settings: &AppSettings) -> Self {
        Self {
            preferences: Mutex::new(UiPreferences {
                close_behavior: settings.close_behavior,
            }),
        }
    }

    pub fn snapshot(&self) -> Result<UiPreferences, String> {
        self.preferences
            .lock()
            .map(|preferences| *preferences)
            .map_err(|_| "应用设置缓存不可用".to_string())
    }

    pub fn set_close_behavior(
        &self,
        path: &Path,
        close_behavior: CloseBehavior,
    ) -> Result<UiPreferences, String> {
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "应用设置缓存不可用".to_string())?;
        let saved = update_settings(path, |settings| settings.close_behavior = close_behavior)?;
        *preferences = UiPreferences {
            close_behavior: saved.close_behavior,
        };
        Ok(*preferences)
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    #[serde(default)]
    pub close_behavior: CloseBehavior,
    #[serde(default)]
    pub decode_policy: DecodePolicy,
    #[serde(default = "mrd_ffmpeg::golden_settings")]
    pub ffmpeg: mrd_ffmpeg::FfmpegSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            close_behavior: CloseBehavior::default(),
            decode_policy: DecodePolicy::default(),
            ffmpeg: mrd_ffmpeg::golden_settings(),
        }
    }
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
    let _guard = SETTINGS_FILE_LOCK
        .lock()
        .map_err(|_| "应用设置文件锁不可用".to_string())?;
    load_settings_unlocked(path)
}

fn load_settings_unlocked(path: &Path) -> Result<AppSettings, String> {
    if !path.exists() {
        return Ok(AppSettings::default());
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| format!("读取应用设置失败 ({}): {error}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("解析应用设置失败 ({}): {error}", path.display()))
}

#[cfg(test)]
pub fn save_settings(path: &Path, settings: &AppSettings) -> Result<(), String> {
    let _guard = SETTINGS_FILE_LOCK
        .lock()
        .map_err(|_| "应用设置文件锁不可用".to_string())?;
    save_settings_unlocked(path, settings)
}

pub fn update_settings(
    path: &Path,
    update: impl FnOnce(&mut AppSettings),
) -> Result<AppSettings, String> {
    let _guard = SETTINGS_FILE_LOCK
        .lock()
        .map_err(|_| "应用设置文件锁不可用".to_string())?;
    let mut settings = load_settings_unlocked(path)?;
    update(&mut settings);
    save_settings_unlocked(path, &settings)?;
    Ok(settings)
}

fn save_settings_unlocked(path: &Path, settings: &AppSettings) -> Result<(), String> {
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
    use super::{
        default_settings_path, load_settings, save_settings, update_settings, AppSettings,
        CloseBehavior, DecodePolicy, UiPreferencesCache,
    };

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
                ..AppSettings::default()
            },
        )
        .expect("save settings");
        let nvdec = load_settings(&path).expect("reload nvdec settings");
        assert_eq!(nvdec.decode_policy, DecodePolicy::Nvdec);

        save_settings(
            &path,
            &AppSettings {
                decode_policy: DecodePolicy::Software,
                ..AppSettings::default()
            },
        )
        .expect("save software settings");
        let software = load_settings(&path).expect("reload software settings");
        assert_eq!(software.decode_policy, DecodePolicy::Software);

        std::fs::remove_file(&path).expect("cleanup temp settings");
    }

    #[test]
    fn load_settings_defaults_ffmpeg_to_golden_values() {
        let path = unique_settings_path("ffmpeg-defaults");

        let settings = load_settings(&path).expect("load defaults");

        assert!(settings.ffmpeg.enabled);
        assert_eq!(
            settings.ffmpeg,
            mrd_ffmpeg::FfmpegSettings::golden_for_platform(mrd_ffmpeg::FfmpegPlatform::current())
        );
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

    #[test]
    fn legacy_settings_default_to_hide_to_tray() {
        let settings: AppSettings = serde_json::from_str(r#"{"decode_policy":"software"}"#)
            .expect("load settings created before close preferences");

        assert_eq!(
            serde_json::to_value(settings).expect("serialize settings")["close_behavior"],
            "hide_to_tray"
        );
    }

    #[test]
    fn rejects_unknown_close_behavior_in_saved_settings() {
        let result = serde_json::from_str::<AppSettings>(r#"{"close_behavior":"stop_service"}"#);

        assert!(
            result.is_err(),
            "unsupported close behavior must not be silently accepted"
        );
    }

    #[test]
    fn close_preference_is_restored_on_next_startup() {
        let path = unique_settings_path("close-preference-roundtrip");
        let mut initial = AppSettings::default();
        initial.decode_policy = DecodePolicy::Software;
        initial.ffmpeg.enabled = false;
        initial.ffmpeg.channel = "custom".to_string();
        save_settings(&path, &initial).expect("save initial settings");
        let cache = UiPreferencesCache::from_settings(&initial);

        let confirmed = cache
            .set_close_behavior(&path, CloseBehavior::ExitUi)
            .expect("save close behavior");
        let loaded = load_settings(&path).expect("reload settings");
        let restarted = UiPreferencesCache::from_settings(&loaded);

        assert_eq!(confirmed.close_behavior, CloseBehavior::ExitUi);
        assert_eq!(restarted.snapshot().unwrap(), confirmed);
        assert_eq!(loaded.decode_policy, DecodePolicy::Software);
        assert_eq!(loaded.ffmpeg, initial.ffmpeg);
        std::fs::remove_file(&path).expect("cleanup settings");
    }

    #[test]
    fn failed_save_keeps_confirmed_close_preference_in_cache() {
        let parent_file = unique_settings_path("close-preference-unwritable-parent");
        std::fs::create_dir_all(parent_file.parent().unwrap()).expect("create test directory");
        std::fs::write(&parent_file, "not a directory").expect("create blocked parent");
        let cache = UiPreferencesCache::from_settings(&AppSettings {
            close_behavior: CloseBehavior::ExitUi,
            ..AppSettings::default()
        });

        let result = cache.set_close_behavior(
            &parent_file.join("settings.json"),
            CloseBehavior::HideToTray,
        );

        assert!(result.is_err());
        assert_eq!(
            cache.snapshot().unwrap().close_behavior,
            CloseBehavior::ExitUi
        );
        assert_eq!(
            std::fs::read_to_string(&parent_file).unwrap(),
            "not a directory"
        );
        std::fs::remove_file(&parent_file).expect("cleanup blocked parent");
    }

    #[test]
    fn concurrent_updates_preserve_every_completed_mutation() {
        let path = unique_settings_path("serialized-settings-updates");
        let start = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    for _ in 0..20 {
                        update_settings(&path, |settings| {
                            let previous = settings.ffmpeg.channel.parse::<usize>().unwrap_or(0);
                            settings.ffmpeg.channel = (previous + 1).to_string();
                            settings.close_behavior = CloseBehavior::ExitUi;
                            settings.decode_policy = DecodePolicy::Software;
                        })
                        .expect("persist concurrent mutation");
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("settings update thread");
        }

        let loaded = load_settings(&path).expect("reload settings");
        assert_eq!(loaded.ffmpeg.channel, "160");
        assert_eq!(loaded.close_behavior, CloseBehavior::ExitUi);
        assert_eq!(loaded.decode_policy, DecodePolicy::Software);
        std::fs::remove_file(&path).expect("cleanup settings");
    }

    fn unique_settings_path(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join("mini-remote-desktop-tests")
            .join(format!(
                "{prefix}-{}.json",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time")
                    .as_nanos()
            ))
    }
}
