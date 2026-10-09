import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '../../test/mocks/tauri';
import { AuthProvider } from './AuthContext';
import { ThemeProvider } from './ThemeContext';
import { SettingsModal } from './SettingsModal';

vi.mock('./IpcSessionCard', () => ({ IpcSessionCard: () => <div>已有会话信息</div> }));

const probe = { available: true, ffmpeg_path: 'C:/tools/ffmpeg.exe', ffprobe_path: 'C:/tools/ffprobe.exe', ffmpeg_version: 'ffmpeg test build', ffprobe_version: 'ffprobe test build', reason: null };
const commands = (command: string) => {
  if (command === 'get_ui_preferences') return Promise.resolve({ close_behavior: 'hide_to_tray' });
  if (command === 'ipc_service_health') return Promise.resolve({ running: true, healthy: false, pid: 456 });
  if (command === 'shell_get_status') return Promise.resolve({ service_pid: 456, last_error: null });
  if (command === 'shell_get_autostart_status') return Promise.resolve({ enabled: false, supported: true });
  if (command === 'decode_policy') return Promise.reject(new Error('Use IPC to query decode policy from mrd-service'));
  if (command === 'ffmpeg_probe') return Promise.resolve(probe);
  if (command === 'ipc_list_sessions') return Promise.resolve([]);
  return Promise.resolve(undefined);
};
function show() {
  return render(<ThemeProvider><AuthProvider><SettingsModal open onClose={vi.fn()} /></AuthProvider></ThemeProvider>);
}

