#![allow(dead_code)]

use anyhow::{Context, Result};
use mrd_ipc::DisplayMode;

const DISPLAY_MODE_CONTROL_CAPABILITY: &str = "display_mode_control_v1";

pub fn capability_name() -> &'static str {
    DISPLAY_MODE_CONTROL_CAPABILITY
}

pub fn list_display_modes(source_id: Option<&str>) -> Result<Vec<DisplayMode>> {
    let source_index = parse_display_source_index(source_id)?;
    list_platform_display_modes(source_index)
}

pub fn highest_current_refresh_hz() -> Option<u32> {
    select_highest_current_refresh_hz(&list_current_platform_display_modes().ok()?)
}

pub fn current_display_modes() -> Result<Vec<DisplayMode>> {
    list_current_platform_display_modes()
}

pub fn display_device_name_for_source_id(source_id: &str) -> Result<String> {
    let source_index = parse_display_source_index(Some(source_id))?.unwrap_or(0);
    platform_display_device_name(source_index)
}

pub fn set_display_mode(mode: &DisplayMode) -> Result<(Option<DisplayMode>, DisplayMode)> {
    let source_index = parse_display_source_index(mode.source_id.as_deref())?.unwrap_or(0);
    let previous = current_platform_display_mode(source_index).ok();
    let active = set_platform_display_mode(source_index, mode)?;
    Ok((previous, active))
}

pub fn restore_display_mode(mode: &DisplayMode) -> Result<DisplayMode> {
    let source_index = parse_display_source_index(mode.source_id.as_deref())?.unwrap_or(0);
    set_platform_display_mode(source_index, mode)
}

pub(crate) fn parse_display_source_index(source_id: Option<&str>) -> Result<Option<u32>> {
    let Some(source_id) = source_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let parts = source_id.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        ["windows", "display", index] | ["windows", "display-shared", index] => index
            .parse::<u32>()
            .map(Some)
            .with_context(|| format!("invalid Windows display source index: {source_id}")),
        ["display", index] => index
            .parse::<u32>()
            .map(Some)
            .with_context(|| format!("invalid display source index: {source_id}")),
        _ => anyhow::bail!("source id is not a display source: {source_id}"),
    }
}

pub(crate) fn choose_display_mode(
    modes: &[DisplayMode],
    width: u32,
    height: u32,
    refresh_hz: u32,
) -> Option<DisplayMode> {
    modes
        .iter()
        .filter(|mode| mode.width > 0 && mode.height > 0 && mode.refresh_hz > 0)
        .min_by(|left, right| {
            display_mode_score(left, width, height, refresh_hz)
                .cmp(&display_mode_score(right, width, height, refresh_hz))
                .then_with(|| right.refresh_hz.cmp(&left.refresh_hz))
                .then_with(|| {
                    let left_pixels = u64::from(left.width) * u64::from(left.height);
                    let right_pixels = u64::from(right.width) * u64::from(right.height);
                    right_pixels.cmp(&left_pixels)
                })
        })
        .cloned()
}

fn display_mode_score(
    mode: &DisplayMode,
    width: u32,
    height: u32,
    refresh_hz: u32,
) -> (u64, u32, u32, u32) {
    let mode_aspect = mode.width as f64 / mode.height as f64;
    let target_aspect = width as f64 / height as f64;
    let aspect_delta = ((mode_aspect - target_aspect).abs() * 10_000.0).round() as u64;
    let height_delta = mode.height.abs_diff(height);
    let width_delta = mode.width.abs_diff(width);
    let refresh_delta = mode.refresh_hz.abs_diff(refresh_hz);
    (aspect_delta, height_delta, width_delta, refresh_delta)
}

fn select_highest_current_refresh_hz(modes: &[DisplayMode]) -> Option<u32> {
    modes
        .iter()
        .filter(|mode| mode.is_current && mode.refresh_hz > 0)
        .map(|mode| mode.refresh_hz)
        .max()
}

