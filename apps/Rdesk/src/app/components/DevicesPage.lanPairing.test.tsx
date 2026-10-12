import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "@/test/mocks/tauri";
import { DevicesPage } from "./DevicesPage";
import type { Device } from './deviceData';
import { Monitor } from 'lucide-react';

const deviceListState = vi.hoisted(() => ({ loading: false, error: null as string | null, devices: [] as Device[] }));
const launch = vi.hoisted(() => vi.fn());
vi.mock('../services/remoteDisplayLauncher', () => ({ launchRemoteDisplayForDevice: launch, launchLocalRemoteDisplayTest: vi.fn() }));

vi.mock("react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("./ThemeContext", () => ({ useTheme: () => ({ isDark: false }) }));
vi.mock("./deviceData", () => ({
  useDevices: () => ({
    ...deviceListState, refresh: vi.fn(),
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
    deviceListState.devices = [];
    launch.mockReset();
    launch.mockResolvedValue({ mode: 'route', sessionId: 'paired-device' });
    getMockInvoke().mockResolvedValue({ type: "LanPairingCandidateList", candidates: [] });
  });
  it("offers an explicit pairing entry before any remote session is started", () => {
    render(<DevicesPage />);
    expect(screen.getByRole("button", { name: "局域网配对" })).toBeInTheDocument();
  });
  it('passes the explicit paired choice from the device list to the authenticated launcher', async () => {
    deviceListState.devices = [{ id: 'peer', deviceId: '821456789', name: 'Remembered PC', os: 'Windows 11', icon: Monitor,
      status: 'online', isLocal: false, p2pAvailable: false, ping: null, cpu: null, ram: null, disk: null,
      location: '', lastSeen: '', ip: '', group: '', favorite: false, discoverySources: ['server'], primarySource: 'server', sourceLabel: '服务器', serverAvailable: true }];
    render(<DevicesPage />);
    expect(screen.getByRole('combobox', { name: '连接授权方式' })).toHaveValue('attended');
    fireEvent.change(screen.getByRole('combobox', { name: '连接授权方式' }), { target: { value: 'paired' } });
    fireEvent.click(screen.getByRole('button', { name: 'P2P 连接' }));
    await vi.waitFor(() => expect(launch).toHaveBeenCalledWith('821456789', expect.objectContaining({ accessMode: 'paired' })));
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
