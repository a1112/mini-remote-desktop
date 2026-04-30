mod e2e_support;

use anyhow::Result;
use e2e_support::{run_pipeline_case, E2ePipelineCase, E2ePipelineReport};
use std::{fs, path::PathBuf};

#[test]
fn synthetic_capture_encode_transport_decode_render_matrix() -> Result<()> {
    let cases = [
        E2ePipelineCase {
            name: "qvga_30fps_1mbps",
            width: 320,
            height: 180,
            fps: 30,
            frame_count: 12,
            bitrate_bps: 1_000_000,
            mtu: 1200,
        },
        E2ePipelineCase {
            name: "vga_30fps_4mbps",
            width: 640,
            height: 360,
            fps: 30,
            frame_count: 18,
            bitrate_bps: 4_000_000,
            mtu: 1200,
        },
        E2ePipelineCase {
            name: "vga_60fps_6mbps_small_mtu",
            width: 640,
            height: 360,
            fps: 60,
            frame_count: 24,
            bitrate_bps: 6_000_000,
            mtu: 900,
        },
        E2ePipelineCase {
            name: "hd_30fps_8mbps",
            width: 1280,
            height: 720,
            fps: 30,
            frame_count: 12,
            bitrate_bps: 8_000_000,
            mtu: 1200,
        },
    ];

    let mut reports = Vec::with_capacity(cases.len());
    for case in &cases {
        reports.push(run_pipeline_case(case)?);
    }

    let markdown = render_markdown_report(&reports);
    let report_path = write_report(&markdown)?;
    println!("{markdown}");
    println!("matrix report written to {}", report_path.display());

    Ok(())
}

fn write_report(markdown: &str) -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|tests_dir| tests_dir.parent())
        .unwrap_or(&manifest_dir);
    let report_dir = repo_root.join("target/e2e-matrix");
    fs::create_dir_all(&report_dir)?;

    let report_path = report_dir.join("automated-e2e-matrix-report.md");
    fs::write(&report_path, markdown)?;
    Ok(report_path)
}

fn render_markdown_report(reports: &[E2ePipelineReport]) -> String {
    let mut output = String::from("# Automated E2E Pipeline Matrix Report\n\n");
    output.push_str(
        "Pipeline: synthetic capture -> OpenH264 encode -> QUIC AU transport -> software H.264 decode -> platform renderer upload.\n\n",
    );
    output.push_str("| Status | Case | Resolution | Target FPS | Bitrate | MTU | Frames | AUs | Transported AUs | QUIC datagrams | Decoded | Rendered | Elapsed ms | Wall FPS | Avg ms | P50 ms | P95 ms | Renderer | Pixel format |\n");
    output.push_str("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |\n");

    for report in reports {
        output.push_str(&format!(
            "| pass | {} | {}x{} | {} | {:.1} Mbps | {} | {} | {} | {} | {} | {} | {} | {:.2} | {:.1} | {:.2} | {:.2} | {:.2} | {} | {:?} |\n",
            report.name,
            report.width,
            report.height,
            report.fps,
            report.bitrate_bps as f64 / 1_000_000.0,
            report.mtu,
            report.frame_count,
            report.encoded_access_units,
            report.transported_access_units,
            report.quic_datagrams,
            report.decoded_frames,
            report.rendered_frames,
            report.elapsed_ms,
            report.render_fps,
            report.frame_avg_ms,
            report.frame_p50_ms,
            report.frame_p95_ms,
            report.renderer,
            report.last_pixel_format,
        ));
    }

    let total_rendered = reports
        .iter()
        .map(|report| report.rendered_frames)
        .sum::<usize>();
    let total_datagrams = reports
        .iter()
        .map(|report| report.quic_datagrams)
        .sum::<usize>();
    let total_encoded_bytes = reports
        .iter()
        .map(|report| report.encoded_bytes)
        .sum::<usize>();
    output.push_str(&format!(
        "\nSummary: {} cases passed, {} frames rendered, {} QUIC datagrams, {:.2} MB encoded payload.\n",
        reports.len(),
        total_rendered,
        total_datagrams,
        total_encoded_bytes as f64 / (1024.0 * 1024.0),
    ));
    output
}
