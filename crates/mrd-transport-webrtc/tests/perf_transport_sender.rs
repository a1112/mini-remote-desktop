use std::{fs, path::Path, time::Instant};

use mrd_observability::{ComponentKind, ComponentResult};
use mrd_pipeline_core::{EncodedAccessUnit, VideoCodec};
use mrd_transport_webrtc::H264RtpSender;

#[test]
#[ignore]
fn perf_webrtc_transport_sender_reports_latency_distribution() {
    let sample_count = std::env::var("MRD_COMPONENT_SAMPLES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);
    let case_name = std::env::var("MRD_COMPONENT_CASE_NAME")
        .unwrap_or_else(|_| "transport_sender.webrtc_rtp".into());
    let mut sender = H264RtpSender::new("perf-video", "perf-stream", 30, 1200);
    let access_unit = synthetic_h264_access_unit();

    let mut latencies_ms = Vec::with_capacity(sample_count as usize);
    let mut access_unit_sizes = Vec::with_capacity(sample_count as usize);
    let mut written_bytes = Vec::with_capacity(sample_count as usize);
    let mut packets_per_sample = Vec::with_capacity(sample_count as usize);
    let mut success_count = 0_u64;
    let mut failure_count = 0_u64;
    let started_at = Instant::now();

    for index in 0..sample_count {
        let mut access_unit = access_unit.clone();
        access_unit.timestamp_us = index * 33_000;
        let iter_started_at = Instant::now();
        match sender.packetize_access_unit(&access_unit) {
            Ok(packets) => {
                latencies_ms.push(iter_started_at.elapsed().as_secs_f64() * 1000.0);
                access_unit_sizes.push(access_unit.bytes.len());
                written_bytes.push(
                    packets
                        .iter()
                        .map(|packet| packet.payload.len() + 12)
                        .sum::<usize>(),
                );
                packets_per_sample.push(packets.len());
                success_count += 1;
            }
            Err(_) => {
                failure_count += 1;
            }
        }
    }

    let result = ComponentResult::new(
        ComponentKind::Transport,
        "webrtc_rtp",
        case_name,
        started_at.elapsed().as_secs_f64(),
        success_count,
        failure_count,
        &latencies_ms,
        None,
        None,
        None,
        None,
        Some(&access_unit_sizes),
        Some(&written_bytes),
        Some(&packets_per_sample),
        None,
        None,
    );

    if let Ok(result_path) = std::env::var("MRD_COMPONENT_RESULT_PATH") {
        fs::write(
            Path::new(&result_path),
            serde_json::to_string_pretty(&result).expect("serialize transport perf result"),
        )
        .expect("write transport perf result");
    }

    assert!(result.sample_count > 0);
    assert!(result.latency_ms.p50_ms.is_some());
    assert!(result.latency_ms.p95_ms.is_some());
    assert!(result.latency_ms.p99_ms.is_some());
    assert!(result.access_unit_bytes.is_some());
    assert!(result.written_bytes.is_some());
    assert!(result.packets_per_sample.is_some());
}

fn synthetic_h264_access_unit() -> EncodedAccessUnit {
    let mut bytes = vec![0, 0, 0, 1, 0x67, 0x42, 0xE0, 0x1F, 0x89, 0x8B, 0x60, 0x50, 0x1E, 0xD0];
    bytes.extend_from_slice(&[0, 0, 0, 1, 0x68, 0xCE, 0x06, 0xE2]);
    bytes.extend_from_slice(&[0, 0, 0, 1, 0x65]);
    bytes.extend((0..4096).map(|index| (index % 251) as u8));

    EncodedAccessUnit {
        codec: VideoCodec::H264,
        timestamp_us: 0,
        is_keyframe: true,
        bytes,
    }
}
