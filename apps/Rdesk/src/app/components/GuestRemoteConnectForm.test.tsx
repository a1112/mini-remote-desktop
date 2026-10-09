import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { GuestRemoteConnectForm } from './GuestRemoteConnectForm';

const mocks = vi.hoisted(() => ({ launch: vi.fn(), navigate: vi.fn() }));
vi.mock('react-router', () => ({ useNavigate: () => mocks.navigate }));
vi.mock('../services/remoteDisplayLauncher', () => ({ launchRemoteDisplayForDevice: mocks.launch }));
beforeEach(() => { vi.clearAllMocks(); mocks.launch.mockResolvedValue({ sessionId: 'guest-session', mode: 'route', routePath: '/browser-session/guest-session' }); });

describe('guest remote connection form', () => {
  it('submits an existing nine digit code and password without requiring account login', async () => {
    const user = userEvent.setup(); render(<GuestRemoteConnectForm />);
    await user.type(screen.getByLabelText('远程设备码'), '753 662 296');
    const password = screen.getByLabelText('远端临时密码');
    expect(password).toHaveAttribute('type', 'password');
    await user.type(password, 'ABCD2345');
    await user.click(screen.getByRole('button', { name: '立即连接' }));
    expect(mocks.launch).toHaveBeenCalledWith('753662296', expect.objectContaining({ temporaryPassword: 'ABCD2345', routePreference: 'auto' }));
    expect(password).toHaveValue('');
    expect(mocks.navigate).toHaveBeenCalledWith('/browser-session/guest-session');
  });

  it('rejects malformed input and never changes a device identifier to make it connect', async () => {
    const user = userEvent.setup(); render(<GuestRemoteConnectForm />);
    await user.type(screen.getByLabelText('远程设备码'), '753662296/other');
    await user.type(screen.getByLabelText('远端临时密码'), 'ABCD2345');
    await user.click(screen.getByRole('button', { name: '立即连接' }));
    expect(mocks.launch).not.toHaveBeenCalled(); expect(screen.getByRole('alert')).toHaveTextContent('设备码');
  });

  it('keeps the password out of the URL and clears it even if the server rejects it', async () => {
    mocks.launch.mockRejectedValue(new Error('临时密码错误或已失效'));
    const user = userEvent.setup(); render(<GuestRemoteConnectForm />);
    await user.type(screen.getByLabelText('远程设备码'), '753662296');
    await user.type(screen.getByLabelText('远端临时密码'), 'ABCD2345');
    await user.click(screen.getByRole('button', { name: '立即连接' }));
    expect(screen.getByRole('alert')).toHaveTextContent('临时密码错误或已失效');
    expect(screen.getByLabelText('远端临时密码')).toHaveValue(''); expect(mocks.navigate).not.toHaveBeenCalled();
  });

  it('coalesces submission while waiting for the server and ignores navigation after unmount', async () => {
    let resolve!: (value: unknown) => void; mocks.launch.mockImplementation(() => new Promise(done => { resolve = done; }));
    const user = userEvent.setup(); const view = render(<GuestRemoteConnectForm />);
    await user.type(screen.getByLabelText('远程设备码'), '753662296'); await user.type(screen.getByLabelText('远端临时密码'), 'ABCD2345');
    await user.dblClick(screen.getByRole('button', { name: '立即连接' }));
    expect(mocks.launch).toHaveBeenCalledTimes(1); view.unmount();
    await act(async () => resolve({ sessionId: 'guest-session', mode: 'route', routePath: '/browser-session/guest-session' }));
    expect(mocks.navigate).not.toHaveBeenCalled();
  });
});
