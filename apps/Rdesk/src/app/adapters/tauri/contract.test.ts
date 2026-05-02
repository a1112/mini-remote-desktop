/**
 * Tauri Adapter Contract Tests
 *
 * Validates that the adapter interface matches the Tauri shell commands.
 * This is Layer 2 of the testing architecture - Adapter Contract Tests.
 *
 * If a command is removed/renamed in main.rs, this test should fail.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import * as adapter from './index';

describe('Tauri Adapter Contract', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('window and tray commands', () => {
    it('frameless window commands call registered command names', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(undefined);

      await adapter.startDragWindow();
      await adapter.minimizeWindow();
      await adapter.hideToTray();
      await adapter.showWindow();
      await adapter.centerWindow();
      await adapter.closeWindow();

      expect(mockInvoke).toHaveBeenNthCalledWith(1, 'start_drag_window', undefined);
      expect(mockInvoke).toHaveBeenNthCalledWith(2, 'minimize_window', undefined);
      expect(mockInvoke).toHaveBeenNthCalledWith(3, 'hide_to_tray', undefined);
      expect(mockInvoke).toHaveBeenNthCalledWith(4, 'show_window', undefined);
      expect(mockInvoke).toHaveBeenNthCalledWith(5, 'center_window', undefined);
      expect(mockInvoke).toHaveBeenNthCalledWith(6, 'close_window', undefined);
    });

    it('toggle_maximize_window returns the new maximized state', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      const result = await adapter.toggleMaximizeWindow();

      expect(mockInvoke).toHaveBeenCalledWith('toggle_maximize_window', undefined);
      expect(result.ok && result.value).toBe(true);
    });

    it('window chrome commands pass expected arguments', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke
        .mockResolvedValueOnce(undefined)
        .mockResolvedValueOnce({
          platform: 'Windows',
          effect: 'Mica',
          applied: true,
          detail: 'Native backdrop applied',
        });

      await adapter.setWindowDecorations(false);
      await adapter.applyNativeChrome();

      expect(mockInvoke).toHaveBeenNthCalledWith(1, 'set_window_decorations', {
        decorated: false,
      });
      expect(mockInvoke).toHaveBeenNthCalledWith(2, 'apply_native_chrome', undefined);
    });

    it('remote display window commands call registered command names', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({});

      await adapter.openRemoteDisplayWindow({
        sessionId: 'session-1',
        surfaceId: 'surface-1',
      });
      await adapter.listRemoteDisplayWindows('session-1');
      await adapter.currentRemoteDisplayWindowContext();
      await adapter.configureRemoteDisplayNativeSurface({
        enabled: true,
        rect: { x: 0, y: 44, width: 1280, height: 720 },
      });
      await adapter.presentTestHarnessFrameOnNativeSurface();
      await adapter.closeRemoteDisplayWindow('render-session-1-1');

      expect(mockInvoke).toHaveBeenNthCalledWith(1, 'open_remote_display_window', {
        sessionId: 'session-1',
        surfaceId: 'surface-1',
      });
      expect(mockInvoke).toHaveBeenNthCalledWith(2, 'list_remote_display_windows', {
        sessionId: 'session-1',
      });
      expect(mockInvoke).toHaveBeenNthCalledWith(
        3,
        'current_remote_display_window_context',
        undefined
      );
      expect(mockInvoke).toHaveBeenNthCalledWith(
        4,
        'configure_remote_display_native_surface',
        {
          enabled: true,
          rect: { x: 0, y: 44, width: 1280, height: 720 },
        }
      );
      expect(mockInvoke).toHaveBeenNthCalledWith(
        5,
        'present_test_harness_frame_on_native_surface',
        undefined
      );
      expect(mockInvoke).toHaveBeenNthCalledWith(6, 'close_remote_display_window', {
        label: 'render-session-1-1',
      });
    });

    it('diagnostic commands call registered command names', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke
        .mockResolvedValueOnce({
          app_pid: 1,
          app_exe_path: 'C:/mrd/app.exe',
          current_dir: 'C:/mrd',
          log_dir: 'C:/logs',
          service_exe_path: 'C:/mrd/mrd-service.exe',
          service_stdout_log: 'C:/logs/mrd-service.stdout.log',
          service_stderr_log: 'C:/logs/mrd-service.stderr.log',
        })
        .mockResolvedValueOnce(undefined);

      await adapter.getClientDiagnostics();
      await adapter.openDiagnosticsFolder();

      expect(mockInvoke).toHaveBeenNthCalledWith(1, 'get_client_diagnostics', undefined);
      expect(mockInvoke).toHaveBeenNthCalledWith(2, 'open_diagnostics_folder', undefined);
    });

    it('automation report command calls registered command name', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue('C:/tmp/lan-e2e-report.json');

      await adapter.automationWriteReport({
        scenarioId: 'lan.e2e.remote_display',
        status: 'completed',
      });

      expect(mockInvoke).toHaveBeenCalledWith('automation_write_report', {
        report: {
          scenarioId: 'lan.e2e.remote_display',
          status: 'completed',
        },
      });
    });
  });

  /**
   * Bootstrap and shell lifecycle commands
   */
  describe('service lifecycle commands', () => {
    it('service_bootstrap_if_needed calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      await adapter.serviceBootstrapIfNeeded();

      expect(mockInvoke).toHaveBeenCalledWith('service_bootstrap_if_needed', undefined);
    });

    it('service_start compatibility shim bootstraps via the new command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      await adapter.serviceStart();

      expect(mockInvoke).toHaveBeenCalledWith('service_bootstrap_if_needed', undefined);
    });

    it('service_wait_for_healthy calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      await adapter.serviceWaitForHealthy(30);

      expect(mockInvoke).toHaveBeenCalledWith('service_wait_for_healthy', {
        timeoutSecs: 30,
      });
    });

    it('service_did_bootstrap calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      await adapter.serviceDidBootstrap();

      expect(mockInvoke).toHaveBeenCalledWith('service_did_bootstrap', undefined);
    });

    it('shell_get_status calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({
        service_pid: 12345,
        ui_pid: 54321,
        tray_available: true,
        autostart_enabled: true,
        active_session_count: 0,
        last_error: null,
      });

      await adapter.shellGetStatus();

      expect(mockInvoke).toHaveBeenCalledWith('shell_get_status', undefined);
    });

    it('shell_shutdown_service calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(null);

      await adapter.shellShutdownService('graceful');

      expect(mockInvoke).toHaveBeenCalledWith('shell_shutdown_service', {
        mode: 'graceful',
      });
    });

    it('deprecated lifecycle wrappers return errors instead of calling removed commands', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      const stopResult = await adapter.serviceStop();
      const statusResult = await adapter.serviceStatus();
      const healthResult = await adapter.serviceHealthCheck();
      const pidResult = await adapter.servicePid();
      const restartResult = await adapter.serviceRestart();
      const restartBackoffResult = await adapter.serviceRestartWithBackoff(3);
      const guardResult = await adapter.serviceStartGuard();

      expect(mockInvoke).not.toHaveBeenCalledWith('service_stop', undefined);
      expect(stopResult.ok).toBe(false);
      expect(statusResult.ok).toBe(false);
      expect(healthResult.ok).toBe(false);
      expect(pidResult.ok).toBe(false);
      expect(restartResult.ok).toBe(false);
      expect(restartBackoffResult.ok).toBe(false);
      expect(guardResult.ok).toBe(false);
    });
  });

  /**
   * IPC Device commands
   */
  describe('IPC device commands', () => {
    it('ipc_register_device calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue('device-123');

      await adapter.ipcRegisterDevice('device-123', 'My Device');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_register_device', {
        deviceId: 'device-123',
        deviceName: 'My Device',
      });
    });

    it('ipc_list_devices calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      const mockDevices = [
        { device_id: 'd1', device_name: 'Device 1' },
        { device_id: 'd2', device_name: 'Device 2' },
      ];
      mockInvoke.mockResolvedValue(mockDevices);

      await adapter.ipcListDevices();

      expect(mockInvoke).toHaveBeenCalledWith('ipc_list_devices', undefined);
    });

    it('LAN discovery commands call correct command names', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({
        enabled: true,
        running: true,
        discovery_port: 21116,
        instance_id: 'local',
        last_probe_ms: 1,
        peers: [],
      });

      await adapter.ipcLanDiscoverySnapshot();
      await adapter.ipcRefreshLanDiscovery();

      expect(mockInvoke).toHaveBeenNthCalledWith(1, 'ipc_lan_discovery_snapshot', undefined);
      expect(mockInvoke).toHaveBeenNthCalledWith(2, 'ipc_refresh_lan_discovery', undefined);
    });
  });

  /**
   * IPC Session commands
   */
  describe('IPC session commands', () => {
    it('ipc_start_session calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue('session-123');

      await adapter.ipcStartSession('session-123', 'device-456', 'webrtc');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_start_session', {
        sessionId: 'session-123',
        targetDeviceId: 'device-456',
        transportKind: 'webrtc',
      });
    });

    it('ipc_start_lan_remote_session calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue('session-123');

      await adapter.ipcStartLanRemoteSession('session-123', 'device-456', 'quic');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_start_lan_remote_session', {
        sessionId: 'session-123',
        targetDeviceId: 'device-456',
        transportKind: 'quic',
      });
    });

    it('ipc_accept_session calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue('session-123');

      await adapter.ipcAcceptSession('session-123', 'device-789');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_accept_session', {
        sessionId: 'session-123',
        sourceDeviceId: 'device-789',
      });
    });

    it('ipc_stop_session calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue('session-123');

      await adapter.ipcStopSession('session-123');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_stop_session', {
        sessionId: 'session-123',
      });
    });

    it('ipc_fail_session calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue('session-123');

      await adapter.ipcFailSession('session-123', 'transport lost');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_fail_session', {
        sessionId: 'session-123',
        reason: 'transport lost',
      });
    });

    it('ipc_recover_session calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue('session-123');

      await adapter.ipcRecoverSession('session-123');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_recover_session', {
        sessionId: 'session-123',
      });
    });

    it('ipc_session_snapshot calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      const mockSnapshot = {
        session_id: 'session-123',
        state: 'active',
        sender_active: true,
        receiver_active: true,
      };
      mockInvoke.mockResolvedValue(mockSnapshot);

      await adapter.ipcSessionSnapshot('session-123');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_session_snapshot', {
        sessionId: 'session-123',
      });
    });

    it('ipc_list_sessions calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue([]);

      await adapter.ipcListSessions();

      expect(mockInvoke).toHaveBeenCalledWith('ipc_list_sessions', undefined);
    });

    it('ipc_runtime_snapshot calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({
        sessions: [],
        device_id: null,
        is_registered: false,
      });

      await adapter.ipcRuntimeSnapshot();

      expect(mockInvoke).toHaveBeenCalledWith('ipc_runtime_snapshot', undefined);
    });

    it('ipc_probe_snapshot calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({
        session_id: 'session-123',
        frames_received: 0,
        frames_decoded: 0,
        frames_dropped: 0,
      });

      await adapter.ipcProbeSnapshot('session-123');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_probe_snapshot', {
        sessionId: 'session-123',
      });
    });
  });

  /**
   * IPC Media commands
   */
  describe('IPC media commands', () => {
    it('ipc_start_sender calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue('session-123');

      await adapter.ipcStartSender('session-123');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_start_sender', {
        sessionId: 'session-123',
      });
    });

    it('ipc_start_receiver calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue('session-123');

      await adapter.ipcStartReceiver('session-123');

      expect(mockInvoke).toHaveBeenCalledWith('ipc_start_receiver', {
        sessionId: 'session-123',
      });
    });
  });

  /**
   * Hardware and decode policy commands
   */
  describe('hardware and decode policy commands', () => {
    it('get_hardware_info calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      const mockHardware = {
        cpu_brand: 'Intel Core i7',
        cpu_cores: 8,
        memory_gb: 16,
        gpu_info: 'NVIDIA RTX 3080',
      };
      mockInvoke.mockResolvedValue(mockHardware);

      await adapter.getHardwareInfo();

      expect(mockInvoke).toHaveBeenCalledWith('get_hardware_info', undefined);
    });

    it('get_system_resource_snapshot calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({
        target_name: 'mrd-service',
        target_pid: 1234,
        target_found: true,
        cpu_usage_percent: 12,
        memory_used_mb: 8192,
        memory_total_mb: 32768,
        memory_usage_percent: 25,
        gpu_usage_percent: 8,
        gpu_memory_used_mb: 1024,
        gpu_memory_total_mb: 8192,
        gpu_metrics_available: true,
        gpu_metrics_scope: "system",
        network_rx_bps: 1024,
        network_tx_bps: 2048,
        network_metrics_available: true,
        network_metrics_scope: "system",
        sampled_at_ms: 1,
      });

      await adapter.getSystemResourceSnapshot();

      expect(mockInvoke).toHaveBeenCalledWith(
        'get_system_resource_snapshot',
        undefined
      );
    });

    it('nvdec_runtime_probe calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockRejectedValue(new Error('moved to mrd-service'));

      const result = await adapter.nvdecRuntimeProbe();

      expect(mockInvoke).toHaveBeenCalledWith('nvdec_runtime_probe', undefined);
      expect(result.ok).toBe(false);
    });

    it('decode_policy calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockRejectedValue(new Error('Use IPC'));

      const result = await adapter.decodePolicy();

      expect(mockInvoke).toHaveBeenCalledWith('decode_policy', undefined);
      expect(result.ok).toBe(false);
    });

    it('set_decode_policy calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({ decode_policy: 'nvdec' });

      await adapter.setDecodePolicy('nvdec');

      expect(mockInvoke).toHaveBeenCalledWith('set_decode_policy', {
        decodePolicy: 'nvdec',
      });
    });
  });

  /**
   * Legacy HTTP commands
   */
  describe('legacy HTTP commands', () => {
    it('register_device calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      const mockResponse = {
        device_id: 'dev-123',
        device_name: 'My Device',
        access_token: 'token-abc',
      };
      mockInvoke.mockResolvedValue(mockResponse);

      await adapter.registerDevice({
        motherboardSerial: 'sn-123',
        hostname: 'my-pc',
        osVersion: 'Windows 11',
      });

      expect(mockInvoke).toHaveBeenCalledWith('register_device', {
        motherboardSerial: 'sn-123',
        hostname: 'my-pc',
        osVersion: 'Windows 11',
        deviceName: undefined,
      });
    });

    it('check_device_registration calls correct command with args', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      await adapter.checkDeviceRegistration('sn-123');

      expect(mockInvoke).toHaveBeenCalledWith('check_device_registration', {
        motherboardSerial: 'sn-123',
      });
    });
  });

  /**
   * Legacy WebRTC commands
   */
  describe('legacy WebRTC commands', () => {
    it('webrtc_session_list_via_ipc calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      const mockSessions = ['session-1', 'session-2'];
      mockInvoke.mockResolvedValue(mockSessions);

      await adapter.webrtcSessionListViaIpc();

      expect(mockInvoke).toHaveBeenCalledWith('webrtc_session_list_via_ipc', undefined);
    });
  });

  /**
   * Test Workbench commands
   */
  describe('test workbench commands', () => {
    it('test_list_scenarios calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue([]);

      await adapter.testListScenarios();

      expect(mockInvoke).toHaveBeenCalledWith('test_list_scenarios', undefined);
    });

    it('test_start_run calls correct command with scenario and config', async () => {
      const mockInvoke = getMockInvoke();
      const config = {
        capture_type: 'dxgi' as const,
        encoder_type: 'openh264' as const,
        duration_ms: 5000,
      };
      mockInvoke.mockResolvedValue('run-1');

      await adapter.testStartRun({
        scenarioId: 'matrix',
        config,
      });

      expect(mockInvoke).toHaveBeenCalledWith('test_start_run', {
        scenarioId: 'matrix',
        config,
      });
    });

    it('test_list_window_capture_targets calls correct command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue([]);

      await adapter.testListWindowCaptureTargets();

      expect(mockInvoke).toHaveBeenCalledWith('test_list_window_capture_targets', undefined);
    });

    it('test_list_window_capture_targets_with_previews passes preview limit', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue([]);

      await adapter.testListWindowCaptureTargetsWithPreviews(12);

      expect(mockInvoke).toHaveBeenCalledWith(
        'test_list_window_capture_targets_with_previews',
        { limit: 12 }
      );
    });

    it('test_harness_set_custom calls custom harness command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(null);

      await adapter.testHarnessSetCustom({
        capture: 'dxgi',
        encoder: 'nvenc_h264',
        decoder: 'software',
      });

      expect(mockInvoke).toHaveBeenCalledWith('test_harness_set_custom', {
        capture: 'dxgi',
        encoder: 'nvenc_h264',
        decoder: 'software',
      });
    });

    it('test_harness_get_comparison_result calls comparison command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({});

      await adapter.testHarnessGetComparisonResult();

      expect(mockInvoke).toHaveBeenCalledWith(
        'test_harness_get_comparison_result',
        undefined
      );
    });

    it('test_get_run_metrics calls correct command with run id', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({});

      await adapter.testGetRunMetrics('run-1');

      expect(mockInvoke).toHaveBeenCalledWith('test_get_run_metrics', {
        runId: 'run-1',
      });
    });

    it('test_get_run_artifacts calls correct command with run id', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue([]);

      await adapter.testGetRunArtifacts('run-1');

      expect(mockInvoke).toHaveBeenCalledWith('test_get_run_artifacts', {
        runId: 'run-1',
      });
    });

    it('test preset commands call registered command names', async () => {
      const mockInvoke = getMockInvoke();
      const config = { encoder_type: 'openh264' as const };
      mockInvoke.mockResolvedValueOnce([]);
      mockInvoke.mockResolvedValueOnce('preset-1');
      mockInvoke.mockResolvedValueOnce(undefined);

      await adapter.testListPresets();
      await adapter.testSavePreset({
        name: 'OpenH264 smoke',
        description: 'Software encode smoke test',
        scenarioId: 'encode.openh264',
        config,
      });
      await adapter.testDeletePreset('preset-1');

      expect(mockInvoke).toHaveBeenNthCalledWith(1, 'test_list_presets', undefined);
      expect(mockInvoke).toHaveBeenNthCalledWith(2, 'test_save_preset', {
        name: 'OpenH264 smoke',
        description: 'Software encode smoke test',
        scenarioId: 'encode.openh264',
        config,
      });
      expect(mockInvoke).toHaveBeenNthCalledWith(3, 'test_delete_preset', {
        presetId: 'preset-1',
      });
    });
  });

  /**
   * Error handling
   */
  describe('error handling', () => {
    it('returns error result when invoke throws', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockRejectedValue(new Error('Command failed'));

      const result = await adapter.serviceBootstrapIfNeeded();

      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.error.message).toBe('Command failed');
      }
    });

    it('returns error result with string error', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockRejectedValue('String error');

      const result = await adapter.serviceBootstrapIfNeeded();

      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.error.message).toBe('String error');
      }
    });

    it('returns success result for successful command', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      const result = await adapter.serviceBootstrapIfNeeded();

      expect(result.ok).toBe(true);
      if (result.ok) {
        expect(result.value).toBe(true);
      }
    });
  });
});
