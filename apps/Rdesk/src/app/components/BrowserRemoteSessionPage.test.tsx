import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes, useNavigate } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BrowserRemoteSessionPage } from './BrowserRemoteSessionPage';

type State = { phase: 'waiting_consent' | 'negotiating' | 'streaming' | 'denied' | 'failed' | 'closed'; grantedScopes: string[]; error?: string; route?: 'direct' | 'relay' | null };
type Observer = { onState: (state: State) => void; onVideoStream: (stream: MediaStream) => void };
const mocks = vi.hoisted(() => ({
  attach: vi.fn(), sendInput: vi.fn(), sendPointerAction: vi.fn(), releaseInputs: vi.fn(), close: vi.fn(), markVideoReady: vi.fn(),
  observer: null as Observer | null,
}));
vi.mock('../services/browserRemoteSessionService', () => ({ attachBrowserRemoteSession: mocks.attach }));

function SessionSwitch() {
  const navigate = useNavigate();
  return <button onClick={() => navigate('/browser-session/session-B')}>切换到会话 B</button>;
}
function show() {
  return render(<MemoryRouter initialEntries={['/browser-session/session-browser']}><SessionSwitch /><Routes>
    <Route path="/browser-session/:id" element={<BrowserRemoteSessionPage />} />
    <Route path="/devices" element={<div>设备列表</div>} />
  </Routes></MemoryRouter>);
}
async function state(phase: State['phase'], grantedScopes = ['screen.view', 'input.pointer', 'input.keyboard'], error?: string) {
  await act(async () => mocks.observer?.onState({ phase, grantedScopes, error }));
}
async function videoReady() {
  const video = screen.getByLabelText('远端屏幕') as HTMLVideoElement;
  Object.defineProperties(video, { videoWidth: { configurable: true, value: 1920 }, videoHeight: { configurable: true, value: 1080 } });
  await act(async () => {
    mocks.observer?.onVideoStream(new MediaStream());
    fireEvent.loadedData(video);
  });
  return video;
}
function inputSurface() {
  const surface = screen.getByRole('application', { name: '远程桌面键鼠控制' });
  vi.spyOn(surface, 'getBoundingClientRect').mockReturnValue({ left: 0, top: 0, width: 960, height: 540, right: 960, bottom: 540, x: 0, y: 0, toJSON: () => ({}) });
  return surface;
}
function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function wheelEvent() {
  const wheel = new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaY: 120, deltaX: -30 });
  Object.defineProperties(wheel, { clientX: { value: 100 }, clientY: { value: 100 } });
  return wheel;
}

