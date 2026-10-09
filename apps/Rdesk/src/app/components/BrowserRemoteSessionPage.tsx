import { useCallback, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react';
import { useNavigate, useParams } from 'react-router';
import { ArrowLeft, Maximize2, Minimize2, Monitor, ShieldCheck } from 'lucide-react';
import { attachBrowserRemoteSession, type BrowserRemoteHandle, type BrowserRemoteState } from '../services/browserRemoteSessionService';
import type { ControlInputEvent } from '../adapters/tauri/types';
import { browserRemotePoint, browserVirtualKey, browserWheelDelta } from '../utils/browserRemoteInput';

const initialState: BrowserRemoteState = { phase: 'waiting_consent', grantedScopes: [] };
const phaseLabels = { waiting_consent: '等待远端同意', negotiating: '建立安全连接', streaming: '等待首帧画面', denied: '远端已拒绝', failed: '连接失败', closed: '会话已结束' };
const inactive = (phase: BrowserRemoteState['phase']) => phase === 'denied' || phase === 'failed' || phase === 'closed';
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const pointerButtonMasks = [['left', 1], ['right', 2], ['middle', 4], ['x1', 8], ['x2', 16]] as const;
type PointerButtons = { buttons: number; coordinates: { x: number; y: number } };

export function BrowserRemoteSessionPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const [state, setState] = useState<BrowserRemoteState>(initialState);
  const [firstFrame, setFirstFrame] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const [closing, setClosing] = useState(false);
  const video = useRef<HTMLVideoElement>(null);
  const surface = useRef<HTMLDivElement>(null);
  const leaveButton = useRef<HTMLButtonElement>(null);
  const handle = useRef<BrowserRemoteHandle | null>(null);
  const hasStream = useRef(false);
  const keys = useRef(new Map<string, number>());
  const pointers = useRef(new Map<number, PointerButtons>());
  const alive = useRef(false);
  const locallyClosed = useRef(false);
  const closePending = useRef(false);
  const sessionGeneration = useRef(0);
  const inputGeneration = useRef(0);
  const ready = !locallyClosed.current && firstFrame && state.phase === 'streaming' && state.grantedScopes.includes('screen.view');
  const pointerEnabled = ready && state.grantedScopes.includes('input.pointer');
  const keyboardEnabled = ready && state.grantedScopes.includes('input.keyboard');

  const release = useCallback((reason: string) => {
    inputGeneration.current += 1;
    keys.current.clear();
    pointers.current.clear();
    void handle.current?.releaseInputs(reason).catch(() => undefined);
  }, []);
  const close = useCallback((reason: string) => {
    inputGeneration.current += 1;
    locallyClosed.current = true;
    const owned = handle.current;
    handle.current = null;
    keys.current.clear(); pointers.current.clear();
    hasStream.current = false;
    if (video.current) video.current.srcObject = null;
    if (alive.current) { setFirstFrame(false); setState({ phase: 'closed', grantedScopes: [] }); }
    if (!owned) return Promise.resolve();
    void owned.releaseInputs(reason).catch(() => undefined);
    return owned.close(reason);
  }, []);
  useEffect(() => {
    const generation = ++sessionGeneration.current;
    const current = () => alive.current && sessionGeneration.current === generation;
    const ownedVideo = video.current;
    alive.current = true;
    locallyClosed.current = false;
    closePending.current = false;
    setState(initialState); setFirstFrame(false); setActionError(null); setClosing(false); hasStream.current = false;
    try {
      if (!id) throw new Error('未找到浏览器会话，请从设备列表重新连接');
      handle.current = attachBrowserRemoteSession(id, {
        onState: next => {
          if (!current() || locallyClosed.current) return;
          setState(next);
          if (inactive(next.phase)) { setFirstFrame(false); release('session_inactive'); }
        },
        onVideoStream: stream => {
          if (!current() || locallyClosed.current) return;
          hasStream.current = true; setFirstFrame(false);
          if (video.current) {
            video.current.srcObject = stream;
            void video.current.play().catch(error => { if (current() && !locallyClosed.current && video.current?.srcObject === stream) setActionError(message(error)); });
          }
        },
      });
    } catch (error) {
      setState({ phase: 'failed', grantedScopes: [], error: message(error) });
    }
    const onHidden = () => { if (document.visibilityState !== 'visible') release('document_hidden'); };
    const onBlur = () => release('window_blur');
    const onPageHide = () => { void close('page_hidden').catch(() => undefined); };
    const onFullscreen = () => { if (current()) setFullscreen(document.fullscreenElement === surface.current); };
    document.addEventListener('visibilitychange', onHidden);
    document.addEventListener('fullscreenchange', onFullscreen);
    window.addEventListener('blur', onBlur);
    window.addEventListener('pagehide', onPageHide);
    return () => {
      if (sessionGeneration.current === generation) sessionGeneration.current += 1;
      alive.current = false; hasStream.current = false;
      document.removeEventListener('visibilitychange', onHidden);
      document.removeEventListener('fullscreenchange', onFullscreen);
      window.removeEventListener('blur', onBlur);
      window.removeEventListener('pagehide', onPageHide);
      void close('leave_page').catch(() => undefined);
      if (ownedVideo) ownedVideo.srcObject = null;
    };
  }, [id, close, release]);
  useEffect(() => { if (!pointerEnabled || !keyboardEnabled) release('permissions_changed'); }, [pointerEnabled, keyboardEnabled, release]);

  const send = useCallback((input: ControlInputEvent) => {
    const owned = handle.current;
    const generation = sessionGeneration.current;
    const inputEpoch = inputGeneration.current;
    void owned?.sendInput(input).catch(error => {
      if (alive.current && sessionGeneration.current === generation && inputGeneration.current === inputEpoch && handle.current === owned && !locallyClosed.current) setActionError(message(error));
    });
  }, []);
  const sendPointerAction = useCallback((coordinates: { x: number; y: number }, action: ControlInputEvent) => {
    const owned = handle.current;
    const generation = sessionGeneration.current;
    const inputEpoch = inputGeneration.current;
    // Control owns the only bounded FIFO and the non-coalescing position barrier.
    void owned?.sendPointerAction(coordinates, action).catch(error => {
      if (alive.current && sessionGeneration.current === generation && inputGeneration.current === inputEpoch && handle.current === owned && !locallyClosed.current) setActionError(message(error));
    });
  }, []);
  const markReady = () => {
    const element = video.current;
    if (!hasStream.current || !element || element.videoWidth <= 0 || element.videoHeight <= 0 || inactive(state.phase)) return;
    setFirstFrame(true);
    handle.current?.markVideoReady(element.videoWidth, element.videoHeight);
  };
  const point = useCallback((clientX: number, clientY: number) => {
    const element = video.current;
    if (!surface.current || !element) return null;
    return browserRemotePoint(clientX, clientY, surface.current.getBoundingClientRect(), { width: element.videoWidth, height: element.videoHeight });
  }, []);
  useEffect(() => {
    const element = surface.current;
    if (!element || !pointerEnabled) return;
    const wheel = (event: WheelEvent) => {
      const coordinates = point(event.clientX, event.clientY);
      if (!coordinates) return;
      event.preventDefault();
      const vertical = browserWheelDelta(event.deltaY, event.deltaMode, element.clientHeight);
      const horizontal = browserWheelDelta(event.deltaX, event.deltaMode, element.clientWidth);
      if (vertical) sendPointerAction(coordinates, { kind: 'mouse_wheel', delta: vertical });
      if (horizontal) sendPointerAction(coordinates, { kind: 'mouse_horizontal_wheel', delta: horizontal });
    };
    // A passive root wheel listener cannot prevent the controller page scrolling.
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, [pointerEnabled, point, sendPointerAction]);
  const combinedPointerButtons = () => {
    let buttons = 0;
    for (const pointer of pointers.current.values()) buttons |= pointer.buttons;
    return buttons;
  };
  const setPointerButtons = (pointerId: number, buttons: number, coordinates: { x: number; y: number } | null) => {
    const previous = pointers.current.get(pointerId);
    const position = coordinates ?? previous?.coordinates;
    if (!position) return false;
    const before = combinedPointerButtons();
    if (buttons) pointers.current.set(pointerId, { buttons, coordinates: position });
    else pointers.current.delete(pointerId);
    const after = combinedPointerButtons();
    // Touch contacts share the native left button; release only its last owner.
    for (const [button, mask] of pointerButtonMasks) {
      if ((before & mask) !== (after & mask)) sendPointerAction(position, { kind: 'mouse_button', button, pressed: Boolean(after & mask) });
    }
    return before !== after;
  };
  const endPointerCapture = (target: HTMLDivElement, pointerId: number) => {
    if (!target.hasPointerCapture || target.hasPointerCapture(pointerId)) target.releasePointerCapture?.(pointerId);
  };
  const pointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!pointerEnabled) return;
    const buttons = event.buttons & 31; const coordinates = point(event.clientX, event.clientY);
    if (!buttons || !coordinates) return;
    event.preventDefault(); event.currentTarget.focus();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    if (!setPointerButtons(event.pointerId, buttons, coordinates)) send({ kind: 'mouse_move', ...coordinates });
  };
  const pointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!pointerEnabled) return;
    const coordinates = point(event.clientX, event.clientY);
    const previous = pointers.current.get(event.pointerId);
    let edge = false;
    if (previous) {
      const buttons = event.buttons & 31;
      edge = setPointerButtons(event.pointerId, buttons, coordinates);
      if (!buttons) endPointerCapture(event.currentTarget, event.pointerId);
      event.preventDefault();
    }
    if (coordinates && !edge) { event.preventDefault(); send({ kind: 'mouse_move', ...coordinates }); }
  };
  const pointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!pointerEnabled || !pointers.current.has(event.pointerId)) return;
    event.preventDefault();
    const buttons = event.buttons & 31;
    setPointerButtons(event.pointerId, buttons, point(event.clientX, event.clientY));
    if (!buttons) endPointerCapture(event.currentTarget, event.pointerId);
  };
  const cancelPointer = (event: ReactPointerEvent<HTMLDivElement>, reason: string) => {
    const previous = pointers.current.get(event.pointerId);
    if (!previous) return;
    if (pointers.current.size === 1) release(reason);
    else setPointerButtons(event.pointerId, 0, previous.coordinates);
    endPointerCapture(event.currentTarget, event.pointerId);
  };
  const key = (event: ReactKeyboardEvent<HTMLDivElement>, pressed: boolean) => {
    if (!keyboardEnabled) return;
    const code = browserVirtualKey(event); if (code === null) return;
    event.preventDefault();
    const physicalKey = event.code || event.key;
    if (pressed) {
      if (event.repeat || keys.current.has(physicalKey)) return;
      const alreadyHeld = Array.from(keys.current.values()).includes(code);
      keys.current.set(physicalKey, code);
      if (alreadyHeld) return;
    } else {
      if (!keys.current.delete(physicalKey) || Array.from(keys.current.values()).includes(code)) return;
    }
    send({ kind: 'key', key: { kind: 'virtual_key', code }, pressed });
    if (pressed && event.key === 'Escape') leaveButton.current?.focus();
  };
  const leave = async () => {
    if (closePending.current) return;
    if (!handle.current) { navigate('/devices'); return; }
    closePending.current = true;
    const generation = sessionGeneration.current;
    const current = () => alive.current && sessionGeneration.current === generation;
    setClosing(true); setActionError(null);
    try {
      await close('leave_page');
      if (current()) navigate('/devices');
    } catch (error) {
      if (current()) setActionError(`本地控制已停止，远端结束请求失败：${message(error)}。无法确认远端已停止。`);
    } finally {
      if (current()) { closePending.current = false; setClosing(false); }
    }
  };
  const toggleFullscreen = async () => {
    const generation = sessionGeneration.current;
    const current = () => alive.current && sessionGeneration.current === generation;
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else if (surface.current?.requestFullscreen) await surface.current.requestFullscreen();
      else throw new Error('当前浏览器不支持全屏');
      if (current()) setActionError(null);
    } catch (error) { if (current()) setActionError(message(error)); }
  };
  const label = ready ? '画面已连接' : phaseLabels[state.phase];

  return <main className="flex min-h-dvh flex-col bg-slate-950 text-slate-100">
    <header className="flex flex-wrap items-center gap-3 border-b border-white/10 px-4 py-3">
      <button ref={leaveButton} type="button" onClick={() => void leave()} disabled={closing} aria-label={closing ? '正在结束会话' : inactive(state.phase) ? '返回设备列表' : ready ? '断开并返回设备列表' : '取消连接'} className="flex min-h-10 items-center gap-2 rounded-lg px-3 text-sm hover:bg-white/10 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-cyan-400"><ArrowLeft className="h-4 w-4" />{closing ? '正在结束会话' : inactive(state.phase) ? '返回设备列表' : ready ? '断开连接' : '取消连接'}</button>
      <div className="order-first min-w-0 basis-full sm:order-none sm:basis-auto sm:flex-1"><h1 className="flex items-center gap-2 whitespace-nowrap text-base font-semibold"><Monitor className="h-5 w-5 shrink-0 text-cyan-400" />网页远程桌面</h1><p className="mt-1 break-all text-xs text-slate-400">{id}</p></div>
      <span role="status" className={`rounded-full px-3 py-1 text-sm ${ready ? 'bg-emerald-500/15 text-emerald-300' : 'bg-white/10 text-slate-300'}`}>{label}</span>
      <button type="button" onClick={() => void toggleFullscreen()} disabled={!firstFrame} aria-label={fullscreen ? '退出全屏' : '进入全屏'} className="flex min-h-10 items-center gap-2 rounded-lg border border-white/15 px-3 text-sm hover:bg-white/10 disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-cyan-400">{fullscreen ? <Minimize2 className="h-4 w-4" /> : <Maximize2 className="h-4 w-4" />}全屏</button>
    </header>
    {(state.error || actionError) && <p role="alert" className="break-words border-b border-red-500/20 bg-red-500/10 px-4 py-3 text-sm text-red-200">{state.error || actionError}</p>}
    <div className="flex flex-1 flex-col gap-4 p-3 sm:p-5">
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2 text-xs text-slate-400"><span className="flex items-center gap-1.5"><ShieldCheck className="h-4 w-4 text-cyan-400" />远端同意后才会显示画面；权限由远端授权。</span><span>鼠标：{pointerEnabled ? '已授权' : '未启用'}</span><span>键盘：{keyboardEnabled ? '已授权' : '未启用'}</span>{state.route && <span>链路：{state.route === 'relay' ? '中继' : '直连'}</span>}</div>
      <div ref={surface} role="application" aria-label="远程桌面键鼠控制" aria-describedby="browser-input-help" aria-disabled={!pointerEnabled && !keyboardEnabled} tabIndex={0} className="relative flex min-h-[240px] w-full flex-1 items-center justify-center overflow-hidden rounded-xl border border-white/10 bg-black outline-none focus-visible:border-cyan-400 focus-visible:ring-2 focus-visible:ring-cyan-400/40" style={{ touchAction: pointerEnabled ? 'none' : 'auto' }}
        onPointerDown={pointerDown} onPointerUp={pointerUp} onPointerMove={pointerMove}
        onPointerCancel={event => cancelPointer(event, 'pointer_cancel')} onLostPointerCapture={event => cancelPointer(event, 'pointer_capture_lost')}
        onContextMenu={event => { if (pointerEnabled) event.preventDefault(); }}
        onKeyDown={event => key(event, true)} onKeyUp={event => key(event, false)} onBlur={() => release('surface_blur')}>
        <video ref={video} aria-label="远端屏幕" autoPlay playsInline muted onLoadedData={markReady} onPlaying={markReady} className="absolute inset-0 h-full w-full object-contain" />
        {!firstFrame && <div className="pointer-events-none relative max-w-md px-6 py-12 text-center"><Monitor className="mx-auto mb-4 h-10 w-10 text-slate-600" /><p className="text-sm text-slate-300">{state.phase === 'waiting_consent' ? '请在远端设备上确认本次连接请求' : state.phase === 'negotiating' || state.phase === 'streaming' ? '正在等待远端的首帧画面' : '当前没有远端画面'}</p></div>}
      </div>
      <footer className="flex flex-wrap items-center justify-between gap-3"><p id="browser-input-help" className="text-xs text-slate-400">点击画面或按 Tab 聚焦后控制键鼠；Esc 退出控制焦点。离开页面会停止本地控制，并请求结束本次会话。</p><div className="flex flex-wrap gap-2">{['音频', '文件传输', '无人值守', '远程应用'].map(feature => <button key={feature} type="button" disabled aria-label={`${feature}（暂不支持）`} className="rounded-md border border-white/10 px-2.5 py-1.5 text-xs text-slate-600">{feature} · 暂不支持</button>)}</div></footer>
    </div>
  </main>;
}
