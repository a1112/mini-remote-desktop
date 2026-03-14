import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/tauri", () => ({
  invoke: invokeMock,
}));

describe("realtimeService", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("reads realtime status via tauri invoke", async () => {
    invokeMock.mockResolvedValue({
      running: true,
      reachable: true,
      status: "ok",
      pid: 9532,
    });

    const { getRealtimeStatus } = await import("./realtimeService");
    const result = await getRealtimeStatus();

    expect(invokeMock).toHaveBeenCalledWith("realtime_status");
    expect(result.running).toBe(true);
    expect(result.pid).toBe(9532);
  });

  it("reads nvdec runtime probe via tauri invoke", async () => {
    invokeMock.mockResolvedValue({
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
        {
          codec: "hevc",
          bit_depth_minus8: 2,
          chroma_format: 1,
          runtime_supported: false,
          runtime_reason: "hevc main10 unsupported by runtime",
          wired_supported: false,
          wired_reason: "HEVC Main10 not wired yet",
        },
      ],
    });

    const { getNvdecRuntimeProbe } = await import("./realtimeService");
    const result = await getNvdecRuntimeProbe();

    expect(invokeMock).toHaveBeenCalledWith("nvdec_runtime_probe");
    expect(result.backend).toBe("windows-nvdec");
    expect(result.capability_probes).toHaveLength(2);
    expect(result.capability_probes[1].bit_depth_minus8).toBe(2);
  });

  it("reads and updates decode policy via tauri invoke", async () => {
    invokeMock
      .mockResolvedValueOnce({ decode_policy: "auto" })
      .mockResolvedValueOnce({ decode_policy: "nvdec" });

    const {
      getDecodePolicy,
      setDecodePolicy,
    } = await import("./realtimeService");
    const initial = await getDecodePolicy();
    const updated = await setDecodePolicy("nvdec");

    expect(invokeMock).toHaveBeenNthCalledWith(1, "decode_policy");
    expect(invokeMock).toHaveBeenNthCalledWith(2, "set_decode_policy", {
      decodePolicy: "nvdec",
    });
    expect(initial.decode_policy).toBe("auto");
    expect(updated.decode_policy).toBe("nvdec");
  });

  it("restarts realtime sidecar via tauri invoke", async () => {
    invokeMock.mockResolvedValue({
      running: true,
      reachable: true,
      status: "ok",
      pid: 9532,
    });

    const { restartRealtime } = await import("./realtimeService");
    const result = await restartRealtime();

    expect(invokeMock).toHaveBeenCalledWith("realtime_restart");
    expect(result.status).toBe("ok");
  });
});
