#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRole {
    Render,
    Decode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadTuning {
    pub priority: i32,
    pub affinity_mask: Option<usize>,
    pub mmcss_profile: Option<String>,
    pub mmcss_priority: i32,
}

impl ThreadTuning {
    pub fn from_env(role: ThreadRole) -> Self {
        let role_key = match role {
            ThreadRole::Render => "RENDER",
            ThreadRole::Decode => "DECODE",
        };
        let default_priority = match role {
            ThreadRole::Render => 2,
            ThreadRole::Decode => 1,
        };
        let default_mmcss = match role {
            ThreadRole::Render => Some("Games".to_string()),
            ThreadRole::Decode => Some("Pro Audio".to_string()),
        };
        Self {
            priority: env_i32(
                &format!("MRD_{}_THREAD_PRIORITY", role_key),
                env_i32("MRD_THREAD_PRIORITY", default_priority),
            ),
            affinity_mask: env_parse_affinity(
                &format!("MRD_{}_THREAD_AFFINITY_MASK", role_key),
                env_parse_affinity("MRD_THREAD_AFFINITY_MASK", None),
            ),
            mmcss_profile: env_mmcss(
                &format!("MRD_{}_MMCSS", role_key),
                env_mmcss("MRD_THREAD_MMCSS", default_mmcss),
            ),
            mmcss_priority: env_i32(
                &format!("MRD_{}_MMCSS_PRIORITY", role_key),
                env_i32("MRD_THREAD_MMCSS_PRIORITY", 1),
            ),
        }
    }
}

pub struct ThreadTuningGuard {
    #[cfg(windows)]
    mmcss_handle: Option<windows::Win32::Foundation::HANDLE>,
    #[cfg(not(windows))]
    _placeholder: (),
}

impl Default for ThreadTuningGuard {
    fn default() -> Self {
        Self {
            #[cfg(windows)]
            mmcss_handle: None,
            #[cfg(not(windows))]
            _placeholder: (),
        }
    }
}

#[cfg(windows)]
impl Drop for ThreadTuningGuard {
    fn drop(&mut self) {
        if let Some(h) = self.mmcss_handle.take() {
            unsafe {
                if let Some(f) = avrt_revert() {
                    let _ = f(h);
                }
            }
        }
    }
}

#[cfg(not(windows))]
impl Drop for ThreadTuningGuard {
    fn drop(&mut self) {}
}

pub fn parse_affinity_mask(value: &str) -> Option<usize> {
    let s = value.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return usize::from_str_radix(hex, 16).ok();
    }
    s.parse::<usize>().ok()
}

pub fn apply_current_thread_tuning(role: ThreadRole) -> (ThreadTuning, ThreadTuningGuard) {
    let tuning = ThreadTuning::from_env(role);
    let mut guard = ThreadTuningGuard::default();
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadAffinityMask, SetThreadPriority, THREAD_PRIORITY,
        };
        let thread = GetCurrentThread();
        let _ = SetThreadPriority(thread, THREAD_PRIORITY(tuning.priority));
        if let Some(mask) = tuning.affinity_mask {
            let _ = SetThreadAffinityMask(thread, mask);
        }
        if let Some(profile) = tuning.mmcss_profile.as_deref() {
            if let Some(set_char) = avrt_set_mmcss_char() {
                let mut task_index = 0u32;
                let wide = to_wide(profile);
                let h = set_char(windows::core::PCWSTR(wide.as_ptr()), &mut task_index);
                if !h.is_invalid() {
                    guard.mmcss_handle = Some(h);
                    if let Some(set_prio) = avrt_set_mmcss_priority() {
                        let _ = set_prio(h, tuning.mmcss_priority.clamp(-2, 2));
                    }
                }
            }
        }
    }
    (tuning, guard)
}

fn env_i32(name: &str, default: i32) -> i32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<i32>().ok())
        .unwrap_or(default)
}

fn env_parse_affinity(name: &str, default: Option<usize>) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|v| parse_affinity_mask(&v))
        .or(default)
}

fn env_mmcss(name: &str, default: Option<String>) -> Option<String> {
    match std::env::var(name).ok() {
        None => default,
        Some(raw) => {
            let value = raw.trim();
            if value.is_empty()
                || value.eq_ignore_ascii_case("0")
                || value.eq_ignore_ascii_case("off")
                || value.eq_ignore_ascii_case("none")
                || value.eq_ignore_ascii_case("disable")
            {
                None
            } else {
                Some(value.to_string())
            }
        }
    }
}

#[cfg(windows)]
unsafe fn avrt_set_mmcss_char(
) -> Option<unsafe extern "system" fn(windows::core::PCWSTR, *mut u32) -> windows::Win32::Foundation::HANDLE> {
    use windows::core::PCSTR;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
    let lib = LoadLibraryA(PCSTR(b"avrt.dll\0".as_ptr())).ok()?;
    let p = GetProcAddress(lib, PCSTR(b"AvSetMmThreadCharacteristicsW\0".as_ptr()))?;
    Some(std::mem::transmute(p))
}

#[cfg(windows)]
unsafe fn avrt_set_mmcss_priority(
) -> Option<unsafe extern "system" fn(windows::Win32::Foundation::HANDLE, i32) -> i32> {
    use windows::core::PCSTR;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
    let lib = LoadLibraryA(PCSTR(b"avrt.dll\0".as_ptr())).ok()?;
    let p = GetProcAddress(lib, PCSTR(b"AvSetMmThreadPriority\0".as_ptr()))?;
    Some(std::mem::transmute(p))
}

#[cfg(windows)]
unsafe fn avrt_revert(
) -> Option<unsafe extern "system" fn(windows::Win32::Foundation::HANDLE) -> i32> {
    use windows::core::PCSTR;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
    let lib = LoadLibraryA(PCSTR(b"avrt.dll\0".as_ptr())).ok()?;
    let p = GetProcAddress(lib, PCSTR(b"AvRevertMmThreadCharacteristics\0".as_ptr()))?;
    Some(std::mem::transmute(p))
}

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_affinity_mask_supports_hex() {
        assert_eq!(parse_affinity_mask("0x3"), Some(3));
        assert_eq!(parse_affinity_mask("0X10"), Some(16));
    }

    #[test]
    fn parse_affinity_mask_supports_decimal() {
        assert_eq!(parse_affinity_mask("7"), Some(7));
    }

    #[test]
    fn decode_defaults_enable_mmcss_profile() {
        std::env::remove_var("MRD_DECODE_MMCSS");
        let t = ThreadTuning::from_env(ThreadRole::Decode);
        assert_eq!(t.mmcss_profile.as_deref(), Some("Pro Audio"));
    }
}
