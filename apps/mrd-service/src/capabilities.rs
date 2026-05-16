use mrd_ipc::{
    CapabilityConstraint, CapabilityConstraintStatus, CapabilityDomain, CapabilityItem,
    CapabilityPlatform, CapabilityProfile, CapabilitySnapshot, CapabilityStatus,
};
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: u32 = 1;

pub fn local_capability_snapshot() -> CapabilitySnapshot {
    let platform = current_platform();
    CapabilitySnapshot {
        schema_version: SCHEMA_VERSION,
        platform: platform.clone(),
        service_version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: local_capabilities(platform),
        constraints: default_constraints(),
        profiles: default_profiles(),
        updated_at_ms: now_ms(),
    }
}

fn local_capabilities(platform: CapabilityPlatform) -> Vec<CapabilityItem> {
    let mut items = Vec::new();

    add_capture_capabilities(&mut items, &platform);
    add_capture_source_capabilities(&mut items, &platform);
    add_encode_capabilities(&mut items, &platform);
    add_decode_capabilities(&mut items, &platform);
    add_render_capabilities(&mut items, &platform);
    add_memory_capabilities(&mut items, &platform);
    add_transport_capabilities(&mut items, &platform);
    add_control_capabilities(&mut items, &platform);
    add_audio_capabilities(&mut items, &platform);
    add_service_capabilities(&mut items, &platform);
    add_security_capabilities(&mut items, &platform);

    items
}

fn add_capture_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    match platform {
        CapabilityPlatform::Windows => {
            push_available(
                items,
                platform,
                CapabilityDomain::Capture,
                "capture.dxgi",
                "DXGI",
            );
            push_available(
                items,
                platform,
                CapabilityDomain::Capture,
                "capture.winrt",
                "WinRT/WGC",
            );
        }
        CapabilityPlatform::Macos => {
            push_supported(
                items,
                platform,
                CapabilityDomain::Capture,
                "capture.macos",
                "ScreenCaptureKit",
                "macOS capture is available through the Rdesk harness path.",
            );
        }
        CapabilityPlatform::Linux => {
            #[cfg(target_os = "linux")]
            let status = if mrd_capture_pipewire::PipewireScreenCapture::is_wayland_available() {
                if mrd_capture_pipewire::PipewireScreenCapture::is_pipewire_available() {
                    (
                        CapabilityStatus::Supported,
                        "Wayland capture requires portal session approval before it is usable.",
                    )
                } else {
                    (
                        CapabilityStatus::DriverMissing,
                        "Wayland capture requires PipeWire and xdg-desktop-portal runtime support.",
                    )
                }
            } else if mrd_capture_pipewire::PipewireScreenCapture::is_x11_available() {
                (
                    CapabilityStatus::Available,
                    "X11 capture backend is available for the current desktop session.",
                )
            } else {
                (
                    CapabilityStatus::Unsupported,
                    "No DISPLAY or WAYLAND_DISPLAY session was detected.",
                )
            };

            #[cfg(not(target_os = "linux"))]
            let status = (
                CapabilityStatus::Unsupported,
                "Linux capture probe is only compiled on Linux.",
            );

            push_item(
                items,
                platform,
                CapabilityDomain::Capture,
                "capture.linux",
                "Linux screen capture",
                status.0,
                Some(status.1),
            );
        }
        _ => {}
    }

    push_available(
        items,
        platform,
        CapabilityDomain::Capture,
        "capture.synthetic",
        "Synthetic capture",
    );
}

fn add_capture_source_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    push_available(
        items,
        platform,
        CapabilityDomain::CaptureSource,
        "capture_source.display",
        "Display capture",
    );
    let shared_status = if matches!(platform, CapabilityPlatform::Windows) {
        CapabilityStatus::Available
    } else {
        CapabilityStatus::Unimplemented
    };
    push_item(
        items,
        platform,
        CapabilityDomain::CaptureSource,
        "capture_source.display_shared",
        "Shared display capture",
        shared_status,
        if matches!(platform, CapabilityPlatform::Windows) {
            None
        } else {
            Some("Shared desktop texture capture is not wired for this platform.")
        },
    );
    push_available(
        items,
        platform,
        CapabilityDomain::CaptureSource,
        "capture_source.window",
        "Window capture",
    );
}