fn display_device_name_from_raw(raw: &[u16]) -> Option<String> {
    let end = raw.iter().position(|unit| *unit == 0).unwrap_or(raw.len());
    if end == 0 {
        return None;
    }
    let value = String::from_utf16_lossy(&raw[..end]);
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WindowsDisplayTarget {
    source_index: u32,
    device_name: String,
    primary: bool,
    left: i32,
    top: i32,
    width: u32,
    height: u32,
}

fn assign_windows_display_source_indices(
    mut targets: Vec<WindowsDisplayTarget>,
) -> Vec<WindowsDisplayTarget> {
    targets.sort_by_key(|target| {
        (
            !target.primary,
            target.left,
            target.top,
            target.device_name.to_ascii_lowercase(),
        )
    });
    for (index, target) in targets.iter_mut().enumerate() {
        target.source_index = index as u32;
    }
    targets
}

#[cfg(windows)]
fn list_platform_display_modes(source_index: Option<u32>) -> Result<Vec<DisplayMode>> {
    use std::collections::BTreeMap;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_MODE,
    };

    let target = windows_display_target_for_source_index(source_index.unwrap_or(0))?;
    let device_name = wide_null(&target.device_name);
    let current = unsafe {
        let mut value = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if !EnumDisplaySettingsW(
            PCWSTR(device_name.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut value,
        )
        .as_bool()
        {
            None
        } else {
            Some(value)
        }
    };

    let mut modes = BTreeMap::<(u32, u32, u32, u32), DisplayMode>::new();
    let mut mode_index = 0;
    loop {
        let mut dev_mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let ok = unsafe {
            EnumDisplaySettingsW(
                PCWSTR(device_name.as_ptr()),
                ENUM_DISPLAY_SETTINGS_MODE(mode_index),
                &mut dev_mode,
            )
            .as_bool()
        };
        if !ok {
            break;
        }

        let width = dev_mode.dmPelsWidth;
        let height = dev_mode.dmPelsHeight;
        let refresh_hz = dev_mode.dmDisplayFrequency;
        let bit_depth = dev_mode.dmBitsPerPel;
        if width > 0 && height > 0 && refresh_hz > 0 {
            let is_current = current.as_ref().is_some_and(|value| {
                value.dmPelsWidth == width
                    && value.dmPelsHeight == height
                    && value.dmDisplayFrequency == refresh_hz
            });
            let mode = DisplayMode {
                id: format!(
                    "windows:display:{}:{width}x{height}@{refresh_hz}",
                    target.source_index
                ),
                source_id: Some(format!("windows:display-shared:{}", target.source_index)),
                width,
                height,
                refresh_hz,
                bit_depth: Some(bit_depth),
                is_current,
            };
            modes.insert((width, height, refresh_hz, bit_depth), mode);
        }
        mode_index += 1;
    }

    let mut values = modes.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .is_current
            .cmp(&left.is_current)
            .then_with(|| right.width.cmp(&left.width))
            .then_with(|| right.height.cmp(&left.height))
            .then_with(|| right.refresh_hz.cmp(&left.refresh_hz))
    });
    Ok(values)
}

