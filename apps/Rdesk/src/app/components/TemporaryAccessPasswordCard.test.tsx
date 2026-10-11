import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { TemporaryAccessPasswordCard } from './TemporaryAccessPasswordCard';

const mocks = vi.hoisted(() => ({ status: vi.fn(), secret: vi.fn(), rotate: vi.fn(), disable: vi.fn() }));
vi.mock('../services/temporaryAccessService', () => ({
  getTemporaryAccessStatus: mocks.status, readTemporaryAccessPassword: mocks.secret,
  rotateTemporaryAccessPassword: mocks.rotate, disableTemporaryAccess: mocks.disable,
}));
const ready = () => ({ enabled: true, ready: true, generation: 1, expires_at_ms: Date.now() + 600000, reason: null });
beforeEach(() => { vi.clearAllMocks(); mocks.status.mockResolvedValue(ready()); mocks.secret.mockResolvedValue({ status: ready(), password: 'ABCD2345' }); mocks.rotate.mockResolvedValue({ ...ready(), generation: 2 }); mocks.disable.mockResolvedValue({ ...ready(), ready: false, enabled: false }); });

describe('local temporary password controls', () => {
  it('shows manual refresh instructions without a password rotation countdown', async () => {
    mocks.status.mockResolvedValue({ ...ready(), refresh_mode: 'manual' });
    render(<TemporaryAccessPasswordCard />);
    await screen.findByText('临时密码可用');
    expect(screen.getByText('手动刷新，刷新后旧密码失效。')).toBeInTheDocument();
    expect(screen.getByText('服务重启后会生成新密码。')).toBeInTheDocument();
    expect(screen.queryByText(/剩余 \d+:\d+/)).not.toBeInTheDocument();
    expect(mocks.secret).not.toHaveBeenCalled();
  });

  it.each([undefined, 'automatic'])('keeps the countdown for an unchanged service with mode %s', async refresh_mode => {
    mocks.status.mockResolvedValue({ ...ready(), ...(refresh_mode ? { refresh_mode } : {}) });
    render(<TemporaryAccessPasswordCard />);
    await screen.findByText('临时密码可用');
    expect(screen.getByText(/剩余 \d+:\d+/)).toBeInTheDocument();
    expect(screen.queryByText('手动刷新，刷新后旧密码失效。')).not.toBeInTheDocument();
  });

  it('keeps polling through the former rotation deadline without reading or rotating the password', async () => {
    vi.useFakeTimers();
    mocks.status.mockImplementation(async () => ({ ...ready(), refresh_mode: 'manual' }));
    let view: ReturnType<typeof render> | undefined;
    try {
      await act(async () => { view = render(<TemporaryAccessPasswordCard />); });
      expect(screen.getByText('临时密码可用')).toBeInTheDocument();
      await act(async () => { await vi.advanceTimersByTimeAsync(660000); });
      expect(mocks.status.mock.calls.length).toBeGreaterThan(1);
      expect(mocks.rotate).not.toHaveBeenCalled();
      expect(mocks.secret).not.toHaveBeenCalled();
      expect(screen.getByText('临时密码可用')).toBeInTheDocument();
      expect(screen.queryByText(/剩余 \d+:\d+/)).not.toBeInTheDocument();
    } finally { view?.unmount(); vi.useRealTimers(); }
  });

  it('hides a revealed secret at its original deadline even when a manual publication renews', async () => {
    vi.useFakeTimers();
    const initial = { ...ready(), refresh_mode: 'manual', expires_at_ms: Date.now() + 6000 };
    mocks.status.mockResolvedValueOnce(initial).mockImplementation(async () => ({ ...ready(), refresh_mode: 'manual' }));
    mocks.secret.mockResolvedValue({ status: initial, password: 'ABCD2345' });
    let view: ReturnType<typeof render> | undefined;
    try {
      await act(async () => { view = render(<TemporaryAccessPasswordCard />); });
      await act(async () => { fireEvent.click(screen.getByRole('button', { name: '显示临时密码' })); });
      expect(screen.getByText('ABCD2345')).toBeInTheDocument();
      await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
      expect(screen.getByText('临时密码可用')).toBeInTheDocument();
      expect(screen.queryByText('ABCD2345')).not.toBeInTheDocument();
      expect(mocks.secret).toHaveBeenCalledTimes(1);
      expect(mocks.rotate).not.toHaveBeenCalled();
    } finally { view?.unmount(); vi.useRealTimers(); }
  });

  it('marks expired manual access unavailable without claiming the unchanged password expired', async () => {
    mocks.status.mockResolvedValue({ ...ready(), refresh_mode: 'manual', ready: false, expires_at_ms: Date.now() - 1 });
    render(<TemporaryAccessPasswordCard />);
    expect(await screen.findByText('临时访问暂不可用')).toBeInTheDocument();
    expect(screen.queryByText('临时密码已过期')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: '复制临时密码' })).toBeDisabled();
  });

  it('shows a hidden usable password without reading a secret until reveal is requested', async () => {
    render(<TemporaryAccessPasswordCard />);
    expect(await screen.findByText('临时密码可用')).toBeInTheDocument();
    expect(screen.queryByText('ABCD2345')).not.toBeInTheDocument(); expect(mocks.secret).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: '显示临时密码' }));
    expect(await screen.findByText('ABCD2345')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '隐藏临时密码' }));
    expect(screen.queryByText('ABCD2345')).not.toBeInTheDocument();
  });

  it('copies the exact secret only through the protected read without revealing it on screen', async () => {
    const user = userEvent.setup(); const copy = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue();
    render(<TemporaryAccessPasswordCard />); await screen.findByText('临时密码可用');
    await user.click(screen.getByRole('button', { name: '复制临时密码' }));
    expect(copy).toHaveBeenCalledWith('ABCD2345'); expect(screen.queryByText('ABCD2345')).not.toBeInTheDocument();
  });

  it('drops a previously displayed password when refresh changes its generation', async () => {
    mocks.status.mockResolvedValue({ ...ready(), refresh_mode: 'manual' });
    mocks.secret.mockResolvedValue({ status: { ...ready(), refresh_mode: 'manual' }, password: 'ABCD2345' });
    mocks.rotate.mockResolvedValue({ ...ready(), refresh_mode: 'manual', generation: 2 });
    render(<TemporaryAccessPasswordCard />); await screen.findByText('临时密码可用');
    await userEvent.click(screen.getByRole('button', { name: '显示临时密码' })); await screen.findByText('ABCD2345');
    await userEvent.click(screen.getByRole('button', { name: '刷新临时密码' }));
    expect(mocks.rotate).toHaveBeenCalledTimes(1); expect(screen.queryByText('ABCD2345')).not.toBeInTheDocument();
  });

  it('rejects a stale secret response instead of copying an older generation', async () => {
    const user = userEvent.setup(); const copy = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue();
    mocks.secret.mockResolvedValue({ status: { ...ready(), generation: 0 }, password: 'ABCD2345' });
    render(<TemporaryAccessPasswordCard />); await screen.findByText('临时密码可用');
    await user.click(screen.getByRole('button', { name: '复制临时密码' }));
    expect(copy).not.toHaveBeenCalled(); expect(screen.queryByText('ABCD2345')).not.toBeInTheDocument(); expect(screen.getByRole('alert')).toHaveTextContent('已变化');
  });

  it('disables temporary access and removes any displayed secret', async () => {
    render(<TemporaryAccessPasswordCard />); await screen.findByText('临时密码可用');
    await userEvent.click(screen.getByRole('button', { name: '显示临时密码' })); await screen.findByText('ABCD2345');
    await userEvent.click(screen.getByRole('button', { name: '关闭临时访问' }));
    expect(mocks.disable).toHaveBeenCalledTimes(1); expect(await screen.findByText('临时访问已关闭')).toBeInTheDocument();
    expect(screen.queryByText('ABCD2345')).not.toBeInTheDocument(); expect(screen.getByRole('button', { name: '复制临时密码' })).toBeDisabled();
  });

  it('ignores a secret response after the component unmounts', async () => {
    let resolve!: (value: unknown) => void; mocks.secret.mockImplementation(() => new Promise(done => { resolve = done; }));
    const view = render(<TemporaryAccessPasswordCard />); await screen.findByText('临时密码可用');
    await userEvent.click(screen.getByRole('button', { name: '显示临时密码' })); view.unmount();
    await act(async () => resolve({ status: ready(), password: 'ABCD2345' })); expect(screen.queryByText('ABCD2345')).not.toBeInTheDocument();
  });

  it.each(['显示临时密码', '复制临时密码'])('discards a late secret after the page is hidden during %s', async name => {
    const user = userEvent.setup(); const copy = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue();
    let resolve!: (value: unknown) => void; mocks.secret.mockImplementation(() => new Promise(done => { resolve = done; }));
    let hidden = false; const visibility = vi.spyOn(document, 'hidden', 'get').mockImplementation(() => hidden);
    try {
      render(<TemporaryAccessPasswordCard />); await screen.findByText('临时密码可用');
      await user.click(screen.getByRole('button', { name }));
      await act(async () => { hidden = true; document.dispatchEvent(new Event('visibilitychange')); resolve({ status: ready(), password: 'ABCD2345' }); });
      expect(screen.queryByText('ABCD2345')).not.toBeInTheDocument(); expect(copy).not.toHaveBeenCalled();
      await act(async () => { hidden = false; document.dispatchEvent(new Event('visibilitychange')); });
      expect(screen.queryByText('ABCD2345')).not.toBeInTheDocument();
    } finally { visibility.mockRestore(); }
  });
});