fn add_encode_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    push_degraded(
        items,
        platform,
        CapabilityDomain::Encode,
        "encode.openh264",
        "OpenH264",
        "Software encoder fallback; usable but below hardware path parity.",
    );

    let nvidia_status = if matches!(
        platform,
        CapabilityPlatform::Windows | CapabilityPlatform::Linux
    ) {
        CapabilityStatus::Supported
    } else {
        CapabilityStatus::Unsupported
    };
    let nvidia_reason = if nvidia_status == CapabilityStatus::Supported {
        Some("NVIDIA runtime probing is owned by the Rdesk harness for this phase.")
    } else {
        Some("NVENC is not supported on this platform in the current product mode.")
    };

    for (id, label) in [
        ("encode.nvenc_h264", "NVENC H.264"),
        ("encode.nvenc_hevc", "NVENC HEVC"),
        ("encode.nvenc_hevc_main10", "NVENC HEVC Main10"),
        ("encode.nvenc_av1", "NVENC AV1"),
    ] {
        push_item(
            items,
            platform,
            CapabilityDomain::Encode,
            id,
            label,
            nvidia_status.clone(),
            nvidia_reason,
        );
    }

    if matches!(platform, CapabilityPlatform::Macos) {
        push_supported(
            items,
            platform,
            CapabilityDomain::Encode,
            "encode.videotoolbox_h264",
            "VideoToolbox H.264",
            "VideoToolbox encode is wired in the Rdesk harness path.",
        );
    }
}

fn add_decode_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    push_degraded(
        items,
        platform,
        CapabilityDomain::Decode,
        "decode.software",
        "Software H.264 decode",
        "Software decoder fallback; usable but below hardware path parity.",
    );

    let nvdec_status = if matches!(platform, CapabilityPlatform::Windows) {
        CapabilityStatus::Supported
    } else {
        CapabilityStatus::Unimplemented
    };
    push_item(
        items,
        platform,
        CapabilityDomain::Decode,
        "decode.nvdec",
        "NVDEC",
        nvdec_status,
        Some("NVDEC runtime probing is owned by the Rdesk harness for this phase."),
    );

    if matches!(platform, CapabilityPlatform::Linux) {
        #[cfg(target_os = "linux")]
        let (status, reason) = match mrd_decode::probe_linux_h264_hardware_available() {
            Ok(label) => (
                CapabilityStatus::Supported,
                format!("{label} is available through the Linux GStreamer decode path."),
            ),
            Err(error) => (
                CapabilityStatus::DriverMissing,
                format!(
                    "Linux H.264 hardware decode requires GStreamer plus a VA/NVIDIA H.264 decoder element: {error}"
                ),
            ),
        };

        #[cfg(not(target_os = "linux"))]
        let (status, reason) = (
            CapabilityStatus::Unsupported,
            "Linux H.264 hardware decode is only compiled on Linux.".to_string(),
        );

        push_item(
            items,
            platform,
            CapabilityDomain::Decode,
            "decode.linux_h264",
            "Linux H.264 hardware decode",
            status,
            Some(reason.as_str()),
        );

        #[cfg(target_os = "linux")]
        let (status, reason) = match mrd_decode::probe_linux_hevc_hardware_available() {
            Ok(label) => (
                CapabilityStatus::Supported,
                format!("{label} is available through the Linux GStreamer decode path."),
            ),
            Err(error) => (
                CapabilityStatus::DriverMissing,
                format!(
                    "Linux HEVC hardware decode requires GStreamer plus a VA/NVIDIA HEVC decoder element: {error}"
                ),
            ),
        };

        #[cfg(not(target_os = "linux"))]
        let (status, reason) = (
            CapabilityStatus::Unsupported,
            "Linux HEVC hardware decode is only compiled on Linux.".to_string(),
        );

        push_item(
            items,
            platform,
            CapabilityDomain::Decode,
            "decode.linux_hevc",
            "Linux HEVC hardware decode",
            status,
            Some(reason.as_str()),
        );

        #[cfg(target_os = "linux")]
        let (status, reason) = match mrd_decode::probe_linux_hevc_main10_hardware_available() {
            Ok(label) => (
                CapabilityStatus::Supported,
                format!("{label} is available through the Linux GStreamer decode path."),
            ),
            Err(error) => (
                CapabilityStatus::DriverMissing,
                format!(
                    "Linux HEVC Main10 hardware decode requires GStreamer plus a VA/NVIDIA HEVC decoder element: {error}"
                ),
            ),
        };

        #[cfg(not(target_os = "linux"))]
        let (status, reason) = (
            CapabilityStatus::Unsupported,
            "Linux HEVC Main10 hardware decode is only compiled on Linux.".to_string(),
        );

        push_item(
            items,
            platform,
            CapabilityDomain::Decode,
            "decode.linux_hevc_main10",
            "Linux HEVC Main10 hardware decode",
            status,
            Some(reason.as_str()),
        );
    }

    if matches!(platform, CapabilityPlatform::Macos) {
        push_supported(
            items,
            platform,
            CapabilityDomain::Decode,
            "decode.videotoolbox",
            "VideoToolbox decode",
            "VideoToolbox decode is planned for native macOS parity.",
        );
    }
}

