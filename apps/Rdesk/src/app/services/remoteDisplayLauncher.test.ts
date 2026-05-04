import { beforeEach, describe, expect, it, vi } from "vitest";
import { launchRemoteDisplayForDevice } from "./remoteDisplayLauncher";

const mocks = vi.hoisted(() => ({
  openRemoteDisplayWindow: vi.fn(),
  startLanRemoteSession: vi.fn(),
  startSession: vi.fn(),
  saveWebRemoteSession: vi.fn(),
  getDeviceInfo: vi.fn(),
  initialize: vi.fn(),
}));

vi.mock("../adapters/tauri", () => ({
  openRemoteDisplayWindow: mocks.openRemoteDisplayWindow,
}));

vi.mock("../utils/runtime", () => ({
  isTauriRuntime: () => true,
}));

vi.mock("./deviceService", () => ({
  deviceService: {
    getDeviceInfo: mocks.getDeviceInfo,
    initialize: mocks.initialize,
  },
}));

vi.mock("./ipcSessionService", () => ({
  startLanRemoteSession: mocks.startLanRemoteSession,
  startSession: mocks.startSession,
}));

vi.mock("./webRemoteSessionService", () => ({
  saveWebRemoteSession: mocks.saveWebRemoteSession,
}));

describe("launchRemoteDisplayForDevice", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.openRemoteDisplayWindow.mockResolvedValue({
      ok: true,
      value: { label: "render-local-display-test-1" },
    });
    mocks.startLanRemoteSession.mockResolvedValue("p2p-quic-session");
    mocks.startSession.mockResolvedValue("service-session");
    mocks.getDeviceInfo.mockReturnValue({
      device_id: "local-device",
      device_name: "Local PC",
    });
    mocks.initialize.mockResolvedValue({
      device_id: "local-device",
      device_name: "Local PC",
    });
  });

  it("starts a local E2E display flow when the selected target is this device", async () => {
    const result = await launchRemoteDisplayForDevice("local-device", {
      sessionId: "local-display-test-explicit",
      transportKind: "quic",
      lanP2P: true,
    });

    expect(mocks.startSession).not.toHaveBeenCalled();
    expect(mocks.startLanRemoteSession).not.toHaveBeenCalled();
    expect(mocks.openRemoteDisplayWindow).toHaveBeenCalledWith({
      sessionId: "local-display-test-explicit",
    });
    expect(result).toEqual({
      sessionId: "local-display-test-explicit",
      windowLabel: "render-local-display-test-1",
      mode: "native_window",
    });
  });

  it("requests the default 2K144 QUIC media profile for LAN P2P remote display", async () => {
    await launchRemoteDisplayForDevice("remote-device", {
      sessionId: "p2p-quic-session",
      transportKind: "quic",
      lanP2P: true,
    });

    expect(mocks.startLanRemoteSession).toHaveBeenCalledWith(
      "p2p-quic-session",
      "remote-device",
      "quic",
      {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 64,
        codec: "h264",
      }
    );
  });
});
