use mrd_ipc::{
    CapabilityConstraint, CapabilityConstraintStatus, CapabilityDomain, CapabilityItem,
    CapabilityPlatform, CapabilityProfile, CapabilitySnapshot, CapabilityStatus, LanPeerInfo,
    MediaProfile, ScenarioEvaluation, ScenarioEvaluationReason, ScenarioEvaluationStatus,
};
use std::{
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy)]
enum CapabilityProbeMode {
    Runtime,
    Static,
}

pub fn local_capability_snapshot() -> CapabilitySnapshot {
    local_capability_snapshot_with_mode(CapabilityProbeMode::Runtime)
}

pub fn local_capability_snapshot_static() -> CapabilitySnapshot {
    local_capability_snapshot_with_mode(CapabilityProbeMode::Static)
}

fn local_capability_snapshot_with_mode(probe_mode: CapabilityProbeMode) -> CapabilitySnapshot {
    let platform = current_platform();
    CapabilitySnapshot {
        schema_version: SCHEMA_VERSION,
        platform: platform.clone(),
        service_version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: local_capabilities(platform, probe_mode),
        constraints: default_constraints(),
        profiles: default_profiles(),
        updated_at_ms: now_ms(),
    }
}

pub fn evaluate_scenario_profile_against_snapshot(
    snapshot: &CapabilitySnapshot,
    scenario_id: &str,
    requested_profile: Option<MediaProfile>,
) -> ScenarioEvaluation {
    evaluate_against_snapshot(snapshot, scenario_id, requested_profile)
}

pub fn peer_capability_snapshot(peer: &LanPeerInfo) -> CapabilitySnapshot {
    let capabilities = peer
        .transports
        .iter()
        .map(|transport| format!("transport.{transport}"))
        .chain(peer.media_capabilities.iter().cloned())
        .map(|id| CapabilityItem {
            label: id.clone(),
            domain: capability_domain_from_id(&id),
            id,
            status: CapabilityStatus::Available,
            platform: CapabilityPlatform::Unknown,
            reason: None,
            detail: Some(format!("advertised by LAN peer {}", peer.device_id.0)),
            requires: Vec::new(),
            conflicts_with: Vec::new(),
            depends_on: Vec::new(),
            fallback_ids: Vec::new(),
            last_probe_time_ms: None,
        })
        .collect();

    CapabilitySnapshot {
        schema_version: SCHEMA_VERSION,
        platform: CapabilityPlatform::Unknown,
        service_version: peer
            .service_build_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        capabilities,
        constraints: default_constraints(),
        profiles: default_profiles(),
        updated_at_ms: now_ms(),
    }
}

