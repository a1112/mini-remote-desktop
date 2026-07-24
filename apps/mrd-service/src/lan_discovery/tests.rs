use super::*;
#[cfg(target_os = "macos")]
use std::thread;

#[derive(Clone)]
struct SharedRecordingInputInjector {
    events: std::sync::Arc<std::sync::Mutex<Vec<mrd_input::InputEvent>>>,
}

impl SharedRecordingInputInjector {
    fn new(events: std::sync::Arc<std::sync::Mutex<Vec<mrd_input::InputEvent>>>) -> Self {
        Self { events }
    }
}

impl mrd_input::InputInjector for SharedRecordingInputInjector {
    fn is_available(&self) -> bool {
        true
    }

    fn inject(&mut self, event: &mrd_input::InputEvent) -> Result<(), mrd_input::InputError> {
        self.events.lock().expect("record input event").push(*event);
        Ok(())
    }
}

#[test]
fn lan_discovery_control_input_ack_serializes_stable_tagged_protocol() {
    let packet = LanDiscoveryPacket::ControlInputAck {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: "instance-1".to_string(),
        session_id: "session-1".to_string(),
        event_id: 42,
        accepted: true,
        message: None,
        lane: Some(mrd_ipc::ControlInputLane::Reliable),
        event_count: 1,
        timestamp_ms: 1234,
    };

    let value = serde_json::to_value(packet).expect("serialize control input ack");

    assert_eq!(value["type"], "control_input_ack");
    assert_eq!(value["session_id"], "session-1");
    assert_eq!(value["event_id"], 42);
    assert_eq!(value["accepted"], true);
    assert_eq!(value["lane"], "reliable");
    assert_eq!(value["event_count"], 1);
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct WindowsTestVirtualScreen {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

#[cfg(windows)]
struct CursorRestoreGuard {
    position: (i32, i32),
}

#[cfg(windows)]
impl CursorRestoreGuard {
    fn new(position: (i32, i32)) -> Self {
        Self { position }
    }
}

#[cfg(windows)]
impl Drop for CursorRestoreGuard {
    fn drop(&mut self) {
        let _ = force_cursor_position(self.position);
    }
}

#[cfg(windows)]
static KEYBOARD_SMOKE_EVENTS: OnceLock<StdMutex<Vec<KeyboardSmokeEvent>>> = OnceLock::new();

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyboardSmokeEvent {
    KeyDown(u16),
    KeyUp(u16),
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyboardSmokeResult {
    key_down: bool,
    key_up: bool,
}

#[cfg(windows)]
struct KeyboardSmokeWindow {
    hwnd: windows::Win32::Foundation::HWND,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyboardSmokeFocusSnapshot {
    hwnd: isize,
    foreground: isize,
    focus: isize,
}

#[cfg(windows)]
impl KeyboardSmokeWindow {
    fn create() -> windows::core::Result<Self> {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::HINSTANCE;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, RegisterClassW, ShowWindow, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
            SW_SHOW, WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
        };

        keyboard_smoke_events()
            .lock()
            .expect("clear keyboard smoke events")
            .clear();

        let class_name = wide_null(&format!(
            "MrdServiceKeyboardSmoke{}{}",
            std::process::id(),
            now_ms()
        ));
        let title = wide_null("MRD service LAN input keyboard smoke");
        unsafe {
            let hmodule = GetModuleHandleW(None)?;
            let hinstance = HINSTANCE(hmodule.0);
            let window_class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(keyboard_smoke_wnd_proc),
                hInstance: hinstance,
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            if RegisterClassW(&window_class) == 0 {
                return Err(windows::core::Error::from_thread());
            }

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                360,
                180,
                None,
                None,
                Some(hinstance),
                None,
            )?;
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
            pump_keyboard_smoke_window_messages();

            Ok(Self { hwnd })
        }
    }

    fn focus(&mut self) {
        use windows::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
        use windows::Win32::UI::WindowsAndMessaging::{BringWindowToTop, SetForegroundWindow};

        unsafe {
            let _ = BringWindowToTop(self.hwnd);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetActiveWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
        }
        let deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < deadline {
            pump_keyboard_smoke_window_messages();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    async fn wait_for_key_events(
        &mut self,
        virtual_key: u16,
        timeout: Duration,
    ) -> windows::core::Result<KeyboardSmokeResult> {
        let deadline = Instant::now() + timeout;
        loop {
            pump_keyboard_smoke_window_messages();
            let result = keyboard_smoke_result(virtual_key);
            if result.key_down && result.key_up {
                return Ok(result);
            }
            if Instant::now() >= deadline {
                return Ok(result);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn focus_snapshot(&self) -> KeyboardSmokeFocusSnapshot {
        unsafe {
            KeyboardSmokeFocusSnapshot {
                hwnd: self.hwnd.0 as isize,
                foreground: windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0
                    as isize,
                focus: windows::Win32::UI::Input::KeyboardAndMouse::GetFocus().0 as isize,
            }
        }
    }
}

#[cfg(windows)]
impl Drop for KeyboardSmokeWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
        }
        pump_keyboard_smoke_window_messages();
    }
}

#[cfg(windows)]
unsafe extern "system" fn keyboard_smoke_wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_KEYDOWN, WM_KEYUP};

    match message {
        WM_KEYDOWN => {
            keyboard_smoke_events()
                .lock()
                .expect("record keyboard smoke key down")
                .push(KeyboardSmokeEvent::KeyDown(wparam.0 as u16));
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_KEYUP => {
            keyboard_smoke_events()
                .lock()
                .expect("record keyboard smoke key up")
                .push(KeyboardSmokeEvent::KeyUp(wparam.0 as u16));
            windows::Win32::Foundation::LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(windows)]
fn keyboard_smoke_events() -> &'static StdMutex<Vec<KeyboardSmokeEvent>> {
    KEYBOARD_SMOKE_EVENTS.get_or_init(|| StdMutex::new(Vec::new()))
}

#[cfg(windows)]
fn keyboard_smoke_result(virtual_key: u16) -> KeyboardSmokeResult {
    let ime_process_key = windows::Win32::UI::Input::KeyboardAndMouse::VK_PROCESSKEY.0;
    let events = keyboard_smoke_events()
        .lock()
        .expect("read keyboard smoke events");
    KeyboardSmokeResult {
        key_down: events.iter().any(|event| {
            matches!(
                *event,
                KeyboardSmokeEvent::KeyDown(key)
                    if key == virtual_key || key == ime_process_key
            )
        }),
        key_up: events.iter().any(|event| {
            matches!(
                *event,
                KeyboardSmokeEvent::KeyUp(key)
                    if key == virtual_key || key == ime_process_key
            )
        }),
    }
}

#[cfg(windows)]
fn pump_keyboard_smoke_window_messages() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn current_cursor_position() -> windows::core::Result<(i32, i32)> {
    let mut point = windows::Win32::Foundation::POINT::default();
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point)?;
    }
    Ok((point.x, point.y))
}

#[cfg(windows)]
fn current_virtual_screen() -> WindowsTestVirtualScreen {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    WindowsTestVirtualScreen {
        left: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
        top: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
        width: unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) },
        height: unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) },
    }
}

#[cfg(windows)]
fn cursor_smoke_target(
    start: (i32, i32),
    screen: WindowsTestVirtualScreen,
    delta: i32,
) -> (i32, i32) {
    let right = screen.left.saturating_add(screen.width.saturating_sub(1));
    let bottom = screen.top.saturating_add(screen.height.saturating_sub(1));
    (
        offset_inside_range(start.0, screen.left, right, delta),
        offset_inside_range(start.1, screen.top, bottom, delta),
    )
}

#[cfg(windows)]
fn offset_inside_range(value: i32, min: i32, max: i32, delta: i32) -> i32 {
    if value.saturating_add(delta) <= max {
        value.saturating_add(delta)
    } else {
        value.saturating_sub(delta).max(min)
    }
}

#[cfg(windows)]
async fn wait_for_cursor_near(
    expected: (i32, i32),
    tolerance: i32,
    timeout: Duration,
) -> windows::core::Result<Option<(i32, i32)>> {
    let deadline = Instant::now() + timeout;
    loop {
        let current = current_cursor_position()?;
        if cursor_distance(current, expected) <= tolerance {
            return Ok(Some(current));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(windows)]
fn cursor_distance(left: (i32, i32), right: (i32, i32)) -> i32 {
    left.0.abs_diff(right.0).max(left.1.abs_diff(right.1)) as i32
}

#[cfg(windows)]
fn force_cursor_position(position: (i32, i32)) -> windows::core::Result<()> {
    unsafe { windows::Win32::UI::WindowsAndMessaging::SetCursorPos(position.0, position.1) }
}

#[test]
fn dynamic_window_fps_enters_active_tier_on_changed_frame() {
    let mut policy = DynamicWindowFpsPolicy::new(120);
    let decision = policy.update(DynamicWindowFpsInput {
        frame_changed: true,
        input_active: false,
        source_available: true,
        active_window_capture_count: 1,
    });
    assert_eq!(decision.tier, DynamicWindowFpsTier::Active);
    assert_eq!(decision.target_fps, 120);
}

#[test]
fn successful_window_capture_frame_is_dynamic_fps_activity() {
    let input = window_dynamic_fps_input_for_captured_frame(3);

    assert!(input.frame_changed);
    assert!(input.source_available);
    assert_eq!(input.active_window_capture_count, 3);
}

#[test]
fn winrt_no_frame_timeout_is_dynamic_fps_idle_not_source_loss() {
    let error = anyhow::anyhow!(
        "failed to capture LAN desktop frame: WinRT capture produced no frame within 1000 ms"
    );

    assert!(is_winrt_window_capture_no_frame_timeout(&error));
    let input = window_dynamic_fps_input_for_capture_error(&error, 2);
    assert!(!input.frame_changed);
    assert!(input.source_available);
    assert_eq!(input.active_window_capture_count, 2);
}

#[test]
fn non_timeout_window_capture_error_is_dynamic_fps_source_loss() {
    let error = anyhow::anyhow!("failed to capture LAN desktop frame: access denied");

    assert!(!is_winrt_window_capture_no_frame_timeout(&error));
    let input = window_dynamic_fps_input_for_capture_error(&error, 2);
    assert!(!input.frame_changed);
    assert!(!input.source_available);
    assert_eq!(input.active_window_capture_count, 2);
}

#[test]
fn invalid_window_source_error_is_source_loss_not_display_fallback() {
    let error = window_capture_source_error("windows:window:0x0", "window hwnd must not be zero");

    assert_eq!(error.code, "WINDOW_CAPTURE_SOURCE_NOT_FOUND");
    assert!(error.message.contains("windows:window:0x0"));
    assert!(!error.message.contains("display"));
}

#[test]
fn dynamic_window_fps_caps_idle_window() {
    let mut policy = DynamicWindowFpsPolicy::new(120);
    for _ in 0..10 {
        policy.update(DynamicWindowFpsInput {
            frame_changed: false,
            input_active: false,
            source_available: true,
            active_window_capture_count: 1,
        });
    }
    let decision = policy.current();
    assert_eq!(decision.tier, DynamicWindowFpsTier::Idle);
    assert_eq!(decision.target_fps, 15);
}

#[test]
fn dynamic_window_fps_reduces_background_fps_under_multi_window_pressure() {
    let mut policy = DynamicWindowFpsPolicy::new(144);
    let decision = policy.update(DynamicWindowFpsInput {
        frame_changed: true,
        input_active: false,
        source_available: true,
        active_window_capture_count: 3,
    });
    assert_eq!(decision.tier, DynamicWindowFpsTier::Active);
    assert_eq!(decision.target_fps, 60);
}

#[test]
fn dynamic_window_fps_suspended_keeps_nonzero_heartbeat_target() {
    let mut policy = DynamicWindowFpsPolicy::new(120);
    let decision = policy.update(DynamicWindowFpsInput {
        frame_changed: false,
        input_active: false,
        source_available: false,
        active_window_capture_count: 1,
    });

    assert_eq!(decision.tier, DynamicWindowFpsTier::Suspended);
    assert_eq!(decision.target_fps, 1);
}

#[test]
fn dynamic_window_fps_recovers_from_suspended_to_active_on_changed_frame() {
    let mut policy = DynamicWindowFpsPolicy::new(120);
    let suspended = policy.update(DynamicWindowFpsInput {
        frame_changed: false,
        input_active: false,
        source_available: false,
        active_window_capture_count: 1,
    });
    assert_eq!(suspended.tier, DynamicWindowFpsTier::Suspended);

    let decision = policy.update(DynamicWindowFpsInput {
        frame_changed: true,
        input_active: false,
        source_available: true,
        active_window_capture_count: 1,
    });

    assert_eq!(decision.tier, DynamicWindowFpsTier::Active);
    assert_eq!(decision.target_fps, 120);
}

#[test]
fn dynamic_window_fps_recovers_from_idle_to_active_on_input() {
    let mut policy = DynamicWindowFpsPolicy::new(120);
    for _ in 0..10 {
        policy.update(DynamicWindowFpsInput {
            frame_changed: false,
            input_active: false,
            source_available: true,
            active_window_capture_count: 1,
        });
    }
    assert_eq!(policy.current().tier, DynamicWindowFpsTier::Idle);

    let decision = policy.update(DynamicWindowFpsInput {
        frame_changed: false,
        input_active: true,
        source_available: true,
        active_window_capture_count: 1,
    });

    assert_eq!(decision.tier, DynamicWindowFpsTier::Active);
    assert_eq!(decision.target_fps, 120);
}

#[test]
fn dynamic_window_fps_config_changes_when_profile_fps_changes() {
    let source_id = "window:1234";
    let profile_60 = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        ..MediaProfile::default()
    };
    let profile_120 = MediaProfile {
        fps: 120,
        ..profile_60.clone()
    };

    assert_eq!(
        lan_capture_config_key(source_id, &profile_60),
        lan_capture_config_key(source_id, &profile_120)
    );
    assert_ne!(
        dynamic_window_fps_config_key(source_id, &profile_60),
        dynamic_window_fps_config_key(source_id, &profile_120)
    );
}

#[test]
fn media_frame_interval_uses_dynamic_window_target_when_present() {
    let profile = MediaProfile {
        fps: 144,
        ..MediaProfile::default()
    };
    let decision = DynamicWindowFpsDecision {
        tier: DynamicWindowFpsTier::Idle,
        target_fps: 12,
    };

    assert_eq!(
        media_frame_interval_for_dynamic_decision(&profile, Some(decision)),
        Duration::from_micros(83_333)
    );
}

#[test]
fn dynamic_window_fps_interval_falls_back_to_profile_target_when_decision_absent() {
    let profile = MediaProfile {
        fps: 25,
        ..MediaProfile::default()
    };

    assert_eq!(
        media_frame_interval_for_dynamic_decision(&profile, None),
        Duration::from_micros(40_000)
    );
}

#[test]
fn dynamic_window_fps_interval_clamps_zero_target_to_one_fps() {
    let profile = MediaProfile {
        fps: 144,
        ..MediaProfile::default()
    };
    let decision = DynamicWindowFpsDecision {
        tier: DynamicWindowFpsTier::Suspended,
        target_fps: 0,
    };

    assert_eq!(
        media_frame_interval_for_dynamic_decision(&profile, Some(decision)),
        Duration::from_secs(1)
    );
}

#[test]
fn lan_discovery_config_reads_env_port_and_probe_endpoints() {
    let config = LanDiscoveryConfig::from_env_lookup(|key| match key {
        "MRD_LAN_DISCOVERY_PORT" => Some("21216".to_string()),
        "MRD_LAN_DISCOVERY_PROBE_ENDPOINTS" => Some("127.0.0.1:21217, 127.0.0.1:21218".to_string()),
        _ => None,
    })
    .expect("env config");

    assert_eq!(config.discovery_port, 21216);
    assert_eq!(
        config.probe_endpoints,
        vec![
            "127.0.0.1:21217".parse::<SocketAddr>().unwrap(),
            "127.0.0.1:21218".parse::<SocketAddr>().unwrap(),
        ]
    );
}

#[test]
fn lan_protocol_module_exposes_stable_wire_versions_and_transports() {
    assert_eq!(super::protocol::PROTOCOL_VERSION, 1);
    assert_eq!(super::protocol::LAN_MEDIA_PROTOCOL_VERSION, 3);
    assert!(
        super::protocol::DISCOVERY_PACKET_BUFFER_BYTES
            > super::protocol::DISCOVERY_SAFE_UDP_PAYLOAD_BYTES
    );
    assert_eq!(super::protocol::LAN_QUIC_MEDIA_TRANSPORT, "quic_datagram");
    assert_eq!(
        super::protocol::LAN_QUIC_RELIABLE_MEDIA_TRANSPORT,
        "quic_stream_media_v2"
    );
    assert_eq!(
        super::protocol::LAN_INPUT_CONTROL_TRANSPORT,
        "input_control_v1"
    );
    assert_eq!(
        super::protocol::LAN_REMOTE_POWER_CONTROL_TRANSPORT,
        "remote_power_control_v1"
    );
}

#[test]
fn lan_media_test_impairment_is_disabled_by_default() {
    let config = LanMediaTestImpairment::from_env_lookup(|_| None).expect("default config");
    assert!(!config.enabled());
    assert_eq!(config.effective_datagram_size(1200), 1200);
}

#[test]
fn lan_media_test_impairment_uses_seeded_loss_decisions() {
    let mut impairment = LanMediaTestImpairment::from_env_lookup(|key| match key {
        "MRD_LAN_TEST_IMPAIRMENT_LOSS_PCT" => Some("100".to_string()),
        "MRD_LAN_TEST_IMPAIRMENT_BASE_DELAY_MS" => Some("2".to_string()),
        "MRD_LAN_TEST_IMPAIRMENT_JITTER_MS" => Some("3".to_string()),
        "MRD_LAN_TEST_IMPAIRMENT_MTU_BYTES" => Some("900".to_string()),
        "MRD_LAN_TEST_IMPAIRMENT_SEED" => Some("42".to_string()),
        _ => None,
    })
    .expect("impairment config");

    assert!(impairment.enabled());
    assert_eq!(impairment.effective_datagram_size(1200), 900);
    let decision = impairment.next_datagram_decision();
    assert!(decision.drop_datagram);
    assert!(decision.delay >= Duration::from_millis(2));
    assert!(decision.delay <= Duration::from_millis(5));
}

#[test]
fn lan_instance_ids_are_unique_within_same_process_millisecond() {
    let ids = (0..8).map(|_| new_instance_id()).collect::<Vec<_>>();
    let unique_ids = ids.iter().collect::<std::collections::HashSet<_>>();

    assert_eq!(unique_ids.len(), ids.len());
}

#[tokio::test]
async fn snapshot_exposes_recent_peer() {
    let state = LanDiscoveryState::default();
    state
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "remote-instance".to_string(),
                device_id: "remote-device".to_string(),
                device_name: "Remote Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: 1,
                discovery_port: 21116,
                transports: vec!["webrtc".to_string()],
                service_build_id: None,
                media_protocol_version: None,
                media_capabilities: Vec::new(),
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            "192.168.1.50:21116".parse().unwrap(),
        )
        .await;

    let snapshot = state.snapshot().await;
    assert_eq!(snapshot.peers.len(), 1);
    assert_eq!(snapshot.peers[0].device_id.0, "remote-device");
    assert_eq!(snapshot.peers[0].p2p_control_addr, "192.168.1.50:21116");
    assert!(snapshot.peers[0].p2p_available);
    assert_eq!(snapshot.peers[0].media_capabilities, Vec::<String>::new());
}

#[tokio::test]
async fn snapshot_exposes_lan_media_v3_peer_capabilities_with_v2_rollout_compatibility() {
    let state = LanDiscoveryState::default();
    state
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "remote-instance".to_string(),
                device_id: "remote-device".to_string(),
                device_name: "Remote Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: PROTOCOL_VERSION,
                discovery_port: 21116,
                transports: vec![
                    "quic".to_string(),
                    LAN_QUIC_MEDIA_TRANSPORT.to_string(),
                    LAN_QUIC_MEDIA_V2_TRANSPORT.to_string(),
                ],
                service_build_id: Some("build-a".to_string()),
                media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                media_capabilities: lan_media_capabilities(),
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            "192.168.1.50:21116".parse().unwrap(),
        )
        .await;

    let peer = state.snapshot().await.peers.pop().expect("peer");

    assert_eq!(peer.service_build_id.as_deref(), Some("build-a"));
    assert_eq!(peer.media_protocol_version, Some(3));
    #[cfg(windows)]
    for capability in [
        LAN_CAPTURE_DXGI_CAPABILITY,
        LAN_ENCODE_NVENC_H264_CAPABILITY,
        LAN_ENCODE_NVENC_HEVC_CAPABILITY,
        LAN_ENCODE_NVENC_HEVC_MAIN10_CAPABILITY,
        LAN_ENCODE_NVENC_AV1_CAPABILITY,
        LAN_DECODE_NVDEC_CAPABILITY,
        LAN_DECODE_NVDEC_HEVC_CAPABILITY,
        LAN_DECODE_NVDEC_HEVC_MAIN10_CAPABILITY,
        LAN_DECODE_NVDEC_AV1_CAPABILITY,
        LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY,
        LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY,
        LAN_MEDIA_AV1_MAIN_420_8BIT_CAPABILITY,
        LAN_MEDIA_COLOR_MODE_CAPABILITY,
        LAN_RENDER_D3D11_NATIVE_CAPABILITY,
        LAN_RENDER_D3D11_SHARED_NV12_CAPABILITY,
    ] {
        assert!(peer.media_capabilities.contains(&capability.to_string()));
    }
    #[cfg(target_os = "macos")]
    {
        for capability in [
            LAN_CAPTURE_MACOS_CAPABILITY,
            LAN_RENDER_MACOS_NATIVE_CAPABILITY,
        ] {
            assert!(peer.media_capabilities.contains(&capability.to_string()));
        }
        let probe = probe_macos_lan_media_capabilities();
        assert_eq!(
            peer.media_capabilities
                .contains(&LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()),
            probe.videotoolbox_h264_encoder
        );
        assert_eq!(
            peer.media_capabilities
                .contains(&LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()),
            probe.videotoolbox_hevc_encoder
        );
        assert_eq!(
            peer.media_capabilities
                .contains(&LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string()),
            probe.videotoolbox_hevc_encoder
        );
        assert_eq!(
            peer.media_capabilities
                .contains(&LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()),
            probe.videotoolbox_h264_decoder
        );
        assert_eq!(
            peer.media_capabilities
                .contains(&LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()),
            probe.videotoolbox_hevc_decoder
        );
        assert_eq!(
            peer.media_capabilities
                .contains(&LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string()),
            probe.videotoolbox_h264_decoder && probe.videotoolbox_hevc_decoder
        );
    }
    assert!(peer
        .media_capabilities
        .contains(&LAN_QUIC_RELIABLE_MEDIA_TRANSPORT.to_string()));
    assert!(peer
        .media_capabilities
        .contains(&LAN_QUIC_MEDIA_V2_TRANSPORT.to_string()));
    assert!(peer
        .media_capabilities
        .contains(&LAN_QUIC_MEDIA_V3_TRANSPORT.to_string()));
}

