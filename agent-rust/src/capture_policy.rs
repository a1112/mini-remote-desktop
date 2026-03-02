use agent_rust::CaptureConfig;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    Dxgi,
    Powershell,
    Dummy,
}

impl CaptureBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            CaptureBackend::Dxgi => "dxgi",
            CaptureBackend::Powershell => "powershell",
            CaptureBackend::Dummy => "dummy",
        }
    }
}

pub fn choose_backend(cfg: &CaptureConfig) -> (CaptureBackend, Vec<String>) {
    let mut logs = Vec::new();
    let requested = cfg.backend.to_ascii_lowercase();
    let mut order = match requested.as_str() {
        "dxgi" => vec![CaptureBackend::Dxgi],
        "powershell" => vec![CaptureBackend::Powershell],
        "dummy" => vec![CaptureBackend::Dummy],
        _ => vec![
            CaptureBackend::Dxgi,
            CaptureBackend::Powershell,
            CaptureBackend::Dummy,
        ],
    };

    if !cfg.allow_fallback {
        order.truncate(1);
    }

    for backend in order {
        match probe_backend(backend) {
            Ok(_) => {
                logs.push(format!("capture backend selected: {}", backend.as_str()));
                return (backend, logs);
            }
            Err(e) => {
                logs.push(format!(
                    "capture backend {} unavailable: {e}",
                    backend.as_str()
                ));
            }
        }
    }

    logs.push("all requested backends failed, fallback to dummy".to_string());
    (CaptureBackend::Dummy, logs)
}

fn probe_backend(backend: CaptureBackend) -> Result<(), String> {
    match backend {
        CaptureBackend::Dxgi => {
            let screens =
                screenshots::Screen::all().map_err(|e| format!("list screens failed: {e}"))?;
            let screen = screens
                .first()
                .ok_or_else(|| "no screen found".to_string())?;
            screen
                .capture()
                .map_err(|e| format!("dxgi capture failed: {e}"))?;
            Ok(())
        }
        CaptureBackend::Powershell => probe_powershell(),
        CaptureBackend::Dummy => Ok(()),
    }
}

fn probe_powershell() -> Result<(), String> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "$PSVersionTable.PSVersion.ToString()",
        ])
        .output()
        .map_err(|e| format!("powershell spawn failed: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}
