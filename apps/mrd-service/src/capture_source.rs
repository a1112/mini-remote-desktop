use anyhow::Result;
use mrd_ipc::CaptureSource;

const DEFAULT_CAPTURE_SOURCE_LIMIT: usize = 24;
const MAX_CAPTURE_SOURCE_LIMIT: usize = 48;
#[cfg(target_os = "macos")]
pub const TEST_SYNTHETIC_CV_CAPTURE_SOURCE_ID: &str = "test:synthetic-cv";
#[cfg(target_os = "macos")]
const TEST_SYNTHETIC_CV_CAPTURE_ENV: &str = "MRD_LAN_TEST_SYNTHETIC_CAPTURE";

pub fn list_capture_sources(
    _include_previews: bool,
    limit: Option<u32>,
) -> Result<Vec<CaptureSource>> {
    let mut sources = list_capture_sources_impl()?;
    #[cfg(target_os = "macos")]
    if test_synthetic_cv_capture_enabled() {
        sources.push(test_synthetic_cv_capture_source());
    }
    let limit = limit
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_CAPTURE_SOURCE_LIMIT)
        .clamp(1, MAX_CAPTURE_SOURCE_LIMIT);
    sources.truncate(limit);

    Ok(sources)
}

pub fn find_capture_source(source_id: &str) -> Result<CaptureSource> {
    let source_id = source_id.trim();
    if source_id.is_empty() {
        anyhow::bail!("capture source id is empty");
    }

    #[cfg(target_os = "macos")]
    if test_synthetic_cv_capture_enabled() && is_test_synthetic_cv_capture_source_id(source_id) {
        return Ok(test_synthetic_cv_capture_source());
    }

    list_capture_sources(false, Some(MAX_CAPTURE_SOURCE_LIMIT as u32))?
        .into_iter()
        .find(|source| {
            source.id.eq_ignore_ascii_case(source_id)
                || source
                    .id
                    .rsplit(':')
                    .next()
                    .is_some_and(|handle| handle.eq_ignore_ascii_case(source_id))
        })
        .ok_or_else(|| anyhow::anyhow!("capture source not found: {source_id}"))
}

pub fn preferred_capture_source(sources: &[CaptureSource]) -> Option<CaptureSource> {
    sources
        .iter()
        .find(|source| source.source_kind == "display_shared")
        .or_else(|| {
            sources
                .iter()
                .find(|source| source.source_kind == "display")
        })
        .or_else(|| sources.iter().find(|source| source.source_kind == "window"))
        .or_else(|| sources.first())
        .cloned()
}

#[allow(dead_code)]
pub fn default_capture_source(include_previews: bool) -> Result<CaptureSource> {
    let sources = list_capture_sources(include_previews, Some(MAX_CAPTURE_SOURCE_LIMIT as u32))?;
    preferred_capture_source(&sources)
        .ok_or_else(|| anyhow::anyhow!("no capture sources are available"))
}

#[cfg(target_os = "macos")]
pub fn test_synthetic_cv_capture_enabled() -> bool {
    test_synthetic_cv_capture_enabled_from_env_value(
        std::env::var(TEST_SYNTHETIC_CV_CAPTURE_ENV).ok().as_deref(),
    )
}

#[cfg(target_os = "macos")]
pub fn is_test_synthetic_cv_capture_source_id(source_id: &str) -> bool {
    let source_id = source_id.trim();
    source_id.eq_ignore_ascii_case(TEST_SYNTHETIC_CV_CAPTURE_SOURCE_ID)
        || source_id.eq_ignore_ascii_case("synthetic-cv")
}

