import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { getMockInvoke } from '../../test/mocks/tauri';
import { SettingsModal } from './SettingsModal';

vi.mock('./ThemeContext', () => ({ useTheme: () => ({ isDark: false, theme: 'light', setTheme: vi.fn() }) }));
vi.mock('./IpcSessionCard', () => ({ IpcSessionCard: () => null }));

describe('background service autostart settings', () => {
  beforeEach(() => {
    const invoke = getMockInvoke();
    invoke.mockImplementation((command: string) => {
      if (command === 'shell_get_autostart_status') return Promise.resolve({ enabled: false, supported: true });
      if (command === 'shell_get_status') return Promise.resolve({ service_pid: 123, last_error: null });
      if (command === 'decode_policy') return Promise.resolve({ decode_policy: 'auto' });
      if (command === 'ffmpeg_probe') return Promise.resolve({ available: false });
      return Promise.resolve(undefined);
    });
  });

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
    await userEvent.click(await screen.findByRole('button', { name: '退出并停止后台服务' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('后台服务停止超时');
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByText('通用设置')).toBeInTheDocument();
  });
});