fn add_render_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    match platform {
        CapabilityPlatform::Windows => {
            push_available(
                items,
                platform,
                CapabilityDomain::Render,
                "render.d3d11",
                "D3D11",
            );
            push_item(
                items,
                platform,
                CapabilityDomain::Render,
                "render.d3d12_native",
                "D3D12 native",
                CapabilityStatus::Unimplemented,
                Some("D3D12 renderer is probe-only and not wired as mainline display."),
            );
            push_supported(
                items,
                platform,
                CapabilityDomain::Render,
                "render.opengl",
                "OpenGL",
                "OpenGL renderer supports CPU-backed frames and WGL/DX interop for D3D11 shared NV12 when available; D3D11 remains the Windows high-performance path.",
            );
        }
        CapabilityPlatform::Macos => push_supported(
            items,
            platform,
            CapabilityDomain::Render,
            "render.macos",
            "Metal",
            "Metal renderer is wired in the Rdesk harness path.",
        ),
        CapabilityPlatform::Linux => push_supported(
            items,
            platform,
            CapabilityDomain::Render,
            "render.linux",
            "Linux native renderer",
            "Linux renderer is wired in the Rdesk harness path.",
        ),
        _ => {}
    }

    push_degraded(
        items,
        platform,
        CapabilityDomain::Render,
        "render.webview",
        "WebView fallback",
        "WebView render is diagnostic fallback, not native display parity.",
    );
}

fn add_memory_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    push_available(
        items,
        platform,
        CapabilityDomain::Memory,
        "memory.cpu",
        "CPU memory",
    );
    let status = if matches!(platform, CapabilityPlatform::Windows) {
        CapabilityStatus::Available
    } else {
        CapabilityStatus::Unimplemented
    };
    push_item(
        items,
        platform,
        CapabilityDomain::Memory,
        "memory.d3d11_shared",
        "D3D11 shared texture",
        status,
        if matches!(platform, CapabilityPlatform::Windows) {
            None
        } else {
            Some("D3D11 shared texture interop is Windows-only.")
        },
    );
}

fn add_transport_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    for (id, label) in [
        ("transport.loopback", "In-process loopback"),
        ("transport.webrtc", "WebRTC RTP"),
        ("transport.quic", "QUIC"),
        ("transport.quic_datagram", "QUIC datagram media"),
        (
            "transport.media_profile_control_v1",
            "Media profile control v1",
        ),
        (
            "transport.capture_source_control_v1",
            "Capture source control v1",
        ),
    ] {
        push_available(items, platform, CapabilityDomain::Transport, id, label);
    }
}

