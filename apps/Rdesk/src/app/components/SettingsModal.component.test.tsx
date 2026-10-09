import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '../../test/mocks/tauri';
import { SettingsModal } from './SettingsModal';

vi.mock('./ThemeContext', () => ({ useTheme: () => ({ isDark: false, theme: 'light', setTheme: vi.fn() }) }));

const probe = { available: true, ffmpeg_path: 'C:/ffmpeg/bin/ffmpeg.exe', ffprobe_path: 'C:/ffmpeg/bin/ffprobe.exe', ffmpeg_version: 'ffmpeg version 8.1.1', ffprobe_version: 'ffprobe version 8.1.1', reason: null };
function commands(command: string) {
  switch (command) {
    case 'get_ui_preferences': return Promise.resolve({ close_behavior: 'hide_to_tray' });
    case 'ipc_list_sessions': return Promise.resolve([]);
    case 'ipc_service_health': return Promise.resolve({ running: true, healthy: true, pid: 123 });
    case 'shell_get_autostart_status': return Promise.resolve({ enabled: false, supported: true });
    case 'decode_policy': return Promise.reject(new Error('Use IPC to query decode policy from mrd-service'));
    case 'ffmpeg_probe': return Promise.resolve(probe);
    default: return Promise.reject(new Error('Unexpected settings command: ' + command));
  }
}

describe('SettingsModal asynchronous behavior', () => {
  beforeEach(() => {
    localStorage.clear();
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
    getMockInvoke().mockImplementation(commands);
  });
  afterEach(() => { delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__; });

  it('refreshes the real health value and process id', async () => {
    let reads = 0;
    getMockInvoke().mockImplementation((command: string) => command === 'ipc_service_health'
      ? Promise.resolve(++reads === 1 ? { running: true, healthy: true, pid: 123 } : { running: true, healthy: false, pid: 456 })
      : commands(command));
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(await screen.findByText('123')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '刷新' }));
    expect(await screen.findByText('456')).toBeInTheDocument();
    expect(screen.getByText('异常')).toBeInTheDocument();
    expect(screen.queryByText('123')).not.toBeInTheDocument();
    expect(reads).toBe(2);
  });

  it('does not block service controls on a slow FFmpeg probe', async () => {
    let resolveProbe!: (value: typeof probe) => void;
    getMockInvoke().mockImplementation((command: string) => command === 'ffmpeg_probe'
      ? new Promise(resolve => { resolveProbe = resolve; }) : commands(command));
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(await screen.findByText('健康')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '刷新' })).toBeEnabled();
    expect(screen.getByRole('button', { name: '停止' })).toBeEnabled();
    await userEvent.click(screen.getByRole('button', { name: '显示' }));
    expect(screen.getByRole('button', { name: '下载或更新 FFmpeg' })).toBeDisabled();
    await act(async () => resolveProbe(probe));
    expect(await screen.findByText(probe.ffmpeg_version)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '下载或更新 FFmpeg' })).toBeEnabled();
  });

  it('starts an absent service and displays confirmed health after bootstrap', async () => {
    let started = false;
    const invoke = getMockInvoke();
    invoke.mockImplementation((command: string) => {
      if (command === 'ipc_service_health') return Promise.resolve({ running: started, healthy: started, pid: started ? 789 : null });
      if (command === 'service_bootstrap_if_needed') { started = true; return Promise.resolve(true); }
      if (command === 'service_wait_for_healthy') return Promise.resolve(true);
      return commands(command);
    });
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(await screen.findByText('未运行')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '启动' }));
    expect(await screen.findByText('789')).toBeInTheDocument();
    expect(screen.getByText('健康')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '启动' })).toBeDisabled();
    expect(invoke).toHaveBeenCalledWith('service_bootstrap_if_needed', undefined);
    expect(invoke).toHaveBeenCalledWith('service_wait_for_healthy', { timeoutSecs: 30 });
  });

  it('retains the confirmed running status when shutdown is rejected', async () => {
    const invoke = getMockInvoke();
    invoke.mockImplementation((command: string) => command === 'shell_shutdown_service'
      ? Promise.reject(new Error('E_MANAGEMENT_COMMAND_DENIED: shutdown rejected')) : commands(command));
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(await screen.findByText('123')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '停止' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('E_MANAGEMENT_COMMAND_DENIED');
    expect(screen.getByText('运行中')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '停止' })).toBeEnabled();
    expect(invoke).not.toHaveBeenCalledWith('service_wait_for_stopped', { timeoutSecs: 30 });
    expect(invoke).not.toHaveBeenCalledWith('service_bootstrap_if_needed', undefined);
  });

  it('displays the installed FFmpeg probe after download', async () => {
    const installed = { ...probe, ffmpeg_version: 'ffmpeg updated build', ffmpeg_path: 'C:/updated/ffmpeg.exe' };
    getMockInvoke().mockImplementation((command: string) => command === 'ffmpeg_download'
      ? Promise.resolve({ install_dir: 'C:/updated', archive_sha256: 'a'.repeat(64), probe: installed }) : commands(command));
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '显示' }));
    expect(await screen.findByText(probe.ffmpeg_version)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '下载或更新 FFmpeg' }));
    expect(await screen.findByText(installed.ffmpeg_version)).toBeInTheDocument();
    expect(screen.getByText(installed.ffmpeg_path)).toBeInTheDocument();
    expect(screen.getByText('FFmpeg 已下载并完成探测')).toBeInTheDocument();
    expect(getMockInvoke()).toHaveBeenCalledWith('ffmpeg_download', undefined);
  });

  it('keeps the previous FFmpeg probe when download fails', async () => {
    getMockInvoke().mockImplementation((command: string) => command === 'ffmpeg_download'
      ? Promise.reject(new Error('FFmpeg 校验失败')) : commands(command));
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '显示' }));
    expect(await screen.findByText(probe.ffmpeg_version)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '下载或更新 FFmpeg' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('FFmpeg 校验失败');
    expect(screen.getByText(probe.ffmpeg_version)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '下载或更新 FFmpeg' })).toBeEnabled();
    expect(screen.queryByText('FFmpeg 已下载并完成探测')).not.toBeInTheDocument();
  });

  it('refreshes FFmpeg after reset without claiming decode support', async () => {
    let probes = 0;
    const invoke = getMockInvoke();
    invoke.mockImplementation((command: string) => {
      if (command === 'ffmpeg_probe') return Promise.resolve({ ...probe, ffmpeg_version: ++probes === 1 ? probe.ffmpeg_version : 'ffmpeg reset build' });
      if (command === 'ffmpeg_reset_golden_settings') return Promise.resolve({ decode_policy: 'auto' });
      return commands(command);
    });
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '显示' }));
    expect(await screen.findByText(probe.ffmpeg_version)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '恢复 FFmpeg 默认配置' }));
    expect(await screen.findByText('ffmpeg reset build')).toBeInTheDocument();
    expect(screen.getByText('FFmpeg 默认配置已恢复')).toBeInTheDocument();
    expect(screen.getByLabelText('解码策略')).toBeDisabled();
    expect(invoke).toHaveBeenCalledWith('ffmpeg_reset_golden_settings', undefined);
    expect(probes).toBe(2);
    expect(invoke.mock.calls.some(([command]) => command === 'set_decode_policy')).toBe(false);
  });
});
