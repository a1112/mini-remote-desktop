/**
 * realtimeService tests
 *
 * Tests deprecation errors for removed commands and validates
 * that mrd-service lifecycle commands still work.
 *
 * Note: This service still directly calls invoke() for backward compatibility.
 * New code should use serviceLifecycleService which goes through the adapter.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke, resetTauriMock } from "../../test/mocks/tauri";

describe("realtimeService", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetTauriMock();
  });

  // ============================================================================
  // DEPRECATED: realtime_* commands (removed)
  // ============================================================================

  describe("deprecated realtime commands", () => {
    it("getRealtimeStatus throws deprecation error", async () => {
      const { getRealtimeStatus } = await import("./realtimeService");

      await expect(getRealtimeStatus()).rejects.toThrow(
        "realtime_status 命令已移除"
      );
    });

    it("startRealtime throws deprecation error", async () => {
      const { startRealtime } = await import("./realtimeService");

      await expect(startRealtime()).rejects.toThrow(
        "realtime_start 命令已移除"
      );
    });

    it("stopRealtime throws deprecation error", async () => {
      const { stopRealtime } = await import("./realtimeService");

      await expect(stopRealtime()).rejects.toThrow(
        "realtime_stop 命令已移除"
      );
    });

    it("restartRealtime throws deprecation error", async () => {
      const { restartRealtime } = await import("./realtimeService");

      await expect(restartRealtime()).rejects.toThrow(
        "realtime_restart 命令已移除"
      );
    });
  });

  // ============================================================================
  // NVDEC runtime probe (moved to mrd-service)
  // ============================================================================

  describe("nvdec runtime probe", () => {
    it("throws deprecation error when backend returns moved error", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockRejectedValue(
        new Error("nvdec_runtime_probe moved to mrd-service - use rdesk-legacy-harness for testing")
      );

      const { getNvdecRuntimeProbe } = await import("./realtimeService");

      await expect(getNvdecRuntimeProbe()).rejects.toThrow(
        "NVDEC runtime probe 已迁移到 mrd-service"
      );
    });

    it("re-throws other errors", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockRejectedValue(new Error("Unexpected error"));

      const { getNvdecRuntimeProbe } = await import("./realtimeService");

      await expect(getNvdecRuntimeProbe()).rejects.toThrow("Unexpected error");
    });

    it("returns nvdec probe data when available (legacy behavior)", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({
        backend: "windows-nvdec",
        summary: "nvdec runtime libraries and core exports are present",
        checked_items: ["nvcuda.dll", "nvcuvid.dll"],
        capability_probes: [
          {
            codec: "h264",
            bit_depth_minus8: 0,
            chroma_format: 1,
            runtime_supported: true,
            runtime_reason: "h264 8-bit chroma 1 runtime capability reported by nvdec",
            wired_supported: true,
            wired_reason: "decode path wired",
          },
        ],
      });

      const { getNvdecRuntimeProbe } = await import("./realtimeService");
      const result = await getNvdecRuntimeProbe();

      expect(result.backend).toBe("windows-nvdec");
      expect(result.capability_probes).toHaveLength(1);
    });
  });

  // ============================================================================
  // Decode policy (managed by mrd-service)
  // ============================================================================

  describe("decode policy", () => {
    it("throws deprecation error when backend returns IPC error", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockRejectedValue(
        new Error("Use IPC to query decode policy from mrd-service")
      );

      const { getDecodePolicy } = await import("./realtimeService");

      await expect(getDecodePolicy()).rejects.toThrow(
        "Decode policy 现在由 mrd-service 管理"
      );
    });

    it("setDecodePolicy still works (saves to settings file)", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue({ decode_policy: "nvdec" });

      const { setDecodePolicy } = await import("./realtimeService");
      const result = await setDecodePolicy("nvdec");

      expect(result.decode_policy).toBe("nvdec");
    });
  });

  // ============================================================================
  // mrd-service lifecycle commands (new)
  // ============================================================================

  describe("mrd-service lifecycle commands", () => {
    it("checks service status", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      const { serviceStatus } = await import("./realtimeService");
      const result = await serviceStatus();

      expect(result).toBe(true);
    });

    it("starts the service", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      const { serviceStart } = await import("./realtimeService");
      const result = await serviceStart();

      expect(result).toBe(true);
    });

    it("stops the service", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      const { serviceStop } = await import("./realtimeService");
      const result = await serviceStop();

      expect(result).toBe(true);
    });

    it("restarts the service", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      const { serviceRestart } = await import("./realtimeService");
      const result = await serviceRestart();

      expect(result).toBe(true);
    });

    it("gets service PID", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(12345);

      const { servicePid } = await import("./realtimeService");
      const result = await servicePid();

      expect(result).toBe(12345);
    });

    it("performs health check", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      const { serviceHealthCheck } = await import("./realtimeService");
      const result = await serviceHealthCheck();

      expect(result).toBe(true);
    });

    it("waits for service to be healthy", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      const { serviceWaitForHealthy } = await import("./realtimeService");
      const result = await serviceWaitForHealthy(30);

      expect(result).toBe(true);
    });

    it("restarts service with backoff", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      const { serviceRestartWithBackoff } = await import("./realtimeService");
      const result = await serviceRestartWithBackoff(3);

      expect(result).toBe(true);
    });

    it("starts service guard", async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue("Guard started with handle: 1");

      const { serviceStartGuard } = await import("./realtimeService");
      const result = await serviceStartGuard();

      expect(result).toBe("Guard started with handle: 1");
    });
  });
});