#[cfg(windows)]
#[tokio::test]
async fn announcement_advertises_keyboard_mouse_input_control() {
    let app_state = Arc::new(AppState::new());
    app_state.devices.lock().await.register(
        DeviceId("local-device".to_string()),
        "Local Device".to_string(),
    );

    let announcement = build_announcement(&app_state)
        .await
        .expect("registered device announcement");

    assert!(announcement
        .transports
        .contains(&LAN_INPUT_CONTROL_TRANSPORT.to_string()));
    assert!(announcement
        .transports
        .contains(&LAN_REMOTE_POWER_CONTROL_TRANSPORT.to_string()));
    assert!(announcement
        .media_capabilities
        .contains(&LAN_INPUT_CONTROL_CAPABILITY.to_string()));
}

#[cfg(windows)]
#[tokio::test]
async fn announcement_omits_keyboard_mouse_input_control_when_injector_unavailable() {
    let app_state = Arc::new(AppState::new());
    app_state.devices.lock().await.register(
        DeviceId("local-device".to_string()),
        "Local Device".to_string(),
    );
    app_state
        .replace_control_input_for_test(mrd_input::UnsupportedInputInjector::new("blocked by test"))
        .await;

    let announcement = build_announcement(&app_state)
        .await
        .expect("registered device announcement");

    assert!(!announcement
        .transports
        .contains(&LAN_INPUT_CONTROL_TRANSPORT.to_string()));
    assert!(announcement
        .transports
        .contains(&LAN_REMOTE_POWER_CONTROL_TRANSPORT.to_string()));
    assert!(!announcement
        .media_capabilities
        .contains(&LAN_INPUT_CONTROL_CAPABILITY.to_string()));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_lan_media_capabilities_follow_videotoolbox_probe() {
    let without_videotoolbox =
        macos_lan_media_capabilities_from_probe(MacosLanMediaCapabilityProbe {
            videotoolbox_h264_encoder: false,
            videotoolbox_hevc_encoder: false,
            videotoolbox_h264_decoder: false,
            videotoolbox_hevc_decoder: false,
        });
    assert!(without_videotoolbox.contains(&LAN_CAPTURE_MACOS_CAPABILITY.to_string()));
    assert!(without_videotoolbox.contains(&LAN_RENDER_MACOS_NATIVE_CAPABILITY.to_string()));
    assert!(!without_videotoolbox.contains(&LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()));
    assert!(!without_videotoolbox.contains(&LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()));
    assert!(!without_videotoolbox.contains(&LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string()));
    assert!(!without_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()));
    assert!(!without_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()));
    assert!(!without_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string()));

    let h264_decode_only = macos_lan_media_capabilities_from_probe(MacosLanMediaCapabilityProbe {
        videotoolbox_h264_encoder: false,
        videotoolbox_hevc_encoder: false,
        videotoolbox_h264_decoder: true,
        videotoolbox_hevc_decoder: false,
    });
    assert!(h264_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()));
    assert!(!h264_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()));
    assert!(!h264_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string()));

    let hevc_decode_only = macos_lan_media_capabilities_from_probe(MacosLanMediaCapabilityProbe {
        videotoolbox_h264_encoder: false,
        videotoolbox_hevc_encoder: false,
        videotoolbox_h264_decoder: false,
        videotoolbox_hevc_decoder: true,
    });
    assert!(!hevc_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()));
    assert!(hevc_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()));
    assert!(!hevc_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string()));

    let with_videotoolbox = macos_lan_media_capabilities_from_probe(MacosLanMediaCapabilityProbe {
        videotoolbox_h264_encoder: true,
        videotoolbox_hevc_encoder: true,
        videotoolbox_h264_decoder: true,
        videotoolbox_hevc_decoder: true,
    });
    assert!(with_videotoolbox.contains(&LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()));
    assert!(with_videotoolbox.contains(&LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()));
    assert!(with_videotoolbox.contains(&LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string()));
    assert!(with_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()));
    assert!(with_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()));
    assert!(with_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string()));
}

#[test]
fn service_build_id_prefers_runtime_override() {
    let build_id = service_build_id_from_lookup(|key| {
        if key == SERVICE_BUILD_ID_ENV {
            Some("peer-runtime-build".to_string())
        } else {
            None
        }
    });

    assert_eq!(build_id, "peer-runtime-build");
}

#[tokio::test]
async fn peer_control_addr_returns_discovered_endpoint() {
    let state = LanDiscoveryState::default();
    state
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "remote-instance".to_string(),
                device_id: "remote-device".to_string(),
                device_name: "Remote Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: 1,
                discovery_port: 21117,
                transports: vec!["webrtc".to_string()],
                service_build_id: None,
                media_protocol_version: None,
                media_capabilities: Vec::new(),
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            "192.168.1.50:21116".parse().unwrap(),
        )
        .await;

    let addr = state
        .peer_control_addr(&DeviceId("remote-device".to_string()))
        .await
        .expect("peer addr");

    assert_eq!(addr.to_string(), "192.168.1.50:21117");
}

#[test]
fn peer_registry_record_projects_addr_snapshot_and_capabilities() {
    let record = super::peer_registry::LanPeerRecord {
        device_id: "remote-device".to_string(),
        device_name: "Remote Device".to_string(),
        device_type: "rdesk".to_string(),
        ip: "192.168.1.50".parse().unwrap(),
        discovery_port: 21116,
        transports: vec!["quic".to_string(), "media.hevc".to_string()],
        protocol_version: 1,
        service_build_id: Some("build-a".to_string()),
        media_protocol_version: Some(3),
        media_capabilities: vec!["media.hevc".to_string()],
        mac_address: Some("AA:BB:CC:DD:EE:FF".to_string()),
        last_seen_ms: 1_000,
    };

    assert_eq!(
        record.control_addr(),
        "192.168.1.50:21116".parse::<SocketAddr>().unwrap()
    );
    assert_eq!(
        record.media_capabilities_with_transports(),
        vec!["media.hevc".to_string(), "quic".to_string()]
    );

    let info = record.to_peer_info(1_250);
    assert_eq!(info.device_id.0, "remote-device");
    assert_eq!(info.p2p_control_addr, "192.168.1.50:21116");
    assert_eq!(info.mac_address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
    assert_eq!(info.age_ms, 250);
    assert!(info.p2p_available);
}

#[test]
fn peer_registry_prunes_and_queries_records() {
    let mut registry = super::peer_registry::LanPeerRegistry::default();
    registry.upsert(super::peer_registry::LanPeerRecord {
        device_id: "fresh".to_string(),
        device_name: "Fresh Device".to_string(),
        device_type: "rdesk".to_string(),
        ip: "192.168.1.20".parse().unwrap(),
        discovery_port: 21116,
        transports: vec!["quic".to_string()],
        protocol_version: 1,
        service_build_id: None,
        media_protocol_version: Some(3),
        media_capabilities: vec!["media.hevc".to_string()],
        mac_address: None,
        last_seen_ms: 1_000,
    });
    registry.upsert(super::peer_registry::LanPeerRecord {
        device_id: "stale".to_string(),
        device_name: "Stale Device".to_string(),
        device_type: "rdesk".to_string(),
        ip: "192.168.1.21".parse().unwrap(),
        discovery_port: 21117,
        transports: vec!["webrtc".to_string()],
        protocol_version: 1,
        service_build_id: None,
        media_protocol_version: None,
        media_capabilities: Vec::new(),
        mac_address: None,
        last_seen_ms: 100,
    });

    registry.prune_stale(1_500, 600);

    assert_eq!(
        registry
            .control_addr(&DeviceId("fresh".to_string()))
            .unwrap(),
        "192.168.1.20:21116".parse::<SocketAddr>().unwrap()
    );
    assert_eq!(registry.transports(&DeviceId("stale".to_string())), None);
    assert_eq!(
        registry
            .media_capabilities(&DeviceId("fresh".to_string()))
            .unwrap(),
        vec!["media.hevc".to_string(), "quic".to_string()]
    );
    assert_eq!(registry.snapshot(1_500).len(), 1);
}

#[tokio::test]
async fn request_lan_control_input_forwards_to_peer_injector() {
    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );
    let session_id = SessionId("input-session".to_string());
    controller_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
            target_device_id: Some(DeviceId("target-device".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Connected,
            last_error: None,
            sender_active: false,
            receiver_active: true,
        },
    );

    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    target_state
        .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
        .await;
    target_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );

    let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let service_addr = service_socket.local_addr().unwrap();
    let handler_socket = service_socket.clone();
    let handler_state = target_state.clone();
    let handler = tokio::spawn(async move {
        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
        handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
            .await
            .unwrap();
    });

    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "target-instance".to_string(),
                device_id: "target-device".to_string(),
                device_name: "Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: PROTOCOL_VERSION,
                discovery_port: service_addr.port(),
                transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                service_build_id: Some(service_build_id()),
                media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            service_addr,
        )
        .await;

    let result = request_lan_control_input(
        &controller_state,
        &session_id,
        mrd_ipc::ControlInputEvent::MouseButton {
            button: mrd_ipc::ControlInputButton::Left,
            pressed: true,
        },
    )
    .await
    .expect("control input ack");

    assert_eq!(result.lane, mrd_ipc::ControlInputLane::Reliable);
    assert_eq!(result.event_count, 1);
    handler.await.unwrap();

    let snapshot = target_state
        .control_input()
        .lock()
        .await
        .snapshot(session_id.clone());
    assert_eq!(snapshot.reliable.accepted_messages, 1);
    assert_eq!(snapshot.reliable.injected_messages, 1);
    assert_eq!(snapshot.realtime.injected_messages, 0);
}

#[tokio::test]
async fn request_lan_remote_power_routes_to_discovered_peer() {
    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );

    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );

    let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let service_addr = service_socket.local_addr().unwrap();
    let handler_socket = service_socket.clone();
    let handler_state = target_state.clone();
    let handler = tokio::spawn(async move {
        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
        handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
            .await
            .unwrap();
    });

    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "target-instance".to_string(),
                device_id: "target-device".to_string(),
                device_name: "Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: PROTOCOL_VERSION,
                discovery_port: service_addr.port(),
                transports: vec![
                    "quic".to_string(),
                    LAN_REMOTE_POWER_CONTROL_TRANSPORT.to_string(),
                ],
                service_build_id: Some(service_build_id()),
                media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                media_capabilities: vec![],
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            service_addr,
        )
        .await;

    let error = request_lan_remote_device_power_action(
        &controller_state,
        &DeviceId("target-device".to_string()),
        mrd_ipc::RemoteDevicePowerAction::Restart,
    )
    .await
    .expect_err("peer should reject until executor is enabled");

    handler.await.unwrap();
    assert!(error
        .to_string()
        .contains("remote power executor is not enabled"));
}

#[tokio::test]
async fn stopping_controller_session_releases_remote_lan_control_input() {
    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );
    let session_id = SessionId("input-stop-release-session".to_string());
    controller_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
            target_device_id: Some(DeviceId("target-device".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Streaming,
            last_error: None,
            sender_active: false,
            receiver_active: true,
        },
    );

    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    target_state
        .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
        .await;
    target_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );

    let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let service_addr = service_socket.local_addr().unwrap();
    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "target-instance".to_string(),
                device_id: "target-device".to_string(),
                device_name: "Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: PROTOCOL_VERSION,
                discovery_port: service_addr.port(),
                transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                service_build_id: Some(service_build_id()),
                media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            service_addr,
        )
        .await;

    let handler_socket = service_socket.clone();
    let handler_state = target_state.clone();
    let key_down_handler = tokio::spawn(async move {
        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
        handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
            .await
            .unwrap();
    });
    request_lan_control_input(
        &controller_state,
        &session_id,
        mrd_ipc::ControlInputEvent::Key {
            key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
            pressed: true,
        },
    )
    .await
    .expect("key down control input ack");
    key_down_handler.await.unwrap();

    let handler_socket = service_socket.clone();
    let handler_state = target_state.clone();
    let stop_release_handler = tokio::spawn(async move {
        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
        handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
            .await
            .unwrap();
    });
    let response =
        crate::handlers::session::stop_session(&controller_state, session_id.clone()).await;
    assert!(matches!(
        response,
        mrd_ipc::IpcResponse::SessionStopped { .. }
    ));
    timeout(Duration::from_millis(500), stop_release_handler)
        .await
        .expect("stopping controller session should send ReleaseAll to the LAN peer")
        .unwrap();

    let snapshot = target_state
        .control_input()
        .lock()
        .await
        .snapshot(session_id.clone());
    assert_eq!(snapshot.reliable.accepted_messages, 2);
    assert_eq!(snapshot.reliable.injected_messages, 2);
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "manual smoke test: moves the local cursor through LAN control input and restores it"]
async fn lan_control_input_sendinput_smoke_moves_cursor_through_udp_handler() {
    let start = current_cursor_position().expect("read starting cursor position");
    let _restore = CursorRestoreGuard::new(start);
    let target = cursor_smoke_target(start, current_virtual_screen(), 80);
    assert_ne!(target, start, "smoke target must move the cursor");

    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );
    let session_id = SessionId("input-sendinput-smoke-session".to_string());
    controller_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
            target_device_id: Some(DeviceId("target-device".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Connected,
            last_error: None,
            sender_active: false,
            receiver_active: true,
        },
    );

    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    target_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );

    let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let service_addr = service_socket.local_addr().unwrap();
    let handler_socket = service_socket.clone();
    let handler_state = target_state.clone();
    let handler = tokio::spawn(async move {
        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
        handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
            .await
            .unwrap();
    });

    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "target-instance".to_string(),
                device_id: "target-device".to_string(),
                device_name: "Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: PROTOCOL_VERSION,
                discovery_port: service_addr.port(),
                transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                service_build_id: Some(service_build_id()),
                media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            service_addr,
        )
        .await;

    let result = request_lan_control_input(
        &controller_state,
        &session_id,
        mrd_ipc::ControlInputEvent::MouseMove {
            x: target.0,
            y: target.1,
        },
    )
    .await
    .expect("control input ack");
    handler.await.unwrap();
    let moved = wait_for_cursor_near(target, 4, Duration::from_millis(500))
        .await
        .expect("wait for LAN SendInput cursor target");
    let snapshot = target_state
        .control_input()
        .lock()
        .await
        .snapshot(session_id.clone());

    eprintln!(
        "lan sendinput smoke start={start:?} target={target:?} moved={moved:?} lane={:?} snapshot={:?}",
        result.lane,
        snapshot.realtime
    );
    assert_eq!(result.lane, mrd_ipc::ControlInputLane::Realtime);
    assert_eq!(result.event_count, 1);
    assert!(moved.is_some());
    assert_eq!(snapshot.realtime.accepted_messages, 1);
    assert_eq!(snapshot.realtime.injected_messages, 1);
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "manual smoke test: sends a key through LAN control input into a focused window"]
async fn lan_control_input_sendinput_keyboard_smoke_sends_key_through_udp_handler() {
    let mut window = KeyboardSmokeWindow::create().expect("create keyboard smoke window");
    window.focus();

    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );
    let session_id = SessionId("input-sendinput-keyboard-smoke-session".to_string());
    controller_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
            target_device_id: Some(DeviceId("target-device".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Connected,
            last_error: None,
            sender_active: false,
            receiver_active: true,
        },
    );

    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    target_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );

    let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let service_addr = service_socket.local_addr().unwrap();
    let handler_socket = service_socket.clone();
    let handler_state = target_state.clone();
    let handler = tokio::spawn(async move {
        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        for _ in 0..2 {
            let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
            handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
                .await
                .unwrap();
        }
    });

    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "target-instance".to_string(),
                device_id: "target-device".to_string(),
                device_name: "Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: PROTOCOL_VERSION,
                discovery_port: service_addr.port(),
                transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                service_build_id: Some(service_build_id()),
                media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            service_addr,
        )
        .await;

    let key_down = request_lan_control_input(
        &controller_state,
        &session_id,
        mrd_ipc::ControlInputEvent::Key {
            key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
            pressed: true,
        },
    )
    .await
    .expect("control input key-down ack");
    let key_up = request_lan_control_input(
        &controller_state,
        &session_id,
        mrd_ipc::ControlInputEvent::Key {
            key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
            pressed: false,
        },
    )
    .await
    .expect("control input key-up ack");
    handler.await.unwrap();

    let events = window
        .wait_for_key_events(0x41, Duration::from_millis(500))
        .await
        .expect("wait for LAN SendInput key events");
    let snapshot = target_state
        .control_input()
        .lock()
        .await
        .snapshot(session_id.clone());

    eprintln!(
        "lan keyboard sendinput smoke key_down={:?} key_up={:?} focus={:?} events={:?} lane_down={:?} lane_up={:?} snapshot={:?}",
        events.key_down,
        events.key_up,
        window.focus_snapshot(),
        keyboard_smoke_events()
            .lock()
            .expect("read keyboard smoke events"),
        key_down.lane,
        key_up.lane,
        snapshot.reliable
    );
    assert_eq!(key_down.lane, mrd_ipc::ControlInputLane::Reliable);
    assert_eq!(key_up.lane, mrd_ipc::ControlInputLane::Reliable);
    assert_eq!(key_down.event_count, 1);
    assert_eq!(key_up.event_count, 1);
    assert!(events.key_down);
    assert!(events.key_up);
    assert_eq!(snapshot.reliable.accepted_messages, 2);
    assert_eq!(snapshot.reliable.injected_messages, 2);
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "manual smoke test: sends a key through service IPC, LAN control input, and SendInput"]
async fn ipc_control_input_keyboard_smoke_routes_to_lan_sendinput_target_window() {
    let mut window = KeyboardSmokeWindow::create().expect("create keyboard smoke window");
    window.focus();

    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );
    let controller_server = crate::ipc_server::IpcServer::new(controller_state.clone());
    let session_id = SessionId("ipc-input-sendinput-keyboard-smoke-session".to_string());
    controller_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
            target_device_id: Some(DeviceId("target-device".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Connected,
            last_error: None,
            sender_active: false,
            receiver_active: true,
        },
    );

    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    target_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );

    let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let service_addr = service_socket.local_addr().unwrap();
    let handler_socket = service_socket.clone();
    let handler_state = target_state.clone();
    let handler = tokio::spawn(async move {
        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        for _ in 0..2 {
            let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
            handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
                .await
                .unwrap();
        }
    });

    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "target-instance".to_string(),
                device_id: "target-device".to_string(),
                device_name: "Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: PROTOCOL_VERSION,
                discovery_port: service_addr.port(),
                transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                service_build_id: Some(service_build_id()),
                media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            service_addr,
        )
        .await;

    let key_down = controller_server
        .handle_request(mrd_ipc::IpcRequest::SendControlInput {
            session_id: session_id.clone(),
            event: mrd_ipc::ControlInputEvent::Key {
                key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                pressed: true,
            },
        })
        .await;
    let key_up = controller_server
        .handle_request(mrd_ipc::IpcRequest::SendControlInput {
            session_id: session_id.clone(),
            event: mrd_ipc::ControlInputEvent::Key {
                key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                pressed: false,
            },
        })
        .await;
    handler.await.unwrap();

    let events = window
        .wait_for_key_events(0x41, Duration::from_millis(500))
        .await
        .expect("wait for IPC LAN SendInput key events");
    let snapshot = target_state
        .control_input()
        .lock()
        .await
        .snapshot(session_id.clone());

    eprintln!(
        "ipc lan keyboard sendinput smoke key_down={:?} key_up={:?} focus={:?} events={:?} response_down={:?} response_up={:?} snapshot={:?}",
        events.key_down,
        events.key_up,
        window.focus_snapshot(),
        keyboard_smoke_events()
            .lock()
            .expect("read keyboard smoke events"),
        key_down,
        key_up,
        snapshot.reliable
    );
    assert_eq!(
        key_down,
        mrd_ipc::IpcResponse::ControlInputAccepted {
            session_id: session_id.clone(),
            lane: mrd_ipc::ControlInputLane::Reliable,
            event_count: 1,
        }
    );
    assert_eq!(
        key_up,
        mrd_ipc::IpcResponse::ControlInputAccepted {
            session_id: session_id.clone(),
            lane: mrd_ipc::ControlInputLane::Reliable,
            event_count: 1,
        }
    );
    assert!(events.key_down);
    assert!(events.key_up);
    assert_eq!(snapshot.reliable.accepted_messages, 2);
    assert_eq!(snapshot.reliable.injected_messages, 2);
}

