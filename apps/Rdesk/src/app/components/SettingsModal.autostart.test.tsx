import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { getMockInvoke } from '../../test/mocks/tauri';
import { SettingsModal } from './SettingsModal';

vi.mock('./ThemeContext', () => ({ useTheme: () => ({ isDark: false, theme: 'light', setTheme: vi.fn() }) }));

describe('background service autostart settings', () => {
  beforeEach(() => {
    localStorage.clear();
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
    const invoke = getMockInvoke();
    invoke.mockImplementation((command: string) => {
      if (command === 'get_ui_preferences') return Promise.resolve({ close_behavior: 'hide_to_tray' });
      if (command === 'shell_get_autostart_status') return Promise.resolve({ enabled: false, supported: true });
      if (command === 'ipc_list_sessions') return Promise.resolve([]);
      if (command === 'ipc_service_health') return Promise.resolve({ running: true, healthy: true, pid: 123 });
      if (command === 'decode_policy') return Promise.reject(new Error('Use IPC to query decode policy from mrd-service'));
      if (command === 'ffmpeg_probe') return Promise.resolve({ available: false });
      return Promise.reject(new Error('Unexpected settings command: ' + command));
    });
  });
  afterEach(() => { delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__; });

  it('reads the installed service configuration instead of assuming enabled', async () => {
    render(<SettingsModal open onClose={vi.fn()} />);
    const toggle = await screen.findByRole('switch', { name: '后台服务开机启动' });
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(getMockInvoke()).toHaveBeenCalledWith('shell_get_autostart_status', undefined);
  });

  it('disables the switch while saving and displays the actual readback', async () => {
    const invoke = getMockInvoke();
    let resolveSave!: () => void;
    let reads = 0;
    const baseline = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === 'shell_get_autostart_status') {
        reads += 1;
        return Promise.resolve({ enabled: reads > 1, supported: true });
      }
      if (command === 'shell_set_autostart') return new Promise<void>((resolve) => { resolveSave = resolve; });
      return baseline(command, args);
    });
    render(<SettingsModal open onClose={vi.fn()} />);
    const toggle = await screen.findByRole('switch', { name: '后台服务开机启动' });
    await waitFor(() => expect(toggle).toBeEnabled());
    await userEvent.click(toggle);
    expect(invoke).toHaveBeenCalledWith('shell_set_autostart', { enabled: true });
    expect(toggle).toBeDisabled();
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    resolveSave();
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
    expect(toggle).toBeEnabled();
  });

  it('keeps the confirmed state and shows a configuration write error', async () => {
    const invoke = getMockInvoke();
    const baseline = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args?: unknown) => command === 'shell_set_autostart'
      ? Promise.reject(new Error('服务配置修改被拒绝')) : baseline(command, args));
    render(<SettingsModal open onClose={vi.fn()} />);
    const toggle = await screen.findByRole('switch', { name: '后台服务开机启动' });
    await waitFor(() => expect(toggle).toBeEnabled());
    await userEvent.click(toggle);
    expect(await screen.findByRole('alert')).toHaveTextContent('服务配置修改被拒绝');
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(toggle).toBeEnabled();
  });

  it('does not offer a functional switch when the service is not installed', async () => {
    const invoke = getMockInvoke();
    const baseline = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args?: unknown) => command === 'shell_get_autostart_status'
      ? Promise.resolve({ enabled: false, supported: false }) : baseline(command, args));
    render(<SettingsModal open onClose={vi.fn()} />);
    const toggle = await screen.findByRole('switch', { name: '后台服务开机启动' });
    expect(await screen.findByText('当前运行方式不支持开机启动，请先安装后台服务。')).toBeInTheDocument();
    expect(toggle).toBeDisabled();
  });

  it('keeps the settings open and displays a failed stop-and-exit request', async () => {
    const invoke = getMockInvoke();
    const baseline = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args?: unknown) => command === 'shell_quit_ui_and_stop_service'
      ? Promise.reject(new Error('后台服务停止超时')) : baseline(command, args));
    const onClose = vi.fn();
    render(<SettingsModal open onClose={onClose} />);
    const quit = await screen.findByRole('button', { name: '退出并停止后台服务' });
    await waitFor(() => expect(quit).toBeEnabled());
    await userEvent.click(quit);
    expect(await screen.findByRole('alert')).toHaveTextContent('后台服务停止超时');
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByText('通用设置')).toBeInTheDocument();
  });

  it('keeps the confirmed state when the actual readback differs from the requested value', async () => {
    const invoke = getMockInvoke();
    const baseline = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args?: unknown) => command === 'shell_set_autostart'
      ? Promise.resolve(undefined) : baseline(command, args));
    render(<SettingsModal open onClose={vi.fn()} />);
    const toggle = await screen.findByRole('switch', { name: '后台服务开机启动' });
    await waitFor(() => expect(toggle).toBeEnabled());
    await userEvent.click(toggle);
    expect(await screen.findByText('开机启动配置已保存')).toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(invoke.mock.calls.filter(([command]) => command === 'shell_get_autostart_status')).toHaveLength(2);
  });

  it('retains the last confirmed value when readback fails after a write', async () => {
    const invoke = getMockInvoke();
    const baseline = invoke.getMockImplementation()!;
    let reads = 0;
    invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === 'shell_get_autostart_status') return ++reads === 1
        ? Promise.resolve({ enabled: false, supported: true }) : Promise.reject(new Error('读取服务启动配置失败'));
      if (command === 'shell_set_autostart') return Promise.resolve(undefined);
      return baseline(command, args);
    });
    render(<SettingsModal open onClose={vi.fn()} />);
    const toggle = await screen.findByRole('switch', { name: '后台服务开机启动' });
    await waitFor(() => expect(toggle).toBeEnabled());
    await userEvent.click(toggle);
    expect(await screen.findByRole('alert')).toHaveTextContent('读取服务启动配置失败');
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(toggle).toBeEnabled();
    expect(screen.queryByText('开机启动配置已保存')).not.toBeInTheDocument();
  });

  it('does not let a previous opening save failure replace the reopened state', async () => {
    const invoke = getMockInvoke();
    const baseline = invoke.getMockImplementation()!;
    let rejectSave!: (reason: Error) => void;
    let reads = 0;
    invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === 'shell_get_autostart_status') return Promise.resolve({ enabled: ++reads > 1, supported: true });
      if (command === 'shell_set_autostart') return new Promise<void>((_, reject) => { rejectSave = reject; });
      return baseline(command, args);
    });
    const onClose = vi.fn();
    const view = render(<SettingsModal open onClose={onClose} />);
    const firstToggle = await screen.findByRole('switch', { name: '后台服务开机启动' });
    await waitFor(() => expect(firstToggle).toBeEnabled());
    await userEvent.click(firstToggle);
    expect(firstToggle).toBeDisabled();
    view.rerender(<SettingsModal open={false} onClose={onClose} />);
    view.rerender(<SettingsModal open onClose={onClose} />);
    const newToggle = await screen.findByRole('switch', { name: '后台服务开机启动' });
    await waitFor(() => expect(newToggle).toHaveAttribute('aria-checked', 'true'));
    await act(async () => rejectSave(new Error('旧保存请求失败')));
    expect(newToggle).toHaveAttribute('aria-checked', 'true');
    expect(newToggle).toBeEnabled();
    expect(screen.queryByText('旧保存请求失败')).not.toBeInTheDocument();
  });
});
