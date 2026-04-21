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