fn evaluate_against_snapshot(
    snapshot: &CapabilitySnapshot,
    scenario_id: &str,
    requested_profile: Option<MediaProfile>,
) -> ScenarioEvaluation {
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == scenario_id)
        .cloned();
    let required_capabilities = profile
        .as_ref()
        .map(|profile| profile.required_capabilities.clone())
        .unwrap_or_default();

    let mut missing_capabilities = Vec::new();
    let mut degraded = false;
    let mut reasons = Vec::new();
    for capability_id in &required_capabilities {
        match snapshot
            .capabilities
            .iter()
            .find(|item| item.id == *capability_id)
        {
            Some(item) if capability_status_runs(&item.status) => {
                if matches!(
                    item.status,
                    CapabilityStatus::Supported | CapabilityStatus::Degraded
                ) {
                    degraded = true;
                    reasons.push(reason(
                        "capability.degraded",
                        "warning",
                        format!(
                            "{} is {}, runtime may run below preferred parity.",
                            item.id,
                            capability_status_label(&item.status)
                        ),
                        Some(item.id.clone()),
                    ));
                }
            }
            Some(item) => {
                missing_capabilities.push(item.id.clone());
                reasons.push(reason(
                    "capability.blocked",
                    "error",
                    item.reason.clone().unwrap_or_else(|| {
                        format!(
                            "{} is {} and cannot satisfy this scenario.",
                            item.id,
                            capability_status_label(&item.status)
                        )
                    }),
                    Some(item.id.clone()),
                ));
            }
            None => {
                missing_capabilities.push(capability_id.clone());
                reasons.push(reason(
                    "capability.missing",
                    "error",
                    format!("{capability_id} is not advertised by this endpoint."),
                    Some(capability_id.clone()),
                ));
            }
        }
    }

    let mut selected_profile = requested_profile.or_else(|| profile.as_ref().map(profile_to_media));
    if let Some(selected) = selected_profile.as_mut() {
        if selected.codec.trim().is_empty() {
            selected.codec = profile
                .as_ref()
                .map(|profile| profile.codec.clone())
                .unwrap_or_else(|| "h264".to_string());
        }
    }

    let status = if profile.is_none() && selected_profile.is_none() {
        reasons.push(reason(
            "profile.unknown",
            "error",
            format!("Scenario profile {scenario_id} is not known by this service."),
            None,
        ));
        ScenarioEvaluationStatus::Blocked
    } else if !missing_capabilities.is_empty() {
        ScenarioEvaluationStatus::Blocked
    } else if degraded {
        ScenarioEvaluationStatus::Degraded
    } else {
        reasons.push(reason(
            "profile.ready",
            "info",
            "All required capabilities are present.".to_string(),
            None,
        ));
        ScenarioEvaluationStatus::Ready
    };

    let fallback_profile = if matches!(status, ScenarioEvaluationStatus::Blocked) {
        snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == "diagnostic.software")
            .map(profile_to_media)
    } else {
        None
    };

    ScenarioEvaluation {
        scenario_id: scenario_id.to_string(),
        status,
        selected_profile,
        transport_kind: Some(transport_for_scenario(scenario_id, &required_capabilities)),
        reasons,
        required_capabilities,
        missing_capabilities,
        fallback_profile,
    }
}

fn profile_to_media(profile: &CapabilityProfile) -> MediaProfile {
    MediaProfile {
        width: profile.width,
        height: profile.height,
        fps: profile.fps,
        bitrate_mbps: profile.bitrate_mbps,
        codec: profile.codec.clone(),
        ..MediaProfile::default()
    }
}

fn capability_status_runs(status: &CapabilityStatus) -> bool {
    matches!(
        status,
        CapabilityStatus::Available
            | CapabilityStatus::Usable
            | CapabilityStatus::Supported
            | CapabilityStatus::Degraded
    )
}

fn capability_status_label(status: &CapabilityStatus) -> &'static str {
    match status {
        CapabilityStatus::Supported => "supported",
        CapabilityStatus::Available => "available",
        CapabilityStatus::Usable => "usable",
        CapabilityStatus::Degraded => "degraded",
        CapabilityStatus::PermissionMissing => "permission_missing",
        CapabilityStatus::DriverMissing => "driver_missing",
        CapabilityStatus::HardwareMissing => "hardware_missing",
        CapabilityStatus::Unimplemented => "unimplemented",
        CapabilityStatus::Unsupported => "unsupported",
        CapabilityStatus::Unknown => "unknown",
    }
}

fn reason(
    code: impl Into<String>,
    severity: impl Into<String>,
    message: impl Into<String>,
    capability_id: Option<String>,
) -> ScenarioEvaluationReason {
    ScenarioEvaluationReason {
        code: code.into(),
        severity: severity.into(),
        message: message.into(),
        capability_id,
    }
}

fn transport_for_scenario(scenario_id: &str, required_capabilities: &[String]) -> String {
    if scenario_id.starts_with("wan.")
        || required_capabilities
            .iter()
            .any(|id| id == "transport.webrtc")
    {
        "webrtc".to_string()
    } else if required_capabilities
        .iter()
        .any(|id| id == "transport.quic" || id == "transport.quic_datagram")
        || scenario_id.starts_with("lan.")
        || scenario_id.starts_with("quality.")
    {
        "quic".to_string()
    } else {
        "loopback".to_string()
    }
}