#[tokio::test]
async fn accepted_lan_control_input_scales_mouse_move_to_selected_source_size() {
    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    target_state
        .replace_control_input_for_test(SharedRecordingInputInjector::new(recorded.clone()))
        .await;
    let session_id = SessionId("input-scale-session".to_string());
    target_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );
    target_state.capture_sources.lock().await.set(
        session_id.clone(),
        CaptureSourceSelection {
            session_id: session_id.clone(),
            source: CaptureSource {
                id: TEST_SYNTHETIC_CAPTURE_SOURCE_ID.to_string(),
                platform: "test".to_string(),
                source_kind: "display".to_string(),
                title: "Synthetic 2K desktop source".to_string(),
                class_name: "SyntheticCapture".to_string(),
                width: 2560,
                height: 1440,
                process_id: 0,
                app_name: Some("mrd-service test source".to_string()),
                bundle_identifier: None,
                preview_data_url: None,
                preview_width: None,
                preview_height: None,
            },
            status: "selected".to_string(),
            reason: None,
        },
    );
    let mut selected = default_media_profile();
    selected.width = 1280;
    selected.height = 720;
    target_state.media_profiles.lock().await.set(
        session_id.clone(),
        MediaProfileNegotiation {
            requested: selected.clone(),
            selected,
            status: "accepted".to_string(),
            reason: None,
            selected_source_id: Some(TEST_SYNTHETIC_CAPTURE_SOURCE_ID.to_string()),
            selected_width: Some(1280),
            selected_height: Some(720),
            downgrade_reason: None,
        },
    );

    let ack = accept_or_replay_lan_control_input(
        &target_state,
        &session_id,
        "controller-device",
        11,
        &mrd_ipc::ControlInputEvent::MouseMove { x: 640, y: 360 },
    )
    .await;

    assert!(ack.accepted);
    assert_eq!(ack.lane, Some(mrd_ipc::ControlInputLane::Realtime));
    assert_eq!(
        recorded.lock().expect("recorded input").as_slice(),
        &[mrd_input::InputEvent::MouseMove { x: 1280, y: 720 }]
    );
}

#[tokio::test]
async fn accepted_lan_control_input_requires_active_sender_on_target() {
    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    target_state
        .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
        .await;
    let session_id = SessionId("input-target-not-ready-session".to_string());
    target_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: false,
            receiver_active: false,
        },
    );

    let ack = accept_or_replay_lan_control_input(
        &target_state,
        &session_id,
        "controller-device",
        12,
        &mrd_ipc::ControlInputEvent::MouseMove { x: 10, y: 20 },
    )
    .await;

    assert!(!ack.accepted);
    assert!(ack
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("active sender"));
    assert_eq!(ack.lane, None);
    assert_eq!(ack.event_count, 0);

    let snapshot = target_state
        .control_input()
        .lock()
        .await
        .snapshot(session_id);
    assert_eq!(snapshot.realtime.accepted_messages, 0);
    assert_eq!(snapshot.reliable.accepted_messages, 0);
}

#[tokio::test]
async fn reliable_lan_control_input_retries_after_missing_ack() {
    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );
    let session_id = SessionId("input-retry-session".to_string());
    controller_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
            target_device_id: Some(DeviceId("target-device".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Connected,
            last_error: None,
            sender_active: false,
            receiver_active: true,
        },
    );

    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    target_state
        .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
        .await;
    target_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );

    let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let service_addr = service_socket.local_addr().unwrap();
    let handler_socket = service_socket.clone();
    let handler_state = target_state.clone();
    let attempts = Arc::new(AtomicU64::new(0));
    let handler_attempts = attempts.clone();
    let handler = tokio::spawn(async move {
        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (_len, _addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
        handler_attempts.fetch_add(1, Ordering::SeqCst);

        let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
        handler_attempts.fetch_add(1, Ordering::SeqCst);
        handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
            .await
            .unwrap();
    });

    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "target-instance".to_string(),
                device_id: "target-device".to_string(),
                device_name: "Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: PROTOCOL_VERSION,
                discovery_port: service_addr.port(),
                transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                service_build_id: Some(service_build_id()),
                media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            service_addr,
        )
        .await;

    let result = request_lan_control_input(
        &controller_state,
        &session_id,
        mrd_ipc::ControlInputEvent::MouseButton {
            button: mrd_ipc::ControlInputButton::Left,
            pressed: true,
        },
    )
    .await;

    if result.is_err() {
        handler.abort();
    }
    let result = result.expect("reliable control input should retry after a missing ack");
    handler.await.unwrap();

    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(result.lane, mrd_ipc::ControlInputLane::Reliable);
    assert_eq!(result.event_count, 1);
}

#[tokio::test]
async fn duplicate_reliable_lan_control_input_replays_ack_without_reinjecting() {
    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    target_state
        .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
        .await;
    let session_id = SessionId("input-dedupe-session".to_string());
    target_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );

    let event = mrd_ipc::ControlInputEvent::Key {
        key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
        pressed: true,
    };

    let first = accept_or_replay_lan_control_input(
        &target_state,
        &session_id,
        "controller-device",
        42,
        &event,
    )
    .await;
    let second = accept_or_replay_lan_control_input(
        &target_state,
        &session_id,
        "controller-device",
        42,
        &event,
    )
    .await;

    assert!(first.accepted);
    assert_eq!(second.accepted, first.accepted);
    assert_eq!(second.lane, first.lane);
    assert_eq!(second.event_count, first.event_count);
    let snapshot = target_state
        .control_input()
        .lock()
        .await
        .snapshot(session_id);
    assert_eq!(snapshot.reliable.accepted_messages, 1);
    assert_eq!(snapshot.reliable.injected_messages, 1);
}

#[tokio::test]
async fn realtime_lan_control_input_does_not_retry_without_ack() {
    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );
    let session_id = SessionId("input-realtime-session".to_string());
    controller_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
            target_device_id: Some(DeviceId("target-device".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Connected,
            last_error: None,
            sender_active: false,
            receiver_active: true,
        },
    );

    let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let service_addr = service_socket.local_addr().unwrap();
    let handler_socket = service_socket.clone();
    let attempts = Arc::new(AtomicU64::new(0));
    let handler_attempts = attempts.clone();
    let handler = tokio::spawn(async move {
        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (_len, _addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
        handler_attempts.fetch_add(1, Ordering::SeqCst);
    });

    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "target-instance".to_string(),
                device_id: "target-device".to_string(),
                device_name: "Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: PROTOCOL_VERSION,
                discovery_port: service_addr.port(),
                transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                service_build_id: Some(service_build_id()),
                media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            service_addr,
        )
        .await;

    let result = request_lan_control_input(
        &controller_state,
        &session_id,
        mrd_ipc::ControlInputEvent::MouseMove { x: 10, y: 20 },
    )
    .await;

    handler.await.unwrap();
    assert!(result.is_err());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn remote_session_request_auto_accepts_session() {
    let app_state = Arc::new(AppState::new());
    app_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );

    let service_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let ack_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let request = LanDiscoveryPacket::RemoteSessionRequest {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: "controller-instance".to_string(),
        session_id: "session-1".to_string(),
        source_device_id: "controller-device".to_string(),
        source_device_name: "Controller".to_string(),
        transport_kind: "quic".to_string(),
        source_discovery_port: Some(21116),
        source_media_capabilities: lan_media_capabilities(),
        requested_media_profile: Some(MediaProfile {
            width: 3840,
            height: 2160,
            fps: 240,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        }),
        timestamp_ms: now_ms(),
    };
    let bytes = serde_json::to_vec(&request).unwrap();

    handle_packet(
        &service_socket,
        &app_state,
        &bytes,
        ack_socket.local_addr().unwrap(),
    )
    .await
    .unwrap();

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(1), ack_socket.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len]).unwrap();
    match ack {
        LanDiscoveryPacket::RemoteSessionAck {
            session_id,
            accepted,
            media,
            media_profile,
            ..
        } => {
            assert_eq!(session_id, "session-1");
            assert!(accepted);
            let media = media.expect("QUIC media bootstrap");
            assert_eq!(media.transport_kind, "quic");
            let quic = media.quic.expect("QUIC bootstrap details");
            assert!(!quic.listen_addr.ends_with(":0"));
            assert!(!quic.server_name.is_empty());
            assert!(!quic.cert_der.is_empty());
            let negotiation = media_profile.expect("media profile negotiation");
            assert_eq!(negotiation.status, "downgraded");
            assert_eq!(negotiation.selected.width, LAN_MEDIA_TARGET_WIDTH);
            assert_eq!(negotiation.selected.height, LAN_MEDIA_TARGET_HEIGHT);
            assert_eq!(negotiation.selected.fps, 240);
            assert_eq!(
                negotiation.selected.bitrate_mbps,
                LAN_MEDIA_TARGET_BITRATE_MBPS
            );
            assert_eq!(negotiation.selected.codec, "hevc");
            assert_eq!(negotiation.selected.codec_profile.as_deref(), Some("main"));
            assert_eq!(
                negotiation.selected.chroma_subsampling.as_deref(),
                Some("4:2:0")
            );
        }
        _ => panic!("expected remote session ack"),
    }

    let sessions = app_state.sessions.lock().await;
    let snapshot = sessions
        .get(&SessionId("session-1".to_string()))
        .expect("accepted session");
    assert_eq!(
        snapshot.source_device_id,
        Some(DeviceId("controller-device".to_string()))
    );
    assert_eq!(snapshot.transport, "quic");
    assert_eq!(snapshot.lifecycle_state, SessionLifecycleState::Listening);
    assert!(snapshot.sender_active);
    assert!(snapshot.local_listen_addr.is_some());
    assert!(app_state.peer_media_capabilities.lock().await.supports(
        &SessionId("session-1".to_string()),
        LAN_QUIC_RELIABLE_MEDIA_TRANSPORT
    ));
}

#[tokio::test]
async fn remote_session_request_rejects_webrtc_until_media_path_exists() {
    let app_state = Arc::new(AppState::new());
    app_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );

    let service_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let ack_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let request = LanDiscoveryPacket::RemoteSessionRequest {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: "controller-instance".to_string(),
        session_id: "session-1".to_string(),
        source_device_id: "controller-device".to_string(),
        source_device_name: "Controller".to_string(),
        transport_kind: "webrtc".to_string(),
        source_discovery_port: None,
        source_media_capabilities: Vec::new(),
        requested_media_profile: None,
        timestamp_ms: now_ms(),
    };
    let bytes = serde_json::to_vec(&request).unwrap();

    handle_packet(
        &service_socket,
        &app_state,
        &bytes,
        ack_socket.local_addr().unwrap(),
    )
    .await
    .unwrap();

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(1), ack_socket.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len]).unwrap();
    match ack {
        LanDiscoveryPacket::RemoteSessionAck {
            accepted, message, ..
        } => {
            assert!(!accepted);
            assert!(message
                .expect("reject message")
                .contains("WebRTC media path is not implemented"));
        }
        _ => panic!("expected remote session ack"),
    }
}

#[tokio::test]
#[ignore = "TODO: fix flaky integration test - requires full media pipeline in test environment"]
async fn request_lan_remote_session_records_quic_datagram_frames() {
    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );
    tokio::time::sleep(Duration::from_millis(1)).await;

    let target_state = Arc::new(AppState::new());
    target_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );

    let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let service_addr = service_socket.local_addr().unwrap();
    let handler_socket = service_socket.clone();
    let handler_state = target_state.clone();
    let handler = tokio::spawn(async move {
        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
        handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
            .await
            .unwrap();
    });

    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "target-instance".to_string(),
                device_id: "target-device".to_string(),
                device_name: "Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: 1,
                discovery_port: service_addr.port(),
                transports: vec![
                    "quic".to_string(),
                    LAN_QUIC_MEDIA_TRANSPORT.to_string(),
                    LAN_QUIC_MEDIA_PROFILE_TRANSPORT.to_string(),
                    LAN_QUIC_MEDIA_V2_TRANSPORT.to_string(),
                    LAN_MEDIA_PROFILE_CONTROL_TRANSPORT.to_string(),
                ],
                service_build_id: Some(service_build_id()),
                media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                media_capabilities: lan_media_capabilities(),
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            service_addr,
        )
        .await;

    let session_id = SessionId("session-quic-media".to_string());
    request_lan_remote_session(
        &controller_state,
        &DeviceId("target-device".to_string()),
        &session_id,
        "quic",
        Some(MediaProfile {
            width: 640,
            height: 360,
            fps: 60,
            bitrate_mbps: 5,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        }),
    )
    .await
    .unwrap();
    handler.await.unwrap();

    let mut snapshot = controller_state.probes.lock().await.snapshot(&session_id);
    for _ in 0..40 {
        if snapshot.frames_decoded > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        snapshot = controller_state.probes.lock().await.snapshot(&session_id);
    }

    assert!(snapshot.frames_received > 0);
    assert!(snapshot.frames_decoded > 0);
    assert!(snapshot.media_probe_valid);
    assert_eq!(
        snapshot.media_probe_format.as_deref(),
        Some("h264_desktop_frame")
    );
    assert_eq!(snapshot.media_probe_width, Some(640));
    assert_eq!(snapshot.media_probe_height, Some(360));
    assert!(snapshot.last_media_sequence.unwrap_or_default() > 0);
    assert!(snapshot
        .last_media_payload_hash
        .as_deref()
        .unwrap_or_default()
        .starts_with("fnv1a64:"));
    assert_eq!(snapshot.media_probe_target_fps, Some(60));
    assert_eq!(snapshot.media_probe_target_bitrate_mbps, Some(5));
    assert!(snapshot.media_probe_payload_bytes.unwrap_or_default() > 0);
    assert!(snapshot.latest_frame_data_url.is_none());
    let session_snapshot = controller_state
        .sessions
        .lock()
        .await
        .get(&session_id)
        .cloned()
        .expect("controller session snapshot");
    assert!(
        session_snapshot.receiver_active,
        "controller should mark the LAN QUIC receiver active after connecting"
    );
    assert_eq!(
        session_snapshot.lifecycle_state,
        SessionLifecycleState::Streaming
    );
    assert!(
        controller_state
            .media_tasks
            .lock()
            .await
            .active_count(&session_id)
            > 0,
        "controller should register the LAN receiver media task"
    );

    crate::handlers::session::stop_session(&controller_state, session_id.clone()).await;
    let stopped_snapshot = controller_state.probes.lock().await.snapshot(&session_id);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after_stop_snapshot = controller_state.probes.lock().await.snapshot(&session_id);
    assert_eq!(
        after_stop_snapshot.frames_decoded, stopped_snapshot.frames_decoded,
        "stopped LAN receiver must not keep recording probe frames"
    );
}

#[tokio::test]
async fn request_lan_remote_session_rejects_legacy_quic_peer_without_media_capability() {
    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );

    let peer_addr: SocketAddr = "127.0.0.1:32216".parse().unwrap();
    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "legacy-target-instance".to_string(),
                device_id: "legacy-target-device".to_string(),
                device_name: "Legacy Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: 1,
                discovery_port: peer_addr.port(),
                transports: vec!["quic".to_string()],
                service_build_id: None,
                media_protocol_version: None,
                media_capabilities: Vec::new(),
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            peer_addr,
        )
        .await;

    let error = request_lan_remote_session(
        &controller_state,
        &DeviceId("legacy-target-device".to_string()),
        &SessionId("session-legacy-peer".to_string()),
        "quic",
        None,
    )
    .await
    .expect_err("legacy QUIC peer should fail before session request");

    assert!(error.to_string().contains("quic_datagram"));
    assert!(error.to_string().contains("Rebuild and restart"));
}

#[tokio::test]
async fn request_lan_remote_session_rejects_peer_without_2k144_media_profile() {
    let controller_state = Arc::new(AppState::new());
    controller_state.devices.lock().await.register(
        DeviceId("controller-device".to_string()),
        "Controller Device".to_string(),
    );

    let peer_addr: SocketAddr = "127.0.0.1:32217".parse().unwrap();
    controller_state
        .lan_discovery
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "stale-target-instance".to_string(),
                device_id: "stale-target-device".to_string(),
                device_name: "Stale Target Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: 1,
                discovery_port: peer_addr.port(),
                transports: vec!["quic".to_string(), LAN_QUIC_MEDIA_TRANSPORT.to_string()],
                service_build_id: None,
                media_protocol_version: None,
                media_capabilities: Vec::new(),
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            peer_addr,
        )
        .await;

    let error = request_lan_remote_session(
        &controller_state,
        &DeviceId("stale-target-device".to_string()),
        &SessionId("session-stale-peer".to_string()),
        "quic",
        None,
    )
    .await
    .expect_err("stale QUIC datagram peer should fail before session request");

    assert!(error.to_string().contains("quic_datagram_2k144"));
    assert!(error.to_string().contains("Rebuild and restart"));
}

#[tokio::test]
async fn snapshot_ignores_own_instance() {
    let state = LanDiscoveryState::default();
    state
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: state.instance_id().to_string(),
                device_id: "self-device".to_string(),
                device_name: "Self".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: 1,
                discovery_port: 21116,
                transports: vec!["webrtc".to_string()],
                service_build_id: None,
                media_protocol_version: None,
                media_capabilities: Vec::new(),
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            "127.0.0.1:21116".parse().unwrap(),
        )
        .await;

    assert!(state.snapshot().await.peers.is_empty());
}

#[tokio::test]
async fn request_probe_and_wait_returns_after_peer_update() {
    let state = Arc::new(LanDiscoveryState::default());
    let waiting_state = state.clone();
    let waiter = tokio::spawn(async move {
        waiting_state
            .request_probe_and_wait(Duration::from_secs(1))
            .await
    });

    state
        .upsert_peer(
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: "remote-instance".to_string(),
                device_id: "remote-device".to_string(),
                device_name: "Remote Device".to_string(),
                device_type: "rdesk".to_string(),
                protocol_version: 1,
                discovery_port: 21116,
                transports: vec!["webrtc".to_string(), "quic".to_string()],
                service_build_id: None,
                media_protocol_version: None,
                media_capabilities: Vec::new(),
                mac_address: None,
                timestamp_ms: now_ms(),
            },
            "192.168.1.50:21116".parse().unwrap(),
        )
        .await;

    let snapshot = waiter.await.unwrap();
    assert_eq!(snapshot.peers.len(), 1);
    assert_eq!(snapshot.peers[0].device_id.0, "remote-device");
}

#[test]
fn discovery_packet_requires_rdesk_namespace() {
    assert!(is_valid_discovery_packet(DISCOVERY_MAGIC, DISCOVERY_APP_ID));
    assert!(!is_valid_discovery_packet(DISCOVERY_MAGIC, "rsharemouse"));
}

#[test]
fn media_probe_frame_uses_hevc_compressed_profile() {
    let profile = default_media_profile();
    let frame = build_media_probe_frame(42, 123_456, &profile);
    let stats = decode_media_probe_frame(&frame).unwrap();

    assert_eq!(stats.sequence, 42);
    assert_eq!(stats.width, 2560);
    assert_eq!(stats.height, 1600);
    assert_eq!(stats.target_fps, 165);
    assert_eq!(stats.target_bitrate_mbps, 120);
    assert_eq!(stats.format, "compressed_hevc_test_pattern");
    assert!(stats.bytes_received < (2560_u64 * 1600 * 4));
    assert!(stats.payload_hash.starts_with("fnv1a64:"));
}

#[test]
fn media_profile_negotiation_clamps_to_lan_capability() {
    let negotiation = negotiate_media_profile(Some(MediaProfile {
        width: 3840,
        height: 2160,
        fps: 300,
        bitrate_mbps: 160,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    }))
    .unwrap();

    assert_eq!(negotiation.status, "downgraded");
    assert_eq!(negotiation.selected.width, 2560);
    assert_eq!(negotiation.selected.height, 1600);
    assert_eq!(negotiation.selected.fps, 249);
    assert_eq!(negotiation.selected.bitrate_mbps, 120);
    assert_eq!(negotiation.selected.codec, "hevc");
}

#[test]
fn media_profile_negotiation_preserves_supported_hevc_main_420_profile() {
    let negotiation = negotiate_media_profile(Some(MediaProfile {
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 120,
        codec: "hevc".to_string(),
        codec_profile: Some("main".to_string()),
        bit_depth: Some(8),
        chroma_subsampling: Some("4:2:0".to_string()),
        pixel_format: Some("nv12".to_string()),
        hdr_enabled: Some(false),
        ..MediaProfile::default()
    }))
    .unwrap();

    assert_eq!(negotiation.status, "accepted");
    assert_eq!(negotiation.selected.codec, "hevc");
    assert_eq!(negotiation.selected.codec_profile.as_deref(), Some("main"));
    assert_eq!(
        negotiation.selected.chroma_subsampling.as_deref(),
        Some("4:2:0")
    );
    assert_eq!(negotiation.selected.pixel_format.as_deref(), Some("nv12"));
    assert_eq!(negotiation.selected.hdr_enabled, Some(false));
}

