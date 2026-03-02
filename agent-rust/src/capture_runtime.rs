use anyhow::{Context, Result, anyhow};
use image::ImageReader;
use std::io::Cursor;
use std::process::Command;
use std::time::Instant;

#[derive(Clone)]
pub struct RawFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub enum FrameCapturer {
    Dxgi { screen: screenshots::Screen },
    Powershell,
    Dummy,
}

impl FrameCapturer {
    pub fn capture(&mut self) -> Result<(Vec<u8>, u32, u32)> {
        match self {
            FrameCapturer::Dxgi { screen } => {
                let img = screen.capture().context("dxgi capture failed")?;
                Ok((img.as_raw().to_vec(), img.width(), img.height()))
            }
            FrameCapturer::Powershell => capture_via_powershell(),
            FrameCapturer::Dummy => {
                let w = 640_u32;
                let h = 360_u32;
                let mut rgba = vec![0_u8; (w * h * 4) as usize];
                for px in rgba.chunks_exact_mut(4) {
                    px[0] = 16;
                    px[1] = 16;
                    px[2] = 16;
                    px[3] = 255;
                }
                Ok((rgba, w, h))
            }
        }
    }
}

pub fn build_frame_capturer(
    backend: crate::capture_policy::CaptureBackend,
) -> Result<FrameCapturer> {
    match backend {
        crate::capture_policy::CaptureBackend::Dxgi => {
            let screens = screenshots::Screen::all().context("list screens failed")?;
            let screen = screens
                .first()
                .ok_or_else(|| anyhow!("no screen found"))?
                .clone();
            Ok(FrameCapturer::Dxgi { screen })
        }
        crate::capture_policy::CaptureBackend::Powershell => Ok(FrameCapturer::Powershell),
        crate::capture_policy::CaptureBackend::Dummy => Ok(FrameCapturer::Dummy),
    }
}

pub fn detect_input_resolution() -> Result<(u32, u32)> {
    let screens = screenshots::Screen::all().context("list screens failed")?;
    let screen = screens.first().ok_or_else(|| anyhow!("no screen found"))?;
    let img = screen
        .capture()
        .context("capture for resolution detect failed")?;
    Ok((img.width(), img.height()))
}

pub fn resize_rgba_fast(
    rgba: &[u8],
    width: u32,
    height: u32,
    target_width: u32,
    target_height: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    let resized = image::imageops::resize(
        &img,
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    );
    Some((resized.into_raw(), target_width, target_height))
}

pub fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now {
        std::thread::sleep(deadline - now);
    }
}

fn capture_via_powershell() -> Result<(Vec<u8>, u32, u32)> {
    let temp_path = std::env::temp_dir().join("mini-rust-agent-ps-capture.jpg");
    let path = temp_path
        .to_str()
        .ok_or_else(|| anyhow!("temp path invalid"))?
        .replace('\'', "''");

    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         Add-Type -AssemblyName System.Drawing; \
         $b=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \
         $bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height; \
         $g=[System.Drawing.Graphics]::FromImage($bmp); \
         $g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size); \
         $bmp.Save('{path}', [System.Drawing.Imaging.ImageFormat]::Jpeg); \
         $g.Dispose(); $bmp.Dispose(); \
         Write-Output ($b.Width.ToString() + ',' + $b.Height.ToString());"
    );

    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .context("powershell capture spawn failed")?;
    if !out.status.success() {
        return Err(anyhow!(
            "powershell capture failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let size_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut parts = size_str.split(',');
    let width = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| anyhow!("parse width failed"))?;
    let height = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| anyhow!("parse height failed"))?;

    let jpg = std::fs::read(&temp_path).context("read captured jpeg failed")?;
    let img = ImageReader::new(Cursor::new(jpg))
        .with_guessed_format()
        .context("guess image format failed")?
        .decode()
        .context("decode jpeg failed")?
        .to_rgba8();

    Ok((img.as_raw().to_vec(), width, height))
}