fn add_control_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    let status = if matches!(platform, CapabilityPlatform::Windows) {
        CapabilityStatus::Supported
    } else {
        CapabilityStatus::Unimplemented
    };
    push_item(
        items,
        platform,
        CapabilityDomain::Control,
        "control.keyboard_mouse",
        "Keyboard and mouse control",
        status,
        Some("Input injection is not yet a service-owned cross-platform adapter."),
    );
}

fn add_audio_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    push_item(
        items,
        platform,
        CapabilityDomain::Audio,
        "audio.system",
        "System audio",
        CapabilityStatus::Unimplemented,
        Some("Audio capture/playback is outside the current media pipeline."),
    );
}

fn add_service_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    push_available(
        items,
        platform,
        CapabilityDomain::Service,
        "service.ipc",
        "Local IPC",
    );
    push_available(
        items,
        platform,
        CapabilityDomain::Service,
        "service.lan_discovery",
        "LAN discovery",
    );
    push_supported(
        items,
        platform,
        CapabilityDomain::Service,
        "service.tray",
        "Service tray",
        "Tray availability depends on the active desktop environment.",
    );
    push_supported(
        items,
        platform,
        CapabilityDomain::Service,
        "service.autostart",
        "Autostart",
        "Autostart support is provided by platform shell adapters.",
    );
}

fn add_security_capabilities(items: &mut Vec<CapabilityItem>, platform: &CapabilityPlatform) {
    push_available(
        items,
        platform,
        CapabilityDomain::Security,
        "security.quic_tls",
        "QUIC TLS",
    );
    push_supported(
        items,
        platform,
        CapabilityDomain::Security,
        "security.consent",
        "Session consent",
        "Consent and pairing UX are still being migrated into service-owned flows.",
    );
}

fn default_constraints() -> Vec<CapabilityConstraint> {
    vec![
        CapabilityConstraint {
            id: "openh264_requires_cpu_input".to_string(),
            applies_to: vec![
                "encode.openh264".to_string(),
                "memory.d3d11_shared".to_string(),
            ],
            status: CapabilityConstraintStatus::Block,
            reason: "OpenH264 requires CPU-backed input unless an explicit copy step is inserted."
                .to_string(),
            fallback_ids: vec!["memory.cpu".to_string()],
        },
        CapabilityConstraint {
            id: "d3d12_probe_only".to_string(),
            applies_to: vec!["render.d3d12_native".to_string()],
            status: CapabilityConstraintStatus::Block,
            reason: "D3D12 native renderer is probe-only and not wired as mainline display."
                .to_string(),
            fallback_ids: vec!["render.d3d11".to_string(), "render.webview".to_string()],
        },
        CapabilityConstraint {
            id: "opengl_d3d11_shared_interop_hybrid".to_string(),
            applies_to: vec![
                "render.opengl".to_string(),
                "memory.d3d11_shared".to_string(),
            ],
            status: CapabilityConstraintStatus::Degrade,
            reason: "OpenGL accepts D3D11 shared NV12 through WGL/DX interop when available and readback fallback otherwise; D3D11 native remains preferred for parity."
                .to_string(),
            fallback_ids: vec!["render.d3d11".to_string()],
        },
        CapabilityConstraint {
            id: "webview_degraded_render".to_string(),
            applies_to: vec!["render.webview".to_string()],
            status: CapabilityConstraintStatus::Degrade,
            reason: "WebView render is a visual fallback, not native renderer parity.".to_string(),
            fallback_ids: Vec::new(),
        },
    ]
}

