mod e2e_support;

use anyhow::Result;
use e2e_support::{
    run_threaded_transport_pipeline_case, ThreadedTransportPipelineCase,
    ThreadedTransportPipelineReport,
};
use std::{fs, path::PathBuf};

#[test]
fn threaded_capture_encode_quic_transport_decode_render_pipeline() -> Result<()> {
    let case = configured_case();
    let report = run_threaded_transport_pipeline_case(&case)?;

    assert_eq!(report.transport, "quic_quinn_loopback");
    assert_eq!(report.media_protocol, "quic_media_v3_datagram");
    assert!(report.encoded_access_units > 0);
    assert!(report.quic_datagrams_sent > 0);
    assert!(report.quic_datagrams_received > 0);
    assert!(report.transported_access_units > 0);
    assert!(report.decoded_frames > 0);
    assert_eq!(report.decoded_frames, report.rendered_frames);
    assert_ne!(report.sender_local_addr, report.receiver_local_addr);

    let markdown = render_markdown_report(&report);
    let report_path = write_report(&markdown)?;
    println!("{markdown}");
    println!(
        "threaded transport report written to {}",
        report_path.display()
    );

    Ok(())
}

fn configured_case() -> ThreadedTransportPipelineCase {
    ThreadedTransportPipelineCase {
        name: "threaded_quic_media_v3",
        width: env_usize("MRD_THREADED_TRANSPORT_WIDTH", 640),
        height: env_usize("MRD_THREADED_TRANSPORT_HEIGHT", 360),
        fps: env_u32("MRD_THREADED_TRANSPORT_FPS", 30),
        frame_count: env_usize("MRD_THREADED_TRANSPORT_FRAME_COUNT", 18),
        bitrate_bps: env_u32("MRD_THREADED_TRANSPORT_BITRATE_BPS", 4_000_000),
        mtu: env_usize("MRD_THREADED_TRANSPORT_MTU", 1200),
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

fn write_report(markdown: &str) -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|tests_dir| tests_dir.parent())
        .unwrap_or(&manifest_dir);
    let report_dir = std::env::var_os("MRD_THREADED_TRANSPORT_REPORT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("target/e2e-threaded-transport"));
    fs::create_dir_all(&report_dir)?;

    let report_path = report_dir.join("threaded-transport-e2e-report.md");
    fs::write(&report_path, markdown)?;
    Ok(report_path)
}

fn render_markdown_report(report: &ThreadedTransportPipelineReport) -> String {
    let mut output = String::from("# Threaded Transport E2E Report\n\n");
    output.push_str(
        "Pipeline: sender node capture -> OpenH264 encode -> QUIC media v3 datagram loopback -> receiver node reassemble -> software H.264 decode -> platform renderer upload.\n\n",
    );
    output.push_str("| Status | Case | Resolution | Target FPS | Bitrate | MTU | Sender | Receiver | Encoded AUs | Sent datagrams | Received datagrams | Reassembled AUs | Decoded | Rendered | Elapsed ms | Sender FPS | Render FPS | Renderer | Pixel format |\n");
    output.push_str("| --- | --- | ---: | ---: | ---: | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |\n");
    output.push_str(&format!(
        "| pass | {} | {}x{} | {} | {:.1} Mbps | {} | {} -> {} | {} <- {} | {} | {} | {} | {} | {} | {} | {:.2} | {:.1} | {:.1} | {} | {:?} |\n",
        report.name,
        report.width,
        report.height,
        report.fps,
        report.bitrate_bps as f64 / 1_000_000.0,
        report.mtu,
        report.sender_node,
        report.sender_peer_addr,
        report.receiver_node,
        report.receiver_peer_addr,
        report.encoded_access_units,
        report.quic_datagrams_sent,
        report.quic_datagrams_received,
        report.transported_access_units,
        report.decoded_frames,
        report.rendered_frames,
        report.elapsed_ms,
        report.sender_fps,
        report.render_fps,
        report.renderer,
        report.last_pixel_format,
    ));
    output.push_str(&format!(
        "\nTransport: {} / {}. Sender local: {}. Receiver local: {}.\n",
        report.transport,
        report.media_protocol,
        report.sender_local_addr,
        report.receiver_local_addr,
    ));
    output.push_str(&format!(
        "Reassembler: completed={}, expired={}, evicted={}, duplicate_fragments={}, rejected_fragments={}.\n",
        report.reassembler_completed_frames,
        report.reassembler_expired_frames,
        report.reassembler_evicted_frames,
        report.reassembler_duplicate_fragments,
        report.reassembler_rejected_fragments,
    ));
    output
}