#[test]
fn media_profile_negotiation_normalizes_h265_aliases_to_hevc() {
    for codec in ["h265", "H.265", " HEVC "] {
        let negotiation = negotiate_media_profile(Some(MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: codec.to_string(),
            ..MediaProfile::default()
        }))
        .unwrap();

        assert_eq!(negotiation.selected.codec, "hevc");
        assert_eq!(negotiation.selected.codec_profile.as_deref(), Some("main"));
        assert_eq!(
            negotiation.selected.chroma_subsampling.as_deref(),
            Some("4:2:0")
        );
        assert_eq!(negotiation.selected.pixel_format.as_deref(), Some("nv12"));
        assert_eq!(
            LanAccessUnitCodec::from_profile(&negotiation.selected),
            LanAccessUnitCodec::Hevc
        );
    }
}

#[test]
fn media_profile_negotiation_preserves_av1_profiles() {
    let negotiation = negotiate_media_profile(Some(MediaProfile {
        width: 1920,
        height: 1080,
        fps: 144,
        bitrate_mbps: 20,
        codec: "av1".to_string(),
        codec_profile: Some("main".to_string()),
        bit_depth: Some(8),
        chroma_subsampling: Some("4:2:0".to_string()),
        pixel_format: Some("nv12".to_string()),
        hdr_enabled: Some(false),
        ..MediaProfile::default()
    }))
    .unwrap();

    assert_eq!(negotiation.status, "accepted");
    assert_eq!(negotiation.selected.codec, "av1");
    assert_eq!(negotiation.selected.codec_profile.as_deref(), Some("main"));
    assert_eq!(negotiation.selected.bit_depth, Some(8));
    assert_eq!(
        LanAccessUnitCodec::from_profile(&negotiation.selected),
        LanAccessUnitCodec::Av1
    );
}

#[test]
fn media_profile_negotiation_allows_high_refresh_canary_profiles() {
    let negotiation = negotiate_media_profile(Some(MediaProfile {
        width: 1920,
        height: 1080,
        fps: 249,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    }))
    .unwrap();

    assert_eq!(negotiation.status, "accepted");
    assert_eq!(negotiation.selected.width, 1920);
    assert_eq!(negotiation.selected.height, 1080);
    assert_eq!(negotiation.selected.fps, 249);
    assert_eq!(negotiation.selected.bitrate_mbps, 20);
}

#[test]
fn requested_hevc_profile_requires_peer_hevc_media_capabilities() {
    let error = ensure_peer_supports_requested_media(
        &DeviceId("mac-target".to_string()),
        "quic",
        &test_required_lan_media_transports(),
        Some(&MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 40,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        }),
        &["videotoolbox_h264".to_string()],
    )
    .expect_err("HEVC request should require HEVC encoder and media profile caps");

    let message = error.to_string();
    assert!(message.contains("hevc encoder"));
    assert!(message.contains(LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY));
    assert!(message.contains("mac-target"));
}

#[test]
fn requested_av1_profile_requires_peer_av1_media_capabilities() {
    let error = ensure_peer_supports_requested_media(
        &DeviceId("windows-target".to_string()),
        "quic",
        &test_required_lan_media_transports(),
        Some(&MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 40,
            codec: "av1".to_string(),
            ..MediaProfile::default()
        }),
        &["encode.nvenc_hevc".to_string()],
    )
    .expect_err("AV1 request should require AV1 encoder capability");

    let message = error.to_string();
    assert!(message.contains("av1 encoder"));
    assert!(message.contains(LAN_MEDIA_AV1_MAIN_420_8BIT_CAPABILITY));
    assert!(message.contains("windows-target"));
}

#[test]
fn requested_av1_profile_accepts_peer_av1_media_capabilities() {
    ensure_peer_supports_requested_media(
        &DeviceId("windows-target".to_string()),
        "quic",
        &test_required_lan_media_transports(),
        Some(&MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 40,
            codec: "AV1".to_string(),
            ..MediaProfile::default()
        }),
        &[
            "encode.nvenc_av1".to_string(),
            LAN_MEDIA_AV1_MAIN_420_8BIT_CAPABILITY.to_string(),
        ],
    )
    .expect("AV1-capable peer should pass AV1 request preflight");
}

#[test]
fn requested_hevc_profile_accepts_macos_videotoolbox_capabilities() {
    ensure_peer_supports_requested_media(
        &DeviceId("mac-target".to_string()),
        "quic",
        &test_required_lan_media_transports(),
        Some(&MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 40,
            codec: "HEVC".to_string(),
            ..MediaProfile::default()
        }),
        &[
            "videotoolbox_hevc".to_string(),
            LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string(),
        ],
    )
    .expect("macOS VideoToolbox HEVC peer should pass HEVC request preflight");
}

#[test]
fn requested_non_full_color_profile_requires_peer_color_mode_capability() {
    let error = ensure_peer_supports_requested_media(
        &DeviceId("windows-target".to_string()),
        "quic",
        &test_required_lan_media_transports(),
        Some(&MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 12,
            codec: "h264".to_string(),
            color_mode: Some("grayscale".to_string()),
            color_pipeline: Some("sdr8".to_string()),
            ..MediaProfile::default()
        }),
        &["encode.nvenc_h264".to_string()],
    )
    .expect_err("non-full color modes require an explicit peer color transform capability");

    let message = error.to_string();
    assert!(message.contains("media.color_mode_v1"));
    assert!(message.contains("color=grayscale"));
}

#[test]
fn requested_hdr_main10_profile_requires_peer_main10_media_capabilities() {
    let error = ensure_peer_supports_requested_media(
        &DeviceId("windows-target".to_string()),
        "quic",
        &test_required_lan_media_transports(),
        Some(&MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            codec_profile: Some("main10".to_string()),
            bit_depth: Some(10),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("p010".to_string()),
            color_pipeline: Some("hdr_main10".to_string()),
            ..MediaProfile::default()
        }),
        &[
            "encode.nvenc_hevc".to_string(),
            LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string(),
        ],
    )
    .expect_err("HDR/Main10 HEVC must not be accepted as 8-bit HEVC");

    let message = error.to_string();
    assert!(message.contains("encode.nvenc_hevc_main10"));
    assert!(message.contains("media.hevc_main10_420_10bit"));
    assert!(message.contains("pipeline=hdr_main10"));
}

#[tokio::test]
async fn remote_session_accept_rejects_source_without_selected_hevc_decoder() {
    let app_state = Arc::new(AppState::new());
    app_state
        .devices
        .lock()
        .await
        .register(DeviceId("mac-target".to_string()), "Mac Target".to_string());

    let result = accept_lan_remote_session(
        &app_state,
        SessionId("session-hevc-receiver-missing".to_string()),
        DeviceId("mac-controller".to_string()),
        "quic".to_string(),
        vec!["videotoolbox_h264".to_string()],
        Some(MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 40,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        }),
    )
    .await;

    assert!(!result.accepted);
    assert!(result
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("hevc decoder"));
    assert!(app_state
        .sessions
        .lock()
        .await
        .get(&SessionId("session-hevc-receiver-missing".to_string()))
        .is_none());
}

#[tokio::test]
async fn media_profile_update_rejects_receiver_without_selected_hevc_decoder() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("session-hevc-update-receiver-missing".to_string());
    app_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("mac-controller".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );
    app_state
        .peer_media_capabilities
        .lock()
        .await
        .set(session_id.clone(), vec!["videotoolbox_h264".to_string()]);

    let error = accept_lan_media_profile_update(
        &app_state,
        &session_id,
        MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 40,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        },
    )
    .await
    .expect_err("HEVC update should require receiver HEVC decoder caps");

    assert!(error.to_string().contains("hevc decoder"));
    assert!(app_state
        .media_profiles
        .lock()
        .await
        .get(&session_id)
        .is_none());
}

#[test]
fn lan_media_reassembler_config_allows_decode_backpressure() {
    let config = lan_media_reassembler_config();

    assert!(config.frame_timeout >= Duration::from_millis(1_000));
    assert!(config.max_pending_frames >= 128);
}

#[test]
fn lan_media_frame_orderer_holds_late_frames_until_gap_arrives() {
    let mut orderer = LanMediaFrameOrderer::new(8);

    let first = orderer.push(test_quic_au_frame(1, false));
    let third = orderer.push(test_quic_au_frame(3, false));
    let ready = orderer.push(test_quic_au_frame(2, false));

    assert_eq!(frame_ids(&first), vec![1]);
    assert!(third.is_empty());
    assert_eq!(frame_ids(&ready), vec![2, 3]);
    assert!(!orderer.take_skipped_gap());
}

#[test]
fn lan_media_frame_orderer_handles_v3_media_frames() {
    let mut orderer = LanMediaFrameOrderer::<QuicMediaFrame>::new(8);

    let first = orderer.push(test_quic_media_frame(1, true));
    let third = orderer.push(test_quic_media_frame(3, false));
    let ready = orderer.push(test_quic_media_frame(2, false));

    assert_eq!(media_frame_ids(&first), vec![1]);
    assert!(third.is_empty());
    assert_eq!(media_frame_ids(&ready), vec![2, 3]);
}

#[test]
fn lan_media_frame_orderer_skips_gap_when_pending_limit_is_reached() {
    let mut orderer = LanMediaFrameOrderer::new(2);

    assert_eq!(
        frame_ids(&orderer.push(test_quic_au_frame(10, true))),
        vec![10]
    );
    assert!(orderer.push(test_quic_au_frame(12, false)).is_empty());
    let ready = orderer.push(test_quic_au_frame(13, false));

    assert_eq!(frame_ids(&ready), vec![12, 13]);
    assert!(orderer.take_skipped_gap());
    assert!(!orderer.take_skipped_gap());
}

#[test]
fn lan_media_frame_orderer_releases_first_late_frame_at_low_latency_limit() {
    let mut orderer = LanMediaFrameOrderer::new(1);

    assert_eq!(
        frame_ids(&orderer.push(test_quic_au_frame(20, true))),
        vec![20]
    );
    let ready = orderer.push(test_quic_au_frame(22, false));

    assert_eq!(frame_ids(&ready), vec![22]);
    assert!(orderer.take_skipped_gap());
}

#[test]
fn production_lan_media_frame_orderer_absorbs_short_high_refresh_reordering() {
    let mut orderer = LanMediaFrameOrderer::new(LAN_MEDIA_RECEIVER_REORDER_MAX_PENDING_FRAMES);

    assert_eq!(
        frame_ids(&orderer.push(test_quic_au_frame(100, true))),
        vec![100]
    );
    assert!(orderer.push(test_quic_au_frame(102, false)).is_empty());
    assert!(orderer.push(test_quic_au_frame(103, false)).is_empty());
    let ready = orderer.push(test_quic_au_frame(101, false));

    assert_eq!(frame_ids(&ready), vec![101, 102, 103]);
}

#[test]
fn decoder_candidate_preference_keeps_fallback_backend_first() {
    let candidates = prioritize_lan_receiver_decoder_candidates(
        vec!["nvdec", "h264_software"],
        Some("h264_software"),
    );

    assert_eq!(candidates, vec!["h264_software", "nvdec"]);
}

#[cfg(windows)]
#[test]
fn windows_receiver_decoder_defaults_to_hardware_then_ffmpeg_fallback() {
    assert_eq!(
        default_lan_receiver_decoder_candidates(LanAccessUnitCodec::H264),
        &[
            "nvdec_d3d11_shared",
            "nvdec",
            "ffmpeg_h264",
            "h264_software"
        ]
    );
    assert_eq!(
        default_lan_receiver_decoder_candidates(LanAccessUnitCodec::Hevc),
        &["nvdec_hevc_d3d11_shared", "nvdec_hevc", "ffmpeg_hevc"]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_receiver_decoder_defaults_to_videotoolbox_then_ffmpeg_fallback() {
    assert_eq!(
        default_lan_receiver_decoder_candidates(LanAccessUnitCodec::H264),
        &["videotoolbox", "ffmpeg_h264", "h264_software"]
    );
    assert_eq!(
        default_lan_receiver_decoder_candidates(LanAccessUnitCodec::Hevc),
        &["videotoolbox_hevc", "ffmpeg_hevc"]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_receiver_decoder_videotoolbox_preference_is_codec_specific() {
    assert_eq!(
        preferred_lan_receiver_decoder_candidates_from_preference(
            LanAccessUnitCodec::H264,
            "videotoolbox"
        ),
        vec!["videotoolbox", "h264_software"]
    );
    assert_eq!(
        preferred_lan_receiver_decoder_candidates_from_preference(
            LanAccessUnitCodec::Hevc,
            "videotoolbox"
        ),
        vec!["videotoolbox_hevc", "ffmpeg_hevc"]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_receiver_decoder_backends_create_videotoolbox_decoders() {
    let h264 = create_lan_video_decoder("videotoolbox").expect("create H.264 VideoToolbox decoder");
    assert_eq!(
        h264.output_memory_kind(),
        mrd_pipeline_core::FrameMemoryKind::Cpu
    );

    let hevc =
        create_lan_video_decoder("videotoolbox_hevc").expect("create HEVC VideoToolbox decoder");
    assert_eq!(
        hevc.output_memory_kind(),
        mrd_pipeline_core::FrameMemoryKind::Cpu
    );
}

fn test_quic_au_frame(frame_id: u32, is_keyframe: bool) -> QuicAuFrame {
    let payload = [frame_id as u8, u8::from(is_keyframe)];
    let datagrams =
        fragment_access_unit(frame_id, u64::from(frame_id), is_keyframe, &payload, 1200)
            .expect("fragmented frame");
    let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig::default());
    reassembler
        .push_datagram(&datagrams[0])
        .expect("reassembled frame")
        .expect("complete frame")
}

fn frame_ids(frames: &[QuicAuFrame]) -> Vec<u32> {
    frames.iter().map(|frame| frame.frame_id).collect()
}

fn test_required_lan_media_transports() -> Vec<String> {
    vec![
        LAN_QUIC_MEDIA_TRANSPORT.to_string(),
        LAN_QUIC_MEDIA_PROFILE_TRANSPORT.to_string(),
        LAN_QUIC_MEDIA_V2_TRANSPORT.to_string(),
        LAN_MEDIA_PROFILE_CONTROL_TRANSPORT.to_string(),
    ]
}

fn test_quic_media_frame(frame_id: u32, is_keyframe: bool) -> QuicMediaFrame {
    QuicMediaFrame {
        payload_type: QuicMediaPayloadType::AccessUnit,
        codec: QuicMediaCodec::H264,
        profile_id: 123,
        frame_id,
        timestamp_us: u64::from(frame_id),
        flags: if is_keyframe {
            mrd_transport_quic_quinn::QUIC_MEDIA_V3_FLAG_KEYFRAME
        } else {
            0
        },
        payload: bytes::Bytes::from_static(b"h264-au"),
    }
}

fn media_frame_ids(frames: &[QuicMediaFrame]) -> Vec<u32> {
    frames.iter().map(|frame| frame.frame_id).collect()
}

#[tokio::test]
async fn capture_source_selection_changes_active_sender_session() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("capture-source-session".to_string());
    app_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );

    let source = mrd_ipc::CaptureSource {
        id: "windows:window:0x1234".to_string(),
        platform: "windows".to_string(),
        source_kind: "window".to_string(),
        title: "Target App".to_string(),
        class_name: "ApplicationFrameWindow".to_string(),
        width: 1280,
        height: 720,
        process_id: 4242,
        app_name: Some("Target App".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    };
    let selection = accept_lan_capture_source_select_from_sources(
        &app_state,
        &session_id,
        "windows:window:0x1234",
        vec![source],
    )
    .await
    .unwrap();

    assert_eq!(selection.status, "selected");
    assert_eq!(selection.source.id, "windows:window:0x1234");
    assert_eq!(
        app_state
            .capture_sources
            .lock()
            .await
            .get(&session_id)
            .expect("selected capture source")
            .source
            .source_kind,
        "window"
    );
}

#[tokio::test]
async fn capture_source_selection_reconciles_media_profile_to_source_dimensions() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("capture-source-profile-session".to_string());
    app_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );
    app_state.media_profiles.lock().await.set(
        session_id.clone(),
        negotiate_media_profile(Some(MediaProfile {
            width: 1920,
            height: 1080,
            fps: 120,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        }))
        .unwrap(),
    );

    let source = mrd_ipc::CaptureSource {
        id: "linux:display:1".to_string(),
        platform: "linux".to_string(),
        source_kind: "display".to_string(),
        title: "Linux Display".to_string(),
        class_name: "PipeWirePortal".to_string(),
        width: 1728,
        height: 1080,
        process_id: 0,
        app_name: Some("Display".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    };

    accept_lan_capture_source_select_from_sources(
        &app_state,
        &session_id,
        "linux:display:1",
        vec![source],
    )
    .await
    .unwrap();

    let negotiation = app_state
        .media_profiles
        .lock()
        .await
        .get(&session_id)
        .expect("reconciled media profile");
    assert_eq!(
        negotiation.selected_source_id.as_deref(),
        Some("linux:display:1")
    );
    assert_eq!(negotiation.selected.width, 1728);
    assert_eq!(negotiation.selected.height, 1080);
    assert_eq!(negotiation.selected_width, Some(1728));
    assert_eq!(negotiation.selected_height, Some(1080));
    assert_eq!(negotiation.status, "downgraded");
    assert_eq!(
        negotiation.downgrade_reason.as_deref(),
        Some("matched selected capture source dimensions and aspect ratio")
    );
}

#[tokio::test]
async fn capture_source_selection_preserves_source_aspect_ratio() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("capture-source-aspect-session".to_string());
    app_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );
    app_state.media_profiles.lock().await.set(
        session_id.clone(),
        negotiate_media_profile(Some(MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        }))
        .unwrap(),
    );

    let source = mrd_ipc::CaptureSource {
        id: "windows:display:0".to_string(),
        platform: "windows".to_string(),
        source_kind: "display".to_string(),
        title: "Display 1".to_string(),
        class_name: "Monitor".to_string(),
        width: 2560,
        height: 1600,
        process_id: 0,
        app_name: Some("Display".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    };

    accept_lan_capture_source_select_from_sources(
        &app_state,
        &session_id,
        "windows:display:0",
        vec![source],
    )
    .await
    .unwrap();

    let negotiation = app_state
        .media_profiles
        .lock()
        .await
        .get(&session_id)
        .expect("reconciled media profile");
    assert_eq!(
        negotiation.selected_source_id.as_deref(),
        Some("windows:display:0")
    );
    assert_eq!(negotiation.selected.width, 1728);
    assert_eq!(negotiation.selected.height, 1080);
    assert_eq!(negotiation.selected_width, Some(1728));
    assert_eq!(negotiation.selected_height, Some(1080));
    assert_eq!(negotiation.status, "downgraded");
    assert_eq!(
        negotiation.downgrade_reason.as_deref(),
        Some("matched selected capture source dimensions and aspect ratio")
    );
}

#[tokio::test]
async fn display_mode_set_chooses_matching_mode_and_records_restore() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("display-mode-session".to_string());
    app_state
        .sessions
        .lock()
        .await
        .insert(session_id.clone(), sender_snapshot(&session_id));
    let modes = vec![
        display_mode("current", 2560, 1600, 60, true),
        display_mode("target", 1920, 1080, 144, false),
    ];

    let change = accept_lan_display_mode_set_from_modes(
        &app_state,
        &session_id,
        display_mode("requested", 1920, 1080, 144, false),
        true,
        modes,
    )
    .await
    .unwrap();

    assert_eq!(change.status, "changed");
    assert_eq!(
        change.previous.as_ref().map(|mode| mode.id.as_str()),
        Some("current")
    );
    assert_eq!(
        change.active.as_ref().map(|mode| mode.id.as_str()),
        Some("target")
    );
    assert_eq!(
        app_state
            .display_modes
            .lock()
            .await
            .restore_mode(&session_id)
            .as_ref()
            .map(|mode| mode.id.as_str()),
        Some("current")
    );
}

#[tokio::test]
async fn display_mode_set_clamps_media_profile_to_active_refresh() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("display-mode-profile-session".to_string());
    app_state
        .sessions
        .lock()
        .await
        .insert(session_id.clone(), sender_snapshot(&session_id));
    app_state.media_profiles.lock().await.set(
        session_id.clone(),
        negotiate_media_profile(Some(MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        }))
        .unwrap(),
    );
    let modes = vec![
        display_mode("current", 2560, 1440, 144, true),
        display_mode("active", 1920, 1200, 144, false),
    ];

    accept_lan_display_mode_set_from_modes(
        &app_state,
        &session_id,
        display_mode("requested", 2560, 1600, 165, false),
        true,
        modes,
    )
    .await
    .unwrap();

    let negotiation = app_state
        .media_profiles
        .lock()
        .await
        .get(&session_id)
        .expect("profile after display mode set");
    assert_eq!(negotiation.selected.width, 1920);
    assert_eq!(negotiation.selected.height, 1200);
    assert_eq!(negotiation.selected.fps, 144);
    assert_eq!(negotiation.status, "downgraded");
    assert_eq!(
        negotiation.downgrade_reason.as_deref(),
        Some("matched active display mode dimensions and refresh rate")
    );
}

