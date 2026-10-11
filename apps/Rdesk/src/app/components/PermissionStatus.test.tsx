import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CapabilitySnapshot, CapabilityStatus } from '../adapters/tauri/types';
import { PermissionStatus } from './PermissionStatus';

const mocks = vi.hoisted(() => ({ snapshot: vi.fn() }));
vi.mock('../adapters/tauri/commands', () => ({ ipcCapabilitySnapshot: mocks.snapshot }));
vi.mock('./ThemeContext', () => ({ useTheme: () => ({ isDark: false }) }));

function snapshot(capture: CapabilityStatus = 'available', input: CapabilityStatus = 'available'): CapabilitySnapshot {
  return { schema_version: 1, platform: 'macos', service_version: 'test', updated_at_ms: Date.now(),
    capabilities: [
      { id: 'capture.macos', domain: 'capture', label: 'ScreenCaptureKit', status: capture, platform: 'macos' },
      { id: 'control.keyboard_mouse', domain: 'control', label: 'Keyboard and mouse', status: input, platform: 'macos' },
    ], constraints: [], profiles: [] };
}
async function tick(ms = 0) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }
function native(value = true) {
  if (value) Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
  else Reflect.deleteProperty(window, '__TAURI_INTERNALS__');
}
function trigger() { return screen.getByRole('button', { name: /^本机系统权限：/ }); }

