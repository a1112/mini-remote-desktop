import { TEMPORARY_PASSWORD_PATTERN } from '../utils/temporaryPassword';
import { useEffect, useId, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { ArrowRight, Eye, EyeOff, Loader2 } from 'lucide-react';
import { launchRemoteDisplayForDevice } from '../services/remoteDisplayLauncher';
import { closeBrowserRemoteSession } from '../services/browserRemoteSessionService';
import { DEVICE_CODE_INPUT_ERROR, parseRemoteDeviceInput } from '../utils/deviceCode';

export function GuestRemoteConnectForm() {
  const navigate = useNavigate();
  const passwordId = useId();
  const [deviceCode, setDeviceCode] = useState('');
  const [password, setPassword] = useState('');
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
        transportKind: 'webrtc', routePreference: 'auto', targetDeviceName: '远程设备',
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
    <p className="text-xs leading-relaxed opacity-65">双方无需登录。输入设备码和临时密码后，请在远端确认画面与键鼠权限；已有的 9 位设备码仍可使用。</p>
    <button type="submit" disabled={!deviceCode.trim() || !password || busy} className="flex w-full items-center justify-center gap-2 rounded-lg bg-blue-600 px-3 py-2.5 text-sm text-white hover:bg-blue-500 disabled:opacity-40">
      {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <ArrowRight className="h-4 w-4" />}{busy ? '正在验证…' : '立即连接'}
    </button>
    {error && <p role="alert" className="break-words rounded-lg bg-red-500/10 px-3 py-2 text-xs text-red-500">{error}</p>}
  </form>;
}
