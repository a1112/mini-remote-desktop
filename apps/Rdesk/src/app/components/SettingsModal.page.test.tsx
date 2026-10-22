import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { getMockInvoke } from '../../test/mocks/tauri';
import { SettingsModal } from './SettingsModal';

vi.mock('./ThemeContext', () => ({ useTheme: () => ({ isDark: false, theme: 'light', setTheme: vi.fn() }) }));

const probe = { available: true, ffmpeg_path: 'C:/ffmpeg/bin/ffmpeg.exe', ffprobe_path: 'C:/ffmpeg/bin/ffprobe.exe', ffmpeg_version: 'ffmpeg version 8.1.1', ffprobe_version: 'ffprobe version 8.1.1', reason: null };
function commands(command: string) {
  switch (command) {
    case 'get_ui_preferences': return Promise.resolve({ close_behavior: 'hide_to_tray' });
    case 'ipc_list_sessions': return Promise.resolve([]);
    case 'ipc_service_health': return Promise.resolve({ running: true, healthy: true, pid: 12345 });
    case 'shell_get_autostart_status': return Promise.resolve({ enabled: true, supported: true });
    case 'decode_policy': return Promise.reject(new Error('Use IPC to query decode policy from mrd-service'));
    case 'ffmpeg_probe': return Promise.resolve(probe);
    default: return Promise.reject(new Error('Unexpected settings command: ' + command));
  }
}

describe('SettingsModal page behavior', () => {
  beforeEach(() => {
    localStorage.clear();
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
    getMockInvoke().mockImplementation(commands);
  });
  afterEach(() => { delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__; });

  it('does not render or load settings while closed', () => {
    render(<SettingsModal open={false} onClose={vi.fn()} />);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(getMockInvoke()).not.toHaveBeenCalled();
  });

  it('shows all seven categories and their corresponding sections', async () => {
    render(<SettingsModal open onClose={vi.fn()} />);
    expect(screen.getByRole('dialog', { name: '设置' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: '通用设置' })).toBeInTheDocument();
    const categories = within(screen.getByRole('navigation', { name: '设置分类' }));
    const sections = [['通用', '通用设置'], ['安全', '安全设置'], ['网络', '网络设置'], ['显示', '显示设置'], ['音频与输入', '音频与输入设置'], ['通知', '通知设置'], ['账户', '账户设置']];
    expect(categories.getAllByRole('button')).toHaveLength(sections.length);
    for (const [name, title] of sections) {
      const category = categories.getByRole('button', { name });
      await userEvent.click(category);
      expect(category).toHaveAttribute('aria-pressed', 'true');
      expect(screen.getByRole('region', { name: title })).toBeInTheDocument();
    }
  });

  it('displays one real health snapshot and its process id', async () => {
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(await screen.findByText('运行中')).toBeInTheDocument();
    expect(screen.getByText('健康')).toBeInTheDocument();
    expect(screen.getByText('12345')).toBeInTheDocument();
    expect(getMockInvoke().mock.calls.filter(([command]) => command === 'ipc_service_health')).toHaveLength(1);
    expect(getMockInvoke()).not.toHaveBeenCalledWith('shell_get_status', undefined);
    expect(screen.getByText('已有连接')).toBeInTheDocument();
    expect(await screen.findByText('暂无连接')).toBeInTheDocument();
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /注册/ })).not.toBeInTheDocument();
  });

  it('shows an absent service as stopped and allows starting it', async () => {
    getMockInvoke().mockImplementation((command: string) => command === 'ipc_service_health'
      ? Promise.reject(new Error('connection refused')) : commands(command));
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(await screen.findByText('未运行')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '启动' })).toBeEnabled();
    expect(screen.getByRole('button', { name: '停止' })).toBeDisabled();
  });

  it('does not misreport permission errors as a stopped service', async () => {
    getMockInvoke().mockImplementation((command: string) => command === 'ipc_service_health'
      ? Promise.reject(new Error('named pipe access denied')) : commands(command));
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('named pipe access denied');
    expect(screen.queryByText('未运行')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: '启动' })).toBeDisabled();
  });

  it('places media tools in display settings and disables unavailable decode settings', async () => {
    render(<SettingsModal open onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(screen.queryByText('媒体解码')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '显示' }));
    expect(await screen.findByText(probe.ffmpeg_version)).toBeInTheDocument();
    expect(screen.getByText(probe.ffmpeg_path)).toBeInTheDocument();
    expect(screen.getByLabelText('解码策略')).toBeDisabled();
    expect(await screen.findByText(/Use IPC to query decode policy from mrd-service/)).toBeInTheDocument();
    expect(getMockInvoke().mock.calls.some(([command]) => command === 'set_decode_policy')).toBe(false);
  });

  it('closes through the named close button', async () => {
    const onClose = vi.fn();
    render(<SettingsModal open onClose={onClose} />);
    await userEvent.click(screen.getByRole('button', { name: '关闭设置' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('handles Escape only while the modal is open', async () => {
    const onClose = vi.fn();
    const result = render(<SettingsModal open onClose={onClose} />);
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
    result.rerender(<SettingsModal open={false} onClose={onClose} />);
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
