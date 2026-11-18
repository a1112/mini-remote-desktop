import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AuthModal } from "./AuthModal";

const mocks = vi.hoisted(() => ({
  login: vi.fn(), bindDevice: vi.fn(), native: false,
  rAuthAvailable: vi.fn(), signInWithRAuth: vi.fn(),
}));

vi.mock("./ThemeContext", () => ({
  useTheme: () => ({ isDark: false }),
}));

vi.mock("./AuthContext", () => ({
  useAuth: () => ({ login: mocks.login }),
}));

vi.mock("../services/deviceService", () => ({
  deviceService: { bindDevice: mocks.bindDevice },
}));

vi.mock("../services/rAuth", () => ({
  rAuthAvailable: mocks.rAuthAvailable,
  signInWithRAuth: mocks.signInWithRAuth,
}));
vi.mock("../utils/runtime", () => ({ isTauriRuntime: () => mocks.native }));

describe("AuthModal credential safety", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.native = false;
    mocks.rAuthAvailable.mockResolvedValue(false);
  });

  it("opens with empty credentials and no default-account autofill control", () => {
    render(<AuthModal open onClose={vi.fn()} />);

    expect(screen.getByPlaceholderText("请输入账号")).toHaveValue("");
    expect(screen.getByPlaceholderText("输入密码")).toHaveValue("");
    expect(screen.queryByText("默认账户")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "填入" })).not.toBeInTheDocument();
  });

  it('uses R-AUTH to set the Rdesk account before binding the desktop device', async () => {
    mocks.native = true;
    mocks.rAuthAvailable.mockResolvedValue(true);
    mocks.signInWithRAuth.mockResolvedValue({ access_token: 'new-session', user_id: 'user-1', username: 'account', role: 'user' });
    const close = vi.fn();
    render(<AuthModal open onClose={close} />);

    fireEvent.click(await screen.findByRole('button', { name: '使用 R-AUTH 登录' }));

    await waitFor(() => expect(close).toHaveBeenCalledOnce());
    expect(mocks.login).toHaveBeenCalledWith('new-session', { id: 'user-1', username: 'account', role: 'user' });
    expect(mocks.bindDevice).toHaveBeenCalledOnce();
    expect(mocks.login.mock.invocationCallOrder[0]).toBeLessThan(mocks.bindDevice.mock.invocationCallOrder[0]!);
  });

  it('shows a binding failure even when R-AUTH sign-in succeeds', async () => {
    mocks.native = true;
    mocks.rAuthAvailable.mockResolvedValue(true);
    mocks.signInWithRAuth.mockResolvedValue({ access_token: 'new-session', user_id: 'user-1', username: 'account', role: 'user' });
    mocks.bindDevice.mockResolvedValue({ success: false, message: '这台设备已绑定到其他账户' });
    const close = vi.fn();
    render(<AuthModal open onClose={close} />);

    fireEvent.click(await screen.findByRole('button', { name: '使用 R-AUTH 登录' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('这台设备已绑定到其他账户');
    expect(screen.getByRole('button', { name: 'R-AUTH 已登录' })).toBeDisabled();
    expect(close).not.toHaveBeenCalled();
  });
});
