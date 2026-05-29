use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use mrd_decode::create_decoder;
use mrd_encode_openh264::OpenH264Encoder;
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoEncoder};
use serde_json::json;

#[test]
#[ignore]
fn perf_ffmpeg_decode_compare_reports_results() {
    let samples = env_usize("MRD_FFMPEG_PERF_SAMPLES").unwrap_or(120);
    let width = env_usize("MRD_FFMPEG_PERF_WIDTH").unwrap_or(1280);
    let height = env_usize("MRD_FFMPEG_PERF_HEIGHT").unwrap_or(720);
    let fps = env_u32("MRD_FFMPEG_PERF_FPS").unwrap_or(30);
    let ffmpeg = ffmpeg_path();
    let artifact_dir = artifact_dir();
    fs::create_dir_all(&artifact_dir).expect("create artifact dir");

    let access_units = generate_h264_access_units(width, height, fps, samples);
    let encoded_frame_count = access_units.len();
    assert!(encoded_frame_count > 0, "OpenH264 produced no access units");
    let input_path = artifact_dir.join(format!(
        "openh264-{width}x{height}-requested{samples}-encoded{encoded_frame_count}.h264"
    ));
    write_h264_stream(&input_path, &access_units);

    let software = run_mrd_decoder("h264_software", &access_units);
    let nvdec = run_mrd_decoder("nvdec", &access_units);
    let ffmpeg_rgb24 = run_ffmpeg_decoder(
        "ffmpeg_cli_rgb24",
        &ffmpeg,
        &input_path,
        encoded_frame_count,
        "rgb24",
    );
    let ffmpeg_nv12 = run_ffmpeg_decoder(
        "ffmpeg_cli_nv12",
        &ffmpeg,
        &input_path,
        encoded_frame_count,
        "nv12",
    );

    let report = json!({
        "width": width,
        "height": height,
        "fps": fps,
        "requested_sample_count": samples,
        "sample_count": encoded_frame_count,
        "encoded_frame_count": encoded_frame_count,
        "input_path": input_path,
        "backends": [software, nvdec, ffmpeg_rgb24, ffmpeg_nv12],
    });
    let report_path = artifact_dir.join("ffmpeg-decode-compare.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&report).expect("serialize report"),
    )
    .expect("write report");
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    println!("report_path={}", report_path.display());
}

fn generate_h264_access_units(
    width: usize,
    height: usize,
    fps: u32,
    samples: usize,
) -> Vec<Vec<u8>> {
    let mut encoder = OpenH264Encoder::new_with_bitrate(width, height, fps, 12_000_000)
        .expect("create OpenH264 encoder");
    let mut access_units = Vec::with_capacity(samples);
    for frame_index in 0..samples {
        let frame = synthetic_bgra_frame(width, height, frame_index as u64, fps);
        let encoded = encoder.encode(&frame).expect("encode synthetic frame");
        access_units.extend(
            encoded
                .into_iter()
                .filter(|access_unit| !access_unit.bytes.is_empty())
                .map(|access_unit| access_unit.bytes),
        );
    }
    access_units
}

fn synthetic_bgra_frame(width: usize, height: usize, frame_index: u64, fps: u32) -> CapturedFrame {
    let mut data = vec![0_u8; width * height * 4];
    for (index, pixel) in data.chunks_exact_mut(4).enumerate() {
        let x = (index % width) as u8;
        let y = ((index / width) % 256) as u8;
        let t = frame_index as u8;
        pixel[0] = x.wrapping_add(t);
        pixel[1] = y.wrapping_mul(2).wrapping_add(t);
        pixel[2] = x ^ y ^ t;
        pixel[3] = 255;
    }
    let timestamp_us = frame_index.saturating_mul(1_000_000 / u64::from(fps.max(1)));
    CapturedFrame::from_cpu(width, height, FramePixelFormat::Bgra32, timestamp_us, data)
}

fn write_h264_stream(path: &Path, access_units: &[Vec<u8>]) {
    let mut bytes = Vec::new();
    for access_unit in access_units {
        bytes.extend_from_slice(access_unit);
    }
    fs::write(path, bytes).expect("write h264 stream");
}

fn run_mrd_decoder(id: &str, access_units: &[Vec<u8>]) -> serde_json::Value {
    let mut decoder = match create_decoder(id) {
        Ok(decoder) => decoder,
        Err(error) => {
            return json!({
                "backend": id,
                "available": false,
                "error": error.to_string(),
            });
        }
    };

    let started = Instant::now();
    let mut decoded_frames = 0_usize;
    let mut errors = 0_usize;
    for access_unit in access_units {
        match decoder.push_access_unit(access_unit) {
            Ok(()) => decoded_frames += decoder.drain_decoded_frames().len(),
            Err(_) => errors += 1,
        }
    }
    let elapsed_s = started.elapsed().as_secs_f64();
    json!({
        "backend": id,
        "available": true,
        "decoded_frames": decoded_frames,
        "errors": errors,
        "elapsed_s": elapsed_s,
        "throughput_fps": decoded_frames as f64 / elapsed_s.max(f64::EPSILON),
    })
}

fn run_ffmpeg_decoder(
    backend: &str,
    ffmpeg: &Path,
    input_path: &Path,
    expected_frames: usize,
    pixel_format: &str,
) -> serde_json::Value {
    if !ffmpeg.is_file() {
        return json!({
            "backend": backend,
            "available": false,
            "error": format!("ffmpeg executable not found: {}", ffmpeg.display()),
        });
    }

    let output_path = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let started = Instant::now();
    let output = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-f", "h264", "-i"])
        .arg(input_path)
        .args([
            "-f",
            "rawvideo",
            "-pix_fmt",
            pixel_format,
            "-y",
            output_path,
        ])
        .output()
        .expect("run ffmpeg decode");
    let elapsed_s = started.elapsed().as_secs_f64();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    json!({
        "backend": backend,
        "available": output.status.success(),
        "decoded_frames": if output.status.success() { expected_frames } else { 0 },
        "errors": if output.status.success() { 0 } else { 1 },
        "elapsed_s": elapsed_s,
        "pixel_format": pixel_format,
        "throughput_fps": if output.status.success() { expected_frames as f64 / elapsed_s.max(f64::EPSILON) } else { 0.0 },
        "error": if output.status.success() { serde_json::Value::Null } else { json!(stderr) },
        "path": ffmpeg,
    })
}

fn ffmpeg_path() -> PathBuf {
    if let Ok(path) = std::env::var("MRD_FFMPEG_BIN") {
        return PathBuf::from(path);
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata)
            .join("mini-remote-desktop")
            .join("tools")
            .join("ffmpeg")
            .join("release-essentials")
            .join("bin")
            .join(if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            });
    }
    PathBuf::from("ffmpeg")
}

fn artifact_dir() -> PathBuf {
    let dir = std::env::var("MRD_FFMPEG_PERF_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from("artifacts")
                .join("ffmpeg-perf")
                .join(timestamp())
        });
    if dir.is_absolute() {
        dir
    } else {
        workspace_root().join(dir)
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("mrd-decode crate should live under workspace crates directory")
        .to_path_buf()
}

fn timestamp() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time");
    duration.as_secs().to_string()
}

fn env_usize(key: &str) -> Option<usize> {
    std::env::var(key).ok()?.parse().ok()
}

fn env_u32(key: &str) -> Option<u32> {
    std::env::var(key).ok()?.parse().ok()
}