describe('independent browser remote desktop', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.observer = null;
    mocks.sendInput.mockResolvedValue(undefined);
    mocks.sendPointerAction.mockResolvedValue(undefined);
    mocks.releaseInputs.mockResolvedValue(undefined);
    mocks.close.mockResolvedValue(undefined);
    mocks.attach.mockImplementation((_id: string, observer: Observer) => {
      mocks.observer = observer;
      observer.onState({ phase: 'waiting_consent', grantedScopes: [] });
      return { sendInput: mocks.sendInput, sendPointerAction: mocks.sendPointerAction, releaseInputs: mocks.releaseInputs, close: mocks.close, markVideoReady: mocks.markVideoReady };
    });
    vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(undefined);
  });
  afterEach(() => vi.restoreAllMocks());

  it('attaches the temporary identity and waits for remote consent', async () => {
    show();
    expect(await screen.findByText('等待远端同意')).toBeInTheDocument();
    expect(mocks.attach).toHaveBeenCalledWith('session-browser', expect.objectContaining({ onState: expect.any(Function), onVideoStream: expect.any(Function) }));
    expect(screen.getByText(/远端同意后才会显示画面/)).toBeInTheDocument();
    expect(screen.getByRole('application')).toHaveAttribute('aria-disabled', 'true');
    for (const name of ['音频（暂不支持）', '文件传输（暂不支持）', '无人值守（暂不支持）', '远程应用（暂不支持）']) expect(screen.getByRole('button', { name })).toBeDisabled();
  });

  it('does not enable input on negotiation or a claimed streaming state before a real first frame', async () => {
    show();
    await state('negotiating');
    expect(screen.getByText('建立安全连接')).toBeInTheDocument();
    await state('streaming');
    const surface = inputSurface();
    fireEvent.keyDown(surface, { key: 'a', code: 'KeyA' });
    fireEvent.pointerDown(surface, { button: 0, buttons: 1, clientX: 480, clientY: 270, pointerId: 1 });
    expect(mocks.sendInput).not.toHaveBeenCalled();
    expect(screen.queryByText('画面已连接')).not.toBeInTheDocument();
    await videoReady();
    expect(mocks.markVideoReady).toHaveBeenCalledWith(1920, 1080);
    expect(screen.getByText('画面已连接')).toBeInTheDocument();
    expect(surface).toHaveAttribute('aria-disabled', 'false');
  });

  it('respects independent pointer and keyboard scope grants', async () => {
    show();
    await state('streaming', ['screen.view', 'input.pointer']);
    await videoReady();
    const surface = inputSurface();
    fireEvent.keyDown(surface, { key: 'a', code: 'KeyA' });
    expect(mocks.sendInput).not.toHaveBeenCalled();
    fireEvent.pointerDown(surface, { button: 2, buttons: 2, clientX: 480, clientY: 270, pointerId: 1 });
    expect(mocks.sendPointerAction).toHaveBeenCalledWith({ x: 960, y: 540 }, { kind: 'mouse_button', button: 'right', pressed: true });
    expect(mocks.sendInput).not.toHaveBeenCalled();
    mocks.sendInput.mockClear();
    await state('streaming', ['screen.view', 'input.keyboard']);
    fireEvent.pointerDown(surface, { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 2 });
    fireEvent.keyDown(surface, { key: 'Control', code: 'ControlLeft' });
    fireEvent.keyDown(surface, { key: 'a', code: 'KeyA' });
    fireEvent.keyUp(surface, { key: 'a', code: 'KeyA' });
    expect(mocks.sendInput.mock.calls.map(([event]) => event.kind)).toEqual(['key', 'key', 'key']);
  });

  it('forwards mouse buttons and both wheel axes as atomic positioned actions', async () => {
    show(); await state('streaming'); await videoReady();
    const surface = inputSurface();
    fireEvent.pointerDown(surface, { button: 1, buttons: 4, clientX: 100, clientY: 100, pointerId: 3 });
    fireEvent.pointerUp(surface, { buttons: 0, button: 1, clientX: 100, clientY: 100, pointerId: 3 });
    const wheel = wheelEvent(); fireEvent(surface, wheel);
    expect(wheel.defaultPrevented).toBe(true);
    expect(mocks.sendPointerAction.mock.calls).toEqual([
      [{ x: 200, y: 200 }, { kind: 'mouse_button', button: 'middle', pressed: true }],
      [{ x: 200, y: 200 }, { kind: 'mouse_button', button: 'middle', pressed: false }],
      [{ x: 200, y: 200 }, { kind: 'mouse_wheel', delta: -120 }],
      [{ x: 200, y: 200 }, { kind: 'mouse_horizontal_wheel', delta: 30 }],
    ]);
    expect(mocks.sendInput).not.toHaveBeenCalled();
  });

  it.each([[0, 'left'], [1, 'middle'], [2, 'right']] as const)('uses the non-coalescing transaction for %s button down and up', async (button, name) => {
    show(); await state('streaming'); await videoReady();
    const transaction = deferred(); mocks.sendPointerAction.mockReturnValue(transaction.promise);
    const surface = inputSurface();
    fireEvent.pointerDown(surface, { button, buttons: [1, 4, 2, 8, 16][button], clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerUp(surface, { button, buttons: 0, clientX: 200, clientY: 200, pointerId: 1 });
    expect(mocks.sendPointerAction.mock.calls).toEqual([
      [{ x: 200, y: 200 }, { kind: 'mouse_button', button: name, pressed: true }],
      [{ x: 400, y: 400 }, { kind: 'mouse_button', button: name, pressed: false }],
    ]);
    expect(mocks.sendInput).not.toHaveBeenCalled();
    await act(async () => transaction.resolve());
  });

  it('hands wheel actions directly to the sole bounded control queue without accumulating page closures', async () => {
    show(); await state('streaming'); await videoReady();
    const transaction = deferred(); mocks.sendPointerAction.mockReturnValue(transaction.promise);
    const surface = inputSurface();
    for (let index = 0; index < 10; index += 1) fireEvent(surface, wheelEvent());
    expect(mocks.sendPointerAction).toHaveBeenCalledTimes(20);
    expect(mocks.sendInput).not.toHaveBeenCalled();
    await act(async () => transaction.resolve());
  });

  it('reports bounded control rejection without falling back to unsafe discrete input', async () => {
    show(); await state('streaming'); await videoReady();
    mocks.sendPointerAction.mockRejectedValue(new Error('输入队列已满'));
    fireEvent.pointerDown(inputSurface(), { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 1 });
    expect(await screen.findByRole('alert')).toHaveTextContent('输入队列已满');
    expect(mocks.sendInput).not.toHaveBeenCalled();
  });

  it('keeps continuous pointer movement on its ordinary coalescing path', async () => {
    show(); await state('streaming'); await videoReady();
    fireEvent.pointerMove(inputSurface(), { button: -1, buttons: 0, clientX: 100, clientY: 100, pointerId: 1 });
    expect(mocks.sendInput).toHaveBeenCalledWith({ kind: 'mouse_move', x: 200, y: 200 });
    expect(mocks.sendPointerAction).not.toHaveBeenCalled();
  });

  it('handles the standard four-event left/right chord without leaving either button held', async () => {
    show(); await state('streaming'); await videoReady();
    const surface = inputSurface();
    const releaseCapture = vi.fn();
    Object.defineProperty(surface, 'releasePointerCapture', { configurable: true, value: releaseCapture });
    fireEvent.pointerDown(surface, { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 1, pointerType: 'mouse' });
    fireEvent.pointerMove(surface, { button: 2, buttons: 3, clientX: 100, clientY: 100, pointerId: 1, pointerType: 'mouse' });
    fireEvent.pointerMove(surface, { button: 0, buttons: 2, clientX: 100, clientY: 100, pointerId: 1, pointerType: 'mouse' });
    expect(releaseCapture).not.toHaveBeenCalled();
    fireEvent.pointerUp(surface, { button: 2, buttons: 0, clientX: 100, clientY: 100, pointerId: 1, pointerType: 'mouse' });
    expect(mocks.sendPointerAction.mock.calls).toEqual([
      [{ x: 200, y: 200 }, { kind: 'mouse_button', button: 'left', pressed: true }],
      [{ x: 200, y: 200 }, { kind: 'mouse_button', button: 'right', pressed: true }],
      [{ x: 200, y: 200 }, { kind: 'mouse_button', button: 'left', pressed: false }],
      [{ x: 200, y: 200 }, { kind: 'mouse_button', button: 'right', pressed: false }],
    ]);
    expect(releaseCapture).toHaveBeenCalledTimes(1);
    expect(releaseCapture).toHaveBeenCalledWith(1);
  });

  it.each([[2, 2, 'right'], [1, 4, 'middle'], [3, 8, 'x1'], [4, 16, 'x2']] as const)('recognizes standard chord edges for button %s without overlapping pointerdown events', async (button, mask, name) => {
    show(); await state('streaming'); await videoReady();
    const surface = inputSurface();
    fireEvent.pointerDown(surface, { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 1, pointerType: 'mouse' });
    fireEvent.pointerMove(surface, { button, buttons: 1 | mask, clientX: 110, clientY: 110, pointerId: 1, pointerType: 'mouse' });
    fireEvent.pointerMove(surface, { button: 0, buttons: mask, clientX: 120, clientY: 120, pointerId: 1, pointerType: 'mouse' });
    fireEvent.pointerUp(surface, { button, buttons: 0, clientX: 130, clientY: 130, pointerId: 1, pointerType: 'mouse' });
    expect(mocks.sendPointerAction.mock.calls.map(([, action]) => action)).toEqual([
      { kind: 'mouse_button', button: 'left', pressed: true },
      { kind: 'mouse_button', button: name, pressed: true },
      { kind: 'mouse_button', button: 'left', pressed: false },
      { kind: 'mouse_button', button: name, pressed: false },
    ]);
  });

  it('releases every tracked button on the final pointerup even if an intermediate edge event was missed', async () => {
    show(); await state('streaming'); await videoReady();
    const surface = inputSurface();
    fireEvent.pointerDown(surface, { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 1, pointerType: 'mouse' });
    fireEvent.pointerMove(surface, { button: 2, buttons: 3, clientX: 100, clientY: 100, pointerId: 1, pointerType: 'mouse' });
    fireEvent.pointerUp(surface, { button: 2, buttons: 0, clientX: 100, clientY: 100, pointerId: 1, pointerType: 'mouse' });
    expect(mocks.sendPointerAction.mock.calls.map(([, action]) => action).filter(action => !action.pressed)).toEqual([
      { kind: 'mouse_button', button: 'left', pressed: false }, { kind: 'mouse_button', button: 'right', pressed: false },
    ]);
  });

  it('does not clear another touch pointer when one contact is cancelled', async () => {
    show(); await state('streaming'); await videoReady();
    const surface = inputSurface();
    mocks.releaseInputs.mockClear();
    fireEvent.pointerDown(surface, { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 11, pointerType: 'touch', isPrimary: true });
    fireEvent.pointerDown(surface, { button: 0, buttons: 1, clientX: 200, clientY: 200, pointerId: 12, pointerType: 'touch', isPrimary: false });
    fireEvent.pointerCancel(surface, { button: 0, buttons: 0, clientX: 100, clientY: 100, pointerId: 11, pointerType: 'touch', isPrimary: true });
    expect(mocks.releaseInputs).not.toHaveBeenCalled();
    expect(mocks.sendPointerAction.mock.calls.filter(([, action]) => action.kind === 'mouse_button' && !action.pressed)).toHaveLength(0);
    fireEvent.pointerMove(surface, { button: -1, buttons: 1, clientX: 210, clientY: 210, pointerId: 12, pointerType: 'touch', isPrimary: false });
    fireEvent.pointerUp(surface, { button: 0, buttons: 0, clientX: 210, clientY: 210, pointerId: 12, pointerType: 'touch', isPrimary: false });
    expect(mocks.sendPointerAction.mock.calls.map(([, action]) => action)).toEqual([
      { kind: 'mouse_button', button: 'left', pressed: true }, { kind: 'mouse_button', button: 'left', pressed: false },
    ]);
  });

  it('does not report the expected rejection of a cancelled pointer transaction as a new input failure', async () => {
    show(); await state('streaming'); await videoReady();
    const action = deferred(); mocks.sendPointerAction.mockReturnValueOnce(action.promise);
    const surface = inputSurface();
    fireEvent.pointerDown(surface, { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 1, pointerType: 'mouse' });
    fireEvent.pointerCancel(surface, { button: 0, buttons: 0, pointerId: 1, pointerType: 'mouse' });
    await act(async () => action.reject(new Error('待发送指针输入已取消')));
    expect(mocks.releaseInputs).toHaveBeenCalledWith('pointer_cancel');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('keeps a generic modifier held until both physical sides have been released', async () => {
    show(); await state('streaming'); await videoReady();
    const surface = inputSurface();
    fireEvent.keyDown(surface, { key: 'Shift', code: 'ShiftLeft' });
    fireEvent.keyDown(surface, { key: 'Shift', code: 'ShiftRight' });
    fireEvent.keyUp(surface, { key: 'Shift', code: 'ShiftLeft' });
    expect(mocks.sendInput.mock.calls.map(([event]) => event)).toEqual([{ kind: 'key', key: { kind: 'virtual_key', code: 0x10 }, pressed: true }]);
    fireEvent.keyUp(surface, { key: 'Shift', code: 'ShiftRight' });
    expect(mocks.sendInput).toHaveBeenLastCalledWith({ kind: 'key', key: { kind: 'virtual_key', code: 0x10 }, pressed: false });
  });

  it('keeps input confirmation failures visible to the user', async () => {
    show(); await state('streaming'); await videoReady();
    mocks.sendInput.mockRejectedValue(new Error('远端输入确认超时'));
    fireEvent.keyDown(inputSurface(), { key: 'a', code: 'KeyA' });
    expect(await screen.findByRole('alert')).toHaveTextContent('远端输入确认超时');
  });

  it('allows Escape to leave the control focus and releases remote held keys', async () => {
    show(); await state('streaming'); await videoReady();
    const surface = inputSurface(); surface.focus();
    fireEvent.keyDown(surface, { key: 'Escape', code: 'Escape' });
    expect(screen.getByRole('button', { name: '断开并返回设备列表' })).toHaveFocus();
    expect(mocks.releaseInputs).toHaveBeenCalledWith('surface_blur');
  });

  it('rejects letterbox clicks and releases input when a captured pointer is cancelled', async () => {
    show(); await state('streaming'); await videoReady();
    const surface = inputSurface();
    vi.mocked(surface.getBoundingClientRect).mockReturnValue({ left: 0, top: 0, width: 1000, height: 1000, right: 1000, bottom: 1000, x: 0, y: 0, toJSON: () => ({}) });
    fireEvent.pointerDown(surface, { button: 0, buttons: 1, clientX: 500, clientY: 20, pointerId: 1 });
    expect(mocks.sendInput).not.toHaveBeenCalled();
    fireEvent.pointerDown(surface, { button: 0, buttons: 1, clientX: 500, clientY: 500, pointerId: 1 });
    fireEvent.pointerCancel(surface, { pointerId: 1 });
    expect(mocks.releaseInputs).toHaveBeenCalledWith('pointer_cancel');
  });

  it('releases held inputs on blur, hidden document, and disconnection', async () => {
    show(); await state('streaming'); await videoReady();
    fireEvent.blur(inputSurface());
    expect(mocks.releaseInputs).toHaveBeenCalledWith('surface_blur');
    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
    fireEvent(document, new Event('visibilitychange'));
    expect(mocks.releaseInputs).toHaveBeenCalledWith('document_hidden');
    await state('failed', ['screen.view'], 'ICE 连接失败');
    expect(mocks.releaseInputs).toHaveBeenCalledWith('session_inactive');
    expect(screen.getByRole('alert')).toHaveTextContent('ICE 连接失败');
    expect(screen.getByRole('application')).toHaveAttribute('aria-disabled', 'true');
    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
  });

  it.each([['denied', '远端已拒绝'], ['closed', '会话已结束']] as const)('shows the authoritative %s state', async (phase, label) => {
    show(); await state(phase);
    expect(screen.getByText(label)).toBeInTheDocument();
    expect(screen.getByRole('application')).toHaveAttribute('aria-disabled', 'true');
  });

  it('shows a missing temporary identity without pretending to resume after refresh', async () => {
    mocks.attach.mockImplementation(() => { throw new Error('临时会话身份已失效，请从设备列表重新连接'); });
    show();
    expect(await screen.findByRole('alert')).toHaveTextContent('临时会话身份已失效');
    expect(screen.getByRole('button', { name: '返回设备列表' })).toBeEnabled();
  });

  it('cancels the browser-owned session before returning and ignores late callbacks', async () => {
    const view = show();
    await userEvent.click(screen.getByRole('button', { name: '取消连接' }));
    expect(await screen.findByText('设备列表')).toBeInTheDocument();
    expect(mocks.releaseInputs).toHaveBeenCalledWith('leave_page');
    expect(mocks.close).toHaveBeenCalledTimes(1);
    expect(mocks.close).toHaveBeenCalledWith('leave_page');
    await state('streaming');
    expect(screen.queryByText('画面已连接')).not.toBeInTheDocument();
    view.unmount();
  });

  it('disables local input immediately and waits for explicit remote close confirmation before navigating', async () => {
    show(); await state('streaming'); await videoReady();
    const closing = deferred(); mocks.close.mockReturnValue(closing.promise);
    await userEvent.click(screen.getByRole('button', { name: '断开并返回设备列表' }));
    expect(screen.queryByText('设备列表')).not.toBeInTheDocument();
    expect(screen.getByRole('application')).toHaveAttribute('aria-disabled', 'true');
    expect(screen.getByRole('button', { name: '正在结束会话' })).toBeDisabled();
    await act(async () => closing.resolve());
    expect(await screen.findByText('设备列表')).toBeInTheDocument();
  });

  it('does not bypass a pending close when repeated clicks arrive in the same render', async () => {
    show(); await state('streaming'); await videoReady();
    const closing = deferred(); mocks.close.mockReturnValue(closing.promise);
    const button = screen.getByRole('button', { name: '断开并返回设备列表' });
    act(() => { fireEvent.click(button); fireEvent.click(button); });
    expect(screen.queryByText('设备列表')).not.toBeInTheDocument();
    expect(mocks.close).toHaveBeenCalledTimes(1);
    await act(async () => closing.resolve());
    expect(await screen.findByText('设备列表')).toBeInTheDocument();
  });

  it('keeps a closed page and visible warning when explicit remote close fails, then allows returning', async () => {
    show(); await state('streaming'); await videoReady();
    const closing = deferred(); mocks.close.mockReturnValue(closing.promise);
    await userEvent.click(screen.getByRole('button', { name: '断开并返回设备列表' }));
    await act(async () => closing.reject(new Error('HTTP 503 远端撤销不可用')));
    expect(screen.queryByText('设备列表')).not.toBeInTheDocument();
    expect(await screen.findByRole('alert')).toHaveTextContent('HTTP 503 远端撤销不可用');
    expect(screen.getByRole('alert')).toHaveTextContent('无法确认远端已停止');
    expect(screen.getByRole('application')).toHaveAttribute('aria-disabled', 'true');
    await state('streaming');
    expect(screen.getByText('会话已结束')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '返回设备列表' }));
    expect(await screen.findByText('设备列表')).toBeInTheDocument();
    expect(mocks.close).toHaveBeenCalledTimes(1);
  });

  it('does not let a previous session observer or close rejection overwrite or release the next route session', async () => {
    show();
    const oldObserver = mocks.observer!;
    const oldClose = deferred(); mocks.close.mockReturnValueOnce(oldClose.promise);
    await userEvent.click(screen.getByRole('button', { name: '取消连接' }));
    await userEvent.click(screen.getByRole('button', { name: '切换到会话 B' }));
    await state('streaming'); await videoReady();
    const video = screen.getByLabelText('远端屏幕') as HTMLVideoElement;
    const currentStream = video.srcObject;
    const releases = mocks.releaseInputs.mock.calls.length;
    await act(async () => {
      oldObserver.onVideoStream(new MediaStream());
      oldObserver.onState({ phase: 'failed', grantedScopes: [], error: '会话 A 关闭失败' });
    });
    expect(video.srcObject).toBe(currentStream);
    expect(screen.getByText('画面已连接')).toBeInTheDocument();
    expect(mocks.releaseInputs).toHaveBeenCalledTimes(releases);
    await act(async () => oldClose.reject(new Error('旧会话 A close HTTP 503')));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByText('画面已连接')).toBeInTheDocument();
    expect(screen.queryByText('设备列表')).not.toBeInTheDocument();
  });

  it('does not navigate away from a reused route when the old session close succeeds late', async () => {
    show();
    const oldClose = deferred(); mocks.close.mockReturnValueOnce(oldClose.promise);
    await userEvent.click(screen.getByRole('button', { name: '取消连接' }));
    await userEvent.click(screen.getByRole('button', { name: '切换到会话 B' }));
    await state('streaming'); await videoReady();
    await act(async () => oldClose.resolve());
    expect(screen.queryByText('设备列表')).not.toBeInTheDocument();
    expect(screen.getByText('画面已连接')).toBeInTheDocument();
  });

  it('does not surface an old positioned input rejection in the next route session', async () => {
    show(); await state('streaming'); await videoReady();
    const oldInput = deferred(); mocks.sendPointerAction.mockReturnValueOnce(oldInput.promise);
    fireEvent.pointerDown(inputSurface(), { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 1 });
    await userEvent.click(screen.getByRole('button', { name: '切换到会话 B' }));
    await state('streaming'); await videoReady();
    await act(async () => oldInput.reject(new Error('旧会话输入确认失败')));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByText('画面已连接')).toBeInTheDocument();
  });

  it('does not surface a previous stream play rejection after the route session changes', async () => {
    show(); await state('streaming');
    const oldPlay = deferred(); vi.mocked(HTMLMediaElement.prototype.play).mockReturnValueOnce(oldPlay.promise);
    await videoReady();
    await userEvent.click(screen.getByRole('button', { name: '切换到会话 B' }));
    await state('streaming'); await videoReady();
    await act(async () => oldPlay.reject(new Error('旧流播放失败')));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByText('画面已连接')).toBeInTheDocument();
  });

  it('cleans input and the owned session when unmounted', async () => {
    const view = show(); await state('streaming'); const video = await videoReady();
    view.unmount();
    expect(mocks.releaseInputs).toHaveBeenCalledWith('leave_page');
    expect(mocks.close).toHaveBeenCalledTimes(1);
    expect(video.srcObject).toBeNull();
  });

  it('supports a keyboard-accessible fullscreen button and displays failures', async () => {
    show(); await state('streaming'); await videoReady();
    const full = screen.getByRole('button', { name: '进入全屏' });
    const surface = screen.getByRole('application');
    Object.defineProperty(surface, 'requestFullscreen', { configurable: true, value: vi.fn().mockRejectedValue(new Error('全屏被浏览器拒绝')) });
    await userEvent.click(full);
    expect(await screen.findByRole('alert')).toHaveTextContent('全屏被浏览器拒绝');
  });
});
