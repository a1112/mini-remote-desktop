import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { HomePage } from "./HomePage";

const mocks = vi.hoisted(() => ({
  launchRemoteDisplayForDevice: vi.fn(),
  navigate: vi.fn(),
  enrollmentRequired: false,
  refreshRegistration: vi.fn(),
  localDeviceId: "0123456789",
  native: true,
}));
vi.mock('../utils/runtime', () => ({ isTauriRuntime: () => mocks.native }));

vi.mock("react-router", () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock("./ThemeContext", () => ({
  useTheme: () => ({ isDark: false }),
}));

vi.mock("../services/deviceService", () => ({
  deviceService: { renameDevice: vi.fn() },
  useDeviceRegistration: () => ({
    deviceId: mocks.localDeviceId,
    deviceName: "Local PC",
    registrationError: mocks.enrollmentRequired ? "需要设备登记码，请向服务器管理员获取一次性登记码后注册" : null,
    refresh: mocks.refreshRegistration,
  }),
}));

vi.mock("./DeviceRegisterModal", () => ({
  DeviceRegisterModal: ({ isOpen, onSuccess }: { isOpen: boolean; onSuccess: () => void }) =>
    isOpen ? <button onClick={() => onSuccess()}>提交设备登记码</button> : null,
}));

vi.mock("../services/remoteDisplayLauncher", () => ({
  launchRemoteDisplayForDevice: mocks.launchRemoteDisplayForDevice,
}));

vi.mock("./deviceData", () => ({
  useDevices: () => ({
    devices: [{
      id: "device-1", name: "办公室电脑", deviceId: "821456789",
      os: "Windows 11", icon: () => null, status: "online", isLocal: false,
    }],
    loading: false,
  }),
}));

vi.mock("../services/connectionHistoryService", () => ({
  useConnectionHistory: () => [{
    sessionId: "previous-session", peerDeviceId: "821456789",
    role: "controller", startedAt: 1_000, endedAt: 2_000,
  }],
}));

describe("HomePage secure remote launch", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.enrollmentRequired = false;
    mocks.native = true;
    mocks.launchRemoteDisplayForDevice.mockResolvedValue({
      sessionId: "secure-session",
      windowLabel: null,
      mode: "route",
    });
  });

  it("offers reachable server enrollment and refreshes the displayed identity after success", async () => {
    mocks.enrollmentRequired = true;
    const user = userEvent.setup();
    render(<HomePage />);
    expect(screen.getByText(/需要设备登记码/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "登记到服务器" }));
    await user.click(screen.getByRole("button", { name: "提交设备登记码" }));
    expect(mocks.refreshRegistration).toHaveBeenCalled();
  });

  it('shows browser controller mode without offering host device enrollment and follows its independent route', async () => {
    mocks.native = false;
    mocks.launchRemoteDisplayForDevice.mockResolvedValue({ sessionId: 'browser-session', mode: 'route', routePath: '/browser-session/browser-session' });
    render(<HomePage />);
    expect(screen.getByText('网页控制端')).toBeInTheDocument();
    expect(screen.getByText(/无需安装客户端或登记本机/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '登记到服务器' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByText('办公室电脑'));
    expect(mocks.navigate).toHaveBeenCalledWith('/browser-session/browser-session');
  });

  it("shows a ten digit device code in readable groups without losing its leading zero", () => {
    render(<HomePage />);
    expect(screen.getByText("012 345 6789")).toBeInTheDocument();
    expect(screen.getByText("10 位设备码")).toBeInTheDocument();
  });

  it("preserves leading zeroes when connecting with a grouped ten digit code", async () => {
    const user = userEvent.setup();
    render(<HomePage />);
    await user.type(screen.getByPlaceholderText("输入 10 位设备码"), "012 345 6789");
    await user.click(screen.getByRole("button", { name: "立即连接" }));
    expect(mocks.launchRemoteDisplayForDevice).toHaveBeenCalledWith("0123456789", expect.anything());
  });

  it("copies all ten device digits without display separators", async () => {
    const user = userEvent.setup();
    const write = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    render(<HomePage />);
    await user.click(screen.getByRole("button", { name: "复制设备码" }));
    expect(write).toHaveBeenCalledWith("0123456789");
  });

  it("rejects invalid characters without connecting to a modified device code", async () => {
    const user = userEvent.setup();
    render(<HomePage />);
    await user.type(screen.getByPlaceholderText("输入 10 位设备码"), "0123456789/secret");
    await user.click(screen.getByRole("button", { name: "立即连接" }));
    expect(mocks.launchRemoteDisplayForDevice).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("请输入 10 位数字设备码");
  });

  it("requests an authenticated Auto session for a known recent device", async () => {
    const user = userEvent.setup();
    render(<HomePage />);

    await user.click(screen.getByText("办公室电脑"));

    expect(mocks.launchRemoteDisplayForDevice).toHaveBeenCalledWith(
      "821456789",
      expect.objectContaining({
        targetDeviceName: "办公室电脑",
        targetOs: "Windows 11",
        routePreference: "auto",
      }),
    );
    expect(mocks.navigate).toHaveBeenCalledWith("/session/secure-session");
  });

  it("does not offer an ignored remote password credential", () => {
    render(<HomePage />);

    expect(screen.queryByText("密码（可选）")).not.toBeInTheDocument();
    expect(screen.getByText("目标设备确认授权")).toBeInTheDocument();
  });

  it("never turns a custom device id into a direct session route", async () => {
    const user = userEvent.setup();
    render(<HomePage />);

    await user.type(
      screen.getByPlaceholderText("输入 10 位设备码"),
      "900 123 456",
    );
    await user.click(screen.getByRole("button", { name: "立即连接" }));

    expect(mocks.launchRemoteDisplayForDevice).toHaveBeenCalledWith(
      "900123456",
      expect.objectContaining({ routePreference: "auto" }),
    );
    expect(mocks.navigate).toHaveBeenCalledWith("/session/secure-session");
    expect(mocks.navigate).not.toHaveBeenCalledWith(
      expect.stringContaining("/session/custom"),
    );
  });

  it("coalesces repeated clicks while one secure request is pending", async () => {
    const user = userEvent.setup();
    let resolveLaunch!: (value: {
      sessionId: string;
      windowLabel: null;
      mode: "route";
    }) => void;
    mocks.launchRemoteDisplayForDevice.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveLaunch = resolve;
        }),
    );
    render(<HomePage />);

    await user.dblClick(screen.getByText("办公室电脑"));

    expect(mocks.launchRemoteDisplayForDevice).toHaveBeenCalledTimes(1);
    await act(async () => {
      resolveLaunch({ sessionId: "secure-session", windowLabel: null, mode: "route" });
    });
    await vi.waitFor(() => {
      expect(mocks.navigate).toHaveBeenCalledWith("/session/secure-session");
    });
  });
});
