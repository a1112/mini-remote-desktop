import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "@/test/mocks/tauri";
import { DevicesPage } from "./DevicesPage";

const deviceListState = vi.hoisted(() => ({ loading: false, error: null as string | null }));

vi.mock("react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("./ThemeContext", () => ({ useTheme: () => ({ isDark: false }) }));
vi.mock("./deviceData", () => ({
  useDevices: () => ({
    devices: [], ...deviceListState, refresh: vi.fn(),
    lastUpdated: null, currentDeviceId: "local-device",
  }),
}));
vi.mock("../hooks/useNetworkGroups", () => ({
  useNetworkGroups: () => ({ groups: [], refresh: vi.fn() }),
}));

describe("normal device page first LAN pairing", () => {
  beforeEach(() => {
    deviceListState.loading = false;
    deviceListState.error = null;
    getMockInvoke().mockResolvedValue({ type: "LanPairingCandidateList", candidates: [] });
  });
  it("offers an explicit pairing entry before any remote session is started", () => {
    render(<DevicesPage />);
    expect(screen.getByRole("button", { name: "局域网配对" })).toBeInTheDocument();
  });

  it.each([
    { loading: true, error: null },
    { loading: false, error: "公网 API 不可达" },
  ])("keeps LAN pairing reachable while the device list is unavailable: %j", async (state) => {
    Object.assign(deviceListState, state);
    render(<DevicesPage />);
    fireEvent.click(screen.getByRole("button", { name: "局域网配对" }));
    expect(await screen.findByText(/暂无可配对设备/)).toBeInTheDocument();
    expect(getMockInvoke()).toHaveBeenCalledWith("ipc_secure_remote", { request: { type: "ListLanPairingCandidates" } });
  });
});