describe('local system permission status', () => {
  beforeEach(() => {
    vi.useFakeTimers(); vi.setSystemTime(new Date('2026-10-11T08:00:00Z')); native();
    mocks.snapshot.mockReset().mockImplementation(async () => ({ ok: true, value: snapshot() }));
  });
  afterEach(() => { cleanup(); native(false); vi.useRealTimers(); });

  it('hides completely in a browser without issuing native IPC', async () => {
    native(false);
    Object.defineProperty(window, '__TAURI__', { configurable: true, value: {} });
    const { container } = render(<PermissionStatus />);
    fireEvent.focus(window); await tick(10_000);
    expect(container).toBeEmptyDOMElement(); expect(mocks.snapshot).not.toHaveBeenCalled();
    Reflect.deleteProperty(window, '__TAURI__');
  });

  it('shows actual missing system permissions independently of account or session grants', async () => {
    mocks.snapshot.mockResolvedValue({ ok: true, value: snapshot('permission_missing', 'permission_missing') });
    render(<PermissionStatus />); await tick();
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏待授权，键鼠待授权');
    fireEvent.click(trigger()); await tick();
    expect(screen.getByText('录屏权限缺失')).toBeInTheDocument();
    expect(screen.getByText('辅助功能或事件控制权限缺失')).toBeInTheDocument();
    expect(screen.getByText(/每次连接的应用授权仍需单独确认/)).toBeInTheDocument();
  });

  it('keeps static support pending until a later runtime snapshot is returned', async () => {
    mocks.snapshot.mockResolvedValueOnce({ ok: true, value: snapshot('supported', 'supported') });
    render(<PermissionStatus />); await tick();
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏待检查，键鼠待检查');
    await tick(3_000);
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏可用，键鼠可用');
  });

  it.each([
    ['usable', '可用'], ['degraded', '受限'], ['unsupported', '暂不支持'],
    ['unimplemented', '暂不支持'], ['unknown', '未能确认'], ['driver_missing', '未能确认'],
  ] as const)('maps %s without inventing an authorization', async (status, label) => {
    mocks.snapshot.mockResolvedValue({ ok: true, value: snapshot(status, status) });
    render(<PermissionStatus />); await tick();
    expect(trigger()).toHaveAccessibleName(`本机系统权限：录屏${label}，键鼠${label}`);
  });

  it('ignores synthetic capture when the native capture entry is absent', async () => {
    const value = snapshot(); value.capabilities[0]!.id = 'capture.synthetic';
    mocks.snapshot.mockResolvedValue({ ok: true, value }); render(<PermissionStatus />); await tick();
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏未能确认，键鼠可用');
  });

  it('uses the native Windows capture entry rather than an unrelated available backend', async () => {
    const value = snapshot('permission_missing'); value.platform = 'windows';
    value.capabilities[0] = { ...value.capabilities[0]!, id: 'capture.dxgi', platform: 'windows' };
    value.capabilities[1]!.platform = 'windows';
    value.capabilities.push({ ...value.capabilities[0]!, id: 'capture.synthetic', status: 'available' });
    mocks.snapshot.mockResolvedValue({ ok: true, value }); render(<PermissionStatus />); await tick();
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏待授权，键鼠可用');
  });

  it('clears previously usable status after a failed recheck and sanitizes the error', async () => {
    render(<PermissionStatus />); await tick(3_000);
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏可用，键鼠可用');
    mocks.snapshot.mockResolvedValue({ ok: false, error: { message: 'Bearer private-secret C:\\private\\key' } });
    fireEvent.focus(window); await tick();
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏未能确认，键鼠未能确认');
    fireEvent.click(trigger()); await tick();
    expect(screen.getByText('未能读取本机权限状态，请重新检查。')).toBeInTheDocument();
    expect(screen.queryByText(/private-secret|private\\key/)).not.toBeInTheDocument();
  });

  it('treats an invalid snapshot as unconfirmed', async () => {
    mocks.snapshot.mockResolvedValue({ ok: true, value: { ...snapshot(), capabilities: null } });
    render(<PermissionStatus />); await tick();
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏未能确认，键鼠未能确认');
  });

  it('rejects an unknown snapshot platform rather than claiming input is usable', async () => {
    mocks.snapshot.mockResolvedValue({ ok: true, value: { ...snapshot(), platform: 'not-a-platform' } });
    render(<PermissionStatus />); await tick();
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏未能确认，键鼠未能确认');
  });

  it.each(['platform', 'domain'] as const)('does not accept a capture item with a mismatched %s', async field => {
    const value = snapshot();
    if (field === 'platform') value.capabilities[0]!.platform = 'windows';
    else value.capabilities[0]!.domain = 'control';
    mocks.snapshot.mockResolvedValue({ ok: true, value }); render(<PermissionStatus />); await tick();
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏未能确认，键鼠可用');
  });

  it('does not show stale cached capabilities as currently usable', async () => {
    mocks.snapshot.mockResolvedValue({ ok: true, value: { ...snapshot(), updated_at_ms: Date.now() - 60_000 } });
    render(<PermissionStatus />); await tick(3_000);
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏未能确认，键鼠未能确认');
  });

  it('expires a usable snapshot after thirty seconds without needing a focus event', async () => {
    render(<PermissionStatus />); await tick(3_000);
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏可用，键鼠可用');
    const reads = mocks.snapshot.mock.calls.length;
    await tick(30_000);
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏未能确认，键鼠未能确认');
    expect(mocks.snapshot).toHaveBeenCalledTimes(reads);
  });

  it('supports a read-only retry and labels the timestamp as a snapshot update', async () => {
    mocks.snapshot.mockResolvedValue({ ok: false, error: { message: 'unavailable' } });
    render(<PermissionStatus />); await tick(); fireEvent.click(trigger()); await tick();
    mocks.snapshot.mockImplementation(async () => ({ ok: true, value: snapshot() }));
    fireEvent.click(screen.getByRole('button', { name: '重新检查本机权限' })); await tick(3_000);
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏可用，键鼠可用');
    expect(screen.getByText(/^快照更新时间：/)).toBeInTheDocument();
    expect(screen.queryByText(/最近成功检查/)).not.toBeInTheDocument();
  });

  it('bounds sequential cached refreshes and never overlaps IPC during repeated focus events', async () => {
    let finish!: (value: unknown) => void;
    mocks.snapshot.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }))
      .mockResolvedValue({ ok: true, value: snapshot('supported', 'supported') });
    render(<PermissionStatus />); await tick();
    fireEvent.focus(window); fireEvent.focus(window); fireEvent.click(trigger()); await tick(3_000);
    expect(mocks.snapshot).toHaveBeenCalledTimes(1);
    await act(async () => { finish({ ok: true, value: snapshot('supported', 'supported') }); });
    await tick(10_000);
    expect(mocks.snapshot.mock.calls.length).toBeLessThanOrEqual(3);
    expect(trigger()).toHaveAccessibleName('本机系统权限：录屏待检查，键鼠待检查');
  });

  it('cancels later refreshes when an in-flight result arrives after unmount', async () => {
    let finish!: (value: unknown) => void;
    mocks.snapshot.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const view = render(<PermissionStatus />); await tick(); view.unmount();
    await act(async () => { finish({ ok: true, value: snapshot() }); }); await tick(10_000);
    expect(mocks.snapshot).toHaveBeenCalledTimes(1); expect(view.container).toBeEmptyDOMElement();
  });

  it('stops the sequential timer on unmount', async () => {
    const view = render(<PermissionStatus />); await tick(); view.unmount(); await tick(10_000);
    expect(mocks.snapshot).toHaveBeenCalledTimes(1);
  });
});
