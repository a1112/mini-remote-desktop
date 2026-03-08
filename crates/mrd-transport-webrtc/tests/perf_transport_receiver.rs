use std::{fs, path::Path, time::Instant};

use mrd_observability::{ComponentKind, ComponentResult};
use mrd_transport_webrtc::H264RtpIngress;

#[test]
#[ignore]
fn perf_webrtc_transport_receiver_reports_latency_distribution() {
    let sample_count = std::env::var("MRD_COMPONENT_SAMPLES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);
    let case_name = std::env::var("MRD_COMPONENT_CASE_NAME")
        .unwrap_or_else(|_| "transport_receiver.h264_assemble".into());
    let payloads = receiver_sample_payloads();
    let mut latencies_ms = Vec::with_capacity(sample_count as usize);
    let mut access_unit_sizes = Vec::with_capacity(sample_count as usize);
    let mut payload_bytes = Vec::with_capacity(sample_count as usize);
    let mut packets_per_sample = Vec::with_capacity(sample_count as usize);
    let mut success_count = 0_u64;
    let mut failure_count = 0_u64;
    let started_at = Instant::now();

    for index in 0..sample_count {
        let mut ingress = H264RtpIngress::default();
        let timestamp_us = index * 33_000;
        let iter_started_at = Instant::now();
        let mut emitted = None;
        let mut bytes = 0usize;

        for (payload, marker) in &payloads {
            bytes += payload.len();
            emitted = ingress.push_payload(payload, *marker, timestamp_us);
        }

        match emitted {
            Some(access_unit) => {
                latencies_ms.push(iter_started_at.elapsed().as_secs_f64() * 1000.0);
                access_unit_sizes.push(access_unit.bytes.len());
                payload_bytes.push(bytes);
                packets_per_sample.push(payloads.len());
                success_count += 1;
            }
            None => {
                failure_count += 1;
            }
        }
    }

    let result = ComponentResult::new(
        ComponentKind::Transport,
        "h264_assemble",
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
        Some(&payload_bytes),
        Some(&packets_per_sample),
        None,
        None,
    );

    if let Ok(result_path) = std::env::var("MRD_COMPONENT_RESULT_PATH") {
        fs::write(
            Path::new(&result_path),
            serde_json::to_string_pretty(&result).expect("serialize transport receiver perf result"),
        )
        .expect("write transport receiver perf result");
    }

    assert!(result.sample_count > 0);
    assert!(result.latency_ms.p50_ms.is_some());
    assert!(result.latency_ms.p95_ms.is_some());
    assert!(result.latency_ms.p99_ms.is_some());
    assert!(result.access_unit_bytes.is_some());
    assert!(result.written_bytes.is_some());
    assert!(result.packets_per_sample.is_some());
}

fn receiver_sample_payloads() -> Vec<(Vec<u8>, bool)> {
    vec![
        (vec![24, 0, 2, 0x67, 0x42, 0, 2, 0x68, 0xce], false),
        (vec![0x7c, 0x85, 0xaa, 0xbb], false),
        (vec![0x7c, 0x45, 0xcc, 0xdd], true),
    ]
}