fn capability_domain_from_id(id: &str) -> CapabilityDomain {
    match id.split_once('.').map(|(prefix, _)| prefix).unwrap_or(id) {
        "capture" => CapabilityDomain::Capture,
        "capture_source" => CapabilityDomain::CaptureSource,
        "encode" => CapabilityDomain::Encode,
        "decode" => CapabilityDomain::Decode,
        "render" => CapabilityDomain::Render,
        "memory" => CapabilityDomain::Memory,
        "transport" | "quic" | "webrtc" => CapabilityDomain::Transport,
        "control" => CapabilityDomain::Control,
        "audio" => CapabilityDomain::Audio,
        "service" => CapabilityDomain::Service,
        "security" => CapabilityDomain::Security,
        _ => CapabilityDomain::Service,
    }
}

fn local_capabilities(
    platform: CapabilityPlatform,
    probe_mode: CapabilityProbeMode,
) -> Vec<CapabilityItem> {
    let mut items = Vec::new();

    add_capture_capabilities(&mut items, &platform);
    add_capture_source_capabilities(&mut items, &platform);
    add_encode_capabilities(&mut items, &platform, probe_mode);
    add_decode_capabilities(&mut items, &platform, probe_mode);
    add_render_capabilities(&mut items, &platform, probe_mode);
    add_memory_capabilities(&mut items, &platform, probe_mode);
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

fn add_encode_capabilities(
    items: &mut Vec<CapabilityItem>,
    platform: &CapabilityPlatform,
    probe_mode: CapabilityProbeMode,
) {
    push_degraded(
        items,
        platform,
        CapabilityDomain::Encode,
        "encode.openh264",
        "OpenH264",
        "Software encoder fallback; usable but below hardware path parity.",
    );

    let (h264_status, h264_reason) = match probe_mode {
        CapabilityProbeMode::Runtime => probe_nvenc_h264_status(platform),
        CapabilityProbeMode::Static => static_nvenc_status(platform, "NVENC H.264"),
    };
    push_item(
        items,
        platform,
        CapabilityDomain::Encode,
        "encode.nvenc_h264",
        "NVENC H.264",
        h264_status,
        Some(h264_reason.as_str()),
    );

    let (hevc_status, hevc_reason) = match probe_mode {
        CapabilityProbeMode::Runtime => probe_nvenc_hevc_status(platform),
        CapabilityProbeMode::Static => static_nvenc_status(platform, "NVENC HEVC"),
    };
    push_item(
        items,
        platform,
        CapabilityDomain::Encode,
        "encode.nvenc_hevc",
        "NVENC HEVC",
        hevc_status,
        Some(hevc_reason.as_str()),
    );

    let (hevc_main10_status, hevc_main10_reason) = match probe_mode {
        CapabilityProbeMode::Runtime => probe_nvenc_hevc_main10_status(platform),
        CapabilityProbeMode::Static => static_nvenc_status(platform, "NVENC HEVC Main10"),
    };
    push_item(
        items,
        platform,
        CapabilityDomain::Encode,
        "encode.nvenc_hevc_main10",
        "NVENC HEVC Main10",
        hevc_main10_status,
        Some(hevc_main10_reason.as_str()),
    );

    let (av1_status, av1_reason) = nvenc_av1_status(platform);
    push_item(
        items,
        platform,
        CapabilityDomain::Encode,
        "encode.nvenc_av1",
        "NVENC AV1",
        av1_status,
        Some(av1_reason.as_str()),
    );

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

fn add_decode_capabilities(
    items: &mut Vec<CapabilityItem>,
    platform: &CapabilityPlatform,
    probe_mode: CapabilityProbeMode,
) {
    push_degraded(
        items,
        platform,
        CapabilityDomain::Decode,
        "decode.software",
        "Software H.264 decode",
        "Software decoder fallback; usable but below hardware path parity.",
    );

    let nvdec_status = if matches!(platform, CapabilityPlatform::Windows) {
        let (status, reason) = match probe_mode {
            CapabilityProbeMode::Runtime => probe_nvdec_h264_status(platform),
            CapabilityProbeMode::Static => static_windows_runtime_status("NVDEC H.264"),
        };
        push_item(
            items,
            platform,
            CapabilityDomain::Decode,
            "decode.nvdec",
            "NVDEC",
            status,
            Some(reason.as_str()),
        );
        None
    } else {
        Some((
            CapabilityStatus::Unimplemented,
            "NVDEC runtime probing is only wired for Windows in service-owned capability snapshots.",
        ))
    };
    if let Some((status, reason)) = nvdec_status {
        push_item(
            items,
            platform,
            CapabilityDomain::Decode,
            "decode.nvdec",
            "NVDEC",
            status,
            Some(reason),
        );
    }

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

fn add_render_capabilities(
    items: &mut Vec<CapabilityItem>,
    platform: &CapabilityPlatform,
    probe_mode: CapabilityProbeMode,
) {
    match platform {
        CapabilityPlatform::Windows => {
            let (d3d11_status, d3d11_reason) = match probe_mode {
                CapabilityProbeMode::Runtime => probe_d3d11_render_status(platform),
                CapabilityProbeMode::Static => static_windows_runtime_status("D3D11 renderer"),
            };
            push_item(
                items,
                platform,
                CapabilityDomain::Render,
                "render.d3d11",
                "D3D11",
                d3d11_status,
                Some(d3d11_reason.as_str()),
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

fn add_memory_capabilities(
    items: &mut Vec<CapabilityItem>,
    platform: &CapabilityPlatform,
    probe_mode: CapabilityProbeMode,
) {
    push_available(
        items,
        platform,
        CapabilityDomain::Memory,
        "memory.cpu",
        "CPU memory",
    );
    let (status, reason) = if matches!(platform, CapabilityPlatform::Windows) {
        let (status, reason) = match probe_mode {
            CapabilityProbeMode::Runtime => probe_d3d11_render_status(platform),
            CapabilityProbeMode::Static => static_windows_runtime_status("D3D11 shared texture"),
        };
        (
            status,
            Some(format!(
                "D3D11 shared texture follows D3D11 runtime probe: {reason}"
            )),
        )
    } else {
        (
            CapabilityStatus::Unimplemented,
            Some("D3D11 shared texture interop is Windows-only.".to_string()),
        )
    };
    push_item(
        items,
        platform,
        CapabilityDomain::Memory,
        "memory.d3d11_shared",
        "D3D11 shared texture",
        status,
        reason.as_deref(),
    );
}

fn static_nvenc_status(platform: &CapabilityPlatform, label: &str) -> (CapabilityStatus, String) {
    if matches!(
        platform,
        CapabilityPlatform::Windows | CapabilityPlatform::Linux
    ) {
        (
            CapabilityStatus::Supported,
            format!("{label} is platform-declared; runtime probe refresh is pending."),
        )
    } else {
        unsupported_nvenc_status(label)
    }
}

fn static_windows_runtime_status(label: &str) -> (CapabilityStatus, String) {
    (
        CapabilityStatus::Supported,
        format!("{label} is platform-declared on Windows; runtime probe refresh is pending."),
    )
}

fn probe_nvenc_h264_status(platform: &CapabilityPlatform) -> (CapabilityStatus, String) {
    if !matches!(
        platform,
        CapabilityPlatform::Windows | CapabilityPlatform::Linux
    ) {
        return unsupported_nvenc_status("NVENC H.264");
    }

    #[cfg(any(windows, target_os = "linux"))]
    {
        static RESULT: OnceLock<(CapabilityStatus, String)> = OnceLock::new();
        RESULT
            .get_or_init(|| {
                classify_runtime_probe(
                    "NVENC H.264",
                    mrd_encode_nvenc::NvencH264Encoder::probe_h264_available()
                        .map_err(|error| error.to_string()),
                )
            })
            .clone()
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        unsupported_nvenc_status("NVENC H.264")
    }
}

fn probe_nvenc_hevc_status(platform: &CapabilityPlatform) -> (CapabilityStatus, String) {
    if !matches!(
        platform,
        CapabilityPlatform::Windows | CapabilityPlatform::Linux
    ) {
        return unsupported_nvenc_status("NVENC HEVC");
    }

    #[cfg(any(windows, target_os = "linux"))]
    {
        static RESULT: OnceLock<(CapabilityStatus, String)> = OnceLock::new();
        RESULT
            .get_or_init(|| {
                classify_runtime_probe(
                    "NVENC HEVC",
                    mrd_encode_nvenc::NvencHevcEncoder::probe_hevc_available()
                        .map_err(|error| error.to_string()),
                )
            })
            .clone()
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        unsupported_nvenc_status("NVENC HEVC")
    }
}

fn probe_nvenc_hevc_main10_status(platform: &CapabilityPlatform) -> (CapabilityStatus, String) {
    if !matches!(
        platform,
        CapabilityPlatform::Windows | CapabilityPlatform::Linux
    ) {
        return unsupported_nvenc_status("NVENC HEVC Main10");
    }

    #[cfg(any(windows, target_os = "linux"))]
    {
        static RESULT: OnceLock<(CapabilityStatus, String)> = OnceLock::new();
        RESULT
            .get_or_init(|| {
                classify_runtime_probe(
                    "NVENC HEVC Main10",
                    mrd_encode_nvenc::NvencHevcEncoder::probe_hevc_main10_available()
                        .map_err(|error| error.to_string()),
                )
            })
            .clone()
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        unsupported_nvenc_status("NVENC HEVC Main10")
    }
}

fn nvenc_av1_status(platform: &CapabilityPlatform) -> (CapabilityStatus, String) {
    if matches!(
        platform,
        CapabilityPlatform::Windows | CapabilityPlatform::Linux
    ) {
        (
            CapabilityStatus::Supported,
            "NVENC AV1 is declared as a harness capability; service-owned runtime probe and LAN sender integration are not wired yet.".to_string(),
        )
    } else {
        unsupported_nvenc_status("NVENC AV1")
    }
}

fn unsupported_nvenc_status(label: &str) -> (CapabilityStatus, String) {
    (
        CapabilityStatus::Unsupported,
        format!("{label} is not supported on this platform in the current product mode."),
    )
}

fn probe_nvdec_h264_status(platform: &CapabilityPlatform) -> (CapabilityStatus, String) {
    if !matches!(platform, CapabilityPlatform::Windows) {
        return (
            CapabilityStatus::Unimplemented,
            "NVDEC runtime probing is only wired for Windows in service-owned capability snapshots."
                .to_string(),
        );
    }

    #[cfg(windows)]
    {
        static RESULT: OnceLock<(CapabilityStatus, String)> = OnceLock::new();
        RESULT
            .get_or_init(|| {
                classify_runtime_probe(
                    "NVDEC H.264",
                    mrd_decode_nvdec::probe_h264_available().map_err(|error| error.to_string()),
                )
            })
            .clone()
    }

    #[cfg(not(windows))]
    {
        (
            CapabilityStatus::Unimplemented,
            "NVDEC runtime probing is only compiled on Windows.".to_string(),
        )
    }
}

fn probe_d3d11_render_status(platform: &CapabilityPlatform) -> (CapabilityStatus, String) {
    if !matches!(platform, CapabilityPlatform::Windows) {
        return (
            CapabilityStatus::Unimplemented,
            "D3D11 rendering is Windows-only.".to_string(),
        );
    }

    #[cfg(windows)]
    {
        static RESULT: OnceLock<(CapabilityStatus, String)> = OnceLock::new();
        RESULT
            .get_or_init(|| {
                use mrd_render::RendererFactory as _;
                classify_runtime_probe(
                    "D3D11 renderer",
                    mrd_render_d3d11::D3d11RendererFactory
                        .create()
                        .map(|_| ())
                        .map_err(|error| error.to_string()),
                )
            })
            .clone()
    }

    #[cfg(not(windows))]
    {
        (
            CapabilityStatus::Unimplemented,
            "D3D11 rendering is only compiled on Windows.".to_string(),
        )
    }
}

fn classify_runtime_probe(label: &str, result: Result<(), String>) -> (CapabilityStatus, String) {
    match result {
        Ok(()) => (
            CapabilityStatus::Available,
            format!("{label} runtime probe succeeded."),
        ),
        Err(error) => (
            CapabilityStatus::DriverMissing,
            format!("{label} runtime probe failed: {error}"),
        ),
    }
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
            "lan.1600p165",
            2560,
            1600,
            165,
            80,
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
            .profiles
            .iter()
            .any(|profile| profile.id == "lan.1600p165"));
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