#[cfg(windows)]
fn list_current_platform_display_modes() -> Result<Vec<DisplayMode>> {
    let targets = enumerate_windows_display_targets()?;
    let mut modes = Vec::with_capacity(targets.len());
    let mut first_error = None;

    for target in targets {
        match current_display_mode_for_windows_target(&target) {
            Ok(mode) => modes.push(mode),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    if modes.is_empty() {
        Err(first_error.unwrap_or_else(|| anyhow::anyhow!("no Windows displays found")))
    } else {
        Ok(modes)
    }
}

fn collect_current_platform_display_modes_by_index(
    mut current_mode: impl FnMut(u32) -> Result<DisplayMode>,
) -> Result<Vec<DisplayMode>> {
    let mut modes = Vec::new();
    let mut first_error = None;
    for source_index in 0..32 {
        match current_mode(source_index) {
            Ok(mode) => modes.push(mode),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    if modes.is_empty() {
        Err(first_error.unwrap_or_else(|| anyhow::anyhow!("no Windows displays found")))
    } else {
        Ok(modes)
    }
}

#[cfg(windows)]
fn current_platform_display_mode(source_index: u32) -> Result<DisplayMode> {
    let target = windows_display_target_for_source_index(source_index)?;
    current_display_mode_for_windows_target(&target)
}

#[cfg(windows)]
fn set_platform_display_mode(source_index: u32, mode: &DisplayMode) -> Result<DisplayMode> {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_BITSPERPEL,
        DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH,
    };

    let target = windows_display_target_for_source_index(source_index)?;
    let device_name = wide_null(&target.device_name);
    let dev_mode = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        dmPelsWidth: mode.width,
        dmPelsHeight: mode.height,
        dmDisplayFrequency: mode.refresh_hz,
        dmBitsPerPel: mode.bit_depth.unwrap_or(32),
        dmFields: DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_BITSPERPEL,
        ..Default::default()
    };

    let result = unsafe {
        ChangeDisplaySettingsExW(
            PCWSTR(device_name.as_ptr()),
            Some(&dev_mode),
            None,
            windows::Win32::Graphics::Gdi::CDS_TYPE(0),
            None,
        )
    };
    if result != DISP_CHANGE_SUCCESSFUL {
        anyhow::bail!(
            "Windows rejected display mode {}x{}@{} for display {}: {:?}",
            mode.width,
            mode.height,
            mode.refresh_hz,
            source_index,
            result
        );
    }

    let mut active = mode.clone();
    active.source_id = Some(format!("windows:display-shared:{source_index}"));
    active.is_current = true;
    Ok(active)
}

#[cfg(windows)]
fn current_display_mode_for_windows_target(target: &WindowsDisplayTarget) -> Result<DisplayMode> {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};

    let device_name = wide_null(&target.device_name);
    let mut value = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let ok = unsafe {
        EnumDisplaySettingsW(
            PCWSTR(device_name.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut value,
        )
        .as_bool()
    };
    if !ok {
        anyhow::bail!(
            "current display mode not found for display {} ({})",
            target.source_index,
            target.device_name
        );
    }

    Ok(DisplayMode {
        id: format!(
            "windows:display:{}:{}x{}@{}",
            target.source_index, value.dmPelsWidth, value.dmPelsHeight, value.dmDisplayFrequency
        ),
        source_id: Some(format!("windows:display-shared:{}", target.source_index)),
        width: value.dmPelsWidth,
        height: value.dmPelsHeight,
        refresh_hz: value.dmDisplayFrequency,
        bit_depth: Some(value.dmBitsPerPel),
        is_current: true,
    })
}

#[cfg(windows)]
fn windows_display_target_for_source_index(source_index: u32) -> Result<WindowsDisplayTarget> {
    enumerate_windows_display_targets()?
        .into_iter()
        .find(|target| target.source_index == source_index)
        .ok_or_else(|| anyhow::anyhow!("Windows display target not found for index {source_index}"))
}

#[cfg(windows)]
fn enumerate_windows_display_targets() -> Result<Vec<WindowsDisplayTarget>> {
    let monitor_targets = enumerate_monitor_display_targets()?;
    if !monitor_targets.is_empty() {
        return Ok(assign_windows_display_source_indices(monitor_targets));
    }

    let device_targets = enumerate_display_device_targets();
    if device_targets.is_empty() {
        anyhow::bail!("no Windows displays found")
    } else {
        Ok(assign_windows_display_source_indices(device_targets))
    }
}

#[cfg(windows)]
fn enumerate_monitor_display_targets() -> Result<Vec<WindowsDisplayTarget>> {
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
    };

    unsafe extern "system" fn collect_monitor(
        monitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        let targets = &mut *(data.0 as *mut Vec<WindowsDisplayTarget>);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;

        if !GetMonitorInfoW(monitor, (&mut info as *mut MONITORINFOEXW).cast()).as_bool() {
            return windows::core::BOOL(1);
        }

        let rect = info.monitorInfo.rcMonitor;
        let width = rect.right.saturating_sub(rect.left);
        let height = rect.bottom.saturating_sub(rect.top);
        if width <= 0 || height <= 0 {
            return windows::core::BOOL(1);
        }

        let Some(device_name) = display_device_name_from_raw(&info.szDevice) else {
            return windows::core::BOOL(1);
        };

        targets.push(WindowsDisplayTarget {
            source_index: 0,
            device_name,
            primary: info.monitorInfo.dwFlags & 1 != 0,
            left: rect.left,
            top: rect.top,
            width: width as u32,
            height: height as u32,
        });
        windows::core::BOOL(1)
    }

    let mut targets = Vec::<WindowsDisplayTarget>::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut targets as *mut _ as isize),
        )
        .as_bool()
    };
    if !ok {
        anyhow::bail!(
            "EnumDisplayMonitors failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(targets)
}

#[cfg(windows)]
fn enumerate_display_device_targets() -> Vec<WindowsDisplayTarget> {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayDevicesW, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
        DISPLAY_DEVICE_PRIMARY_DEVICE,
    };

    let mut targets = Vec::new();
    for device_index in 0..32 {
        let mut device = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        let ok =
            unsafe { EnumDisplayDevicesW(PCWSTR::null(), device_index, &mut device, 0).as_bool() };
        if !ok {
            break;
        }
        if device.StateFlags.0 & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP.0 == 0 {
            continue;
        }
        let Some(device_name) = display_device_name_from_raw(&device.DeviceName) else {
            continue;
        };

        targets.push(WindowsDisplayTarget {
            source_index: 0,
            device_name,
            primary: device.StateFlags.0 & DISPLAY_DEVICE_PRIMARY_DEVICE.0 != 0,
            left: device_index as i32,
            top: 0,
            width: 0,
            height: 0,
        });
    }
    targets
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn platform_display_device_name(source_index: u32) -> Result<String> {
    Ok(windows_display_target_for_source_index(source_index)?.device_name)
}

#[cfg(not(windows))]
fn list_platform_display_modes(_source_index: Option<u32>) -> Result<Vec<DisplayMode>> {
    anyhow::bail!("display mode control is currently only available on Windows")
}

#[cfg(not(windows))]
fn list_current_platform_display_modes() -> Result<Vec<DisplayMode>> {
    anyhow::bail!("display mode control is currently only available on Windows")
}

#[cfg(not(windows))]
fn current_platform_display_mode(_source_index: u32) -> Result<DisplayMode> {
    anyhow::bail!("display mode control is currently only available on Windows")
}

#[cfg(not(windows))]
fn set_platform_display_mode(_source_index: u32, _mode: &DisplayMode) -> Result<DisplayMode> {
    anyhow::bail!("display mode control is currently only available on Windows")
}

#[cfg(not(windows))]
fn platform_display_device_name(_source_index: u32) -> Result<String> {
    anyhow::bail!("display mode control is currently only available on Windows")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_ipc::DisplayMode;

    #[test]
    fn display_source_index_accepts_capture_source_ids() {
        assert_eq!(
            parse_display_source_index(Some("windows:display-shared:0")).unwrap(),
            Some(0)
        );
        assert_eq!(
            parse_display_source_index(Some("windows:display:2")).unwrap(),
            Some(2)
        );
        assert_eq!(parse_display_source_index(None).unwrap(), None);
    }

    #[test]
    fn display_source_index_rejects_non_display_sources() {
        let error = parse_display_source_index(Some("windows:window:0x1234")).unwrap_err();
        assert!(error.to_string().contains("not a display source"));
    }

    #[test]
    fn choose_display_mode_prefers_exact_resolution_and_refresh() {
        let modes = vec![
            mode("m1", 2560, 1600, 60, false),
            mode("m2", 1920, 1080, 60, false),
            mode("m3", 1920, 1080, 144, false),
        ];

        let selected = choose_display_mode(&modes, 1920, 1080, 144).unwrap();

        assert_eq!(selected.id, "m3");
    }

    #[test]
    fn choose_display_mode_falls_back_to_same_aspect_refresh() {
        let modes = vec![
            mode("m1", 2560, 1600, 60, false),
            mode("m2", 1728, 1080, 144, false),
            mode("m3", 1920, 1200, 144, false),
        ];

        let selected = choose_display_mode(&modes, 1920, 1080, 144).unwrap();

        assert_eq!(selected.id, "m2");
    }

    #[test]
    fn highest_current_refresh_hz_uses_fastest_current_display() {
        let modes = vec![
            mode("display-0", 2560, 1440, 144, true),
            mode("display-1", 2560, 1440, 180, true),
            mode("not-active", 2560, 1440, 240, false),
        ];

        assert_eq!(select_highest_current_refresh_hz(&modes), Some(180));
    }

    #[test]
    fn current_display_mode_collection_skips_sparse_or_unavailable_indices() {
        let mut results = vec![
            Ok(mode("display-0", 1920, 1080, 60, true)),
            Err(anyhow::anyhow!("display 1 unavailable")),
            Ok(mode("display-2", 2560, 1440, 144, true)),
        ]
        .into_iter();

        let modes = collect_current_platform_display_modes_by_index(|_| {
            results
                .next()
                .unwrap_or_else(|| Err(anyhow::anyhow!("end of synthetic device list")))
        })
        .unwrap();

        assert_eq!(modes.len(), 2);
        assert_eq!(modes[0].id, "display-0");
        assert_eq!(modes[1].id, "display-2");
    }

    #[test]
    fn display_targets_are_ordered_by_primary_then_virtual_geometry() {
        let targets = assign_windows_display_source_indices(vec![
            display_target("\\\\.\\DISPLAY3", false, 2560, 0),
            display_target("\\\\.\\DISPLAY1", true, 3840, 0),
            display_target("\\\\.\\DISPLAY2", false, -2560, 0),
        ]);

        assert_eq!(
            targets
                .iter()
                .map(|target| (target.source_index, target.device_name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (0, "\\\\.\\DISPLAY1"),
                (1, "\\\\.\\DISPLAY2"),
                (2, "\\\\.\\DISPLAY3"),
            ]
        );
    }

    #[test]
    fn display_device_name_from_raw_trims_nul_terminated_utf16() {
        let mut raw = [0_u16; 32];
        for (index, unit) in "\\\\.\\DISPLAY2".encode_utf16().enumerate() {
            raw[index] = unit;
        }

        assert_eq!(
            display_device_name_from_raw(&raw),
            Some("\\\\.\\DISPLAY2".to_string())
        );
    }

    fn display_target(
        device_name: &str,
        primary: bool,
        left: i32,
        top: i32,
    ) -> WindowsDisplayTarget {
        WindowsDisplayTarget {
            source_index: 99,
            device_name: device_name.to_string(),
            primary,
            left,
            top,
            width: 2560,
            height: 1440,
        }
    }

    fn mode(id: &str, width: u32, height: u32, refresh_hz: u32, is_current: bool) -> DisplayMode {
        DisplayMode {
            id: id.to_string(),
            source_id: Some("windows:display-shared:0".to_string()),
            width,
            height,
            refresh_hz,
            bit_depth: Some(32),
            is_current,
        }
    }
}