fn default_profiles() -> Vec<CapabilityProfile> {
    vec![
        profile(
            "smoke.720p30",
            1280,
            720,
            30,
            8,
            "h264",
            vec!["transport.loopback", "encode.openh264", "decode.software"],
        ),
        profile(
            "interactive.1080p60",
            1920,
            1080,
            60,
            20,
            "h264",
            vec![
                "transport.quic_datagram",
                "transport.media_profile_control_v1",
            ],
        ),
        profile(
            "lan.2k144",
            2560,
            1440,
            144,
            64,
            "h264",
            vec![
                "transport.quic_datagram",
                "transport.media_profile_control_v1",
            ],
        ),
        profile(
            "quality.4k60",
            3840,
            2160,
            60,
            80,
            "h264",
            vec![
                "transport.quic_datagram",
                "transport.media_profile_control_v1",
            ],
        ),
        profile(
            "diagnostic.software",
            1280,
            720,
            30,
            6,
            "h264",
            vec![
                "capture.synthetic",
                "encode.openh264",
                "decode.software",
                "render.webview",
            ],
        ),
    ]
}

fn profile(
    id: &str,
    width: u32,
    height: u32,
    fps: u32,
    bitrate_mbps: u32,
    codec: &str,
    required_capabilities: Vec<&str>,
) -> CapabilityProfile {
    CapabilityProfile {
        id: id.to_string(),
        width,
        height,
        fps,
        bitrate_mbps,
        codec: codec.to_string(),
        latency_budget_ms: None,
        min_stable_fps_ratio: Some(0.8),
        max_drop_ratio: Some(0.02),
        required_capabilities: required_capabilities
            .into_iter()
            .map(ToString::to_string)
            .collect(),
    }
}

fn push_available(
    items: &mut Vec<CapabilityItem>,
    platform: &CapabilityPlatform,
    domain: CapabilityDomain,
    id: &str,
    label: &str,
) {
    push_item(
        items,
        platform,
        domain,
        id,
        label,
        CapabilityStatus::Available,
        None,
    );
}

fn push_supported(
    items: &mut Vec<CapabilityItem>,
    platform: &CapabilityPlatform,
    domain: CapabilityDomain,
    id: &str,
    label: &str,
    reason: &str,
) {
    push_item(
        items,
        platform,
        domain,
        id,
        label,
        CapabilityStatus::Supported,
        Some(reason),
    );
}

fn push_degraded(
    items: &mut Vec<CapabilityItem>,
    platform: &CapabilityPlatform,
    domain: CapabilityDomain,
    id: &str,
    label: &str,
    reason: &str,
) {
    push_item(
        items,
        platform,
        domain,
        id,
        label,
        CapabilityStatus::Degraded,
        Some(reason),
    );
}

fn push_item(
    items: &mut Vec<CapabilityItem>,
    platform: &CapabilityPlatform,
    domain: CapabilityDomain,
    id: &str,
    label: &str,
    status: CapabilityStatus,
    reason: Option<&str>,
) {
    items.push(CapabilityItem {
        id: id.to_string(),
        domain,
        label: label.to_string(),
        status,
        platform: platform.clone(),
        reason: reason.map(ToString::to_string),
        detail: None,
        requires: Vec::new(),
        conflicts_with: Vec::new(),
        depends_on: Vec::new(),
        fallback_ids: Vec::new(),
        last_probe_time_ms: None,
    });
}

fn current_platform() -> CapabilityPlatform {
    if cfg!(windows) {
        CapabilityPlatform::Windows
    } else if cfg!(target_os = "macos") {
        CapabilityPlatform::Macos
    } else if cfg!(target_os = "linux") {
        CapabilityPlatform::Linux
    } else {
        CapabilityPlatform::Unknown
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_includes_platform_domains_and_profiles() {
        let snapshot = local_capability_snapshot();

        assert_eq!(snapshot.schema_version, 1);
        assert!(snapshot
            .capabilities
            .iter()
            .any(|item| item.id == "transport.quic_datagram"));
        assert!(snapshot
            .profiles
            .iter()
            .any(|profile| profile.id == "lan.2k144"));
        assert!(snapshot
            .constraints
            .iter()
            .any(|constraint| constraint.id == "openh264_requires_cpu_input"));
        assert!(snapshot
            .constraints
            .iter()
            .any(|constraint| constraint.id == "opengl_d3d11_shared_interop_hybrid"));
        #[cfg(windows)]
        assert!(snapshot
            .capabilities
            .iter()
            .any(|item| item.id == "render.opengl"));
    }
}
