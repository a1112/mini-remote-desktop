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

#[cfg(windows)]
fn list_platform_display_modes(source_index: Option<u32>) -> Result<Vec<DisplayMode>> {
    use std::collections::BTreeMap;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_MODE,
    };

    let index = source_index.unwrap_or(0);
    let device = display_device(index)?;
    let current = unsafe {
        let mut value = DEVMODEW::default();
        value.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if !EnumDisplaySettingsW(
            PCWSTR(device.DeviceName.as_ptr()),
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
        let mut dev_mode = DEVMODEW::default();
        dev_mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        let ok = unsafe {
            EnumDisplaySettingsW(
                PCWSTR(device.DeviceName.as_ptr()),
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
                id: format!("windows:display:{index}:{width}x{height}@{refresh_hz}"),
                source_id: Some(format!("windows:display-shared:{index}")),
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
fn current_platform_display_mode(source_index: u32) -> Result<DisplayMode> {
    let modes = list_platform_display_modes(Some(source_index))?;
    modes
        .into_iter()
        .find(|mode| mode.is_current)
        .ok_or_else(|| anyhow::anyhow!("current display mode not found for display {source_index}"))
}

#[cfg(windows)]
fn set_platform_display_mode(source_index: u32, mode: &DisplayMode) -> Result<DisplayMode> {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_BITSPERPEL,
        DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH,
    };

    let device = display_device(source_index)?;
    let mut dev_mode = DEVMODEW::default();
    dev_mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    dev_mode.dmPelsWidth = mode.width;
    dev_mode.dmPelsHeight = mode.height;
    dev_mode.dmDisplayFrequency = mode.refresh_hz;
    dev_mode.dmBitsPerPel = mode.bit_depth.unwrap_or(32);
    dev_mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_BITSPERPEL;

    let result = unsafe {
        ChangeDisplaySettingsExW(
            PCWSTR(device.DeviceName.as_ptr()),
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
fn display_device(source_index: u32) -> Result<windows::Win32::Graphics::Gdi::DISPLAY_DEVICEW> {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW};

    let mut device = DISPLAY_DEVICEW::default();
    device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
    let ok = unsafe { EnumDisplayDevicesW(PCWSTR::null(), source_index, &mut device, 0).as_bool() };
    if !ok {
        anyhow::bail!("Windows display device not found for index {source_index}");
    }
    Ok(device)
}

#[cfg(not(windows))]
fn list_platform_display_modes(_source_index: Option<u32>) -> Result<Vec<DisplayMode>> {
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