describe('usable core settings', () => {
  beforeEach(() => {
    localStorage.clear();
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
    getMockInvoke().mockImplementation(commands);
  });
  afterEach(() => { delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__; });

  it('saves close behavior and displays the confirmed native value', async () => {
    getMockInvoke().mockImplementation((command: string, args?: Record<string, unknown>) => command === 'set_close_behavior'
      ? Promise.resolve({ close_behavior: args?.closeBehavior }) : commands(command));
    show();
    const control = await screen.findByLabelText('关闭窗口时');
    await waitFor(() => expect(control).toBeEnabled());
    await userEvent.selectOptions(control, 'exit_ui');
    await waitFor(() => expect(control).toHaveValue('exit_ui'));
    expect(getMockInvoke()).toHaveBeenCalledWith('set_close_behavior', { closeBehavior: 'exit_ui' });
  });

  it('keeps the confirmed close behavior when saving fails', async () => {
    getMockInvoke().mockImplementation((command: string) => command === 'set_close_behavior'
      ? Promise.reject(new Error('设置文件不可写')) : commands(command));
    show();
    const control = await screen.findByLabelText('关闭窗口时');
    await waitFor(() => expect(control).toBeEnabled());
    await userEvent.selectOptions(control, 'exit_ui');
    expect(await screen.findByRole('alert')).toHaveTextContent('设置文件不可写');
    expect(control).toHaveValue('hide_to_tray');
  });

  it('persists the chosen theme across settings remounts', async () => {
    const first = show();
    await userEvent.selectOptions(await screen.findByLabelText('界面主题'), 'light');
    await waitFor(() => expect(localStorage.getItem('app-theme')).toBe('light'));
    first.unmount();
    show();
    expect(await screen.findByLabelText('界面主题')).toHaveValue('light');
  });

  it('shows real unhealthy status even when shell last_error is null and removes fake measurements', async () => {
    show();
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(await screen.findByText('运行中')).toBeInTheDocument();
    expect(await screen.findByText('异常')).toBeInTheDocument();
    expect(screen.getByText('456')).toBeInTheDocument();
    expect(screen.queryByText('24ms')).not.toBeInTheDocument();
    expect(screen.queryByText('94 Mbps')).not.toBeInTheDocument();
    expect(screen.queryByText('47 Mbps')).not.toBeInTheDocument();
    expect(screen.getAllByText('未测量')).toHaveLength(3);
  });

  it('shows unsupported security controls without allowing fake changes', async () => {
    show();
    await userEvent.click(screen.getByRole('button', { name: '安全' }));
    expect(screen.getByRole('switch', { name: '双因素认证' })).toBeDisabled();
    expect(screen.getByRole('switch', { name: '连接密码' })).toBeDisabled();
    expect(screen.getAllByText('暂不支持').length).toBeGreaterThan(0);
  });

  it('displays existing connections as a read-only summary', async () => {
    getMockInvoke().mockImplementation((command: string) => command === 'ipc_list_sessions'
      ? Promise.resolve([{ session_id: 'session-existing', peer_device_id: 'remote-device', state: 'streaming', transport_kind: 'quic', role: 'controller', sender_active: false, receiver_active: true }]) : commands(command));
    show();
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(await screen.findByText('remote-device')).toBeInTheDocument();
    expect(screen.getByText('传输中')).toBeInTheDocument();
    expect(screen.getByText('quic')).toBeInTheDocument();
    expect(screen.queryByText('IPC Session Control')).not.toBeInTheDocument();
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    expect(getMockInvoke().mock.calls.some(([cmd]) => /register_device|start_sender|start_receiver/.test(cmd))).toBe(false);
  });

  it('keeps FFmpeg usable when the decode policy command is unavailable', async () => {
    show();
    await userEvent.click(screen.getByRole('button', { name: '显示' }));
    expect(await screen.findByText('ffmpeg test build')).toBeInTheDocument();
    expect(screen.getByLabelText('解码策略')).toBeDisabled();
    expect(screen.getByText(/当前版本尚未接入服务端解码策略设置/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '刷新 FFmpeg 状态' }));
    await waitFor(() => expect(getMockInvoke().mock.calls.filter(([cmd]) => cmd === 'ffmpeg_probe')).toHaveLength(2));
    expect(getMockInvoke().mock.calls.some(([cmd]) => cmd === 'set_decode_policy')).toBe(false);
  });

  it('logs out through the shared auth context and preserves preferences', async () => {
    localStorage.setItem('rdesk_access_token', 'test-token');
    localStorage.setItem('rdesk_auth_user', JSON.stringify({ id: '1', username: '测试账户', role: 'user' }));
    localStorage.setItem('app-theme', 'light');
    const event = vi.fn();
    window.addEventListener('rdesk-auth-changed', event);
    const result = show();
    await userEvent.click(screen.getByRole('button', { name: '账户' }));
    await userEvent.click(await screen.findByRole('button', { name: '退出登录' }));
    await waitFor(() => expect(screen.getByText('未登录')).toBeInTheDocument());
    expect(localStorage.getItem('rdesk_access_token')).toBeNull();
    expect(localStorage.getItem('rdesk_auth_user')).toBeNull();
    expect(localStorage.getItem('app-theme')).toBe('light');
    expect(event).toHaveBeenCalledTimes(1);
    expect(getMockInvoke().mock.calls.some(([cmd]) => /unbind|shutdown|quit/.test(cmd))).toBe(false);
    result.unmount();
    window.removeEventListener('rdesk-auth-changed', event);
  });

  it('ignores a previous opening health response after reopening', async () => {
    let resolveOld!: (value: unknown) => void;
    let reads = 0;
    getMockInvoke().mockImplementation((command: string) => {
      if (command === 'ipc_service_health') {
        reads += 1;
        return reads === 1 ? new Promise(resolve => { resolveOld = resolve; }) : Promise.resolve({ running: true, healthy: true, pid: 987 });
      }
      return commands(command);
    });
    const onClose = vi.fn();
    const first = render(<SettingsModal open onClose={onClose} />);
    await waitFor(() => expect(reads).toBe(1));
    first.rerender(<SettingsModal open={false} onClose={onClose} />);
    first.rerender(<SettingsModal open onClose={onClose} />);
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(await screen.findByText('987')).toBeInTheDocument();
    await act(async () => resolveOld({ running: false, healthy: false, pid: null }));
    expect(screen.getByText('987')).toBeInTheDocument();
    expect(screen.queryByText('未运行')).not.toBeInTheDocument();
  });

  it('waits for an in-flight FFmpeg download across closing and reopening', async () => {
    let finishDownload!: (value: unknown) => void;
    getMockInvoke().mockImplementation((command: string) => command === 'ffmpeg_download'
      ? new Promise(resolve => { finishDownload = resolve; }) : commands(command));
    const onClose = vi.fn();
    const view = render(<SettingsModal open onClose={onClose} />);
    await userEvent.click(screen.getByRole('button', { name: '显示' }));
    const download = await screen.findByRole('button', { name: '下载或更新 FFmpeg' });
    await waitFor(() => expect(download).toBeEnabled());
    await userEvent.click(download);
    await waitFor(() => expect(finishDownload).toBeDefined());
    try {
      view.rerender(<SettingsModal open={false} onClose={onClose} />);
      view.rerender(<SettingsModal open onClose={onClose} />);
      await userEvent.click(screen.getByRole('button', { name: '显示' }));
      expect(screen.getByRole('button', { name: '下载或更新 FFmpeg' })).toBeDisabled();
      expect(screen.getByRole('button', { name: '恢复 FFmpeg 默认配置' })).toBeDisabled();
      expect(getMockInvoke().mock.calls.filter(([cmd]) => cmd === 'ffmpeg_probe')).toHaveLength(1);
      expect(getMockInvoke().mock.calls.filter(([cmd]) => cmd === 'ffmpeg_download')).toHaveLength(1);
    } finally {
      await act(async () => finishDownload({ probe, install_dir: 'C:/tools', archive_sha256: 'a'.repeat(64) }));
    }
    await waitFor(() => expect(screen.getByRole('button', { name: '下载或更新 FFmpeg' })).toBeEnabled());
    expect(getMockInvoke().mock.calls.filter(([cmd]) => cmd === 'ffmpeg_probe')).toHaveLength(2);
  });
});
