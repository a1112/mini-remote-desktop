import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TitleBar } from './TitleBar';

const mocks = vi.hoisted(() => ({ snapshot: vi.fn(), window: vi.fn() }));
vi.mock('../adapters/tauri/commands', () => ({ ipcCapabilitySnapshot: mocks.snapshot }));
vi.mock('./ThemeContext', () => ({ useTheme: () => ({ isDark: false }) }));
vi.mock('./AuthContext', () => ({ useAuth: () => ({ logout: vi.fn(), logoutError: null }) }));
vi.mock('./DetailBarContext', () => ({ useDetailBar: () => ({ collapsed: false, payload: null }) }));
vi.mock('../services/fileTransferListService', () => ({ useFileTransfers: () => ({ transfers: [], loading: false, error: null }), transferName: vi.fn(), formatTransferBytes: vi.fn(), transferStatusLabel: {} }));
vi.mock('../utils/tauriWindow', () => ({ withTauriWindow: mocks.window }));

describe('permission control placement', () => {
  beforeEach(() => {
    localStorage.clear(); mocks.snapshot.mockReset().mockResolvedValue({ ok: false, error: { message: 'offline' } });
    mocks.window.mockReset();
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
  });
  afterEach(() => { cleanup(); Reflect.deleteProperty(window, '__TAURI_INTERNALS__'); });

  it('places local permissions immediately before the connection-history action without requiring login', async () => {
    render(<TitleBar />); await act(async () => {});
    const permission = screen.getByRole('button', { name: /^本机系统权限：/ });
    const history = screen.getByTitle('连接记录');
    expect(history.previousElementSibling?.contains(permission)).toBe(true);
    expect(permission).toHaveAttribute('data-no-drag', 'true');
    expect(screen.getByTitle('登录')).toBeInTheDocument();
  });

  it('omits native permission status in the browser without an offline error or IPC', async () => {
    Reflect.deleteProperty(window, '__TAURI_INTERNALS__'); render(<TitleBar />); await act(async () => {});
    expect(screen.queryByRole('button', { name: /^本机系统权限：/ })).not.toBeInTheDocument();
    expect(screen.queryByText(/mrd-service offline|本机权限/)).not.toBeInTheDocument();
    expect(mocks.snapshot).not.toHaveBeenCalled(); expect(screen.getByTitle('连接记录')).toBeInTheDocument();
  });

  it('does not start dragging the window when a permission detail paragraph is clicked', async () => {
    render(<TitleBar />); await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: /^本机系统权限：/ })); await act(async () => {});
    const detail = screen.getByText('未能读取本机权限状态，请重新检查。');
    mocks.window.mockClear();
    fireEvent.mouseDown(detail, { button: 0, detail: 1 });
    expect(mocks.window).not.toHaveBeenCalled();
  });
});

it("focuses device search with the keyboard shortcut and submits the entered query", async () => {
  const onSearch = vi.fn();
  render(<TitleBar onSearchDevices={onSearch} />);
  await act(async () => {});
  fireEvent.keyDown(window, { key: "k", ctrlKey: true });
  const input = screen.getByRole("searchbox", { name: "搜索设备" });
  expect(input).toHaveFocus();
  fireEvent.change(input, { target: { value: "  工作电脑  " } });
  fireEvent.submit(screen.getByRole("search", { name: "查找设备" }));
  expect(onSearch).toHaveBeenCalledWith("工作电脑");
});
