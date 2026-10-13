import { TEMPORARY_PASSWORD_PATTERN } from '../utils/temporaryPassword';
import { useEffect, useId, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { ArrowRight, Eye, EyeOff, Loader2 } from 'lucide-react';
import { launchRemoteDisplayForDevice } from '../services/remoteDisplayLauncher';
import { closeBrowserRemoteSession } from '../services/browserRemoteSessionService';
import { DEVICE_CODE_INPUT_ERROR, parseRemoteDeviceInput } from '../utils/deviceCode';
import type { MediaProfile } from '../adapters/tauri/types';

const DEFAULT_PICTURE_PRESET = { value: '1080p30', label: '标准（1080p · 30帧）', profile: { width: 1920, height: 1080, fps: 30, bitrate_mbps: 10, codec: 'h264' } };
const PICTURE_PRESETS = [
  DEFAULT_PICTURE_PRESET,
  { value: '1080p60', label: '流畅（1080p · 60帧）', profile: { width: 1920, height: 1080, fps: 60, bitrate_mbps: 20, codec: 'h264' } },
  { value: '1440p120', label: '高性能（1440p · 120帧）', profile: { width: 2560, height: 1440, fps: 120, bitrate_mbps: 40, codec: 'h264' } },
  { value: '1080p60-low', label: '省流流畅（1080p · 60帧 · 4Mbps）', profile: { width: 1920, height: 1080, fps: 60, bitrate_mbps: 4, codec: 'h264' } },
  { value: '1080p30-low', label: '省流（1080p · 30帧 · 4Mbps）', profile: { width: 1920, height: 1080, fps: 30, bitrate_mbps: 4, codec: 'h264' } },
] satisfies Array<{ value: string; label: string; profile: MediaProfile }>;

export function GuestRemoteConnectForm() {
  const navigate = useNavigate();
  const passwordId = useId();
  const [deviceCode, setDeviceCode] = useState('');
  const [password, setPassword] = useState('');
  const [picturePreset, setPicturePreset] = useState(DEFAULT_PICTURE_PRESET);
  const [routePreference, setRoutePreference] = useState<'auto' | 'wan_relay'>('auto');
  const [accessMode, setAccessMode] = useState<'attended' | 'paired'>('attended');
  const [revealed, setRevealed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const inFlight = useRef<AbortController | null>(null);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; inFlight.current?.abort(); };
  }, []);

  const connect = async () => {
    if (inFlight.current) return;
    const target = parseRemoteDeviceInput(deviceCode);
    if (!target) { setError(DEVICE_CODE_INPUT_ERROR); return; }
    if (!TEMPORARY_PASSWORD_PATTERN.test(password)) { setError('请输入远端设备显示的 8 位临时密码（大写字母和 2–9 数字，不含 I/O）'); return; }
    const cancellation = new AbortController();
    inFlight.current = cancellation;
    setBusy(true); setError(null); setPassword(''); setRevealed(false);
    try {
      const result = await launchRemoteDisplayForDevice(target.deviceId, {
        temporaryPassword: password, signal: cancellation.signal,
        transportKind: 'webrtc', routePreference, targetDeviceName: '远程设备',
        requestedProfile: { ...picturePreset.profile },
        accessMode,
      });
      if (!alive.current || cancellation.signal.aborted) {
        await closeBrowserRemoteSession(result.sessionId, 'connection_form_left'); return;
      }
      inFlight.current = null;
      navigate(result.routePath ?? `/browser-session/${encodeURIComponent(result.sessionId)}`);
    } catch (cause) {
      if (alive.current && !cancellation.signal.aborted) setError(cause instanceof Error ? cause.message : '临时密码连接失败');
    } finally {
      if (inFlight.current === cancellation) inFlight.current = null;
      if (alive.current) setBusy(false);
    }
  };

  return <form className="space-y-3" aria-label="免登录临时密码连接" onSubmit={event => { event.preventDefault(); void connect(); }}>
    <label className="block text-sm">远程设备码
      <input aria-label="远程设备码" placeholder="输入 10 位设备码" autoComplete="off" spellCheck={false} value={deviceCode}
        onChange={event => setDeviceCode(event.target.value)} disabled={busy} className="mt-1.5 w-full rounded-lg border border-current/20 bg-transparent px-3 py-2.5 outline-none focus:ring-2 focus:ring-blue-500/30" />
    </label>
    <div className="text-sm"><label htmlFor={passwordId}>远端临时密码</label>
      <span className="relative mt-1.5 block">
        <input id={passwordId} aria-label="远端临时密码" type={revealed ? 'text' : 'password'} placeholder="输入 8 位临时密码" autoComplete="off" autoCapitalize="characters" spellCheck={false}
          value={password} onChange={event => setPassword(event.target.value)} disabled={busy} maxLength={8}
          className="w-full rounded-lg border border-current/20 bg-transparent px-3 py-2.5 pr-12 outline-none focus:ring-2 focus:ring-blue-500/30" />
        <button type="button" onClick={() => setRevealed(value => !value)} disabled={busy} aria-label={revealed ? '隐藏输入密码' : '显示输入密码'} className="absolute inset-y-0 right-0 px-3 opacity-60 hover:opacity-100">
          {revealed ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
        </button>
      </span>
    </div>
    <label className="block text-sm">画面档位
      <select aria-label="画面档位" value={picturePreset.value} disabled={busy}
        onChange={event => { const next = PICTURE_PRESETS.find(preset => preset.value === event.target.value); if (next) setPicturePreset(next); }}
        className="mt-1.5 w-full rounded-lg border border-current/20 bg-transparent px-3 py-2.5 outline-none focus:ring-2 focus:ring-blue-500/30">
        {PICTURE_PRESETS.map(preset => <option key={preset.value} value={preset.value}>{preset.label}</option>)}
      </select>
    </label>
    <label className="block text-sm">连接路线
      <select aria-label="连接路线" value={routePreference} disabled={busy}
        onChange={event => { const next = event.target.value; if (next === 'auto' || next === 'wan_relay') setRoutePreference(next); }}
        className="mt-1.5 w-full rounded-lg border border-current/20 bg-transparent px-3 py-2.5">
        <option value="auto">自动（优先直连）</option><option value="wan_relay">仅中继</option>
      </select>
    </label>
    <label className="block text-sm">连接授权方式
      <select aria-label="连接授权方式" value={accessMode} disabled={busy} onChange={event => setAccessMode(event.target.value as 'attended' | 'paired')}
        className="mt-1.5 w-full rounded-lg border border-current/20 bg-transparent px-3 py-2.5">
        <option value="attended">首次连接或仅允许本次</option><option value="paired">已配对，免重复确认</option>
      </select>
    </label>
    <p className="text-xs leading-relaxed opacity-65">首次连接请在远端配对并记住所选权限，之后可选择已配对连接。每次仍需有效临时密码；密码刷新后需重新本机确认和配对。已有的 9 位设备码仍可使用。</p>
    <button type="submit" disabled={!deviceCode.trim() || !password || busy} className="flex w-full items-center justify-center gap-2 rounded-lg bg-blue-600 px-3 py-2.5 text-sm text-white hover:bg-blue-500 disabled:opacity-40">
      {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <ArrowRight className="h-4 w-4" />}{busy ? '正在验证…' : '立即连接'}
    </button>
    {error && <p role="alert" className="break-words rounded-lg bg-red-500/10 px-3 py-2 text-xs text-red-500">{error}</p>}
  </form>;
}
