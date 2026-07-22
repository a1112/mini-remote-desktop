use std::{fs, path::Path, time::Instant};

use mrd_observability::{ComponentKind, ComponentResult};
use mrd_transport_quic_quinn::{fragment_access_unit, QuicAuReassembler, QuinnDatagramPair};

#[tokio::test]
#[ignore]
async fn perf_quinn_transport_receiver_reports_latency_distribution() {
    let sample_count = std::env::var("MRD_COMPONENT_SAMPLES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);
    let case_name = std::env::var("MRD_COMPONENT_CASE_NAME")
        .unwrap_or_else(|_| "transport_receiver.quic_quinn".into());
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("initialize quinn loopback pair");
    let max_datagram_size = pair
        .client
        .max_datagram_size()
        .expect("quinn max datagram size");
    let payload = vec![0xa5; max_datagram_size * 2 + 333];

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
        for datagram in &datagrams {
            pair.client
                .send_datagram(datagram.clone())
                .expect("send client datagram");
        }
        let iter_started_at = Instant::now();
        let mut reassembler = QuicAuReassembler::default();
        let mut completed = None;
        for _ in 0..datagrams.len() {
            match pair.server.read_datagram().await {
                Ok(read_payload) => {
                    completed = reassembler
                        .push_datagram(&read_payload)
                        .expect("reassemble datagram")
                        .or(completed);
                }
                Err(_) => {
                    failure_count += 1;
                    break;
                }
            }
        }
        if let Some(frame) = completed {
            latencies_ms.push(iter_started_at.elapsed().as_secs_f64() * 1000.0);
            payload_bytes.push(frame.payload.len());
            packets_per_sample.push(datagrams.len());
            success_count += 1;
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
            serde_json::to_string_pretty(&result).expect("serialize quic receiver perf result"),
        )
        .expect("write quic receiver perf result");
    }

    assert!(result.sample_count > 0);
    assert!(result.latency_ms.p50_ms.is_some());
    assert!(result.latency_ms.p95_ms.is_some());
    assert!(result.latency_ms.p99_ms.is_some());
    assert!(result.written_bytes.is_some());
    assert!(result.packets_per_sample.is_some());
}
