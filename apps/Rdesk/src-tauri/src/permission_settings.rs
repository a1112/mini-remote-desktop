//! Settings navigation is allowlisted and never treated as a permission request.
fn destination(permission: &str) -> Result<&'static str, String> {
    match permission {
        "screen_capture" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        }
        "accessibility" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        }
        _ => Err("Unsupported permission settings destination".into()),
    }
}

pub fn open(permission: &str) -> Result<(), String> {
    let destination = destination(permission)?;
    #[cfg(target_os = "macos")]
    {
        // Privacy anchors are best-effort, undocumented navigation. Keep manual
        // instructions in the UI even when Launch Services accepts the URL.
        let opened = std::process::Command::new("/usr/bin/open")
            .arg(destination)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if opened {
            return Ok(());
        }
        let fallback = std::process::Command::new("/usr/bin/open")
            .args(["-b", "com.apple.systempreferences"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if fallback {
            Ok(())
        } else {
            Err("Unable to open System Settings".into())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = destination;
        Err("Permission settings navigation is only available on macOS".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_accepts_fixed_destinations() {
        assert!(destination("screen_capture")
            .unwrap()
            .ends_with("?Privacy_ScreenCapture"));
        assert!(destination("accessibility")
            .unwrap()
            .ends_with("?Privacy_Accessibility"));
        for value in [
            "",
            "full_disk_access",
            "https://example.com",
            "screen_capture;echo unsafe",
        ] {
            assert!(destination(value).is_err());
        }
    }
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_is_unsupported_and_does_not_open_anything() {
        assert!(open("screen_capture")
            .unwrap_err()
            .contains("only available on macOS"));
    }
}