#[tokio::test]
async fn remote_display_mode_ack_updates_controller_expected_profile() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("controller-display-mode-profile-session".to_string());
    app_state
        .sessions
        .lock()
        .await
        .insert(session_id.clone(), sender_snapshot(&session_id));
    app_state.media_profiles.lock().await.set(
        session_id.clone(),
        negotiate_media_profile(Some(MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        }))
        .unwrap(),
    );
    store_capture_source_selection(
        &app_state,
        &session_id,
        CaptureSourceSelection {
            session_id: session_id.clone(),
            source: mrd_ipc::CaptureSource {
                id: "windows:display-shared:0".to_string(),
                platform: "windows".to_string(),
                source_kind: "display_shared".to_string(),
                title: "Display 1".to_string(),
                class_name: "WinRTMonitorShared".to_string(),
                width: 1920,
                height: 1200,
                process_id: 0,
                app_name: Some("Display".to_string()),
                bundle_identifier: None,
                preview_data_url: None,
                preview_width: None,
                preview_height: None,
            },
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    record_remote_display_mode_change(
        &app_state,
        &session_id,
        &DisplayModeChange {
            session_id: session_id.clone(),
            requested: Some(display_mode("requested", 1920, 1200, 144, false)),
            previous: Some(display_mode("previous", 2560, 1600, 165, true)),
            active: Some(display_mode("active", 1920, 1200, 144, true)),
            status: "changed".to_string(),
            reason: None,
            restore_required: true,
        },
    )
    .await;

    let negotiation = app_state
        .media_profiles
        .lock()
        .await
        .get(&session_id)
        .expect("controller profile after display mode ack");
    assert_eq!(negotiation.selected.width, 1920);
    assert_eq!(negotiation.selected.height, 1200);
    assert_eq!(negotiation.selected.fps, 144);
    assert_eq!(
        negotiation.downgrade_reason.as_deref(),
        Some("matched active display mode dimensions and refresh rate")
    );
}

#[tokio::test]
async fn capture_source_selection_tracks_different_windows_per_session() {
    let app_state = Arc::new(AppState::default());
    let session_a = SessionId("window-a".to_string());
    let session_b = SessionId("window-b".to_string());

    store_capture_source_selection(
        &app_state,
        &session_a,
        CaptureSourceSelection {
            session_id: session_a.clone(),
            source: test_window_capture_source("windows:window:0x1111"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    store_capture_source_selection(
        &app_state,
        &session_b,
        CaptureSourceSelection {
            session_id: session_b.clone(),
            source: test_window_capture_source("windows:window:0x2222"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    assert_eq!(
        selected_capture_source_id(&app_state, &session_a)
            .await
            .unwrap(),
        "windows:window:0x1111"
    );
    assert_eq!(
        selected_capture_source_id(&app_state, &session_b)
            .await
            .unwrap(),
        "windows:window:0x2222"
    );
}

#[tokio::test]
async fn active_window_capture_count_counts_selected_window_sessions() {
    let app_state = Arc::new(AppState::default());
    let session_a = SessionId("window-a".to_string());
    let session_b = SessionId("window-b".to_string());
    let session_display = SessionId("display".to_string());

    {
        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            session_a.clone(),
            sender_snapshot_for_source(&session_a, "controller-a"),
        );
        sessions.insert(
            session_b.clone(),
            sender_snapshot_for_source(&session_b, "controller-b"),
        );
        sessions.insert(
            session_display.clone(),
            sender_snapshot_for_source(&session_display, "controller-c"),
        );
    }

    store_capture_source_selection(
        &app_state,
        &session_a,
        CaptureSourceSelection {
            session_id: session_a.clone(),
            source: test_window_capture_source("windows:window:0x1111"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;
    store_capture_source_selection(
        &app_state,
        &session_b,
        CaptureSourceSelection {
            session_id: session_b.clone(),
            source: test_window_capture_source("windows:window:0x2222"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;
    store_capture_source_selection(
        &app_state,
        &session_display,
        CaptureSourceSelection {
            session_id: session_display.clone(),
            source: test_display_capture_source("windows:display-shared:0"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    assert_eq!(active_window_capture_count(&app_state).await, 2);
}

#[tokio::test]
async fn window_sender_selection_keeps_same_source_device_sessions_active() {
    let app_state = Arc::new(AppState::default());
    let next_session = SessionId("new-window-controller-a".to_string());
    let old_display = SessionId("old-display-controller-a".to_string());
    let old_window = SessionId("old-window-controller-a".to_string());

    {
        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            next_session.clone(),
            sender_snapshot_for_source(&next_session, "controller-a"),
        );
        sessions.insert(
            old_display.clone(),
            sender_snapshot_for_source(&old_display, "controller-a"),
        );
        sessions.insert(
            old_window.clone(),
            sender_snapshot_for_source(&old_window, "controller-a"),
        );
    }

    store_capture_source_selection(
        &app_state,
        &old_display,
        CaptureSourceSelection {
            session_id: old_display.clone(),
            source: test_display_capture_source("windows:display-shared:0"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;
    store_capture_source_selection(
        &app_state,
        &old_window,
        CaptureSourceSelection {
            session_id: old_window.clone(),
            source: test_window_capture_source("windows:window:0x1111"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    close_existing_display_lan_sender_sessions_for_source(
        &app_state,
        &next_session,
        &test_window_capture_source("windows:window:0x2222"),
    )
    .await;

    let sessions = app_state.sessions.lock().await;
    assert!(sessions.get(&old_display).unwrap().sender_active);
    assert_eq!(
        sessions.get(&old_display).unwrap().lifecycle_state,
        SessionLifecycleState::Listening
    );
    assert!(sessions.get(&old_window).unwrap().sender_active);
    assert_eq!(
        sessions.get(&old_window).unwrap().lifecycle_state,
        SessionLifecycleState::Listening
    );
}

#[tokio::test]
async fn display_sender_selection_closes_existing_display_sessions_for_same_controller_or_source() {
    let app_state = Arc::new(AppState::default());
    let next_session = SessionId("new-display-controller-a".to_string());
    let old_display = SessionId("old-display-controller-a".to_string());
    let old_window = SessionId("old-window-controller-a".to_string());
    let other_controller_other_source = SessionId("display-controller-b-other".to_string());
    let other_controller_same_source = SessionId("display-controller-b-same".to_string());

    {
        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            next_session.clone(),
            sender_snapshot_for_source(&next_session, "controller-a"),
        );
        sessions.insert(
            old_display.clone(),
            sender_snapshot_for_source(&old_display, "controller-a"),
        );
        sessions.insert(
            old_window.clone(),
            sender_snapshot_for_source(&old_window, "controller-a"),
        );
        sessions.insert(
            other_controller_other_source.clone(),
            sender_snapshot_for_source(&other_controller_other_source, "controller-b"),
        );
        sessions.insert(
            other_controller_same_source.clone(),
            sender_snapshot_for_source(&other_controller_same_source, "controller-b"),
        );
    }

    store_capture_source_selection(
        &app_state,
        &old_display,
        CaptureSourceSelection {
            session_id: old_display.clone(),
            source: test_display_capture_source("windows:display-shared:0"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;
    store_capture_source_selection(
        &app_state,
        &old_window,
        CaptureSourceSelection {
            session_id: old_window.clone(),
            source: test_window_capture_source("windows:window:0x1111"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;
    store_capture_source_selection(
        &app_state,
        &other_controller_other_source,
        CaptureSourceSelection {
            session_id: other_controller_other_source.clone(),
            source: test_display_capture_source("windows:display-shared:1"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;
    store_capture_source_selection(
        &app_state,
        &other_controller_same_source,
        CaptureSourceSelection {
            session_id: other_controller_same_source.clone(),
            source: test_display_capture_source("windows:display-shared:2"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    close_existing_display_lan_sender_sessions_for_source(
        &app_state,
        &next_session,
        &test_display_capture_source("windows:display-shared:2"),
    )
    .await;

    let sessions = app_state.sessions.lock().await;
    assert_eq!(
        sessions.get(&old_display).unwrap().lifecycle_state,
        SessionLifecycleState::Closed
    );
    assert!(!sessions.get(&old_display).unwrap().sender_active);
    assert!(sessions.get(&old_window).unwrap().sender_active);
    assert_eq!(
        sessions.get(&old_window).unwrap().lifecycle_state,
        SessionLifecycleState::Listening
    );
    assert!(
        sessions
            .get(&other_controller_other_source)
            .unwrap()
            .sender_active
    );
    assert_eq!(
        sessions
            .get(&other_controller_other_source)
            .unwrap()
            .lifecycle_state,
        SessionLifecycleState::Listening
    );
    assert_eq!(
        sessions
            .get(&other_controller_same_source)
            .unwrap()
            .lifecycle_state,
        SessionLifecycleState::Closed
    );
    assert!(
        !sessions
            .get(&other_controller_same_source)
            .unwrap()
            .sender_active
    );
}

#[tokio::test]
async fn window_receiver_selection_keeps_same_target_sessions_active() {
    let app_state = Arc::new(AppState::default());
    let next_session = SessionId("new-window-target-a".to_string());
    let old_display = SessionId("old-display-target-a".to_string());
    let old_window = SessionId("old-window-target-a".to_string());

    {
        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            next_session.clone(),
            receiver_snapshot_for_target(&next_session, "target-a"),
        );
        sessions.insert(
            old_display.clone(),
            receiver_snapshot_for_target(&old_display, "target-a"),
        );
        sessions.insert(
            old_window.clone(),
            receiver_snapshot_for_target(&old_window, "target-a"),
        );
    }

    store_capture_source_selection(
        &app_state,
        &old_display,
        CaptureSourceSelection {
            session_id: old_display.clone(),
            source: test_display_capture_source("windows:display-shared:0"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;
    store_capture_source_selection(
        &app_state,
        &old_window,
        CaptureSourceSelection {
            session_id: old_window.clone(),
            source: test_window_capture_source("windows:window:0x1111"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    close_existing_display_lan_receiver_sessions_for_target(
        &app_state,
        &next_session,
        &test_window_capture_source("windows:window:0x2222"),
    )
    .await;

    let sessions = app_state.sessions.lock().await;
    assert!(sessions.get(&old_display).unwrap().receiver_active);
    assert_eq!(
        sessions.get(&old_display).unwrap().lifecycle_state,
        SessionLifecycleState::Streaming
    );
    assert!(sessions.get(&old_window).unwrap().receiver_active);
    assert_eq!(
        sessions.get(&old_window).unwrap().lifecycle_state,
        SessionLifecycleState::Streaming
    );
}

#[tokio::test]
async fn display_receiver_selection_closes_only_existing_display_sessions_for_same_target() {
    let app_state = Arc::new(AppState::default());
    let next_session = SessionId("new-display-target-a".to_string());
    let old_display = SessionId("old-display-target-a".to_string());
    let old_window = SessionId("old-window-target-a".to_string());
    let other_target = SessionId("display-target-b".to_string());

    {
        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            next_session.clone(),
            receiver_snapshot_for_target(&next_session, "target-a"),
        );
        sessions.insert(
            old_display.clone(),
            receiver_snapshot_for_target(&old_display, "target-a"),
        );
        sessions.insert(
            old_window.clone(),
            receiver_snapshot_for_target(&old_window, "target-a"),
        );
        sessions.insert(
            other_target.clone(),
            receiver_snapshot_for_target(&other_target, "target-b"),
        );
    }

    store_capture_source_selection(
        &app_state,
        &old_display,
        CaptureSourceSelection {
            session_id: old_display.clone(),
            source: test_display_capture_source("windows:display-shared:0"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;
    store_capture_source_selection(
        &app_state,
        &old_window,
        CaptureSourceSelection {
            session_id: old_window.clone(),
            source: test_window_capture_source("windows:window:0x1111"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;
    store_capture_source_selection(
        &app_state,
        &other_target,
        CaptureSourceSelection {
            session_id: other_target.clone(),
            source: test_display_capture_source("windows:display-shared:1"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    close_existing_display_lan_receiver_sessions_for_target(
        &app_state,
        &next_session,
        &test_display_capture_source("windows:display-shared:2"),
    )
    .await;

    let sessions = app_state.sessions.lock().await;
    assert_eq!(
        sessions.get(&old_display).unwrap().lifecycle_state,
        SessionLifecycleState::Closed
    );
    assert!(!sessions.get(&old_display).unwrap().receiver_active);
    assert!(sessions.get(&old_window).unwrap().receiver_active);
    assert_eq!(
        sessions.get(&old_window).unwrap().lifecycle_state,
        SessionLifecycleState::Streaming
    );
    assert!(sessions.get(&other_target).unwrap().receiver_active);
    assert_eq!(
        sessions.get(&other_target).unwrap().lifecycle_state,
        SessionLifecycleState::Streaming
    );
}

#[tokio::test]
async fn active_window_capture_count_ignores_remote_and_inactive_selections() {
    let app_state = Arc::new(AppState::default());
    let active_sender = SessionId("active-sender".to_string());
    let remote_controller = SessionId("remote-controller".to_string());
    let failed_sender = SessionId("failed-sender".to_string());

    {
        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            active_sender.clone(),
            sender_snapshot_for_source(&active_sender, "controller-a"),
        );
        sessions.insert(
            remote_controller.clone(),
            receiver_snapshot_for_target(&remote_controller, "target-a"),
        );
        sessions.insert(
            failed_sender.clone(),
            SessionSnapshot {
                lifecycle_state: SessionLifecycleState::Failed {
                    message: "failed".to_string(),
                },
                sender_active: false,
                ..sender_snapshot_for_source(&failed_sender, "controller-b")
            },
        );
    }

    for session_id in [&active_sender, &remote_controller, &failed_sender] {
        store_capture_source_selection(
            &app_state,
            session_id,
            CaptureSourceSelection {
                session_id: session_id.clone(),
                source: test_window_capture_source("windows:window:0x1111"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;
    }

    assert_eq!(active_window_capture_count(&app_state).await, 1);
}

#[tokio::test]
async fn display_mode_restore_uses_original_temporary_mode() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("display-mode-restore-session".to_string());
    app_state
        .sessions
        .lock()
        .await
        .insert(session_id.clone(), sender_snapshot(&session_id));
    {
        app_state.display_modes.lock().await.record_change(
            session_id.clone(),
            display_mode("requested", 1920, 1080, 144, false),
            Some(display_mode("current", 2560, 1600, 60, true)),
            display_mode("target", 1920, 1080, 144, true),
            true,
        );
    }

    let change = accept_lan_display_mode_restore_with_mode(
        &app_state,
        &session_id,
        display_mode("current", 2560, 1600, 60, false),
    )
    .await
    .unwrap();

    assert_eq!(change.status, "restored");
    assert_eq!(
        change.active.as_ref().map(|mode| mode.id.as_str()),
        Some("current")
    );
    assert!(app_state
        .display_modes
        .lock()
        .await
        .restore_mode(&session_id)
        .is_none());
}

#[tokio::test]
async fn remote_capture_source_selection_reconciles_controller_profile() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("controller-capture-source-profile-session".to_string());
    app_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
            target_device_id: Some(DeviceId("target-device".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Streaming,
            last_error: None,
            sender_active: false,
            receiver_active: true,
        },
    );
    app_state.media_profiles.lock().await.set(
        session_id.clone(),
        negotiate_media_profile(Some(MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        }))
        .unwrap(),
    );

    store_capture_source_selection(
        &app_state,
        &session_id,
        CaptureSourceSelection {
            session_id: session_id.clone(),
            source: mrd_ipc::CaptureSource {
                id: "windows:display:0".to_string(),
                platform: "windows".to_string(),
                source_kind: "display".to_string(),
                title: "Display 1".to_string(),
                class_name: "Monitor".to_string(),
                width: 2560,
                height: 1600,
                process_id: 0,
                app_name: Some("Display".to_string()),
                bundle_identifier: None,
                preview_data_url: None,
                preview_width: None,
                preview_height: None,
            },
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    let negotiation = app_state
        .media_profiles
        .lock()
        .await
        .get(&session_id)
        .expect("controller profile reconciled to remote source");
    assert_eq!(negotiation.selected.width, 1728);
    assert_eq!(negotiation.selected.height, 1080);
    assert_eq!(
        negotiation.selected_source_id.as_deref(),
        Some("windows:display:0")
    );
    assert_eq!(negotiation.status, "downgraded");
    assert_eq!(
        app_state
            .capture_sources
            .lock()
            .await
            .get(&session_id)
            .expect("stored remote capture source")
            .source
            .id,
        "windows:display:0"
    );
}

#[test]
fn prepare_frame_for_h264_keeps_cpu_frame_when_dimensions_match() {
    let data = vec![7_u8; 64 * 32 * 4];
    let frame = CapturedFrame::from_cpu(64, 32, FramePixelFormat::Bgra32, 1234, data.clone());
    let profile = MediaProfile {
        width: 64,
        height: 32,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    let prepared = prepare_frame_for_h264(frame, &profile).expect("prepared frame");

    assert_eq!(prepared.width, 64);
    assert_eq!(prepared.height, 32);
    assert_eq!(prepared.pixel_format, FramePixelFormat::Bgra32);
    assert_eq!(prepared.data, data);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_lan_capture_stream_fps_requests_headroom() {
    let profile = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    assert_eq!(macos_lan_capture_stream_fps(&profile), 120);

    let high_refresh = MediaProfile {
        fps: 165,
        ..profile
    };
    assert_eq!(macos_lan_capture_stream_fps(&high_refresh), 240);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_capture_pump_repeat_pacing_defaults_to_headroom() {
    let profile = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    assert_eq!(macos_capture_pump_repeat_pacing_fps(&profile), 144);
    assert_eq!(
        macos_capture_pump_repeat_frame_interval(&profile),
        media_frame_interval_for_fps(144)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_capture_pump_repeats_latest_by_default_and_allows_opt_out() {
    assert!(lan_capture_pump_repeat_latest_from_env_value(None));
    assert!(lan_capture_pump_repeat_latest_from_env_value(Some("true")));
    assert!(!lan_capture_pump_repeat_latest_from_env_value(Some("false")));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_capture_pump_repeat_grace_uses_capture_headroom() {
    let profile = MediaProfile {
        width: 1280,
        height: 720,
        fps: 60,
        bitrate_mbps: 10,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    assert_eq!(
        macos_capture_pump_repeat_grace_timeout(&profile),
        LAN_CAPTURE_PUMP_REPEAT_GRACE_MAX
    );

    let high_refresh = MediaProfile {
        fps: 165,
        ..profile
    };
    assert_eq!(
        macos_capture_pump_repeat_grace_timeout(&high_refresh),
        media_frame_interval_for_fps(240) / 2
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_capture_pump_waits_for_fresh_frame_before_repeating_latest() {
    let latest_frame = CapturedFrame::from_cpu(1, 1, FramePixelFormat::Bgra32, 1, vec![0; 4]);
    let fresh_frame = CapturedFrame::from_cpu(1, 1, FramePixelFormat::Bgra32, 2, vec![1; 4]);
    let shared = Arc::new((
        StdMutex::new(MacosPumpedLanFrameState {
            frames: VecDeque::new(),
            latest_frame: Some(latest_frame),
            sequence: 1,
            error: None,
        }),
        StdCondvar::new(),
    ));
    let producer_shared = shared.clone();
    let producer = thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(5));
        let (lock, cvar) = &*producer_shared;
        let mut state = lock.lock().expect("capture pump state");
        state.latest_frame = Some(fresh_frame.clone());
        state.frames.push_back(fresh_frame);
        state.sequence = state.sequence.wrapping_add(1).max(1);
        cvar.notify_all();
    });
    let mut capture = MacosPumpedLanFrameCapture {
        shared,
        stop: Arc::new(AtomicBool::new(false)),
        worker: None,
        repeat_grace_timeout: Duration::from_millis(50),
    };

    let captured = capture.capture_frame().expect("capture pumped frame");
    producer.join().expect("producer thread");

    assert!(!captured.repeated_latest_frame);
    assert_eq!(captured.frame.timestamp_us, 2);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_capture_pump_repeats_retained_frame_when_fresh_frame_misses_grace() {
    let latest_frame =
        CapturedFrame::from_cpu(1, 1, FramePixelFormat::Bgra32, 1, vec![7, 8, 9, 255]);
    let shared = Arc::new((
        StdMutex::new(MacosPumpedLanFrameState {
            frames: VecDeque::new(),
            latest_frame: Some(latest_frame),
            sequence: 1,
            error: None,
        }),
        StdCondvar::new(),
    ));
    let mut capture = MacosPumpedLanFrameCapture {
        shared,
        stop: Arc::new(AtomicBool::new(false)),
        worker: None,
        repeat_grace_timeout: Duration::from_millis(1),
    };

    let captured = capture.capture_frame().expect("repeat retained frame");

    assert!(captured.repeated_latest_frame);
    assert_eq!(captured.frame.data, vec![7, 8, 9, 255]);
    assert!(captured.frame.timestamp_us > 1);
}

#[cfg(windows)]
#[test]
fn prepare_frame_for_h264_accepts_exact_d3d11_shared_frame() {
    let frame = CapturedFrame::from_d3d11_shared_bgra(64, 32, 1234, 0x1234, 64 * 4);
    let profile = MediaProfile {
        width: 64,
        height: 32,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    let prepared = prepare_frame_for_h264(frame, &profile).expect("prepared shared frame");

    assert_eq!(prepared.width, 64);
    assert_eq!(prepared.height, 32);
    assert!(prepared.data.is_empty());
    assert!(prepared.d3d11_shared_bgra().is_some());
}

#[cfg(windows)]
#[test]
fn prepare_frame_for_h264_rejects_scaled_d3d11_shared_frame() {
    let frame = CapturedFrame::from_d3d11_shared_bgra(128, 64, 1234, 0x1234, 128 * 4);
    let profile = MediaProfile {
        width: 64,
        height: 32,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    let error = prepare_frame_for_h264(frame, &profile).expect_err("shared scale rejected");

    assert!(error
        .to_string()
        .contains("requires exact selected profile"));
}

#[test]
fn window_h264_capture_dimensions_makes_odd_profile_dimensions_even_and_non_zero() {
    assert_eq!(window_h264_capture_dimensions(1001, 777), (1000, 776));
    assert_eq!(window_h264_capture_dimensions(1, 1), (2, 2));
}

#[test]
fn window_h264_capture_dimensions_returns_even_dimensions_for_odd_window_profile() {
    let (width, height) = window_h264_capture_dimensions(1001, 777);

    assert_eq!(width % 2, 0);
    assert_eq!(height % 2, 0);
    assert!(width >= 2);
    assert!(height >= 2);
}

#[test]
fn h264_target_dimensions_scale_down_with_aspect_ratio_and_even_bounds() {
    let profile = MediaProfile {
        width: 1280,
        height: 720,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    assert_eq!(h264_target_dimensions(2560, 1600, &profile), (1152, 720));
    assert_eq!(h264_target_dimensions(1279, 719, &profile), (1278, 718));
    assert_eq!(h264_target_dimensions(1, 1, &profile), (2, 2));
}

#[test]
fn capture_sources_ack_strips_preview_payload_before_udp_fit() {
    let sources = (0..24)
        .map(|index| mrd_ipc::CaptureSource {
            id: format!("windows:window:0x{:X}", index + 0x1000),
            platform: "windows".to_string(),
            source_kind: "window".to_string(),
            title: format!("Target App {index}"),
            class_name: "ApplicationFrameWindow".to_string(),
            width: 1280,
            height: 720,
            process_id: 4242 + index,
            app_name: Some(format!("Target App {index}")),
            bundle_identifier: None,
            preview_data_url: Some(format!("legacy-preview-payload-{}", "A".repeat(8_000))),
            preview_width: Some(240),
            preview_height: Some(135),
        })
        .collect();

    let packet = capture_sources::fit_capture_sources_ack_packet(
        "target-instance".to_string(),
        "capture-source-session".to_string(),
        true,
        Some("listed".to_string()),
        sources,
    );

    assert!(capture_sources::serialized_packet_len(&packet) <= DISCOVERY_SAFE_UDP_PAYLOAD_BYTES);
    let LanDiscoveryPacket::CaptureSourcesAck { sources, .. } = packet else {
        panic!("expected capture sources ack");
    };
    assert!(sources
        .iter()
        .all(|source| source.preview_data_url.is_none()));
    assert!(sources.iter().all(|source| source.preview_width.is_none()));
    assert!(sources.iter().all(|source| source.preview_height.is_none()));
}

#[test]
fn dynamic_media_probe_frame_preserves_selected_profile() {
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let frame = build_media_probe_frame(7, 99_000, &profile);
    let stats = decode_media_probe_frame(&frame).unwrap();

    assert_eq!(stats.width, 1920);
    assert_eq!(stats.height, 1080);
    assert_eq!(stats.target_fps, 60);
    assert_eq!(stats.target_bitrate_mbps, 20);
    assert_eq!(stats.payload_bytes, media_payload_bytes(&profile) as u32);
    assert_eq!(stats.format, "compressed_h264_test_pattern");
}

#[test]
fn dynamic_hevc_media_probe_frame_preserves_codec() {
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let frame = build_media_probe_frame(7, 99_000, &profile);
    let stats = decode_media_probe_frame(&frame).unwrap();

    assert_eq!(stats.width, 1920);
    assert_eq!(stats.height, 1080);
    assert_eq!(stats.target_fps, 60);
    assert_eq!(stats.target_bitrate_mbps, 20);
    assert_eq!(stats.payload_bytes, media_payload_bytes(&profile) as u32);
    assert_eq!(stats.format, "compressed_hevc_test_pattern");
}

#[test]
fn dynamic_h265_alias_media_probe_frame_uses_hevc_format() {
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "H.265".to_string(),
        ..MediaProfile::default()
    };
    let frame = build_media_probe_frame(7, 99_000, &profile);
    let stats = decode_media_probe_frame(&frame).unwrap();

    assert_eq!(
        LanAccessUnitCodec::from_profile(&profile),
        LanAccessUnitCodec::Hevc
    );
    assert_eq!(stats.format, "compressed_hevc_test_pattern");
}

#[test]
fn decoded_video_probe_format_accepts_h265_aliases() {
    assert_eq!(decoded_video_probe_format("h265"), "hevc_desktop_frame");
    assert_eq!(decoded_video_probe_format("H.265"), "hevc_desktop_frame");
    assert_eq!(decoded_video_probe_format("h.264"), "h264_desktop_frame");
}

#[test]
fn lan_media_v2_envelope_round_trips_h264_access_unit() {
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 144,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let encoded = encode_lan_media_envelope(LanMediaEnvelope {
        payload_type: LAN_MEDIA_PAYLOAD_H264_ACCESS_UNIT,
        codec: LAN_MEDIA_CODEC_H264,
        sequence: 99,
        timestamp_us: 123_456,
        profile: profile.clone(),
        payload: vec![0, 0, 0, 1, 0x67],
    })
    .unwrap();

    let decoded = decode_lan_media_envelope(&encoded).unwrap();

    assert_eq!(decoded.payload_type, LAN_MEDIA_PAYLOAD_H264_ACCESS_UNIT);
    assert_eq!(decoded.codec, LAN_MEDIA_CODEC_H264);
    assert_eq!(decoded.sequence, 99);
    assert_eq!(decoded.timestamp_us, 123_456);
    assert_eq!(decoded.profile, profile);
    assert_eq!(decoded.payload, vec![0, 0, 0, 1, 0x67]);
}

#[test]
fn access_unit_codec_lives_with_media_access_unit_mapping() {
    let codec =
        super::media_access_unit::LanAccessUnitCodec::from_envelope_codec(LAN_MEDIA_CODEC_HEVC)
            .expect("hevc codec");

    assert_eq!(codec.name(), "hevc");
    assert_eq!(codec.display_name(), "HEVC");
    assert_eq!(codec.envelope_codec(), LAN_MEDIA_CODEC_HEVC);
}

#[test]
fn sender_encoder_state_lives_with_media_sender() {
    let _ = std::mem::size_of::<Option<super::media_sender::LanSenderEncoder>>();
}

#[test]
fn lan_media_v2_envelope_round_trips_hevc_access_unit() {
    let profile = MediaProfile {
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 120,
        codec: "hevc".to_string(),
        codec_profile: Some("main".to_string()),
        bit_depth: Some(8),
        chroma_subsampling: Some("4:2:0".to_string()),
        pixel_format: Some("nv12".to_string()),
        hdr_enabled: Some(false),
        ..MediaProfile::default()
    };
    let encoded = encode_lan_media_envelope(LanMediaEnvelope {
        payload_type: LAN_MEDIA_PAYLOAD_ACCESS_UNIT,
        codec: LAN_MEDIA_CODEC_HEVC,
        sequence: 9,
        timestamp_us: 123_456,
        profile: profile.clone(),
        payload: b"fake-hevc".to_vec(),
    })
    .unwrap();

    let decoded = decode_lan_media_envelope(&encoded).unwrap();

    assert_eq!(decoded.payload_type, LAN_MEDIA_PAYLOAD_ACCESS_UNIT);
    assert_eq!(decoded.codec, LAN_MEDIA_CODEC_HEVC);
    assert_eq!(decoded.profile.codec, "hevc");
    assert_eq!(decoded.profile.codec_profile.as_deref(), Some("main"));
    assert_eq!(decoded.profile.chroma_subsampling.as_deref(), Some("4:2:0"));
    assert_eq!(decoded.payload, b"fake-hevc");
    assert_eq!(decoded.profile, profile);
}

#[test]
fn lan_media_v2_envelope_rejects_legacy_probe_without_magic_fallback() {
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let legacy_probe = build_media_probe_frame(1, 1_000, &profile);

    let error = decode_lan_media_envelope(&legacy_probe).expect_err("legacy probe is not v2");

    assert!(error.to_string().contains("invalid magic"));
    assert!(!error.to_string().contains("legacy probe fallback"));
}

#[tokio::test]
async fn lan_media_v3_frame_converts_to_receiver_envelope() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("media-v3-session".to_string());
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 144,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    app_state.media_profiles.lock().await.set(
        session_id.clone(),
        MediaProfileNegotiation {
            requested: profile.clone(),
            selected: profile.clone(),
            status: "accepted".to_string(),
            reason: None,
            selected_source_id: None,
            selected_width: Some(profile.width),
            selected_height: Some(profile.height),
            downgrade_reason: None,
        },
    );

    let converted = quic_media_v3_frame_to_legacy_frame(
        &app_state,
        &session_id,
        QuicMediaFrame {
            payload_type: QuicMediaPayloadType::AccessUnit,
            codec: QuicMediaCodec::H264,
            profile_id: lan_media_profile_id(&profile),
            frame_id: 42,
            timestamp_us: 123_456,
            flags: 1,
            payload: vec![0, 0, 0, 1, 0x65].into(),
        },
        QuicAuReassemblerStats::default(),
    )
    .await
    .unwrap()
    .expect("converted frame");

    assert_eq!(converted.frame_id, 42);
    assert!(converted.is_keyframe);
    let envelope = decode_lan_media_envelope(&converted.payload).unwrap();
    assert_eq!(envelope.payload_type, LAN_MEDIA_PAYLOAD_H264_ACCESS_UNIT);
    assert_eq!(envelope.codec, LAN_MEDIA_CODEC_H264);
    assert_eq!(envelope.sequence, 42);
    assert_eq!(envelope.profile, profile);
    assert_eq!(envelope.payload, vec![0, 0, 0, 1, 0x65]);
}

#[tokio::test]
async fn lan_media_v3_frame_converts_hevc_to_receiver_envelope() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("media-v3-hevc-session".to_string());
    let profile = MediaProfile {
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 120,
        codec: "hevc".to_string(),
        codec_profile: Some("main".to_string()),
        bit_depth: Some(8),
        chroma_subsampling: Some("4:2:0".to_string()),
        pixel_format: Some("nv12".to_string()),
        hdr_enabled: Some(false),
        ..MediaProfile::default()
    };
    app_state.media_profiles.lock().await.set(
        session_id.clone(),
        MediaProfileNegotiation {
            requested: profile.clone(),
            selected: profile.clone(),
            status: "accepted".to_string(),
            reason: None,
            selected_source_id: None,
            selected_width: Some(profile.width),
            selected_height: Some(profile.height),
            downgrade_reason: None,
        },
    );

    let converted = quic_media_v3_frame_to_legacy_frame(
        &app_state,
        &session_id,
        QuicMediaFrame {
            payload_type: QuicMediaPayloadType::AccessUnit,
            codec: QuicMediaCodec::Hevc,
            profile_id: lan_media_profile_id(&profile),
            frame_id: 12,
            timestamp_us: 77,
            flags: 1,
            payload: b"hevc-au".to_vec().into(),
        },
        QuicAuReassemblerStats::default(),
    )
    .await
    .unwrap()
    .expect("converted frame");

    let envelope = decode_lan_media_envelope(&converted.payload).unwrap();
    assert_eq!(envelope.payload_type, LAN_MEDIA_PAYLOAD_ACCESS_UNIT);
    assert_eq!(envelope.codec, LAN_MEDIA_CODEC_HEVC);
    assert_eq!(envelope.profile.codec, "hevc");
    assert_eq!(envelope.payload, b"hevc-au");
    assert_eq!(envelope.profile, profile);
}

#[tokio::test]
async fn lan_media_v3_profile_mismatch_is_transient_drop() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("media-v3-mismatch-session".to_string());
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 144,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let stale_profile = MediaProfile {
        fps: 60,
        ..profile.clone()
    };
    app_state.media_profiles.lock().await.set(
        session_id.clone(),
        MediaProfileNegotiation {
            requested: profile.clone(),
            selected: profile,
            status: "accepted".to_string(),
            reason: None,
            selected_source_id: None,
            selected_width: Some(1920),
            selected_height: Some(1080),
            downgrade_reason: None,
        },
    );

    let converted = quic_media_v3_frame_to_legacy_frame(
        &app_state,
        &session_id,
        QuicMediaFrame {
            payload_type: QuicMediaPayloadType::AccessUnit,
            codec: QuicMediaCodec::H264,
            profile_id: lan_media_profile_id(&stale_profile),
            frame_id: 7,
            timestamp_us: 123_456,
            flags: 1,
            payload: vec![0, 0, 0, 1, 0x65].into(),
        },
        QuicAuReassemblerStats::default(),
    )
    .await
    .unwrap();

    assert!(converted.is_none());
    let snapshot = app_state.probes.lock().await.snapshot(&session_id);
    assert_eq!(snapshot.frames_received, 1);
    assert_eq!(snapshot.frames_decoded, 0);
    assert_eq!(snapshot.frames_dropped, 1);
    assert_eq!(snapshot.last_error, None);
}

#[test]
fn lan_sender_stats_datagram_round_trips_without_media_sequence() {
    let payload = LanSenderStatsPayload {
        sequence: 123,
        frame_count: 122,
        source_id: Some("windows:display-shared:0".to_string()),
        target_fps: 144,
        target_bitrate_mbps: 20,
        metrics: vec![MediaStageMetrics {
            stage: "sender.encode".to_string(),
            p50_ms: Some(1.2),
            p95_ms: Some(2.4),
        }],
        sender_transport: MediaSenderTransportSnapshot {
            capture_source_id: Some("windows:display-shared:0".to_string()),
            capture_source_kind: Some("display_shared".to_string()),
            capture_memory_path: Some("d3d11_shared_bgra".to_string()),
            dynamic_fps_tier: None,
            target_fps: Some(144),
            frames_completed: 122,
            repeated_latest_frames: 3,
            datagram_fragments_attempted: 4,
            datagram_fragments_sent: 3,
            datagram_fragments_delayed: 0,
            datagram_fragments_dropped_by_impairment: 0,
            datagram_fragments_dropped_for_capacity: 1,
            datagram_fragments_dropped_for_budget: 0,
            datagram_frames_cut_short_for_capacity: 1,
            datagram_frames_cut_short_for_budget: 0,
            reliable_fragments_sent: 0,
            reliable_frames_sent: 0,
            ..MediaSenderTransportSnapshot::default()
        },
        test_impairment: None,
    };

    let encoded = encode_lan_sender_stats_datagram(&payload).unwrap();
    let decoded = decode_lan_sender_stats_datagram(&encoded).unwrap();

    assert_eq!(decoded, Some(payload));
    assert_eq!(
        decode_lan_sender_stats_datagram(b"not-stats").unwrap(),
        None
    );
}

#[test]
fn lan_sender_stats_tracker_accumulates_transport_counters() {
    let mut tracker = LanSenderStatsTracker::new(Instant::now());
    tracker.record_datagram_frame(LanSenderDatagramFrameReport {
        fragments_attempted: 5,
        fragments_sent: 3,
        fragments_delayed: 1,
        fragments_dropped_by_impairment: 1,
        fragments_dropped_for_capacity: 1,
        fragments_dropped_for_budget: 0,
        cut_short_for_capacity: true,
        cut_short_for_budget: false,
    });
    tracker.record_datagram_frame(LanSenderDatagramFrameReport {
        fragments_attempted: 4,
        fragments_sent: 2,
        fragments_delayed: 0,
        fragments_dropped_by_impairment: 0,
        fragments_dropped_for_capacity: 0,
        fragments_dropped_for_budget: 2,
        cut_short_for_capacity: false,
        cut_short_for_budget: true,
    });
    tracker.record_reliable_frame(7, true);
    tracker.record_repeated_latest_frame();
    tracker.record_captured_frame(&CapturedFrame::from_cpu(
        1,
        1,
        FramePixelFormat::Bgra32,
        0,
        vec![0; 4],
    ));
    tracker.record_captured_frame(&CapturedFrame::from_cpu(
        2,
        2,
        FramePixelFormat::Nv12,
        0,
        vec![0; 6],
    ));
    tracker.record_encoded_access_unit(1_024, true);
    tracker.record_encoded_access_unit(256, false);
    tracker.frame_completed();

    assert_eq!(tracker.sender_transport.frames_completed, 1);
    assert_eq!(tracker.sender_transport.repeated_latest_frames, 1);
    assert_eq!(tracker.sender_transport.capture_frame_samples, 2);
    assert_eq!(tracker.sender_transport.capture_cpu_frames, 2);
    assert_eq!(tracker.sender_transport.capture_bgra32_frames, 1);
    assert_eq!(tracker.sender_transport.capture_nv12_frames, 1);
    assert_eq!(tracker.sender_transport.access_units_encoded, 2);
    assert_eq!(tracker.sender_transport.keyframes_encoded, 1);
    assert_eq!(tracker.sender_transport.encoded_access_unit_bytes, 1_280);
    assert_eq!(tracker.sender_transport.datagram_fragments_attempted, 9);
    assert_eq!(tracker.sender_transport.datagram_fragments_sent, 5);
    assert_eq!(tracker.sender_transport.datagram_fragments_delayed, 1);
    assert_eq!(
        tracker
            .sender_transport
            .datagram_fragments_dropped_by_impairment,
        1
    );
    assert_eq!(
        tracker
            .sender_transport
            .datagram_fragments_dropped_for_capacity,
        1
    );
    assert_eq!(
        tracker
            .sender_transport
            .datagram_fragments_dropped_for_budget,
        2
    );
    assert_eq!(
        tracker
            .sender_transport
            .datagram_frames_cut_short_for_capacity,
        1
    );
    assert_eq!(
        tracker
            .sender_transport
            .datagram_frames_cut_short_for_budget,
        1
    );
    assert_eq!(tracker.sender_transport.reliable_fragments_sent, 7);
    assert_eq!(tracker.sender_transport.reliable_frames_sent, 1);
}

#[cfg(windows)]
#[test]
fn windows_lan_sender_uses_monitor_specific_backends_for_display_sources() {
    assert_eq!(
        windows_lan_capture_backend("windows:display-shared:0", false),
        WindowsLanCaptureBackend::DxgiShared
    );
    assert_eq!(
        windows_lan_capture_backend("windows:display:0", true),
        WindowsLanCaptureBackend::Winrt
    );
}

#[cfg(windows)]
#[test]
fn windows_lan_capture_backend_selects_winrt_window_shared_for_window_sources() {
    assert_eq!(
        windows_lan_capture_backend("windows:window:0x1234", true),
        WindowsLanCaptureBackend::WinrtWindowShared
    );
}

#[cfg(windows)]
#[test]
fn windows_lan_capture_backend_keeps_window_sources_on_cpu_when_nvenc_h264_is_unavailable() {
    assert_eq!(
        windows_lan_capture_backend("windows:window:0x1234", false),
        WindowsLanCaptureBackend::Winrt
    );
}

#[cfg(windows)]
#[test]
fn windows_lan_window_shared_capture_uses_shared_texture_when_nvenc_h264_is_available() {
    assert!(windows_lan_window_capture_uses_shared_texture(true));
}

#[cfg(windows)]
#[test]
fn windows_lan_window_shared_capture_uses_cpu_texture_when_nvenc_h264_is_unavailable() {
    assert!(!windows_lan_window_capture_uses_shared_texture(false));
}

#[cfg(windows)]
#[test]
fn windows_lan_capture_backend_keeps_dxgi_shared_for_display_shared_sources() {
    assert_eq!(
        windows_lan_capture_backend("windows:display-shared:1", false),
        WindowsLanCaptureBackend::DxgiShared
    );
}

#[cfg(windows)]
#[test]
fn windows_lan_capture_backend_for_profile_keeps_shared_for_full_size_display() {
    assert_eq!(
        windows_lan_capture_backend_for_profile(
            "windows:display-shared:1",
            2560,
            1440,
            &test_media_profile(2560, 1440),
            false
        ),
        WindowsLanCaptureBackend::DxgiShared
    );
}

#[cfg(windows)]
#[test]
fn windows_lan_capture_backend_for_profile_keeps_shared_for_reduced_display() {
    assert_eq!(
        windows_lan_capture_backend_for_profile(
            "windows:display-shared:1",
            2560,
            1440,
            &test_media_profile(1920, 1080),
            false
        ),
        WindowsLanCaptureBackend::DxgiShared
    );
}

#[cfg(windows)]
#[test]
fn windows_lan_capture_backend_for_profile_keeps_shared_for_full_size_window() {
    assert_eq!(
        windows_lan_capture_backend_for_profile(
            "windows:window:0x1234",
            1280,
            720,
            &test_media_profile(1280, 720),
            true
        ),
        WindowsLanCaptureBackend::WinrtWindowShared
    );
}

#[cfg(windows)]
#[test]
fn windows_lan_capture_backend_for_profile_uses_scaling_path_for_reduced_window() {
    assert_eq!(
        windows_lan_capture_backend_for_profile(
            "windows:window:0x1234",
            1280,
            720,
            &test_media_profile(960, 540),
            true
        ),
        WindowsLanCaptureBackend::Winrt
    );
}

#[cfg(windows)]
fn test_media_profile(width: u32, height: u32) -> MediaProfile {
    MediaProfile {
        width,
        height,
        fps: 144,
        bitrate_mbps: 80,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    }
}

#[cfg(windows)]
#[test]
fn parse_windows_window_source_id_extracts_hwnd() {
    assert_eq!(
        parse_windows_window_source_id("windows:window:0x1234").unwrap(),
        0x1234
    );
}

#[cfg(windows)]
#[test]
fn parse_windows_window_source_id_rejects_display_source() {
    let error = parse_windows_window_source_id("windows:display-shared:1")
        .unwrap_err()
        .to_string();

    assert!(error.contains("window"));
}

#[test]
fn lan_sender_encoder_order_prefers_hardware_before_fallback() {
    let backends = preferred_lan_h264_encoder_backends();
    #[cfg(windows)]
    assert_eq!(backends, ["nvenc_h264", "openh264"]);
    #[cfg(target_os = "macos")]
    assert_eq!(backends, ["videotoolbox_h264", "openh264"]);
    #[cfg(not(any(windows, target_os = "macos")))]
    assert_eq!(backends, ["openh264"]);
}

#[test]
fn lan_quic_media_routes_only_keyframes_reliably() {
    assert!(should_send_access_unit_reliably(true, true, 1024, 1_200));
    assert!(!should_send_access_unit_reliably(
        true,
        false,
        32 * 1024 + 1,
        1_200
    ));
    assert!(!should_send_access_unit_reliably(true, false, 1_200, 1_200));
    assert!(!should_send_access_unit_reliably(true, false, 512, 1_200));
    assert!(!should_send_access_unit_reliably(
        false,
        true,
        32 * 1024 + 1,
        1_200
    ));
}

#[test]
fn lan_quic_media_uses_reliable_frames_for_60fps_and_keeps_high_refresh_opt_in() {
    let profile_1080p = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 144,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let profile_2k = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    assert!(!should_send_access_unit_as_reliable_frame(
        true,
        true,
        2,
        &profile_1080p,
        None
    ));
    assert!(should_send_access_unit_as_reliable_frame(
        true,
        true,
        2,
        &profile_2k,
        None
    ));
    assert!(should_send_access_unit_as_reliable_frame(
        true,
        true,
        1,
        &profile_2k,
        None
    ));
    assert!(!should_send_access_unit_as_reliable_frame(
        true,
        false,
        2,
        &profile_2k,
        None
    ));
    assert!(!should_send_access_unit_as_reliable_frame(
        false,
        true,
        2,
        &profile_2k,
        None
    ));
    assert!(should_send_access_unit_as_reliable_frame(
        true,
        true,
        2,
        &profile_1080p,
        Some(true)
    ));
    assert!(!should_send_access_unit_as_reliable_frame(
        true,
        true,
        2,
        &profile_2k,
        Some(false)
    ));
}

#[test]
fn lan_quic_media_uses_best_effort_only_for_low_latency_bitrate_tiers() {
    let low_latency = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let high_quality_2k144 = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let high_bitrate = MediaProfile {
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 120,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };

    assert!(use_best_effort_media_datagrams(&low_latency));
    assert!(!use_best_effort_media_datagrams(&high_quality_2k144));
    assert!(!use_best_effort_media_datagrams(&high_bitrate));
}

#[test]
fn high_refresh_datagram_send_budget_requires_reliable_media() {
    let high_refresh = MediaProfile {
        width: 2560,
        height: 1600,
        fps: 144,
        bitrate_mbps: 96,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let low_bitrate = MediaProfile {
        bitrate_mbps: 40,
        ..high_refresh.clone()
    };
    let low_refresh = MediaProfile {
        fps: 60,
        ..high_refresh.clone()
    };

    assert_eq!(
        lan_datagram_frame_send_budget(&high_refresh, true),
        Some(LAN_QUIC_DATAGRAM_SEND_BUDGET)
    );
    assert_eq!(lan_datagram_frame_send_budget(&high_refresh, false), None);
    assert_eq!(lan_datagram_frame_send_budget(&low_bitrate, true), None);
    assert_eq!(lan_datagram_frame_send_budget(&low_refresh, true), None);
}

#[test]
fn high_quality_lan_media_keeps_safe_datagram_size_by_default() {
    let profile = MediaProfile {
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 120,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };

    assert_eq!(
        lan_media_datagram_size(1_500, &profile, true),
        LAN_QUIC_FALLBACK_DATAGRAM_BYTES
    );
    assert_eq!(
        lan_media_datagram_size(1_500, &profile, false),
        LAN_QUIC_FALLBACK_DATAGRAM_BYTES
    );
}

#[test]
fn lan_quic_media_prefers_persistent_reliable_stream_when_available() {
    assert_eq!(
        select_reliable_media_send_mode(true, true),
        LanReliableMediaSendMode::Persistent
    );
    assert_eq!(
        select_reliable_media_send_mode(false, true),
        LanReliableMediaSendMode::Persistent
    );
    assert_eq!(
        select_reliable_media_send_mode(true, false),
        LanReliableMediaSendMode::PerMessage
    );
    assert_eq!(
        select_reliable_media_send_mode(false, false),
        LanReliableMediaSendMode::Disabled
    );
}

#[test]
fn high_refresh_reliable_media_prefers_per_message_streams_to_reduce_hol() {
    let high_bitrate = MediaProfile {
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 120,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let stable_bitrate = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };

    assert_eq!(
        select_reliable_media_send_mode_for_profile(true, true, &high_bitrate),
        LanReliableMediaSendMode::PerMessage
    );
    assert_eq!(
        select_reliable_media_send_mode_for_profile(true, true, &stable_bitrate),
        LanReliableMediaSendMode::PerMessage
    );
    assert_eq!(
        select_reliable_media_send_mode_for_profile(false, true, &high_bitrate),
        LanReliableMediaSendMode::Persistent
    );
}

#[test]
fn reliable_whole_frame_media_env_override_parses_truthy_and_falsey_values() {
    assert_eq!(
        reliable_whole_frame_media_override_from_env_value(Some("1")),
        Some(true)
    );
    assert_eq!(
        reliable_whole_frame_media_override_from_env_value(Some("true")),
        Some(true)
    );
    assert_eq!(
        reliable_whole_frame_media_override_from_env_value(Some("0")),
        Some(false)
    );
    assert_eq!(
        reliable_whole_frame_media_override_from_env_value(Some("off")),
        Some(false)
    );
    assert_eq!(
        reliable_whole_frame_media_override_from_env_value(Some("")),
        None
    );
    assert_eq!(
        reliable_whole_frame_media_override_from_env_value(None),
        None
    );
}

#[test]
fn lan_runtime_flag_parser_accepts_common_bool_aliases() {
    assert_eq!(super::runtime_flags::env_bool_override(None), None);
    assert_eq!(super::runtime_flags::env_bool_override(Some("")), None);
    assert_eq!(
        super::runtime_flags::env_bool_override(Some("YES")),
        Some(true)
    );
    assert_eq!(
        super::runtime_flags::env_bool_override(Some("off")),
        Some(false)
    );
    assert_eq!(
        super::runtime_flags::env_bool_override(Some("invalid")),
        None
    );
}

#[test]
fn render_pacing_env_override_parses_truthy_and_falsey_values() {
    assert_eq!(lan_render_pacing_from_env_value(None), None);
    assert_eq!(lan_render_pacing_from_env_value(Some("")), None);
    assert_eq!(lan_render_pacing_from_env_value(Some("0")), Some(false));
    assert_eq!(lan_render_pacing_from_env_value(Some("off")), Some(false));
    assert_eq!(lan_render_pacing_from_env_value(Some("1")), Some(true));
    assert_eq!(lan_render_pacing_from_env_value(Some("true")), Some(true));
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn render_queue_policy_env_parses_values() {
    assert_eq!(lan_render_queue_policy_from_env_value(None), None);
    assert_eq!(lan_render_queue_policy_from_env_value(Some("")), None);
    assert_eq!(
        lan_render_queue_policy_from_env_value(Some("latest")),
        Some(LanRenderQueuePolicy::Latest)
    );
    assert_eq!(
        lan_render_queue_policy_from_env_value(Some("low_latency")),
        Some(LanRenderQueuePolicy::Latest)
    );
    assert_eq!(
        lan_render_queue_policy_from_env_value(Some("paced_fifo")),
        Some(LanRenderQueuePolicy::PacedFifo)
    );
    assert_eq!(
        lan_render_queue_policy_from_env_value(Some("fifo")),
        Some(LanRenderQueuePolicy::PacedFifo)
    );
    assert_eq!(
        lan_render_queue_policy_from_env_value(Some("invalid")),
        None
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn render_queue_policy_defaults_by_platform_and_allows_latest_override() {
    let high_fps = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let low_fps = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    #[cfg(windows)]
    let expected_high_fps_default = LanRenderQueuePolicy::PacedFifo;
    #[cfg(target_os = "macos")]
    let expected_high_fps_default = LanRenderQueuePolicy::Latest;

    assert_eq!(
        lan_render_queue_policy_for_profile_with_override(&high_fps, None),
        expected_high_fps_default
    );
    assert_eq!(
        lan_render_queue_policy_for_profile_with_override(&low_fps, None),
        LanRenderQueuePolicy::PacedFifo
    );
    assert_eq!(
        lan_render_queue_policy_for_profile_with_override(
            &high_fps,
            Some(LanRenderQueuePolicy::Latest)
        ),
        LanRenderQueuePolicy::Latest
    );
}

#[test]
fn media_payload_hash_mode_defaults_to_metadata_for_high_fps() {
    let high_fps = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let low_fps = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    assert_eq!(
        lan_media_payload_hash_mode_for_profile_with_override(&high_fps, None),
        LanMediaPayloadHashMode::Metadata
    );
    assert_eq!(
        lan_media_payload_hash_mode_for_profile_with_override(&low_fps, None),
        LanMediaPayloadHashMode::Full
    );
    assert_eq!(
        lan_media_payload_hash_mode_from_env_value(Some("full")),
        Some(LanMediaPayloadHashMode::Full)
    );
    assert_eq!(
        lan_media_payload_hash_mode_from_env_value(Some("metadata")),
        Some(LanMediaPayloadHashMode::Metadata)
    );
    assert_eq!(
        lan_media_payload_hash_mode_from_env_value(Some("off")),
        Some(LanMediaPayloadHashMode::Disabled)
    );

    let payload = [1, 2, 3, 4, 5, 6, 7, 8];
    assert_eq!(
        lan_media_payload_hash_for_mode(
            LanMediaPayloadHashMode::Full,
            &high_fps,
            42,
            123_456,
            &payload
        ),
        format!("fnv1a64:{:016x}", fnv1a64(&payload))
    );
    assert!(lan_media_payload_hash_for_mode(
        LanMediaPayloadHashMode::Metadata,
        &high_fps,
        42,
        123_456,
        &payload
    )
    .starts_with("fnv1a64:meta:"));
    assert_eq!(
        lan_media_payload_hash_for_mode(
            LanMediaPayloadHashMode::Disabled,
            &high_fps,
            42,
            123_456,
            &payload
        ),
        "fnv1a64:disabled"
    );
}

#[test]
fn lan_keyframe_request_control_datagram_roundtrips() {
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 144,
        bitrate_mbps: 80,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let datagram =
        encode_lan_keyframe_request_datagram(&profile, 7, LAN_QUIC_FALLBACK_DATAGRAM_BYTES)
            .expect("encode keyframe request");

    assert!(decode_lan_keyframe_request_datagram(&datagram).expect("decode request"));
}

#[test]
fn lan_media_profile_identity_includes_color_fields() {
    let base = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let mut grayscale = base.clone();
    grayscale.color_mode = Some("grayscale".to_string());
    let mut hdr = base.clone();
    hdr.color_pipeline = Some("hdr_main10".to_string());

    assert_ne!(
        lan_media_profile_id(&base),
        lan_media_profile_id(&grayscale)
    );
    assert_ne!(lan_media_profile_id(&base), lan_media_profile_id(&hdr));
    assert_ne!(
        fnv1a64_media_metadata(&base, 7, 123_456, 4096),
        fnv1a64_media_metadata(&grayscale, 7, 123_456, 4096)
    );
    assert!(format_media_profile(&grayscale).contains("color=grayscale"));
    assert!(format_media_profile(&hdr).contains("pipeline=hdr_main10"));
}

#[test]
fn lan_keyframe_request_decoder_ignores_access_units() {
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 144,
        bitrate_mbps: 80,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let datagram = fragment_media_payload_v3(
        QuicMediaPayloadType::AccessUnit,
        QuicMediaCodec::H264,
        lan_media_profile_id(&profile),
        1,
        123,
        true,
        &[0, 0, 0, 1, 0x65],
        LAN_QUIC_FALLBACK_DATAGRAM_BYTES,
    )
    .expect("fragment access unit")
    .remove(0);

    assert!(!decode_lan_keyframe_request_datagram(&datagram).expect("decode access unit"));
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn latest_render_queue_policy_skips_pacing_wait() {
    let high_fps = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };

    assert!(!lan_render_policy_allows_service_pacing(
        LanRenderQueuePolicy::Latest,
        &high_fps,
        false
    ));
    assert!(lan_render_policy_allows_service_pacing(
        LanRenderQueuePolicy::PacedFifo,
        &high_fps,
        false
    ));
    assert!(!lan_render_policy_allows_service_pacing(
        LanRenderQueuePolicy::PacedFifo,
        &high_fps,
        true
    ));
    assert_eq!(
        lan_render_queue_capacity_for_policy(&high_fps, LanRenderQueuePolicy::Latest),
        1
    );
    assert_eq!(
        lan_render_queue_capacity_for_policy(&high_fps, LanRenderQueuePolicy::PacedFifo),
        lan_render_queue_capacity_for_profile(&high_fps)
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn latest_render_queue_policy_takes_latest_and_reports_stale_drops() {
    let mut registry = crate::app_state::MediaRenderQueueRegistry::default();
    let session_id = SessionId("latest-render-policy-session".to_string());
    let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
    let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
    let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));
    let fourth = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![10, 11, 12]));

    match registry.enqueue_bounded(session_id.clone(), first, 3) {
        MediaRenderQueueEnqueue::Start(_) => {}
        other => panic!("expected render worker start, got {other:?}"),
    }
    registry.enqueue_bounded(session_id.clone(), second, 3);
    registry.enqueue_bounded(session_id.clone(), third, 3);
    registry.enqueue_bounded(session_id.clone(), fourth.clone(), 3);

    let (next, dropped) = take_next_lan_render_frame_for_policy(
        &mut registry,
        &session_id,
        LanRenderQueuePolicy::Latest,
    );

    assert_eq!(next, Some(fourth));
    assert_eq!(dropped, 2);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn paced_fifo_render_queue_policy_takes_next_without_stale_drops() {
    let mut registry = crate::app_state::MediaRenderQueueRegistry::default();
    let session_id = SessionId("paced-render-policy-session".to_string());
    let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
    let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
    let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));

    match registry.enqueue_bounded(session_id.clone(), first, 3) {
        MediaRenderQueueEnqueue::Start(_) => {}
        other => panic!("expected render worker start, got {other:?}"),
    }
    registry.enqueue_bounded(session_id.clone(), second.clone(), 3);
    registry.enqueue_bounded(session_id.clone(), third, 3);

    let (next, dropped) = take_next_lan_render_frame_for_policy(
        &mut registry,
        &session_id,
        LanRenderQueuePolicy::PacedFifo,
    );

    assert_eq!(next, Some(second));
    assert_eq!(dropped, 0);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn render_queue_capacity_env_keeps_bounded_burst_backlog() {
    assert_eq!(
        lan_render_queue_capacity_from_env_value(None),
        LAN_RENDER_PACING_DEFAULT_MAX_PENDING_FRAMES
    );
    assert_eq!(lan_render_queue_capacity_from_env_value(Some("1")), 1);
    assert_eq!(lan_render_queue_capacity_from_env_value(Some("6")), 6);
    assert_eq!(
        lan_render_queue_capacity_from_env_value(Some("128")),
        LAN_RENDER_PACING_MAX_PENDING_FRAMES_LIMIT
    );
    assert_eq!(
        lan_render_queue_capacity_from_env_value(Some("invalid")),
        LAN_RENDER_PACING_DEFAULT_MAX_PENDING_FRAMES
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn render_pacing_defaults_to_interruptible_refresh_cap() {
    let high_fps = MediaProfile {
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 120,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let low_fps = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    assert!(lan_render_pacing_enabled_for_profile(&high_fps));
    assert_eq!(
        lan_render_queue_capacity_for_profile(&high_fps),
        LAN_RENDER_PACING_DEFAULT_MAX_PENDING_FRAMES
    );
    assert_eq!(
        lan_render_pacing_target_fps_from_values(high_fps.fps, Some(144)),
        144
    );
    assert_eq!(
        lan_render_pacing_target_fps_from_values(high_fps.fps, Some(240)),
        165
    );
    assert_eq!(
        lan_render_cap_target_fps_for_profile(&high_fps),
        Some(lan_render_pacing_target_fps(&high_fps))
    );
    assert_eq!(
        render_profile_requests_high_resolution_timer(&high_fps),
        lan_render_pacing_target_fps(&high_fps) >= LAN_RENDER_PACING_PRECISE_SLEEP_MIN_FPS
    );
    let precise_guard = render_pacing_precise_sleep_guard(120);
    assert!(precise_guard > Duration::ZERO);
    assert!(precise_guard < render_pacing_frame_interval(120));
    assert_eq!(render_pacing_precise_sleep_guard(60), Duration::ZERO);
    assert_eq!(
        lan_render_pacing_render_start_delay(Duration::from_micros(7_000), 144),
        Duration::from_micros(6_750)
    );
    assert_eq!(
        lan_render_pacing_render_start_delay(Duration::from_micros(7_000), 60),
        Duration::from_micros(7_000)
    );
    assert!(!should_interrupt_render_pacing_sleep(0, 3));
    assert!(should_interrupt_render_pacing_sleep(1, 3));
    assert!(should_interrupt_render_pacing_sleep(2, 3));
    assert!(should_interrupt_render_pacing_sleep(1, 1));
    assert_eq!(
        lan_render_pacing_target_fps_from_values(high_fps.fps, None),
        165
    );
    assert!(!lan_render_pacing_enabled_for_profile(&low_fps));
    assert_eq!(lan_render_queue_capacity_for_profile(&low_fps), 1);
    assert_eq!(lan_render_cap_target_fps_for_profile(&low_fps), None);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn surface_renderer_lock_waits_through_short_contention() {
    let mutex = Arc::new(std::sync::Mutex::new(()));
    let guard = mutex.lock().expect("hold test mutex");
    let waiter = {
        let mutex = mutex.clone();
        std::thread::spawn(move || {
            wait_for_mutex_guard(&mutex, Duration::from_millis(20))
                .expect("wait for mutex")
                .is_some()
        })
    };

    std::thread::sleep(Duration::from_millis(2));
    drop(guard);

    assert!(waiter.join().expect("waiter thread"));
}

#[test]
fn below_high_refresh_stability_tier_keeps_delta_frames_on_datagrams_by_default() {
    let stable_bitrate = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 120,
        bitrate_mbps: 80,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };

    assert!(!should_send_access_unit_as_reliable_frame(
        true,
        true,
        64,
        &stable_bitrate,
        None
    ));
}

#[test]
fn high_refresh_stability_tier_keeps_delta_frames_on_datagrams_by_default() {
    let stability_tier = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 64,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };

    assert!(!should_send_access_unit_as_reliable_frame(
        true,
        true,
        64,
        &stability_tier,
        None
    ));
}

#[test]
fn ultra_high_bitrate_uses_reliable_whole_frame_by_default() {
    let ultra_high = MediaProfile {
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 120,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let render_capped = MediaProfile {
        width: 2560,
        height: 1600,
        fps: 144,
        bitrate_mbps: 120,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let high_refresh_2k180 = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 180,
        bitrate_mbps: 100,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let stable_2k144 = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };

    assert!(should_send_access_unit_as_reliable_frame(
        true,
        true,
        64,
        &ultra_high,
        None
    ));
    assert!(should_send_access_unit_as_reliable_frame(
        true,
        true,
        64,
        &render_capped,
        None
    ));
    assert!(should_send_access_unit_as_reliable_frame(
        true,
        true,
        64,
        &high_refresh_2k180,
        None
    ));
    assert!(!should_send_access_unit_as_reliable_frame(
        true,
        true,
        64,
        &stable_2k144,
        None
    ));
    assert!(!should_send_access_unit_as_reliable_frame(
        false,
        true,
        64,
        &ultra_high,
        None
    ));
    assert!(!should_send_access_unit_as_reliable_frame(
        true,
        false,
        64,
        &ultra_high,
        None
    ));
}

#[test]
fn reliable_whole_frame_requires_explicit_override() {
    let high_quality_2k120 = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 120,
        bitrate_mbps: 80,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };

    assert!(!should_send_access_unit_as_reliable_frame(
        true,
        true,
        64,
        &high_quality_2k120,
        None
    ));
    assert!(should_send_access_unit_as_reliable_frame(
        true,
        true,
        64,
        &high_quality_2k120,
        Some(true)
    ));
    assert!(!should_send_access_unit_as_reliable_frame(
        true,
        true,
        64,
        &high_quality_2k120,
        Some(false)
    ));
    assert!(!should_send_access_unit_as_reliable_frame(
        false,
        true,
        64,
        &high_quality_2k120,
        None
    ));
}

#[test]
fn lan_quic_reliable_keyframe_fragments_match_datagram_fragments() {
    let payload = vec![0x33; 4096];
    let fragments = fragment_access_unit(42, 12_345, true, &payload, 1_200).unwrap();
    assert!(fragments.len() > 1);

    let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig {
        frame_timeout: Duration::from_secs(1),
        max_pending_frames: 8,
    });

    assert!(reassembler.push_datagram(&fragments[0]).unwrap().is_none());
    assert!(reassembler.push_datagram(&fragments[0]).unwrap().is_none());

    let mut completed = None;
    for fragment in fragments.iter().skip(1) {
        completed = reassembler.push_datagram(fragment).unwrap();
    }

    let frame = completed.expect("keyframe should complete after all fragments");
    assert_eq!(frame.frame_id, 42);
    assert!(frame.is_keyframe);
    assert_eq!(frame.payload.as_ref(), payload.as_slice());
    assert_eq!(reassembler.stats().duplicate_fragments, 1);
}

#[test]
fn lan_sender_treats_h264_idr_payload_as_keyframe() {
    let idr_annexb = [0, 0, 0, 1, 0x65, 0x88, 0x84];
    let p_slice_annexb = [0, 0, 1, 0x41, 0x9a];
    let idr_avcc = [0, 0, 0, 3, 0x65, 0x88, 0x84];

    assert!(h264_access_unit_is_keyframe(false, &idr_annexb));
    assert!(h264_access_unit_is_keyframe(false, &idr_avcc));
    assert!(!h264_access_unit_is_keyframe(false, &p_slice_annexb));
    assert!(h264_access_unit_is_keyframe(true, &p_slice_annexb));
}

#[test]
fn decoded_frame_to_rgb24_accepts_nv12_decoder_output() {
    let frame = DecodedFrame {
        width: 2,
        height: 2,
        timestamp_us: 0,
        data: DecodedFrameData::CpuNv12 {
            data: vec![235, 235, 235, 235, 128, 128],
            pitch: 2,
        },
    };

    let (width, height, rgb) = decoded_frame_to_rgb24(frame).unwrap();

    assert_eq!((width, height), (2, 2));
    assert_eq!(rgb.len(), 2 * 2 * 3);
    assert!(rgb.iter().all(|channel| *channel >= 250));
}

#[test]
fn media_frame_scheduler_does_not_add_processing_time_to_interval() {
    let start = Instant::now();
    let interval = Duration::from_millis(16);
    let mut next_frame_at = start;

    assert_eq!(
        schedule_next_media_frame(start, &mut next_frame_at, interval),
        None
    );
    assert_eq!(next_frame_at, start + interval);

    assert_eq!(
        schedule_next_media_frame(
            start + Duration::from_millis(15),
            &mut next_frame_at,
            interval
        ),
        Some(start + interval)
    );
    assert_eq!(next_frame_at, start + interval + interval);
}

#[test]
fn media_frame_scheduler_resets_after_large_stall() {
    let start = Instant::now();
    let interval = Duration::from_millis(16);
    let mut next_frame_at = start + interval;
    let now = start + Duration::from_millis(80);

    assert_eq!(
        schedule_next_media_frame(now, &mut next_frame_at, interval),
        None
    );
    assert_eq!(next_frame_at, now + interval);
}

#[test]
fn high_refresh_media_profiles_request_high_resolution_timer() {
    let high_refresh = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let low_refresh = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 30,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    assert!(media_profile_requests_high_resolution_timer(&high_refresh));
    assert!(!media_profile_requests_high_resolution_timer(&low_refresh));
}

#[test]
fn high_refresh_media_profiles_use_precise_sleep_guard() {
    let high_refresh = MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    let low_refresh = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    let guard = media_frame_precise_sleep_guard(&high_refresh);

    assert!(guard > Duration::ZERO);
    assert!(guard < media_frame_interval(&high_refresh));
    assert_eq!(
        media_frame_precise_sleep_guard(&low_refresh),
        Duration::ZERO
    );
    assert_eq!(
        media_frame_precise_sleep_chunk(Duration::from_millis(5), guard),
        Some(Duration::from_millis(1))
    );
    assert_eq!(media_frame_precise_sleep_chunk(guard, guard), None);
}

#[tokio::test]
async fn media_profile_update_changes_active_quic_session_profile() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("profile-update-session".to_string());
    app_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );
    app_state
        .peer_media_capabilities
        .lock()
        .await
        .set(session_id.clone(), vec!["decode.software".to_string()]);

    let negotiation = accept_lan_media_profile_update(
        &app_state,
        &session_id,
        MediaProfile {
            width: 1280,
            height: 720,
            fps: 60,
            bitrate_mbps: 8,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(negotiation.status, "accepted");
    assert_eq!(negotiation.selected.width, 1280);
    assert_eq!(
        app_state
            .media_profiles
            .lock()
            .await
            .get(&session_id)
            .expect("profile update result")
            .selected
            .height,
        720
    );
}

#[cfg(windows)]
#[tokio::test]
async fn d3d11_shared_preview_failure_still_counts_decoded_frame() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("d3d11-shared-preview-session".to_string());
    let frame = DecodedFrame::from_d3d11_shared_nv12(1920, 1080, 123_456, 1, 2);
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    record_lan_decoded_frames(
        &app_state,
        &session_id,
        vec![frame],
        1024,
        60,
        123_456,
        &profile,
        &[1, 2, 3, 4],
    )
    .await;

    let snapshot = app_state.probes.lock().await.snapshot(&session_id);

    assert_eq!(snapshot.frames_decoded, 1);
    assert_eq!(snapshot.last_media_sequence, Some(60));
    assert!(snapshot.latest_frame_data_url.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn upload_lan_render_frame_dispatches_macos_compressed_access_units() {
    use mrd_render::{RenderError, RenderTarget, RendererInstance, RendererSnapshot};

    #[derive(Default)]
    struct CompressedDispatchRenderer {
        decoded_uploads: u64,
        h264_upload: Option<(usize, usize, u64, Vec<u8>)>,
        hevc_upload: Option<(usize, usize, u64, Vec<u8>)>,
    }

    impl RendererInstance for CompressedDispatchRenderer {
        fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
            Ok(())
        }

        fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
            self.decoded_uploads += 1;
            Ok(())
        }

        fn upload_h264_access_unit(
            &mut self,
            width: usize,
            height: usize,
            timestamp_us: u64,
            payload: bytes::Bytes,
        ) -> Result<(), RenderError> {
            self.h264_upload = Some((width, height, timestamp_us, payload.to_vec()));
            Ok(())
        }

        fn upload_hevc_access_unit(
            &mut self,
            width: usize,
            height: usize,
            timestamp_us: u64,
            payload: bytes::Bytes,
        ) -> Result<(), RenderError> {
            self.hevc_upload = Some((width, height, timestamp_us, payload.to_vec()));
            Ok(())
        }

        fn snapshot(&self) -> RendererSnapshot {
            RendererSnapshot {
                attached_to_target: true,
                uploaded_frame_count: self.decoded_uploads,
                presented_frame_count: self.decoded_uploads,
                present_skipped_count: 0,
                render_queue_replacements: None,
                last_present_status: None,
                low_latency_frame_latency_target: None,
                swap_chain_max_frame_latency: None,
                swap_chain_allow_tearing: None,
                swap_chain_waitable_object: None,
                swap_chain_present_mode: None,
                display_refresh_hz: None,
                render_thread_priority: None,
                waitable_wait_count: None,
                waitable_wait_total_ms: None,
                waitable_timeout_count: None,
                last_waitable_wait_ms: None,
                last_render_prepare_wait_ms: None,
                last_render_shared_resource_ms: None,
                last_render_wait_for_drawable_ms: None,
                last_render_encode_commit_ms: None,
                last_render_draw_present_ms: None,
                last_width: 0,
                last_height: 0,
                last_pixel_format: None,
            }
        }
    }

    let mut renderer = CompressedDispatchRenderer::default();

    upload_lan_render_frame(
        &mut renderer,
        MediaRenderFrame::H264AccessUnit {
            width: 640,
            height: 360,
            timestamp_us: 123,
            payload: bytes::Bytes::from_static(b"h264-au"),
        },
    )
    .expect("dispatch H.264 access unit");
    upload_lan_render_frame(
        &mut renderer,
        MediaRenderFrame::HevcAccessUnit {
            width: 1280,
            height: 720,
            timestamp_us: 456,
            payload: bytes::Bytes::from_static(b"hevc-au"),
        },
    )
    .expect("dispatch HEVC access unit");

    assert_eq!(renderer.decoded_uploads, 0);
    assert_eq!(
        renderer.h264_upload,
        Some((640, 360, 123, b"h264-au".to_vec()))
    );
    assert_eq!(
        renderer.hevc_upload,
        Some((1280, 720, 456, b"hevc-au".to_vec()))
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn macos_compressed_proxy_requires_surface_before_claiming_access_units() {
    use mrd_render::{RenderError, RenderTarget, RendererInstance, RendererSnapshot};

    struct NoopSurfaceRenderer;

    impl RendererInstance for NoopSurfaceRenderer {
        fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
            Ok(())
        }

        fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
            Ok(())
        }

        fn upload_h264_access_unit(
            &mut self,
            _width: usize,
            _height: usize,
            _timestamp_us: u64,
            _payload: bytes::Bytes,
        ) -> Result<(), RenderError> {
            Ok(())
        }

        fn upload_hevc_access_unit(
            &mut self,
            _width: usize,
            _height: usize,
            _timestamp_us: u64,
            _payload: bytes::Bytes,
        ) -> Result<(), RenderError> {
            Ok(())
        }

        fn snapshot(&self) -> RendererSnapshot {
            RendererSnapshot {
                attached_to_target: true,
                uploaded_frame_count: 0,
                presented_frame_count: 0,
                present_skipped_count: 0,
                render_queue_replacements: None,
                last_present_status: None,
                low_latency_frame_latency_target: None,
                swap_chain_max_frame_latency: None,
                swap_chain_allow_tearing: None,
                swap_chain_waitable_object: None,
                swap_chain_present_mode: None,
                display_refresh_hz: None,
                render_thread_priority: None,
                waitable_wait_count: None,
                waitable_wait_total_ms: None,
                waitable_timeout_count: None,
                last_waitable_wait_ms: None,
                last_render_prepare_wait_ms: None,
                last_render_shared_resource_ms: None,
                last_render_wait_for_drawable_ms: None,
                last_render_encode_commit_ms: None,
                last_render_draw_present_ms: None,
                last_width: 0,
                last_height: 0,
                last_pixel_format: None,
            }
        }
    }

    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("macos-compressed-surface-gate".to_string());
    let profile = MediaProfile {
        width: 1280,
        height: 720,
        fps: 60,
        bitrate_mbps: 8,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };

    assert!(!macos_render_proxy_compressed_media_surface_available(&app_state, &session_id).await);
    assert!(!render_lan_h264_access_unit_frame(
        &app_state,
        &session_id,
        bytes::Bytes::from_static(b"h264-au"),
        1,
        123,
        &profile,
    )
    .await
    .expect("missing surface should not error"));
    assert_eq!(
        app_state
            .media_render_queues
            .lock()
            .await
            .pending_depth(&session_id),
        0
    );
    assert_eq!(
        app_state
            .probes
            .lock()
            .await
            .snapshot(&session_id)
            .frames_decoded,
        0
    );

    app_state
        .media_surface_renderers
        .lock()
        .await
        .insert_renderer_for_test(&session_id, "surface-1", Box::new(NoopSurfaceRenderer));

    assert!(macos_render_proxy_compressed_media_surface_available(&app_state, &session_id).await);
    assert!(render_lan_h264_access_unit_frame(
        &app_state,
        &session_id,
        bytes::Bytes::from_static(b"h264-au"),
        2,
        456,
        &profile,
    )
    .await
    .expect("surface should accept compressed proxy frame"));
    assert_eq!(
        app_state
            .probes
            .lock()
            .await
            .snapshot(&session_id)
            .frames_decoded,
        1
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[tokio::test]
async fn d3d11_present_skip_is_not_counted_as_presented_frame() {
    use mrd_render::{RenderError, RenderTarget, RendererInstance, RendererSnapshot};

    struct PresentSkipRenderer {
        uploaded: u64,
        skipped: u64,
    }

    impl RendererInstance for PresentSkipRenderer {
        fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
            Ok(())
        }

        fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
            self.uploaded += 1;
            self.skipped += 1;
            Ok(())
        }

        fn snapshot(&self) -> RendererSnapshot {
            RendererSnapshot {
                attached_to_target: true,
                uploaded_frame_count: self.uploaded,
                presented_frame_count: 0,
                present_skipped_count: self.skipped,
                render_queue_replacements: None,
                last_present_status: Some("skipped_still_drawing".to_string()),
                low_latency_frame_latency_target: None,
                swap_chain_max_frame_latency: None,
                swap_chain_allow_tearing: None,
                swap_chain_waitable_object: None,
                swap_chain_present_mode: None,
                display_refresh_hz: None,
                render_thread_priority: None,
                waitable_wait_count: None,
                waitable_wait_total_ms: None,
                waitable_timeout_count: None,
                last_waitable_wait_ms: None,
                last_render_prepare_wait_ms: None,
                last_render_shared_resource_ms: None,
                last_render_wait_for_drawable_ms: None,
                last_render_encode_commit_ms: None,
                last_render_draw_present_ms: None,
                last_width: 1,
                last_height: 1,
                last_pixel_format: None,
            }
        }
    }

    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("present-skip-session".to_string());
    app_state
        .media_surface_renderers
        .lock()
        .await
        .insert_renderer_for_test(
            &session_id,
            "surface-1",
            Box::new(PresentSkipRenderer {
                uploaded: 0,
                skipped: 0,
            }),
        );

    let outcome = render_lan_frame_once(
        app_state,
        session_id,
        MediaRenderFrame::Decoded(RenderFrame::from_bgra32(1, 1, vec![0, 0, 0, 255])),
    )
    .await
    .expect("render one frame");

    match outcome {
        LanRenderTaskOutcome::Rendered {
            presented_frames,
            present_skips,
            ..
        } => {
            assert_eq!(presented_frames, 0);
            assert_eq!(present_skips, 1);
        }
        other => panic!("unexpected render outcome: {other:?}"),
    }
}

#[tokio::test]
async fn media_profile_update_preserves_selected_capture_source_aspect_ratio() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("profile-update-aspect-session".to_string());
    app_state.sessions.lock().await.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller-device".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        },
    );
    app_state
        .peer_media_capabilities
        .lock()
        .await
        .set(session_id.clone(), vec!["decode.software".to_string()]);
    app_state.capture_sources.lock().await.set(
        session_id.clone(),
        CaptureSourceSelection {
            session_id: session_id.clone(),
            source: mrd_ipc::CaptureSource {
                id: "windows:display-shared:0".to_string(),
                platform: "windows".to_string(),
                source_kind: "display_shared".to_string(),
                title: "Display 1".to_string(),
                class_name: "WinRTMonitorShared".to_string(),
                width: 2560,
                height: 1600,
                process_id: 0,
                app_name: Some("Display".to_string()),
                bundle_identifier: None,
                preview_data_url: None,
                preview_width: None,
                preview_height: None,
            },
            status: "selected".to_string(),
            reason: None,
        },
    );

    let negotiation = accept_lan_media_profile_update(
        &app_state,
        &session_id,
        MediaProfile {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(
        negotiation.selected_source_id.as_deref(),
        Some("windows:display-shared:0")
    );
    assert_eq!(negotiation.selected.width, 1728);
    assert_eq!(negotiation.selected.height, 1080);
    assert_eq!(negotiation.selected.fps, 144);
    assert_eq!(negotiation.selected_width, Some(1728));
    assert_eq!(negotiation.selected_height, Some(1080));
    assert_eq!(negotiation.status, "downgraded");
    assert_eq!(
        negotiation.downgrade_reason.as_deref(),
        Some("matched selected capture source dimensions and aspect ratio")
    );
}

#[tokio::test]
async fn capture_source_reselection_can_restore_requested_profile_after_display_mode_change() {
    let app_state = Arc::new(AppState::new());
    let session_id = SessionId("capture-source-restore-session".to_string());
    app_state
        .sessions
        .lock()
        .await
        .insert(session_id.clone(), sender_snapshot(&session_id));
    app_state.media_profiles.lock().await.set(
        session_id.clone(),
        negotiate_media_profile(Some(MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        }))
        .unwrap(),
    );

    let source_before_mode_change = mrd_ipc::CaptureSource {
        id: "windows:display-shared:0".to_string(),
        platform: "windows".to_string(),
        source_kind: "display_shared".to_string(),
        title: "Display 1".to_string(),
        class_name: "WinRTMonitorShared".to_string(),
        width: 2560,
        height: 1600,
        process_id: 0,
        app_name: Some("Display".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    };
    accept_lan_capture_source_select_from_sources(
        &app_state,
        &session_id,
        "windows:display-shared:0",
        vec![source_before_mode_change],
    )
    .await
    .unwrap();
    assert_eq!(
        app_state
            .media_profiles
            .lock()
            .await
            .get(&session_id)
            .expect("profile after first source")
            .selected
            .width,
        1728
    );

    let source_after_mode_change = mrd_ipc::CaptureSource {
        id: "windows:display-shared:0".to_string(),
        platform: "windows".to_string(),
        source_kind: "display_shared".to_string(),
        title: "Display 1".to_string(),
        class_name: "WinRTMonitorShared".to_string(),
        width: 1920,
        height: 1080,
        process_id: 0,
        app_name: Some("Display".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    };
    accept_lan_capture_source_select_from_sources(
        &app_state,
        &session_id,
        "windows:display-shared:0",
        vec![source_after_mode_change],
    )
    .await
    .unwrap();

    let negotiation = app_state
        .media_profiles
        .lock()
        .await
        .get(&session_id)
        .expect("profile after source refresh");
    assert_eq!(negotiation.selected.width, 1920);
    assert_eq!(negotiation.selected.height, 1080);
    assert_eq!(negotiation.selected_width, Some(1920));
    assert_eq!(negotiation.selected_height, Some(1080));
    assert_eq!(negotiation.status, "accepted");
    assert_eq!(negotiation.downgrade_reason, None);
}

#[test]
fn lan_capture_config_changes_when_profile_dimensions_change() {
    let source_id = "windows:display-shared:0";
    let before = MediaProfile {
        width: 1728,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let after = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    };
    let active = lan_capture_config_key(source_id, &before);

    assert!(lan_capture_config_matches(
        Some(&active),
        source_id,
        &before
    ));
    assert!(!lan_capture_config_matches(
        Some(&active),
        source_id,
        &after
    ));
}

fn sender_snapshot(session_id: &SessionId) -> SessionSnapshot {
    sender_snapshot_for_source(session_id, "controller-device")
}

fn sender_snapshot_for_source(session_id: &SessionId, source_device_id: &str) -> SessionSnapshot {
    SessionSnapshot {
        session_id: session_id.clone(),
        transport: "quic".to_string(),
        source_device_id: Some(DeviceId(source_device_id.to_string())),
        target_device_id: None,
        local_listen_addr: None,
        local_server_name: None,
        local_cert_der_b64: None,
        remote_listen_addr: None,
        remote_server_name: None,
        remote_cert_der_b64: None,
        lifecycle_state: SessionLifecycleState::Listening,
        last_error: None,
        sender_active: true,
        receiver_active: false,
    }
}

fn receiver_snapshot_for_target(session_id: &SessionId, target_device_id: &str) -> SessionSnapshot {
    SessionSnapshot {
        session_id: session_id.clone(),
        transport: "quic".to_string(),
        source_device_id: None,
        target_device_id: Some(DeviceId(target_device_id.to_string())),
        local_listen_addr: None,
        local_server_name: None,
        local_cert_der_b64: None,
        remote_listen_addr: None,
        remote_server_name: None,
        remote_cert_der_b64: None,
        lifecycle_state: SessionLifecycleState::Streaming,
        last_error: None,
        sender_active: false,
        receiver_active: true,
    }
}

fn test_window_capture_source(id: &str) -> CaptureSource {
    CaptureSource {
        id: id.to_string(),
        platform: "windows".to_string(),
        source_kind: "window".to_string(),
        title: "Target App".to_string(),
        class_name: "ApplicationFrameWindow".to_string(),
        width: 1280,
        height: 720,
        process_id: 4242,
        app_name: Some("Target App".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    }
}

fn test_display_capture_source(id: &str) -> CaptureSource {
    CaptureSource {
        id: id.to_string(),
        platform: "windows".to_string(),
        source_kind: "display_shared".to_string(),
        title: "Display 1".to_string(),
        class_name: "WinRTMonitorShared".to_string(),
        width: 1920,
        height: 1080,
        process_id: 0,
        app_name: Some("Display".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    }
}

fn display_mode(
    id: &str,
    width: u32,
    height: u32,
    refresh_hz: u32,
    is_current: bool,
) -> mrd_ipc::DisplayMode {
    mrd_ipc::DisplayMode {
        id: id.to_string(),
        source_id: Some("windows:display-shared:0".to_string()),
        width,
        height,
        refresh_hz,
        bit_depth: Some(32),
        is_current,
    }
}