#[cfg(target_os = "macos")]
pub fn test_synthetic_cv_capture_source() -> CaptureSource {
    CaptureSource {
        id: TEST_SYNTHETIC_CV_CAPTURE_SOURCE_ID.to_string(),
        platform: "macos".to_string(),
        source_kind: "display".to_string(),
        title: "Synthetic 2K144 CVPixelBuffer".to_string(),
        class_name: "SyntheticCVPixelBuffer".to_string(),
        width: 2560,
        height: 1440,
        process_id: 0,
        app_name: Some("mrd-service benchmark source".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    }
}

#[cfg(target_os = "macos")]
fn test_synthetic_cv_capture_enabled_from_env_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

#[cfg(windows)]
pub fn create_frame_capture(source_id: &str) -> Result<mrd_capture_winrt::WinrtCapture> {
    let mut capture = match parse_windows_capture_source_ref(source_id)? {
        WindowsCaptureSourceRef::Window(hwnd) => {
            mrd_capture_winrt::WinrtCapture::from_window_handle(hwnd)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?
        }
        WindowsCaptureSourceRef::Display { index }
        | WindowsCaptureSourceRef::DisplayShared { index } => {
            create_windows_monitor_capture(source_id, index)?
        }
    };
    capture
        .start()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(capture)
}

#[cfg(target_os = "macos")]
pub fn create_frame_capture(source_id: &str) -> Result<mrd_capture_macos::MacosScreenCapture> {
    let capture = match parse_macos_capture_source_ref(source_id)? {
        MacosCaptureSourceRef::Display { display_id } => {
            mrd_capture_macos::MacosScreenCapture::new_display_id(display_id)
        }
        MacosCaptureSourceRef::Window { window_id } => {
            mrd_capture_macos::MacosScreenCapture::new_window(window_id)
        }
    }
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(capture)
}

#[cfg(target_os = "linux")]
pub fn create_frame_capture(
    source_id: &str,
) -> Result<mrd_capture_pipewire::PipewireScreenCapture> {
    if tokio::runtime::Handle::try_current().is_ok() {
        anyhow::bail!(
            "Linux capture creation from an async runtime must use create_frame_capture_async"
        )
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| anyhow::anyhow!("create Linux capture runtime failed: {error}"))?;
    runtime.block_on(create_frame_capture_async(source_id))
}

#[cfg(target_os = "linux")]
pub async fn create_frame_capture_async(
    source_id: &str,
) -> Result<mrd_capture_pipewire::PipewireScreenCapture> {
    validate_linux_capture_source_ref(source_id)?;

    let mut capture = mrd_capture_pipewire::PipewireScreenCapture::new()
        .map_err(|error| anyhow::anyhow!("Linux capture init failed: {error}"))?;
    capture
        .start_session()
        .await
        .map_err(|error| anyhow::anyhow!("Linux capture session start failed: {error}"))?;
    Ok(capture)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn create_frame_capture(_source_id: &str) -> Result<()> {
    anyhow::bail!("remote desktop capture is currently only available on Windows, macOS, and Linux")
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsCaptureSourceRef {
    Window(isize),
    Display { index: u32 },
    DisplayShared { index: u32 },
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum WindowsMonitorCaptureTarget {
    DeviceName(String),
    Index(u32),
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacosCaptureSourceRef {
    Display { display_id: u32 },
    Window { window_id: u32 },
}

#[cfg(windows)]
fn list_capture_sources_impl() -> Result<Vec<CaptureSource>> {
    let mut sources = list_windows_display_capture_sources();
    let targets = mrd_capture_winrt::enumerate_window_capture_targets()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    sources.extend(targets.into_iter().map(|target| CaptureSource {
        id: format!("windows:window:0x{:X}", target.hwnd as usize),
        platform: "windows".to_string(),
        source_kind: "window".to_string(),
        title: target.title,
        class_name: target.class_name,
        width: target.width,
        height: target.height,
        process_id: target.process_id,
        app_name: None,
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    }));

    Ok(sources)
}

#[cfg(windows)]
fn list_windows_display_capture_sources() -> Vec<CaptureSource> {
    let current_modes = crate::display_mode::current_display_modes().unwrap_or_default();
    if !current_modes.is_empty() {
        let dxgi_device_names = mrd_capture_dxgi::enumerate_dxgi_output_targets()
            .map(|targets| {
                targets
                    .into_iter()
                    .map(|target| target.device_name.to_ascii_lowercase())
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();

        let mut sources = Vec::with_capacity(current_modes.len().saturating_mul(2));
        for mode in current_modes {
            let Some(shared_source_id) = mode.source_id.clone() else {
                continue;
            };
            let display_index = windows_display_index_from_source_id(&shared_source_id)
                .unwrap_or(sources.len() / 2);
            let display_number = display_index + 1;
            let device_name =
                crate::display_mode::display_device_name_for_source_id(&shared_source_id).ok();
            let has_dxgi_shared = device_name
                .as_deref()
                .map(|name| dxgi_device_names.contains(&name.to_ascii_lowercase()))
                .unwrap_or(false);

            if has_dxgi_shared {
                sources.push(CaptureSource {
                    id: shared_source_id.clone(),
                    platform: "windows".to_string(),
                    source_kind: "display_shared".to_string(),
                    title: format!("Display {display_number} (D3D11 shared copy)"),
                    class_name: device_name
                        .as_ref()
                        .map(|name| format!("DXGIShared:{name}"))
                        .unwrap_or_else(|| "DXGIShared".to_string()),
                    width: mode.width,
                    height: mode.height,
                    process_id: 0,
                    app_name: Some("Display".to_string()),
                    bundle_identifier: None,
                    preview_data_url: None,
                    preview_width: None,
                    preview_height: None,
                });
            }

            sources.push(CaptureSource {
                id: shared_source_id.replacen("windows:display-shared:", "windows:display:", 1),
                platform: "windows".to_string(),
                source_kind: "display".to_string(),
                title: format!("Display {display_number} (full screen copy)"),
                class_name: device_name
                    .as_ref()
                    .map(|name| format!("WinRTMonitor:{name}"))
                    .unwrap_or_else(|| "WinRTMonitor".to_string()),
                width: mode.width,
                height: mode.height,
                process_id: 0,
                app_name: Some("Display".to_string()),
                bundle_identifier: None,
                preview_data_url: None,
                preview_width: None,
                preview_height: None,
            });
        }

        if !sources.is_empty() {
            return sources;
        }
    }

    list_windows_winrt_display_capture_sources()
}

#[cfg(windows)]
fn list_windows_winrt_display_capture_sources() -> Vec<CaptureSource> {
    let Ok(count) = mrd_capture_winrt::get_monitor_count() else {
        return Vec::new();
    };

    let mut dimensions = Vec::with_capacity(count);
    for index in 0..count {
        let Ok(capture) = mrd_capture_winrt::WinrtCapture::from_monitor_index(index as u32) else {
            continue;
        };
        dimensions.push((
            index as u32,
            capture.width() as u32,
            capture.height() as u32,
        ));
    }

    windows_winrt_display_sources_from_indexed_dimensions(dimensions)
}

#[cfg(windows)]
fn windows_winrt_display_sources_from_indexed_dimensions(
    dimensions: Vec<(u32, u32, u32)>,
) -> Vec<CaptureSource> {
    let mut sources = Vec::with_capacity(dimensions.len());
    for (index, width, height) in dimensions {
        let display_number = index + 1;
        sources.push(CaptureSource {
            id: format!("windows:display:{index}"),
            platform: "windows".to_string(),
            source_kind: "display".to_string(),
            title: format!("Display {display_number} (full screen copy)"),
            class_name: "WinRTMonitor".to_string(),
            width,
            height,
            process_id: 0,
            app_name: Some("Display".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        });
    }

    sources
}

#[cfg(all(windows, test))]
fn windows_winrt_display_sources_from_dimensions(
    dimensions: Vec<(u32, u32)>,
) -> Vec<CaptureSource> {
    windows_winrt_display_sources_from_indexed_dimensions(
        dimensions
            .into_iter()
            .enumerate()
            .map(|(index, (width, height))| (index as u32, width, height))
            .collect(),
    )
}

#[cfg(windows)]
fn windows_display_index_from_source_id(source_id: &str) -> Option<usize> {
    source_id
        .trim()
        .rsplit(':')
        .next()
        .and_then(|value| value.parse::<usize>().ok())
}

#[cfg(windows)]
fn create_windows_monitor_capture(
    source_id: &str,
    fallback_index: u32,
) -> Result<mrd_capture_winrt::WinrtCapture> {
    let device_name = crate::display_mode::display_device_name_for_source_id(source_id).ok();
    match windows_monitor_capture_target(device_name, fallback_index) {
        WindowsMonitorCaptureTarget::DeviceName(device_name) => {
            mrd_capture_winrt::WinrtCapture::from_monitor_device_name(&device_name).map_err(
                |error| {
                    anyhow::anyhow!(
                        "WinRT monitor capture by device name '{device_name}' failed: {error}"
                    )
                },
            )
        }
        WindowsMonitorCaptureTarget::Index(index) => {
            mrd_capture_winrt::WinrtCapture::from_monitor_index(index)
                .map_err(|error| anyhow::anyhow!(error.to_string()))
        }
    }
}

#[cfg(windows)]
fn windows_monitor_capture_target(
    device_name: Option<String>,
    fallback_index: u32,
) -> WindowsMonitorCaptureTarget {
    match device_name.map(|value| value.trim().to_string()) {
        Some(device_name) if !device_name.is_empty() => {
            WindowsMonitorCaptureTarget::DeviceName(device_name)
        }
        _ => WindowsMonitorCaptureTarget::Index(fallback_index),
    }
}

#[cfg(target_os = "macos")]
fn list_capture_sources_impl() -> Result<Vec<CaptureSource>> {
    let mut sources = list_macos_display_capture_sources()?;
    match mrd_capture_macos::enumerate_window_capture_targets() {
        Ok(targets) => {
            sources.extend(targets.into_iter().map(|target| CaptureSource {
                id: format!("macos:window:0x{:X}", target.window_id),
                platform: "macos".to_string(),
                source_kind: "window".to_string(),
                title: target.title,
                class_name: "ScreenCaptureKitWindow".to_string(),
                width: target.width,
                height: target.height,
                process_id: target.process_id,
                app_name: non_empty_string(target.app_name),
                bundle_identifier: non_empty_string(target.bundle_identifier),
                preview_data_url: None,
                preview_width: None,
                preview_height: None,
            }));
        }
        Err(error) if sources.is_empty() => {
            return Err(anyhow::anyhow!(error.to_string()));
        }
        Err(_) => {}
    }

    Ok(sources)
}

#[cfg(target_os = "macos")]
fn list_macos_display_capture_sources() -> Result<Vec<CaptureSource>> {
    let targets = mrd_capture_macos::enumerate_display_capture_targets()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(targets
        .into_iter()
        .map(|target| CaptureSource {
            id: format!("macos:display:0x{:X}", target.display_id),
            platform: "macos".to_string(),
            source_kind: "display".to_string(),
            title: target.title,
            class_name: "ScreenCaptureKitDisplay".to_string(),
            width: target.width,
            height: target.height,
            process_id: 0,
            app_name: Some("Display".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        })
        .collect())
}

#[cfg(target_os = "macos")]
fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(target_os = "linux")]
fn list_capture_sources_impl() -> Result<Vec<CaptureSource>> {
    let mut sources = Vec::new();
    let display_targets = mrd_capture_pipewire::PipewireScreenCapture::get_display_targets()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    sources.extend(display_targets.into_iter().map(|target| CaptureSource {
        id: format!("linux:display:{}", target.id),
        platform: "linux".to_string(),
        source_kind: "display".to_string(),
        title: target.name,
        class_name: linux_capture_backend_label().to_string(),
        width: target.width,
        height: target.height,
        process_id: 0,
        app_name: Some("Display".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    }));

    if let Ok(window_targets) = mrd_capture_pipewire::PipewireScreenCapture::get_window_targets() {
        sources.extend(window_targets.into_iter().map(|target| CaptureSource {
            id: format!("linux:window:{}", target.id),
            platform: "linux".to_string(),
            source_kind: "window".to_string(),
            title: target.title,
            class_name: linux_capture_backend_label().to_string(),
            width: target.width,
            height: target.height,
            process_id: 0,
            app_name: Some(target.app_name),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        }));
    }

    Ok(sources)
}

#[cfg(target_os = "linux")]
fn linux_capture_backend_label() -> &'static str {
    if mrd_capture_pipewire::PipewireScreenCapture::is_wayland_available() {
        "PipeWirePortal"
    } else if mrd_capture_pipewire::PipewireScreenCapture::is_x11_available() {
        "X11ScreenCapture"
    } else {
        "LinuxFallbackCapture"
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn list_capture_sources_impl() -> Result<Vec<CaptureSource>> {
    anyhow::bail!("remote window capture source enumeration is currently only available on Windows, macOS, and Linux")
}

#[cfg(windows)]
fn parse_windows_capture_source_ref(source_id: &str) -> Result<WindowsCaptureSourceRef> {
    let trimmed = source_id.trim();
    if let Some(value) = trimmed.strip_prefix("windows:display-shared:") {
        return Ok(WindowsCaptureSourceRef::DisplayShared {
            index: parse_windows_display_index(source_id, value)?,
        });
    }
    if let Some(value) = trimmed.strip_prefix("windows:display:") {
        return Ok(WindowsCaptureSourceRef::Display {
            index: parse_windows_display_index(source_id, value)?,
        });
    }
    if trimmed.starts_with("windows:window:") {
        return Ok(WindowsCaptureSourceRef::Window(
            parse_windows_window_hwnd_source_id(source_id)?,
        ));
    }

    Ok(WindowsCaptureSourceRef::Window(parse_windows_hwnd_value(
        source_id, trimmed,
    )?))
}

#[cfg(windows)]
pub(crate) fn parse_windows_window_hwnd_source_id(source_id: &str) -> Result<isize> {
    let trimmed = source_id.trim();
    let value = trimmed.strip_prefix("windows:window:").ok_or_else(|| {
        anyhow::anyhow!(
            "invalid Windows window capture source id '{source_id}': expected window source"
        )
    })?;
    parse_windows_hwnd_value(source_id, value)
}

#[cfg(target_os = "macos")]
fn parse_macos_capture_source_ref(source_id: &str) -> Result<MacosCaptureSourceRef> {
    let trimmed = source_id.trim();
    if let Some(value) = trimmed.strip_prefix("macos:display:") {
        return Ok(MacosCaptureSourceRef::Display {
            display_id: parse_macos_u32_value(source_id, value)?,
        });
    }
    if let Some(value) = trimmed.strip_prefix("macos:window:") {
        return Ok(MacosCaptureSourceRef::Window {
            window_id: parse_macos_u32_value(source_id, value)?,
        });
    }

    Ok(MacosCaptureSourceRef::Window {
        window_id: parse_macos_u32_value(source_id, trimmed)?,
    })
}

#[cfg(target_os = "macos")]
fn parse_macos_u32_value(source_id: &str, value: &str) -> Result<u32> {
    let value = value.trim();
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16)
    } else {
        value.parse::<u32>()
    };
    parsed
        .map_err(|error| anyhow::anyhow!("invalid macOS capture source id '{source_id}': {error}"))
}

#[cfg(target_os = "linux")]
fn validate_linux_capture_source_ref(source_id: &str) -> Result<()> {
    let trimmed = source_id.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Linux capture source id is empty");
    }

    if let Some(value) = trimmed.strip_prefix("linux:display:") {
        parse_linux_u32_value(source_id, value)?;
        return Ok(());
    }

    if let Some(value) = trimmed.strip_prefix("linux:window:") {
        parse_linux_u32_value(source_id, value)?;
        return Ok(());
    }

    if trimmed == "linux" {
        return Ok(());
    }

    anyhow::bail!("invalid Linux capture source id '{source_id}'")
}

#[cfg(target_os = "linux")]
fn parse_linux_u32_value(source_id: &str, value: &str) -> Result<u32> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|error| anyhow::anyhow!("invalid Linux capture source id '{source_id}': {error}"))
}

#[cfg(windows)]
fn parse_windows_display_index(source_id: &str, value: &str) -> Result<u32> {
    value.trim().parse::<u32>().map_err(|error| {
        anyhow::anyhow!("invalid Windows display source id '{source_id}': {error}")
    })
}

#[cfg(windows)]
fn parse_windows_hwnd_value(source_id: &str, value: &str) -> Result<isize> {
    let value = value.trim();
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    let hwnd = usize::from_str_radix(value, 16)
        .or_else(|_| value.parse::<usize>())
        .map_err(|error| {
            anyhow::anyhow!("invalid Windows capture source id '{source_id}': {error}")
        })?;
    if hwnd == 0 {
        anyhow::bail!("invalid Windows window capture source id '{source_id}': HWND is zero");
    }
    Ok(hwnd as isize)
}

#[cfg(test)]
mod tests {
    use mrd_ipc::CaptureSource;

    fn source(id: &str, source_kind: &str) -> CaptureSource {
        CaptureSource {
            id: id.to_string(),
            platform: "windows".to_string(),
            source_kind: source_kind.to_string(),
            title: id.to_string(),
            class_name: "Test".to_string(),
            width: 1280,
            height: 720,
            process_id: 0,
            app_name: None,
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        }
    }

    #[test]
    fn source_lookup_rejects_empty_id() {
        let error = super::find_capture_source(" ").expect_err("empty id should fail");
        assert!(error.to_string().contains("empty"));
    }

    #[test]
    fn preferred_capture_source_picks_shared_display_before_cpu_sender_path() {
        let sources = vec![
            source("windows:window:0x1234", "window"),
            source("windows:display:0", "display"),
            source("windows:display-shared:0", "display_shared"),
        ];

        let selected = super::preferred_capture_source(&sources).expect("selected source");

        assert_eq!(selected.id, "windows:display-shared:0");
    }

    #[test]
    fn preferred_capture_source_falls_back_to_display_before_window() {
        let sources = vec![
            source("windows:window:0x1234", "window"),
            source("windows:display:0", "display"),
        ];

        let selected = super::preferred_capture_source(&sources).expect("selected source");

        assert_eq!(selected.id, "windows:display:0");
    }

    #[cfg(windows)]
    #[test]
    fn windows_winrt_fallback_does_not_advertise_dxgi_shared_sources() {
        let sources = super::windows_winrt_display_sources_from_dimensions(vec![(1920, 1080)]);

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "windows:display:0");
        assert_eq!(sources[0].source_kind, "display");
        assert!(sources
            .iter()
            .all(|source| source.source_kind != "display_shared"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_winrt_fallback_preserves_sparse_monitor_indices() {
        let sources =
            super::windows_winrt_display_sources_from_indexed_dimensions(vec![(2, 2560, 1440)]);

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "windows:display:2");
        assert_eq!(sources[0].title, "Display 3 (full screen copy)");
    }

    #[cfg(windows)]
    #[test]
    fn windows_capture_source_refs_parse_display_and_window_ids() {
        assert_eq!(
            super::parse_windows_capture_source_ref("windows:display-shared:2").unwrap(),
            super::WindowsCaptureSourceRef::DisplayShared { index: 2 }
        );
        assert_eq!(
            super::parse_windows_capture_source_ref("windows:display:1").unwrap(),
            super::WindowsCaptureSourceRef::Display { index: 1 }
        );
        assert_eq!(
            super::parse_windows_capture_source_ref("windows:window:0x1234").unwrap(),
            super::WindowsCaptureSourceRef::Window(0x1234)
        );
    }

    #[cfg(all(windows, test))]
    #[test]
    fn parse_windows_capture_source_ref_accepts_window_hwnd_hex() {
        assert_eq!(
            super::parse_windows_capture_source_ref("windows:window:0x1234").unwrap(),
            super::WindowsCaptureSourceRef::Window(0x1234)
        );
    }

    #[cfg(all(windows, test))]
    #[test]
    fn parse_windows_capture_source_ref_rejects_empty_window_hwnd() {
        let error = super::parse_windows_capture_source_ref("windows:window:")
            .unwrap_err()
            .to_string();

        assert!(error.contains("window"));
    }

    #[cfg(all(windows, test))]
    #[test]
    fn parse_windows_capture_source_ref_rejects_zero_window_hwnd() {
        let error = super::parse_windows_capture_source_ref("windows:window:0x0")
            .unwrap_err()
            .to_string();

        assert!(error.contains("window"));
    }

    #[cfg(all(windows, test))]
    #[test]
    fn parse_windows_capture_source_ref_rejects_malformed_window_hwnd() {
        let error = super::parse_windows_capture_source_ref("windows:window:not-a-hwnd")
            .unwrap_err()
            .to_string();

        assert!(error.contains("window"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_monitor_capture_target_prefers_device_name_before_index_fallback() {
        assert_eq!(
            super::windows_monitor_capture_target(Some("\\\\.\\DISPLAY7".to_string()), 2),
            super::WindowsMonitorCaptureTarget::DeviceName("\\\\.\\DISPLAY7".to_string())
        );
        assert_eq!(
            super::windows_monitor_capture_target(Some("  ".to_string()), 2),
            super::WindowsMonitorCaptureTarget::Index(2)
        );
        assert_eq!(
            super::windows_monitor_capture_target(None, 2),
            super::WindowsMonitorCaptureTarget::Index(2)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_capture_source_refs_parse_display_and_window_ids() {
        assert_eq!(
            super::parse_macos_capture_source_ref("macos:display:0x1A2B").unwrap(),
            super::MacosCaptureSourceRef::Display { display_id: 0x1A2B }
        );
        assert_eq!(
            super::parse_macos_capture_source_ref("macos:window:0x1234").unwrap(),
            super::MacosCaptureSourceRef::Window { window_id: 0x1234 }
        );
        assert_eq!(
            super::parse_macos_capture_source_ref("5678").unwrap(),
            super::MacosCaptureSourceRef::Window { window_id: 5678 }
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn synthetic_cv_capture_env_parses_truthy_values_only() {
        assert!(super::test_synthetic_cv_capture_enabled_from_env_value(
            Some("1")
        ));
        assert!(super::test_synthetic_cv_capture_enabled_from_env_value(
            Some("true")
        ));
        assert!(super::test_synthetic_cv_capture_enabled_from_env_value(
            Some("on")
        ));
        assert!(!super::test_synthetic_cv_capture_enabled_from_env_value(
            Some("0")
        ));
        assert!(!super::test_synthetic_cv_capture_enabled_from_env_value(
            Some("off")
        ));
        assert!(!super::test_synthetic_cv_capture_enabled_from_env_value(
            None
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn synthetic_cv_capture_source_uses_2k_display_like_shape() {
        let source = super::test_synthetic_cv_capture_source();

        assert_eq!(source.id, super::TEST_SYNTHETIC_CV_CAPTURE_SOURCE_ID);
        assert_eq!(source.platform, "macos");
        assert_eq!(source.source_kind, "display");
        assert_eq!((source.width, source.height), (2560, 1440));
        assert!(super::is_test_synthetic_cv_capture_source_id(
            "synthetic-cv"
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "queries live macOS WindowServer state"]
    fn macos_lists_display_capture_sources() {
        let sources = super::list_capture_sources(false, Some(8)).expect("list capture sources");

        assert!(
            sources
                .iter()
                .any(|source| source.platform == "macos" && source.source_kind == "display"),
            "expected at least one macOS display capture source: {sources:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_capture_source_refs_parse_display_and_window_ids() {
        super::validate_linux_capture_source_ref("linux").unwrap();
        super::validate_linux_capture_source_ref("linux:display:0").unwrap();
        super::validate_linux_capture_source_ref("linux:window:42").unwrap();

        assert!(super::validate_linux_capture_source_ref("linux:display:not-a-number").is_err());
        assert!(super::validate_linux_capture_source_ref("windows:display:0").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_lists_display_capture_sources_with_fallback() {
        let sources = super::list_capture_sources(false, Some(8)).expect("list capture sources");

        assert!(
            sources
                .iter()
                .any(|source| source.platform == "linux" && source.source_kind == "display"),
            "expected at least one Linux display capture source: {sources:?}"
        );
    }
}
