use std::{fs, path::Path, time::Instant};

use mrd_observability::{ComponentKind, ComponentResult};
use mrd_transport_quic_quinn::{fragment_access_unit, QuinnDatagramPair};

#[tokio::test]
#[ignore]
async fn perf_quinn_transport_sender_reports_latency_distribution() {
    let sample_count = std::env::var("MRD_COMPONENT_SAMPLES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);
    let case_name = std::env::var("MRD_COMPONENT_CASE_NAME")
        .unwrap_or_else(|_| "transport_sender.quic_quinn".into());
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("initialize quinn loopback pair");
    let max_datagram_size = pair
        .client
        .max_datagram_size()
        .expect("quinn max datagram size");
    let payload = vec![0x5a; max_datagram_size * 2 + 333];

    let mut latencies_ms = Vec::with_capacity(sample_count as usize);
    let mut payload_bytes = Vec::with_capacity(sample_count as usize);
    let mut packets_per_sample = Vec::with_capacity(sample_count as usize);
    let mut success_count = 0_u64;
    let mut failure_count = 0_u64;
    let started_at = Instant::now();

    for frame_id in 0..sample_count {
        let datagrams = fragment_access_unit(
            frame_id as u32,
            33_000 * frame_id,
            frame_id % 30 == 0,
            &payload,
            max_datagram_size,
        )
        .expect("fragment payload");
        let iter_started_at = Instant::now();
        let send_result = datagrams
            .iter()
            .try_for_each(|datagram| pair.client.send_datagram(datagram.clone()));
        match send_result {
            Ok(()) => {
                latencies_ms.push(iter_started_at.elapsed().as_secs_f64() * 1000.0);
                payload_bytes.push(payload.len());
                packets_per_sample.push(datagrams.len());
                success_count += 1;
                for _ in 0..datagrams.len() {
                    let _ = pair
                        .server
                        .read_datagram()
                        .await
                        .expect("drain server datagram");
                }
            }
            Err(_) => {
                failure_count += 1;
            }
        }
    }

    let result = ComponentResult::new(
        ComponentKind::Transport,
        "quic_quinn",
        case_name,
        started_at.elapsed().as_secs_f64(),
        success_count,
        failure_count,
        &latencies_ms,
        None,
        None,
        None,
        None,
        Some(&payload_bytes),
        Some(&payload_bytes),
        Some(&packets_per_sample),
        None,
        None,
    );

    if let Ok(result_path) = std::env::var("MRD_COMPONENT_RESULT_PATH") {
        fs::write(
            Path::new(&result_path),
            serde_json::to_string_pretty(&result).expect("serialize quic sender perf result"),
        )
        .expect("write quic sender perf result");
    }

    assert!(result.sample_count > 0);
    assert!(result.latency_ms.p50_ms.is_some());
    assert!(result.latency_ms.p95_ms.is_some());
    assert!(result.latency_ms.p99_ms.is_some());
    assert!(result.written_bytes.is_some());
    assert!(result.packets_per_sample.is_some());
}
